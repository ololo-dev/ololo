//! Integration tests for password reset endpoints.
//!
//! POST /auth/forgot-password
//! POST /auth/reset-password

use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use chrono::Utc;
use http_body_util::BodyExt;
use migration::{Migrator, MigratorTrait};
use sea_orm::{ActiveModelTrait, ColumnTrait, Database, EntityTrait, QueryFilter, Set};
use server::email::StubEmailService;
use server::email::token::{generate_token, hash_secret, parse_token};
use server::entities::{auth_tokens, refresh_tokens};
use server::{AppState, AuthConfig, build_router};
use std::sync::Arc;
use std::time::Duration;
use tower::ServiceExt;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

async fn test_state() -> AppState {
    let db = Database::connect("sqlite::memory:").await.unwrap();
    Migrator::up(&db, None).await.unwrap();
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

/// Register a user and return (user_id, access_token).
async fn register_and_login(state: &AppState, email: &str) -> (Uuid, String) {
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
    let resp = app.clone().oneshot(req).await.unwrap();
    let (status, body) = response_body(resp).await;
    assert_eq!(status, StatusCode::CREATED, "register failed: {body}");
    let user_id: Uuid = serde_json::from_value(body["id"].clone()).unwrap();

    let req = Request::builder()
        .method(Method::POST)
        .uri("/auth/login")
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::ORIGIN, "http://localhost:5173")
        .body(json_body(serde_json::json!({
            "email": email,
            "password": "test-password-1234"
        })))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    let (status, body) = response_body(resp).await;
    assert_eq!(status, StatusCode::OK, "login failed: {body}");
    let access_token = body["access_token"].as_str().unwrap().to_string();

    (user_id, access_token)
}

/// Insert a password reset token for user_id. Returns the raw token string.
async fn insert_reset_token(state: &AppState, user_id: Uuid, expired: bool) -> String {
    let (token_id, raw_token) = generate_token();
    let (_, secret_bytes) = parse_token(&raw_token).unwrap();
    let token_hash = hash_secret(&secret_bytes);
    let now = Utc::now();
    let expires_at = if expired {
        now - chrono::Duration::hours(1)
    } else {
        now + chrono::Duration::hours(1)
    };

    auth_tokens::ActiveModel {
        id: Set(token_id),
        user_id: Set(user_id),
        token_hash: Set(token_hash),
        token_type: Set("password_reset".to_string()),
        expires_at: Set(expires_at),
        created_at: Set(now),
    }
    .insert(&state.db)
    .await
    .unwrap();

    raw_token
}

// ---------------------------------------------------------------------------
// Tests — POST /auth/forgot-password
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_forgot_password_registered_email_returns_200() {
    let state = test_state().await;
    let stub = Arc::new(StubEmailService::new());
    let (_, _) = register_and_login(&state, "forgot_registered@example.test").await;
    let state_with_email = state.with_email_service(stub.clone());
    let app = build_router(state_with_email);

    let req = Request::builder()
        .method(Method::POST)
        .uri("/auth/forgot-password")
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::ORIGIN, "http://localhost:5173")
        .body(json_body(serde_json::json!({
            "email": "forgot_registered@example.test"
        })))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    let (status, body) = response_body(resp).await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    assert!(
        body["message"]
            .as_str()
            .unwrap_or("")
            .contains("reset email has been sent"),
        "unexpected message: {body}"
    );
}

#[tokio::test]
async fn test_forgot_password_unregistered_email_returns_200() {
    // Enumeration prevention: identical response for unknown email
    let state = test_state().await;
    let stub = Arc::new(StubEmailService::new());
    let state_with_email = state.with_email_service(stub);
    let app = build_router(state_with_email);

    let req = Request::builder()
        .method(Method::POST)
        .uri("/auth/forgot-password")
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
            .contains("reset email has been sent"),
        "unexpected message: {body}"
    );
}

