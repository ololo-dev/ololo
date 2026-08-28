use crate::auth::AuthError;
use crate::auth::jwt::{AccessClaims, PURPOSE_ACCESS};
use arena_core::auth::PatError;
use chrono::Utc;
use rand::RngCore;
use sea_orm::DatabaseConnection;

/// SHA-256 hash of the PAT string bytes, returned as 64-char lowercase hex.
/// Shared with the game server via arena-core (both validate the same tokens).
pub use arena_core::auth::hash_pat;

/// Generate a PAT: `ololo_` prefix + 64 lowercase hex chars (32 CSPRNG bytes).
pub fn generate_pat() -> String {
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    format!("ololo_{}", hex::encode(bytes))
}

/// Validate that a token has the `ololo_` prefix followed by exactly 64 lowercase hex chars.
pub fn is_valid_pat_format(token: &str) -> bool {
    token.strip_prefix("ololo_").is_some_and(|hex_part| {
        hex_part.len() == 64
            && hex_part
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    })
}

/// Look up a presented PAT in the database.
/// Delegates hash/expiry/constant-time checks to `arena_core::auth`, returns AccessClaims.
pub async fn lookup_pat(
    db: &DatabaseConnection,
    presented_token: &str,
) -> Result<AccessClaims, AuthError> {
    let user_id = arena_core::auth::lookup_pat_user(db, presented_token)
        .await
        .map_err(|e| match e {
            PatError::Expired => AuthError::TokenExpired,
            PatError::Invalid => AuthError::TokenInvalid,
        })?;

    Ok(AccessClaims {
        sub: user_id.to_string(),
        // PATs are not JWTs — email is not stored in the token row.
        // Callers that need the email must join against the users table.
        email: String::new(),
        iat: Utc::now().timestamp(),
        // Expiry is enforced above via the expires_at column; use sentinel max.
        exp: i64::MAX,
        purpose: PURPOSE_ACCESS.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generate_pat_has_correct_prefix_and_length() {
        let pat = generate_pat();
        assert!(pat.starts_with("ololo_"), "PAT must start with ololo_");
        assert_eq!(pat.len(), 6 + 64, "PAT must be ololo_ + 64 hex chars");
    }

    #[test]
    fn generate_pat_hex_suffix_is_lowercase() {
        let pat = generate_pat();
        let hex_part = &pat[6..];
        assert!(
            hex_part
                .chars()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
        );
    }

    #[test]
    fn hash_pat_produces_64_char_lowercase_hex() {
        let hash = hash_pat("ololo_test");
        assert_eq!(hash.len(), 64);
        assert!(
            hash.chars()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
        );
    }

    #[test]
    fn hash_pat_is_deterministic() {
        assert_eq!(hash_pat("ololo_abc"), hash_pat("ololo_abc"));
    }

    #[test]
    fn is_valid_pat_format_accepts_correct() {
        let pat = generate_pat();
        assert!(is_valid_pat_format(&pat));
    }

    #[test]
    fn is_valid_pat_format_rejects_wrong_prefix() {
        assert!(!is_valid_pat_format(
            "bearer_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        ));
    }

    #[test]
    fn is_valid_pat_format_rejects_short_hex() {
        assert!(!is_valid_pat_format("ololo_abc123"));
    }

    #[test]
    fn is_valid_pat_format_rejects_uppercase_hex() {
        let bad = format!("ololo_{}", "A".repeat(64));
        assert!(!is_valid_pat_format(&bad));
    }
}
