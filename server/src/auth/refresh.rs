//! Refresh-token issuance, verification, and rotation per FR-002 / FR-007.
//!
//! On issue: generate 32 random bytes via `OsRng`, hex-encode (the "secret"),
//! argon2-hash the hex string, store the hash in `refresh_tokens.hash` keyed
//! by a fresh row-id UUID. Cookie value format is `<row_id>.<secret>`.
//!
//! On verify-and-rotate: parse cookie, look up row by id, argon2-verify the
//! secret against the stored hash, check `revoked_at IS NULL` and that
//! `expires_at` is in the future, then mark the old row revoked and issue a
//! fresh token in one transaction.

use crate::auth::AuthError;
use crate::auth::password::{hash_password, verify_password};
use arena_core::entities::refresh_tokens;
use argon2::Argon2;
use chrono::{Duration as ChronoDuration, Utc};
use rand::RngCore;
use rand::rngs::OsRng;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, DatabaseConnection, EntityTrait, QueryFilter,
    Set, TransactionTrait,
};
use std::time::Duration;
use uuid::Uuid;

/// Issue a fresh refresh token for `user_id`. Returns the cookie-format
/// `<row_id>.<secret>` string. Caller is responsible for setting cookie
/// attributes.
pub async fn issue_refresh_token<C: ConnectionTrait>(
    conn: &C,
    argon2: &Argon2<'_>,
    user_id: Uuid,
    ttl: Duration,
) -> Result<String, AuthError> {
    let row_id = Uuid::new_v4();
    let mut secret_bytes = [0u8; 32];
    OsRng.fill_bytes(&mut secret_bytes);
    let secret = hex::encode(secret_bytes);
    let hash = hash_password(argon2, &secret)?;
    let now = Utc::now();
    let expires_at = now + ChronoDuration::from_std(ttl).map_err(|_| AuthError::Internal)?;
    let row = refresh_tokens::ActiveModel {
        id: Set(row_id),
        user_id_fk: Set(user_id),
        hash: Set(hash),
        expires_at: Set(expires_at),
        revoked_at: Set(None),
        created_at: Set(now),
    };
    row.insert(conn).await?;
    Ok(format!("{}.{}", row_id, secret))
}

/// How long after rotation a token may be re-presented without being treated
/// as theft.
///
/// Rotation is not serialised across the clients that hold the same cookie: the
/// SvelteKit SSR hook, the browser's proactive refresh timer, a parallel 401
/// retry and a second tab can all POST `/auth/refresh` within the same instant.
/// Exactly one wins the rotation; the losers then present a token that was
/// revoked milliseconds ago. Nuking the family there logs the user out at
/// random, so a replay this fresh is read as that race and served a new token
/// instead. Outside the window a replay is still treated as a compromise.
/// Possession of the secret is required either way — this widens nothing for a
/// caller who does not already hold a legitimately-issued token.
const REUSE_GRACE_SECS: i64 = 30;

/// Verify a refresh-token cookie value and rotate it.
pub async fn verify_and_rotate(
    db: &DatabaseConnection,
    argon2: &Argon2<'_>,
    cookie_value: &str,
    ttl: Duration,
) -> Result<(Uuid, String), AuthError> {
    verify_and_rotate_any(db, argon2, std::slice::from_ref(&cookie_value), ttl).await
}

