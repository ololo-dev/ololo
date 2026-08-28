//! Integration tests for magic-link endpoints.
//!
//! POST /auth/magic-link/request
//! GET  /auth/magic-link/verify

use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use chrono::Utc;
use http_body_util::BodyExt;
use migration::MigratorTrait;
use sea_orm::{ActiveModelTrait, Set};
use server::email::StubEmailService;
use server::email::token::{generate_token, hash_secret, parse_token};
use server::entities::auth_tokens;
use server::{AppState, AuthConfig, build_router};
use std::sync::Arc;
use std::time::Duration;
use tower::ServiceExt;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

async fn test_state() -> AppState {
    let db = sea_orm::Database::connect("sqlite::memory:").await.unwrap();
    migration::Migrator::up(&db, None).await.unwrap();
    let cfg = AuthConfig {
        jwt_signing_key: b"integration-test-secret-32-bytes-or-more-xxxxxxx".to_vec(),
        frontend_origins: vec!["http://localhost:5173".into()],
        access_ttl: Duration::from_secs(900),
        refresh_ttl: Duration::from_secs(30 * 86_400),
        max_agents_per_session: 16,
    };
    AppState::new(db, cfg)
}

fn json_body(v: serde_json::Value) -> Body {
    Body::from(serde_json::to_vec(&v).unwrap())
}

async fn response_body(resp: axum::response::Response) -> (StatusCode, serde_json::Value) {
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let value: serde_json::Value = if bytes.is_empty() {
        serde_json::Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null)
    };
    (status, value)
}

/// Register a user and return user_id.
async fn register_user(state: &AppState, email: &str) -> Uuid {
    let app = build_router(state.clone());
    let req = Request::builder()
        .method(Method::POST)
        .uri("/auth/register")
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::ORIGIN, "http://localhost:5173")
        .body(json_body(serde_json::json!({
            "email": email,
            "password": "test-password-1234"
        })))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    let (status, body) = response_body(resp).await;
    assert_eq!(status, StatusCode::CREATED, "register failed: {body}");
    serde_json::from_value(body["id"].clone()).unwrap()
}

/// Insert a magic-link token for user_id. Returns the raw token string.
async fn insert_magic_link_token(state: &AppState, user_id: Uuid, expired: bool) -> String {
    let (token_id, raw_token) = generate_token();
    let (_, secret_bytes) = parse_token(&raw_token).unwrap();
    let token_hash = hash_secret(&secret_bytes);
    let now = Utc::now();
    let expires_at = if expired {
        now - chrono::Duration::minutes(1)
    } else {
        now + chrono::Duration::minutes(15)
    };

    auth_tokens::ActiveModel {
        id: Set(token_id),
        user_id: Set(user_id),
        token_hash: Set(token_hash),
        token_type: Set("magic_link".to_string()),
        expires_at: Set(expires_at),
        created_at: Set(now),
    }
    .insert(&state.db)
    .await
    .unwrap();

    raw_token
}

// ---------------------------------------------------------------------------
// Tests — POST /auth/magic-link/request
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_magic_link_request_registered_email_returns_200() {
    let state = test_state().await;
    let stub = Arc::new(StubEmailService::new());
    register_user(&state, "magic_registered@example.test").await;
    let state_with_email = state.with_email_service(stub.clone());
    let app = build_router(state_with_email);

    let req = Request::builder()
        .method(Method::POST)
        .uri("/auth/magic-link/request")
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::ORIGIN, "http://localhost:5173")
        .body(json_body(serde_json::json!({
            "email": "magic_registered@example.test"
        })))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    let (status, body) = response_body(resp).await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    assert!(
        body["message"]
            .as_str()
            .unwrap_or("")
            .contains("sign-in link has been sent"),
        "unexpected message: {body}"
    );
}

