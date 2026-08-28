//! HS256 access JWT issue + verify per FR-002.
//!
//! Claims:
//! - `sub`: user id (UUID string)
//! - `email`: user email
//! - `iat`: issued-at (epoch seconds)
//! - `exp`: expiry (epoch seconds)
//! - `purpose`: literal `"access"`. Tokens with any other purpose are
//!   rejected by `verify_access_token`.

use crate::auth::AuthError;
use chrono::Utc;
use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Header, Validation, decode, encode};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use uuid::Uuid;

/// Required value for the `purpose` claim on an access token.
pub const PURPOSE_ACCESS: &str = "access";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AccessClaims {
    pub sub: String,
    pub email: String,
    pub iat: i64,
    pub exp: i64,
    pub purpose: String,
}

impl AccessClaims {
    /// The authenticated user's id (the `sub` claim parsed as a UUID).
    pub fn user_id(&self) -> Result<Uuid, uuid::Error> {
        Uuid::parse_str(&self.sub)
    }
}

/// Issue an access JWT for `user_id` / `email` valid for `ttl`.
pub fn issue_access_token(
    key: &EncodingKey,
    user_id: Uuid,
    email: &str,
    ttl: Duration,
) -> Result<String, AuthError> {
    let now = Utc::now().timestamp();
    let claims = AccessClaims {
        sub: user_id.to_string(),
        email: email.to_string(),
        iat: now,
        exp: now + ttl.as_secs() as i64,
        purpose: PURPOSE_ACCESS.to_string(),
    };
    encode(&Header::new(Algorithm::HS256), &claims, key).map_err(|_| AuthError::TokenIssue)
}

/// Verify an access JWT. Rejects on signature failure, expiry, or any
/// `purpose` other than `"access"`.
pub fn verify_access_token(key: &DecodingKey, token: &str) -> Result<AccessClaims, AuthError> {
    let mut validation = Validation::new(Algorithm::HS256);
    validation.set_required_spec_claims(&["exp", "iat", "sub"]);
    let data = decode::<AccessClaims>(token, key, &validation).map_err(|e| match e.kind() {
        jsonwebtoken::errors::ErrorKind::ExpiredSignature => AuthError::TokenExpired,
        _ => AuthError::TokenInvalid,
    })?;
    if data.claims.purpose != PURPOSE_ACCESS {
        return Err(AuthError::TokenWrongPurpose);
    }
    Ok(data.claims)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys() -> (EncodingKey, DecodingKey) {
        let secret = b"test-secret-at-least-32-bytes-long-xxxxxxxx";
        (
            EncodingKey::from_secret(secret),
            DecodingKey::from_secret(secret),
        )
    }

    #[test]
    fn issue_then_verify_roundtrip() {
        let (e, d) = keys();
        let uid = Uuid::new_v4();
        let token = issue_access_token(&e, uid, "a@b.test", Duration::from_secs(60)).unwrap();
        let claims = verify_access_token(&d, &token).unwrap();
        assert_eq!(claims.sub, uid.to_string());
        assert_eq!(claims.email, "a@b.test");
        assert_eq!(claims.purpose, PURPOSE_ACCESS);
    }

    #[test]
    fn reject_purpose_oauth_state() {
        let (e, d) = keys();
        // Hand-craft a token with purpose="oauth_state".
        let now = Utc::now().timestamp();
        let claims = AccessClaims {
            sub: Uuid::new_v4().to_string(),
            email: "x@y.test".into(),
            iat: now,
            exp: now + 60,
            purpose: "oauth_state".into(),
        };
        let token = encode(&Header::new(Algorithm::HS256), &claims, &e).unwrap();
        let err = verify_access_token(&d, &token).unwrap_err();
        assert!(matches!(err, AuthError::TokenWrongPurpose));
    }

    #[test]
    fn reject_expired() {
        let (e, d) = keys();
        // iat in the past, exp in the past — expired token.
        let claims = AccessClaims {
            sub: Uuid::new_v4().to_string(),
            email: "x@y.test".into(),
            iat: 1_000_000_000,
            exp: 1_000_000_060,
            purpose: PURPOSE_ACCESS.into(),
        };
        let token = encode(&Header::new(Algorithm::HS256), &claims, &e).unwrap();
        let err = verify_access_token(&d, &token).unwrap_err();
        assert!(matches!(err, AuthError::TokenExpired));
    }

    #[test]
    fn reject_bad_signature() {
        let (e, _d) = keys();
        let other = DecodingKey::from_secret(b"different-secret-xxxxxxxxxxxxxxxxxxxx");
        let token = issue_access_token(&e, Uuid::new_v4(), "a@b", Duration::from_secs(60)).unwrap();
        let err = verify_access_token(&other, &token).unwrap_err();
        assert!(matches!(err, AuthError::TokenInvalid));
    }
}