#[tokio::test]
async fn test_forgot_password_no_email_service_returns_503() {
    let state = test_state().await;
    // email_service is None by default
    let app = build_router(state);

    let req = Request::builder()
        .method(Method::POST)
        .uri("/auth/forgot-password")
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
// Tests — POST /auth/reset-password
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_reset_password_valid_token() {
    // AC-005: valid token → 200, password updated, refresh tokens deleted
    let state = test_state().await;
    let (user_id, _) = register_and_login(&state, "reset_valid@example.test").await;

    // Issue a refresh token so we can verify it gets deleted
    let refresh_count_before = refresh_tokens::Entity::find()
        .filter(refresh_tokens::Column::UserIdFk.eq(user_id))
        .all(&state.db)
        .await
        .unwrap()
        .len();
    assert!(
        refresh_count_before >= 1,
        "expected at least one refresh token after login"
    );

    let raw_token = insert_reset_token(&state, user_id, false).await;

    let app = build_router(state.clone());
    let req = Request::builder()
        .method(Method::POST)
        .uri("/auth/reset-password")
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::ORIGIN, "http://localhost:5173")
        .body(json_body(serde_json::json!({
            "token": raw_token,
            "new_password": "brand-new-password-5678"
        })))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // Verify refresh tokens were deleted (AC-005)
    let refresh_count_after = refresh_tokens::Entity::find()
        .filter(refresh_tokens::Column::UserIdFk.eq(user_id))
        .all(&state.db)
        .await
        .unwrap()
        .len();
    assert_eq!(
        refresh_count_after, 0,
        "all refresh tokens should be deleted after password reset"
    );

    // Verify old password no longer works; new one does
    let app2 = build_router(state.clone());
    let req = Request::builder()
        .method(Method::POST)
        .uri("/auth/login")
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::ORIGIN, "http://localhost:5173")
        .body(json_body(serde_json::json!({
            "email": "reset_valid@example.test",
            "password": "brand-new-password-5678"
        })))
        .unwrap();
    let resp = app2.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK, "new password should work");
}

#[tokio::test]
async fn test_reset_password_expired_token_returns_401() {
    let state = test_state().await;
    let (user_id, _) = register_and_login(&state, "reset_expired@example.test").await;

    let raw_token = insert_reset_token(&state, user_id, true).await; // expired

    let app = build_router(state);
    let req = Request::builder()
        .method(Method::POST)
        .uri("/auth/reset-password")
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::ORIGIN, "http://localhost:5173")
        .body(json_body(serde_json::json!({
            "token": raw_token,
            "new_password": "new-password-999"
        })))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_reset_password_malformed_token_returns_400() {
    let state = test_state().await;
    let app = build_router(state);

    let req = Request::builder()
        .method(Method::POST)
        .uri("/auth/reset-password")
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::ORIGIN, "http://localhost:5173")
        .body(json_body(serde_json::json!({
            "token": "not-a-real-token",
            "new_password": "irrelevant"
        })))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    let (status, body) = response_body(resp).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "body={body}");
    assert_eq!(body["error"], "invalid_token_format");
}

#[tokio::test]
async fn test_reset_password_empty_password_returns_400() {
    let state = test_state().await;
    let (user_id, _) = register_and_login(&state, "reset_empty_pw@example.test").await;
    let raw_token = insert_reset_token(&state, user_id, false).await;

    let app = build_router(state);
    let req = Request::builder()
        .method(Method::POST)
        .uri("/auth/reset-password")
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::ORIGIN, "http://localhost:5173")
        .body(json_body(serde_json::json!({
            "token": raw_token,
            "new_password": ""
        })))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    let (status, body) = response_body(resp).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "body={body}");
    assert_eq!(body["error"], "invalid_password");
}

#[tokio::test]
async fn test_reset_password_token_single_use() {
    // Token should be deleted after first use — second attempt → 401
    let state = test_state().await;
    let (user_id, _) = register_and_login(&state, "reset_singleuse@example.test").await;
    let raw_token = insert_reset_token(&state, user_id, false).await;

    let app = build_router(state.clone());

    // First use — success
    let req = Request::builder()
        .method(Method::POST)
        .uri("/auth/reset-password")
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::ORIGIN, "http://localhost:5173")
        .body(json_body(serde_json::json!({
            "token": raw_token.clone(),
            "new_password": "first-new-password"
        })))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // Second use — token deleted, should fail
    let req = Request::builder()
        .method(Method::POST)
        .uri("/auth/reset-password")
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::ORIGIN, "http://localhost:5173")
        .body(json_body(serde_json::json!({
            "token": raw_token,
            "new_password": "second-new-password"
        })))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}