#[tokio::test]
async fn test_magic_link_request_unregistered_email_returns_200() {
    // Enumeration prevention: identical response for unknown email
    let state = test_state().await;
    let stub = Arc::new(StubEmailService::new());
    let state_with_email = state.with_email_service(stub);
    let app = build_router(state_with_email);

    let req = Request::builder()
        .method(Method::POST)
        .uri("/auth/magic-link/request")
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::ORIGIN, "http://localhost:5173")
        .body(json_body(serde_json::json!({
            "email": "nobody@example.test"
        })))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    let (status, body) = response_body(resp).await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    assert!(
        body["message"]
            .as_str()
            .unwrap_or("")
            .contains("sign-in link has been sent"),
        "unexpected message: {body}"
    );
}

#[tokio::test]
async fn test_magic_link_request_no_email_service_returns_503() {
    let state = test_state().await;
    // email_service is None by default
    let app = build_router(state);

    let req = Request::builder()
        .method(Method::POST)
        .uri("/auth/magic-link/request")
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::ORIGIN, "http://localhost:5173")
        .body(json_body(serde_json::json!({
            "email": "anyone@example.test"
        })))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
}

// ---------------------------------------------------------------------------
// Tests — GET /auth/magic-link/verify
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_magic_link_verify_valid_token_sets_cookies_and_redirects() {
    let state = test_state().await;
    let user_id = register_user(&state, "magic_verify@example.test").await;
    let raw_token = insert_magic_link_token(&state, user_id, false).await;

    let app = build_router(state.clone());
    let req = Request::builder()
        .method(Method::GET)
        .uri(format!("/auth/magic-link/verify?token={}", raw_token))
        .header(header::ORIGIN, "http://localhost:5173")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();

    // Should be a redirect (302 Found)
    assert_eq!(resp.status(), StatusCode::FOUND, "expected redirect");

    // Should set arena_access and arena_refresh cookies
    let set_cookie_values: Vec<String> = resp
        .headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok().map(|s| s.to_string()))
        .collect();
    let has_access = set_cookie_values
        .iter()
        .any(|c| c.starts_with("arena_access="));
    let has_refresh = set_cookie_values
        .iter()
        .any(|c| c.starts_with("arena_refresh="));
    assert!(
        has_access,
        "arena_access cookie not set; cookies: {set_cookie_values:?}"
    );
    assert!(
        has_refresh,
        "arena_refresh cookie not set; cookies: {set_cookie_values:?}"
    );

    // Should redirect to / (no next param given)
    let location = resp
        .headers()
        .get(header::LOCATION)
        .and_then(|v| v.to_str().ok());
    assert_eq!(
        location,
        Some("/"),
        "expected redirect to /; got {location:?}"
    );
}

#[tokio::test]
async fn test_magic_link_verify_with_valid_next_path() {
    let state = test_state().await;
    let user_id = register_user(&state, "magic_next@example.test").await;
    let raw_token = insert_magic_link_token(&state, user_id, false).await;

    let app = build_router(state.clone());
    // Pass a full URL with the same host as PUBLIC_APP_URL (defaults to localhost:5173).
    // Percent-encode "://" manually so the URI string parses as a valid http URI.
    let next_encoded = "http%3A%2F%2Flocalhost%3A5173%2Fsessions";
    let req = Request::builder()
        .method(Method::GET)
        .uri(format!(
            "/auth/magic-link/verify?token={}&next={}",
            raw_token, next_encoded
        ))
        .header(header::ORIGIN, "http://localhost:5173")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::FOUND);
    let location = resp
        .headers()
        .get(header::LOCATION)
        .and_then(|v| v.to_str().ok());
    assert_eq!(
        location,
        Some("/sessions"),
        "expected /sessions; got {location:?}"
    );
}

