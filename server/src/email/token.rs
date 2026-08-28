//! Secure token utilities for email verification, password reset, and magic links.
//!
//! Token wire format: `{token_id_hex}.{secret_hex}`
//! - token_id_hex: lowercase hex of a UUID v4 (32 hex chars)
//! - secret_hex: lowercase hex of 32 random bytes (64 hex chars)
//!
//! Storage: `id` column stores token_id UUID; `token_hash` stores hex(SHA-256(secret)).

use rand::RngCore;
use rand::rngs::OsRng;
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use uuid::Uuid;

/// Error type for token parse failures. Maps to HTTP 400.
#[derive(Debug, thiserror::Error)]
pub enum TokenParseError {
    #[error("token must have exactly two dot-separated parts")]
    WrongPartCount,
    #[error("token_id part is not valid hex UUID")]
    InvalidTokenId,
    #[error("secret part is not valid 32-byte hex")]
    InvalidSecret,
}

/// Generate a new token.
///
/// Returns `(token_id, raw_token_string)` where:
/// - `token_id` is the UUID to store as the PK
/// - `raw_token_string` is the full `{token_id_hex}.{secret_hex}` to send to the user
pub fn generate_token() -> (Uuid, String) {
    let token_id = Uuid::new_v4();
    let mut secret = [0u8; 32];
    OsRng.fill_bytes(&mut secret);
    let raw = format!("{}.{}", token_id.simple(), hex::encode(secret));
    (token_id, raw)
}

/// Parse a raw token string into its components.
///
/// Returns `(token_id, secret_bytes)` on success, `TokenParseError` on failure.
pub fn parse_token(raw: &str) -> Result<(Uuid, [u8; 32]), TokenParseError> {
    let parts: Vec<&str> = raw.splitn(2, '.').collect();
    if parts.len() != 2 {
        return Err(TokenParseError::WrongPartCount);
    }

    // Parse token_id: decode 32 hex chars → 16 bytes → UUID
    let id_bytes = hex::decode(parts[0]).map_err(|_| TokenParseError::InvalidTokenId)?;
    if id_bytes.len() != 16 {
        return Err(TokenParseError::InvalidTokenId);
    }
    let token_id = Uuid::from_slice(&id_bytes).map_err(|_| TokenParseError::InvalidTokenId)?;

    // Parse secret: must be exactly 64 hex chars = 32 bytes
    let secret_bytes = hex::decode(parts[1]).map_err(|_| TokenParseError::InvalidSecret)?;
    if secret_bytes.len() != 32 {
        return Err(TokenParseError::InvalidSecret);
    }
    let mut secret = [0u8; 32];
    secret.copy_from_slice(&secret_bytes);

    Ok((token_id, secret))
}

/// Hash a token secret for storage.
///
/// Returns `hex(SHA-256(secret))`.
pub fn hash_secret(secret: &[u8; 32]) -> String {
    let digest = Sha256::digest(secret);
    hex::encode(digest)
}

/// Verify a submitted secret against a stored hash using constant-time comparison.
///
/// Returns `true` if the secret matches. Both operands are compared as 32-byte
/// SHA-256 digests using `subtle::ConstantTimeEq` (constant-time in-memory comparison only;
/// the DB lookup by PK is NOT constant-time by design per NFR-002).
///
/// Returns `false` immediately if the stored_hash_hex cannot be decoded to exactly 32 bytes.
pub fn verify_token(submitted_secret: &[u8; 32], stored_hash_hex: &str) -> bool {
    let stored_bytes = match hex::decode(stored_hash_hex) {
        Ok(b) if b.len() == 32 => b,
        _ => return false,
    };

    let submitted_hash = Sha256::digest(submitted_secret);
    submitted_hash.as_slice().ct_eq(&stored_bytes).into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generate_and_parse_roundtrip() {
        let (token_id, raw) = generate_token();
        let (parsed_id, parsed_secret) = parse_token(&raw).expect("should parse");
        assert_eq!(token_id, parsed_id);
        let hash = hash_secret(&parsed_secret);
        assert!(verify_token(&parsed_secret, &hash));
    }

    #[test]
    fn parse_fails_no_dot() {
        assert!(parse_token("nodothere").is_err());
    }

    #[test]
    fn parse_fails_non_hex_secret() {
        let (_, raw) = generate_token();
        let parts: Vec<&str> = raw.splitn(2, '.').collect();
        let bad = format!("{}.zzzz", parts[0]);
        assert!(parse_token(&bad).is_err());
    }

    #[test]
    fn parse_fails_short_secret() {
        let (_, raw) = generate_token();
        let parts: Vec<&str> = raw.splitn(2, '.').collect();
        let bad = format!("{}.deadbeef", parts[0]); // only 4 bytes
        assert!(parse_token(&bad).is_err());
    }

    #[test]
    fn verify_wrong_secret_returns_false() {
        let wrong = [0u8; 32];
        let (_id, raw) = generate_token();
        let (_id2, secret) = parse_token(&raw).unwrap();
        let hash = hash_secret(&secret);
        assert!(!verify_token(&wrong, &hash));
    }

    #[test]
    fn unique_tokens_have_distinct_nonces() {
        let (_, raw1) = generate_token();
        let (_, raw2) = generate_token();
        assert_ne!(raw1, raw2);
    }
}