/// Verify any of the presented refresh-token cookie values and rotate the first
/// usable one. Returns the user id and the freshly-issued cookie value; the
/// rotated row is marked `revoked_at`.
///
/// Browsers can send several values under the same cookie name (see
/// [`crate::auth::middleware::extract_cookie_all`]), so every candidate is
/// tried before any conclusion is drawn: a live token anywhere in the list
/// wins, and only when none is live does the replay path below apply.
pub async fn verify_and_rotate_any(
    db: &DatabaseConnection,
    argon2: &Argon2<'_>,
    cookie_values: &[&str],
    ttl: Duration,
) -> Result<(Uuid, String), AuthError> {
    let txn = db.begin().await?;
    let now = Utc::now();

    // Most recently revoked row whose secret verified — the replay candidate,
    // only consulted when no live token was found.
    let mut replayed: Option<refresh_tokens::Model> = None;
    let mut saw_expired = false;

    for value in cookie_values {
        let Ok((row_id, secret)) = parse_cookie_value(value) else {
            continue;
        };
        let Some(row) = refresh_tokens::Entity::find_by_id(row_id).one(&txn).await? else {
            continue;
        };
        // Verify the secret FIRST, so only the holder of the real token can
        // trigger any state change below (a row-id-only request must not be
        // able to revoke a victim's session family).
        if !verify_password(argon2, &row.hash, secret)? {
            continue;
        }
        if row.revoked_at.is_some() {
            let newer = replayed
                .as_ref()
                .is_none_or(|best| row.revoked_at > best.revoked_at);
            if newer {
                replayed = Some(row);
            }
            continue;
        }
        if row.expires_at <= now {
            saw_expired = true;
            continue;
        }

        // Live token: rotate it. Revoking the old row and issuing the
        // replacement share one transaction, so a crash cannot revoke without
        // re-issuing (or vice versa).
        let user_id = row.user_id_fk;
        let mut active: refresh_tokens::ActiveModel = row.into();
        active.revoked_at = Set(Some(now));
        active.update(&txn).await?;
        let new_cookie = issue_refresh_token(&txn, argon2, user_id, ttl).await?;
        txn.commit().await?;
        return Ok((user_id, new_cookie));
    }

    // No live token. A verified-but-revoked one is either a benign rotation
    // race (fresh) or a replayed stolen token (stale).
    if let Some(row) = replayed {
        let user_id = row.user_id_fk;
        let revoked_at = row.revoked_at.unwrap_or(now);
        // A rotation race leaves the winner's token live; a revoked family (or a
        // logout) leaves nothing live. Requiring a live sibling keeps the grace
        // window from resurrecting a session that reuse detection just killed —
        // those rows are also "freshly revoked".
        let has_live_sibling = refresh_tokens::Entity::find()
            .filter(refresh_tokens::Column::UserIdFk.eq(user_id))
            .filter(refresh_tokens::Column::RevokedAt.is_null())
            .filter(refresh_tokens::Column::ExpiresAt.gt(now))
            .one(&txn)
            .await?
            .is_some();
        if has_live_sibling && (now - revoked_at).num_seconds() <= REUSE_GRACE_SECS {
            tracing::debug!(%user_id, "refresh: rotation race inside the grace window, re-issuing");
            let new_cookie = issue_refresh_token(&txn, argon2, user_id, ttl).await?;
            txn.commit().await?;
            return Ok((user_id, new_cookie));
        }
        tracing::warn!(%user_id, "refresh: token reuse detected, revoking the family");
        revoke_all_for_user(&txn, user_id).await?;
        txn.commit().await?;
        return Err(AuthError::RefreshRevoked);
    }

    if saw_expired {
        return Err(AuthError::RefreshRevoked);
    }
    Err(AuthError::RefreshInvalid)
}

/// Revoke the refresh-token row identified by `cookie_value` if it exists.
/// Idempotent: silently succeeds if the row is already revoked or absent.
pub async fn revoke(db: &DatabaseConnection, cookie_value: &str) -> Result<(), AuthError> {
    let (row_id, _) = match parse_cookie_value(cookie_value) {
        Ok(v) => v,
        Err(_) => return Ok(()),
    };
    if let Some(row) = refresh_tokens::Entity::find_by_id(row_id).one(db).await?
        && row.revoked_at.is_none()
    {
        let mut active: refresh_tokens::ActiveModel = row.into();
        active.revoked_at = Set(Some(Utc::now()));
        active.update(db).await?;
    }
    Ok(())
}

/// Revoke every active refresh token for a user (used on logout-all paths).
pub async fn revoke_all_for_user<C: ConnectionTrait>(
    conn: &C,
    user_id: Uuid,
) -> Result<(), AuthError> {
    let active = refresh_tokens::Entity::find()
        .filter(refresh_tokens::Column::UserIdFk.eq(user_id))
        .filter(refresh_tokens::Column::RevokedAt.is_null())
        .all(conn)
        .await?;
    let now = Utc::now();
    for row in active {
        let mut a: refresh_tokens::ActiveModel = row.into();
        a.revoked_at = Set(Some(now));
        a.update(conn).await?;
    }
    Ok(())
}

fn parse_cookie_value(value: &str) -> Result<(Uuid, &str), AuthError> {
    let (id_str, secret) = value.split_once('.').ok_or(AuthError::RefreshInvalid)?;
    let row_id = Uuid::parse_str(id_str).map_err(|_| AuthError::RefreshInvalid)?;
    if secret.is_empty() {
        return Err(AuthError::RefreshInvalid);
    }
    Ok((row_id, secret))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_valid_cookie() {
        let id = Uuid::new_v4();
        let v = format!("{}.deadbeef", id);
        let (parsed_id, secret) = parse_cookie_value(&v).unwrap();
        assert_eq!(parsed_id, id);
        assert_eq!(secret, "deadbeef");
    }

    #[test]
    fn parse_rejects_no_dot() {
        assert!(parse_cookie_value("nodot").is_err());
    }

    #[test]
    fn parse_rejects_bad_uuid() {
        assert!(parse_cookie_value("not-a-uuid.secret").is_err());
    }

    #[test]
    fn parse_rejects_empty_secret() {
        let id = Uuid::new_v4();
        assert!(parse_cookie_value(&format!("{}.", id)).is_err());
    }
}