#[tokio::test]
async fn test_magic_link_verify_expired_token_returns_401() {
    let state = test_state().await;
    let user_id = register_user(&state, "magic_expired@example.test").await;
    let raw_token = insert_magic_link_token(&state, user_id, true).await;

    let app = build_router(state);
    let req = Request::builder()
        .method(Method::GET)
        .uri(format!("/auth/magic-link/verify?token={}", raw_token))
        .header(header::ORIGIN, "http://localhost:5173")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_magic_link_verify_malformed_token_returns_400() {
    let state = test_state().await;
    let app = build_router(state);

    let req = Request::builder()
        .method(Method::GET)
        .uri("/auth/magic-link/verify?token=garbage-not-valid")
        .header(header::ORIGIN, "http://localhost:5173")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    let (status, body) = response_body(resp).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "body={body}");
    assert_eq!(body["error"], "invalid_token_format");
}

#[tokio::test]
async fn test_magic_link_verify_open_redirect_prevention() {
    // AC-015: external host, protocol-relative, tab-encoded path all → redirect to /
    let state = test_state().await;
    let user_id = register_user(&state, "magic_openredirect@example.test").await;

    // (bad_next_encoded, description)
    let cases: &[(&str, &str)] = &[
        // https://evil.com — percent-encoded so the URI is valid
        ("https%3A%2F%2Fevil.com", "https://evil.com"),
        // //evil.com — passed raw (valid query param value)
        ("%2F%2Fevil.com", "//evil.com"),
        // /%09/evil.com — relative path with tab character
        ("%2F%09%2Fevil.com", "/%09/evil.com"),
    ];

    for (encoded, description) in cases {
        let raw_token = insert_magic_link_token(&state, user_id, false).await;
        let app = build_router(state.clone());

        let req = Request::builder()
            .method(Method::GET)
            .uri(format!(
                "/auth/magic-link/verify?token={}&next={}",
                raw_token, encoded
            ))
            .header(header::ORIGIN, "http://localhost:5173")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::FOUND, "next={description}");
        let location = resp
            .headers()
            .get(header::LOCATION)
            .and_then(|v| v.to_str().ok());
        assert_eq!(
            location,
            Some("/"),
            "expected redirect to / for bad next={description}; got {location:?}"
        );
    }
}

#[tokio::test]
async fn test_magic_link_verify_single_use() {
    // Token should be deleted after first use — second attempt → 401
    let state = test_state().await;
    let user_id = register_user(&state, "magic_singleuse@example.test").await;
    let raw_token = insert_magic_link_token(&state, user_id, false).await;

    let app = build_router(state.clone());

    // First use — success
    let req = Request::builder()
        .method(Method::GET)
        .uri(format!("/auth/magic-link/verify?token={}", raw_token))
        .header(header::ORIGIN, "http://localhost:5173")
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::FOUND);

    // Second use — token deleted, should fail
    let req = Request::builder()
        .method(Method::GET)
        .uri(format!("/auth/magic-link/verify?token={}", raw_token))
        .header(header::ORIGIN, "http://localhost:5173")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_magic_link_verify_marks_email_verified() {
    use sea_orm::EntityTrait;
    use server::entities::users;

    let state = test_state().await;
    let user_id = register_user(&state, "magic_verifies_email@example.test").await;

    // Fresh registrations start unverified.
    let user = users::Entity::find_by_id(user_id)
        .one(&state.db)
        .await
        .unwrap()
        .unwrap();
    assert!(!user.email_verified, "precondition: user starts unverified");

    let raw_token = insert_magic_link_token(&state, user_id, false).await;
    let app = build_router(state.clone());
    let req = Request::builder()
        .method(Method::GET)
        .uri(format!("/auth/magic-link/verify?token={}", raw_token))
        .header(header::ORIGIN, "http://localhost:5173")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::FOUND);

    // Clicking the emailed link proves mailbox ownership.
    let user = users::Entity::find_by_id(user_id)
        .one(&state.db)
        .await
        .unwrap()
        .unwrap();
    assert!(
        user.email_verified,
        "magic-link login must mark the email verified"
    );
}
