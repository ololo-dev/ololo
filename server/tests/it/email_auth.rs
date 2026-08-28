//! Integration tests for email verification endpoints.
//!
//! GET  /auth/verify-email?token=<raw>
//! POST /auth/verify-email/resend

use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use chrono::Utc;
use http_body_util::BodyExt;
use migration::{Migrator, MigratorTrait};
use sea_orm::{ActiveModelTrait, Database, Set};
use server::email::StubEmailService;
use server::email::token::{generate_token, hash_secret, parse_token};
use server::entities::{auth_tokens, users};
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

    // Register
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

    // Login to get access token
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

/// Insert a verification token for user_id. Returns the raw token string.
async fn insert_verification_token(state: &AppState, user_id: Uuid, expired: bool) -> String {
    let (token_id, raw_token) = generate_token();
    let (_, secret_bytes) = parse_token(&raw_token).unwrap();
    let token_hash = hash_secret(&secret_bytes);
    let now = Utc::now();
    let expires_at = if expired {
        now - chrono::Duration::hours(1)
    } else {
        now + chrono::Duration::hours(24)
    };

    auth_tokens::ActiveModel {
        id: Set(token_id),
        user_id: Set(user_id),
        token_hash: Set(token_hash),
        token_type: Set("email_verification".to_string()),
        expires_at: Set(expires_at),
        created_at: Set(now),
    }
    .insert(&state.db)
    .await
    .unwrap();

    raw_token
}

/// Set email_verified = true for a user directly in DB.
async fn mark_email_verified(state: &AppState, user_id: Uuid) {
    use sea_orm::EntityTrait;
    let user = users::Entity::find_by_id(user_id)
        .one(&state.db)
        .await
        .unwrap()
        .unwrap();
    let mut active: users::ActiveModel = user.into();
    active.email_verified = Set(true);
    active.updated_at = Set(Utc::now());
    active.update(&state.db).await.unwrap();
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_verify_email_valid_token() {
    let state = test_state().await;
    let (user_id, _) = register_and_login(&state, "verify_valid@example.test").await;

    let raw_token = insert_verification_token(&state, user_id, false).await;

    let app = build_router(state.clone());
    let req = Request::builder()
        .method(Method::GET)
        .uri(format!("/auth/verify-email?token={}", raw_token))
        .header(header::ORIGIN, "http://localhost:5173")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::FOUND);
    assert_eq!(
        resp.headers()
            .get(header::LOCATION)
            .and_then(|v| v.to_str().ok()),
        Some("/?emailVerified=1"),
        "success must redirect to the site with the verified flag"
    );

    // Confirm user.email_verified is now true
    use sea_orm::EntityTrait;
    let user = users::Entity::find_by_id(user_id)
        .one(&state.db)
        .await
        .unwrap()
        .unwrap();
    assert!(
        user.email_verified,
        "email_verified should be true after verification"
    );
}

#[tokio::test]
async fn test_verify_email_expired_token() {
    let state = test_state().await;
    let (user_id, _) = register_and_login(&state, "verify_expired@example.test").await;

    let raw_token = insert_verification_token(&state, user_id, true).await;

    let app = build_router(state.clone());
    let req = Request::builder()
        .method(Method::GET)
        .uri(format!("/auth/verify-email?token={}", raw_token))
        .header(header::ORIGIN, "http://localhost:5173")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::FOUND);
    assert_eq!(
        resp.headers()
            .get(header::LOCATION)
            .and_then(|v| v.to_str().ok()),
        Some("/?emailVerified=0")
    );
}

#[tokio::test]
async fn test_verify_email_malformed_token() {
    let state = test_state().await;
    let app = build_router(state);

    let req = Request::builder()
        .method(Method::GET)
        .uri("/auth/verify-email?token=garbage-not-a-real-token")
        .header(header::ORIGIN, "http://localhost:5173")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::FOUND);
    assert_eq!(
        resp.headers()
            .get(header::LOCATION)
            .and_then(|v| v.to_str().ok()),
        Some("/?emailVerified=0")
    );
}

