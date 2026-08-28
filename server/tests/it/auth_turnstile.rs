//! Integration tests for Cloudflare Turnstile CAPTCHA on auth endpoints.
//!
//! Covers all 7 captcha states per AC: disabled (unchanged), enabled+valid,
//! enabled+invalid (403 captcha_invalid), enabled+None (403 captcha_required),
//! enabled+error+fail-open (proceeds), enabled+error+fail-closed (403 captcha_error),
//! unconfigured-but-enabled (403 captcha_required + WARN).

use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use http_body_util::BodyExt;
use migration::{Migrator, MigratorTrait};
use sea_orm::Database;
use server::auth::turnstile::{FakeTurnstileVerifier, TurnstileConfig, VerifyOutcome};
use server::{AppState, AuthConfig, build_router};
use std::sync::Arc;
use std::time::Duration;
use tower::ServiceExt;

fn base_cfg() -> AuthConfig {
    AuthConfig {
        jwt_signing_key: b"integration-test-secret-32-bytes-or-more-xxxxxxx".to_vec(),
        frontend_origins: vec!["http://localhost:5173".into()],
        access_ttl: Duration::from_secs(900),
        refresh_ttl: Duration::from_secs(30 * 86_400),
        max_agents_per_session: 16,
    }
}

async fn test_state_with_turnstile(
    enabled: bool,
    secret: Option<&str>,
    fail_open: bool,
    outcome: VerifyOutcome,
) -> AppState {
    let db = Database::connect("sqlite::memory:").await.unwrap();
    Migrator::up(&db, None).await.unwrap();
    let state = AppState::new(db, base_cfg());
    let cfg = TurnstileConfig {
        enabled,
        sitekey: Some("test-sitekey".to_string()),
        secret: secret.map(|s| s.to_string()),
        fail_open,
        verify_timeout_ms: 3000,
    };
    let verifier: Arc<dyn server::auth::turnstile::TurnstileVerifier> =
        Arc::new(FakeTurnstileVerifier { outcome });
    state.with_turnstile(cfg, verifier)
}

fn json_body(v: serde_json::Value) -> Body {
    Body::from(serde_json::to_vec(&v).unwrap())
}

async fn read_body(resp: axum::response::Response) -> (StatusCode, serde_json::Value) {
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let value: serde_json::Value = if bytes.is_empty() {
        serde_json::Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null)
    };
    (status, value)
}

fn register_req(token: Option<&str>) -> Request<Body> {
    let mut body = serde_json::json!({
        "email": "bob@example.test",
        "password": "secret-password-1",
    });
    if let Some(t) = token {
        body["turnstile_token"] = serde_json::Value::String(t.to_string());
    }
    Request::builder()
        .method(Method::POST)
        .uri("/auth/register")
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::ORIGIN, "http://localhost:5173")
        .body(json_body(body))
        .unwrap()
}

fn login_req(token: Option<&str>) -> Request<Body> {
    let mut body = serde_json::json!({
        "email": "bob@example.test",
        "password": "secret-password-1",
    });
    if let Some(t) = token {
        body["turnstile_token"] = serde_json::Value::String(t.to_string());
    }
    Request::builder()
        .method(Method::POST)
        .uri("/auth/login")
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::ORIGIN, "http://localhost:5173")
        .body(json_body(body))
        .unwrap()
}

fn forgot_req(token: Option<&str>) -> Request<Body> {
    let mut body = serde_json::json!({
        "email": "bob@example.test",
    });
    if let Some(t) = token {
        body["turnstile_token"] = serde_json::Value::String(t.to_string());
    }
    Request::builder()
        .method(Method::POST)
        .uri("/auth/forgot-password")
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::ORIGIN, "http://localhost:5173")
        .body(json_body(body))
        .unwrap()
}

async fn assert_captcha_error(resp: axum::response::Response, expected_code: &str) {
    let (status, body) = read_body(resp).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "body={body}");
    assert_eq!(
        body["error"], expected_code,
        "expected error code {expected_code}, got body={body}"
    );
}

#[tokio::test]
async fn disabled_skips_verification() {
    let state =
        test_state_with_turnstile(false, Some("secret"), true, VerifyOutcome::Invalid).await;
    let app = build_router(state);
    // Register without token — should succeed (Turnstile disabled).
    let resp = app.oneshot(register_req(None)).await.unwrap();
    let (status, _body) = read_body(resp).await;
    assert_eq!(status, StatusCode::CREATED);
}

