use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Header, encode};
use serde::{Deserialize, Serialize};
use server::auth::jwt::{AccessClaims, PURPOSE_ACCESS, issue_access_token, verify_access_token};
use std::time::Duration;
use uuid::Uuid;

fn keys() -> (EncodingKey, DecodingKey) {
    let secret = b"integration-test-secret-32-bytes-or-more-xxxxxxx";
    (
        EncodingKey::from_secret(secret),
        DecodingKey::from_secret(secret),
    )
}

#[test]
fn issue_then_verify_roundtrip() {
    let (e, d) = keys();
    let uid = Uuid::new_v4();
    let token = issue_access_token(&e, uid, "u@example.com", Duration::from_secs(60)).unwrap();
    let claims = verify_access_token(&d, &token).unwrap();
    assert_eq!(claims.sub, uid.to_string());
    assert_eq!(claims.email, "u@example.com");
    assert_eq!(claims.purpose, PURPOSE_ACCESS);
}

#[derive(Serialize, Deserialize)]
struct OAuthStateClaims {
    sub: String,
    email: String,
    iat: i64,
    exp: i64,
    purpose: String,
}

#[test]
fn reject_purpose_oauth_state() {
    let (e, d) = keys();
    let now = chrono::Utc::now().timestamp();
    let bad = OAuthStateClaims {
        sub: Uuid::new_v4().to_string(),
        email: "x@y.test".into(),
        iat: now,
        exp: now + 60,
        purpose: "oauth_state".into(),
    };
    let tok = encode(&Header::new(Algorithm::HS256), &bad, &e).unwrap();
    let err = verify_access_token(&d, &tok).unwrap_err();
    let msg = format!("{err:?}");
    assert!(
        msg.contains("WrongPurpose"),
        "expected WrongPurpose, got {msg}"
    );
}

#[test]
fn reject_expired() {
    let (e, d) = keys();
    let claims = AccessClaims {
        sub: Uuid::new_v4().to_string(),
        email: "x@y.test".into(),
        iat: 1_000_000_000,
        exp: 1_000_000_060,
        purpose: PURPOSE_ACCESS.into(),
    };
    let tok = encode(&Header::new(Algorithm::HS256), &claims, &e).unwrap();
    let err = verify_access_token(&d, &tok).unwrap_err();
    let msg = format!("{err:?}");
    assert!(msg.contains("Expired"), "expected Expired, got {msg}");
}
