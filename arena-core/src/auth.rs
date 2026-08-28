//! Shared personal-access-token (PAT) verification.
//!
//! PATs are opaque `ololo_<64 hex>` strings; only their SHA-256 hash is
//! stored (`cli_tokens.token_hash`). Both the main server (REST, resolve,
//! git-http) and the game server (player-agent WS) authenticate CLI callers
//! against the same table, so the lookup lives here rather than in either
//! binary.

use chrono::Utc;
use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use uuid::Uuid;

use crate::entities::cli_tokens;

#[derive(Debug, thiserror::Error)]
pub enum PatError {
    #[error("token invalid")]
    Invalid,
    #[error("token expired")]
    Expired,
}

/// SHA-256 hash of the PAT string bytes, returned as 64-char lowercase hex.
pub fn hash_pat(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}

/// HTTP header carrying the service-to-service internal-API token
/// (`server` → `game-server` `/internal/*` calls).
pub const INTERNAL_API_HEADER: &str = "x-internal-auth";

/// Derive the shared internal-API token from the JWT signing secret.
///
/// The `/internal/*` endpoints on the game server (judge-run, judge-logs) are
/// on the same public listener as the player WS, so they need service-to-service
/// auth. Both processes already share `JWT_SIGNING_KEY`, so we derive a stable
/// bearer token from it rather than introducing a second coordinated secret.
/// The derivation is a domain-separated SHA-256, so the raw signing key never
/// travels on the wire — a leaked `X-Internal-Auth` header does not expose it.
pub fn internal_api_token(signing_key: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"ololo-internal-api-v1\x00");
    hasher.update(signing_key);
    hex::encode(hasher.finalize())
}

/// Constant-time check of a presented internal-API token against the one
/// derived from `signing_key`.
pub fn verify_internal_api_token(signing_key: &[u8], presented: &str) -> bool {
    let expected = internal_api_token(signing_key);
    expected.as_bytes().ct_eq(presented.as_bytes()).unwrap_u8() == 1
}

/// Verify a presented PAT against `cli_tokens` and return the owning user id.
///
/// Hashes the token, finds the row, checks expiry, and does a constant-time
/// hash compare (defence-in-depth). DB errors surface as `Invalid` so callers
/// answer 401 uniformly without leaking lookup internals.
pub async fn lookup_pat_user(
    db: &DatabaseConnection,
    presented_token: &str,
) -> Result<Uuid, PatError> {
    let presented_hash = hash_pat(presented_token);

    let row = cli_tokens::Entity::find()
        .filter(cli_tokens::Column::TokenHash.eq(&presented_hash))
        .one(db)
        .await
        .map_err(|_| PatError::Invalid)?
        .ok_or(PatError::Invalid)?;

    if Utc::now() > row.expires_at {
        return Err(PatError::Expired);
    }

    if row
        .token_hash
        .as_bytes()
        .ct_eq(presented_hash.as_bytes())
        .unwrap_u8()
        != 1
    {
        return Err(PatError::Invalid);
    }

    Ok(row.user_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use migration::MigratorTrait;
    use sea_orm::{ActiveModelTrait, Set};

    async fn setup_db() -> DatabaseConnection {
        let db = sea_orm::Database::connect("sqlite::memory:")
            .await
            .expect("connect");
        migration::Migrator::up(&db, None).await.expect("migrate");
        db
    }

    async fn insert_user(db: &DatabaseConnection) -> Uuid {
        crate::entities::users::ActiveModel {
            id: Set(Uuid::new_v4()),
            email: Set(format!("u{}@example.com", Uuid::new_v4())),
            password_hash: Set(None),
            display_name: Set("tester".to_string()),
            created_at: Set(Utc::now()),
            updated_at: Set(Utc::now()),
            is_admin: Set(false),
            avatar_url: Set(None),
            email_verified: Set(false),
            username: Set(None),
            plan: Set(crate::quota::PLAN_PREMIUM.to_string()),
            judge_run_limit: Set(None),
            judge_run_credits: Set(0),
        }
        .insert(db)
        .await
        .expect("insert user")
        .id
    }

    async fn insert_token(
        db: &DatabaseConnection,
        user_id: Uuid,
        token: &str,
        expires_in_secs: i64,
    ) {
        cli_tokens::ActiveModel {
            id: Set(Uuid::new_v4()),
            token_hash: Set(hash_pat(token)),
            user_id: Set(user_id),
            created_at: Set(Utc::now().into()),
            expires_at: Set((Utc::now() + chrono::Duration::seconds(expires_in_secs)).into()),
        }
        .insert(db)
        .await
        .expect("insert cli token");
    }

    #[tokio::test]
    async fn valid_pat_resolves_to_owner() {
        let db = setup_db().await;
        let user_id = insert_user(&db).await;
        insert_token(&db, user_id, "ololo_valid", 3600).await;

        let resolved = lookup_pat_user(&db, "ololo_valid").await.expect("lookup");
        assert_eq!(resolved, user_id);
    }

    #[tokio::test]
    async fn unknown_pat_is_invalid() {
        let db = setup_db().await;
        assert!(matches!(
            lookup_pat_user(&db, "ololo_nope").await,
            Err(PatError::Invalid)
        ));
    }

    #[tokio::test]
    async fn expired_pat_is_rejected() {
        let db = setup_db().await;
        let user_id = insert_user(&db).await;
        insert_token(&db, user_id, "ololo_stale", -60).await;

        assert!(matches!(
            lookup_pat_user(&db, "ololo_stale").await,
            Err(PatError::Expired)
        ));
    }

    #[test]
    fn internal_api_token_is_stable_and_verifies() {
        let key = b"a-signing-key-at-least-32-bytes-long!!";
        let token = internal_api_token(key);
        assert_eq!(token.len(), 64, "sha-256 hex");
        assert_eq!(token, internal_api_token(key), "deterministic");
        assert!(verify_internal_api_token(key, &token));
    }

    #[test]
    fn internal_api_token_rejects_wrong_key_or_value() {
        let key = b"a-signing-key-at-least-32-bytes-long!!";
        let other = b"a-different-signing-key-32-bytes-x!!!!";
        assert_ne!(internal_api_token(key), internal_api_token(other));
        assert!(!verify_internal_api_token(key, &internal_api_token(other)));
        assert!(!verify_internal_api_token(key, "not-a-token"));
        assert!(!verify_internal_api_token(key, ""));
        // The raw signing key must never itself be a valid token.
        assert!(!verify_internal_api_token(
            key,
            std::str::from_utf8(key).unwrap()
        ));
    }
}