#[tokio::test]
async fn test_verify_email_duplicate_use() {
    let state = test_state().await;
    let (user_id, _) = register_and_login(&state, "verify_dup@example.test").await;

    let raw_token = insert_verification_token(&state, user_id, false).await;

    let app = build_router(state.clone());

    // First use — should succeed
    let req = Request::builder()
        .method(Method::GET)
        .uri(format!("/auth/verify-email?token={}", raw_token))
        .header(header::ORIGIN, "http://localhost:5173")
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::FOUND);
    assert_eq!(
        resp.headers()
            .get(header::LOCATION)
            .and_then(|v| v.to_str().ok()),
        Some("/?emailVerified=1")
    );

    // Second use — token deleted, must land on the failure redirect
    let req = Request::builder()
        .method(Method::GET)
        .uri(format!("/auth/verify-email?token={}", raw_token))
        .header(header::ORIGIN, "http://localhost:5173")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::FOUND);
    assert_eq!(
        resp.headers()
            .get(header::LOCATION)
            .and_then(|v| v.to_str().ok()),
        Some("/?emailVerified=0")
    );
}

#[tokio::test]
async fn test_resend_already_verified() {
    let state = test_state().await;
    let (user_id, access_token) = register_and_login(&state, "resend_verified@example.test").await;

    mark_email_verified(&state, user_id).await;

    let state_with_email = state.with_email_service(Arc::new(StubEmailService::new()));
    let app = build_router(state_with_email);

    let req = Request::builder()
        .method(Method::POST)
        .uri("/auth/verify-email/resend")
        .header(header::ORIGIN, "http://localhost:5173")
        .header(header::AUTHORIZATION, format!("Bearer {}", access_token))
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    let (status, body) = response_body(resp).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "body={body}");
    assert_eq!(body["error"], "already_verified");
}

#[tokio::test]
async fn test_resend_no_email_service() {
    let state = test_state().await;
    let (_, access_token) = register_and_login(&state, "resend_no_svc@example.test").await;

    // state.email_service is None by default
    let app = build_router(state);

    let req = Request::builder()
        .method(Method::POST)
        .uri("/auth/verify-email/resend")
        .header(header::ORIGIN, "http://localhost:5173")
        .header(header::AUTHORIZATION, format!("Bearer {}", access_token))
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
}

/// Resend must still send when the email_templates rows are absent — the
/// compiled-in builtin template is the fallback.
#[tokio::test]
async fn test_resend_falls_back_to_builtin_template() {
    use sea_orm::EntityTrait;
    use server::entities::email_templates;

    let state = test_state().await;
    let (_, access_token) = register_and_login(&state, "resend_builtin@example.test").await;

    email_templates::Entity::delete_many()
        .exec(&state.db)
        .await
        .unwrap();

    let stub = Arc::new(StubEmailService::new());
    let app = build_router(state.with_email_service(stub.clone()));

    let req = Request::builder()
        .method(Method::POST)
        .uri("/auth/verify-email/resend")
        .header(header::ORIGIN, "http://localhost:5173")
        .header(header::AUTHORIZATION, format!("Bearer {}", access_token))
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let sent = stub.sent().await;
    assert_eq!(sent.len(), 1, "exactly one email must be sent");
    assert!(
        sent[0].body_html.contains("/auth/verify-email?token="),
        "verify link missing from body: {}",
        sent[0].body_html
    );
}

/// A failing provider must surface as an error, not a silent 200.
#[tokio::test]
async fn test_resend_send_failure_returns_502() {
    use server::email::{EmailError, EmailService};

    struct FailingEmailService;

    #[axum::async_trait]
    impl EmailService for FailingEmailService {
        async fn send_email(
            &self,
            _to: &str,
            _subject: &str,
            _body_html: &str,
            _body_text: &str,
        ) -> Result<(), EmailError> {
            Err(EmailError::SendFailure("provider down".into()))
        }
    }

    let state = test_state().await;
    let (_, access_token) = register_and_login(&state, "resend_fail@example.test").await;
    let app = build_router(state.with_email_service(Arc::new(FailingEmailService)));

    let req = Request::builder()
        .method(Method::POST)
        .uri("/auth/verify-email/resend")
        .header(header::ORIGIN, "http://localhost:5173")
        .header(header::AUTHORIZATION, format!("Bearer {}", access_token))
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    let (status, body) = response_body(resp).await;
    assert_eq!(status, StatusCode::BAD_GATEWAY, "body={body}");
    assert_eq!(body["error"], "email_send_failed");
}