#[tokio::test]
async fn enabled_valid_token_passes() {
    let state = test_state_with_turnstile(true, Some("secret"), true, VerifyOutcome::Valid).await;
    let app = build_router(state);
    let resp = app
        .oneshot(register_req(Some("valid-token")))
        .await
        .unwrap();
    let (status, _body) = read_body(resp).await;
    assert_eq!(status, StatusCode::CREATED);
}

#[tokio::test]
async fn enabled_invalid_token_rejected() {
    let state = test_state_with_turnstile(true, Some("secret"), true, VerifyOutcome::Invalid).await;
    let app = build_router(state);
    let resp = app.oneshot(register_req(Some("bad-token"))).await.unwrap();
    assert_captcha_error(resp, "captcha_invalid").await;
}

#[tokio::test]
async fn enabled_none_token_rejected() {
    let state = test_state_with_turnstile(true, Some("secret"), true, VerifyOutcome::Valid).await;
    let app = build_router(state);
    let resp = app.oneshot(register_req(None)).await.unwrap();
    assert_captcha_error(resp, "captcha_required").await;
}

#[tokio::test]
async fn enabled_error_fail_open_proceeds() {
    let state = test_state_with_turnstile(true, Some("secret"), true, VerifyOutcome::Error).await;
    let app = build_router(state);
    let resp = app.oneshot(register_req(Some("any-token"))).await.unwrap();
    let (status, _body) = read_body(resp).await;
    assert_eq!(status, StatusCode::CREATED);
}

#[tokio::test]
async fn enabled_error_fail_closed_rejected() {
    let state = test_state_with_turnstile(true, Some("secret"), false, VerifyOutcome::Error).await;
    let app = build_router(state);
    let resp = app.oneshot(register_req(Some("any-token"))).await.unwrap();
    assert_captcha_error(resp, "captcha_error").await;
}

#[tokio::test]
async fn enabled_no_secret_fails_closed() {
    let state = test_state_with_turnstile(true, None, true, VerifyOutcome::Valid).await;
    let app = build_router(state);
    let resp = app.oneshot(register_req(Some("any-token"))).await.unwrap();
    assert_captcha_error(resp, "captcha_required").await;
}

#[tokio::test]
async fn login_enabled_none_token_rejected() {
    let state = test_state_with_turnstile(true, Some("secret"), true, VerifyOutcome::Valid).await;
    let app = build_router(state);
    let resp = app.oneshot(login_req(None)).await.unwrap();
    assert_captcha_error(resp, "captcha_required").await;
}

#[tokio::test]
async fn login_enabled_valid_token_passes() {
    let state = test_state_with_turnstile(true, Some("secret"), true, VerifyOutcome::Valid).await;
    let app = build_router(state);
    // First register so login can find the user.
    let _ = app
        .clone()
        .oneshot(register_req(Some("valid-token")))
        .await
        .unwrap();
    let resp = app.oneshot(login_req(Some("valid-token"))).await.unwrap();
    let (status, _body) = read_body(resp).await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn forgot_password_enabled_none_token_rejected() {
    let state = test_state_with_turnstile(true, Some("secret"), true, VerifyOutcome::Valid).await;
    let app = build_router(state);
    let resp = app.oneshot(forgot_req(None)).await.unwrap();
    assert_captcha_error(resp, "captcha_required").await;
}

#[tokio::test]
async fn public_turnstile_config_returns_enabled_and_sitekey() {
    let state = test_state_with_turnstile(true, Some("secret"), true, VerifyOutcome::Valid).await;
    let app = build_router(state);
    let req = Request::builder()
        .method(Method::GET)
        .uri("/api/public/turnstile")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    let (status, body) = read_body(resp).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["enabled"], true);
    assert_eq!(body["sitekey"], "test-sitekey");
}

#[tokio::test]
async fn public_turnstile_config_when_disabled() {
    let db = Database::connect("sqlite::memory:").await.unwrap();
    Migrator::up(&db, None).await.unwrap();
    let state = AppState::new(db, base_cfg());
    let cfg = TurnstileConfig {
        enabled: false,
        sitekey: None,
        secret: None,
        fail_open: true,
        verify_timeout_ms: 3000,
    };
    let verifier: Arc<dyn server::auth::turnstile::TurnstileVerifier> =
        Arc::new(FakeTurnstileVerifier {
            outcome: VerifyOutcome::Valid,
        });
    let state = state.with_turnstile(cfg, verifier);
    let app = build_router(state);
    let req = Request::builder()
        .method(Method::GET)
        .uri("/api/public/turnstile")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    let (status, body) = read_body(resp).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["enabled"], false);
    assert!(body["sitekey"].is_null());
}
