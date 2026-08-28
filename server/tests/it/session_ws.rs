//! Session dashboard WebSocket pre-upgrade auth tests.
//!
//! Auth checks happen before the WebSocket upgrade: bearer extraction,
//! PAT lookup, join_code registry lookup, and JWT query-param auth all
//! return HTTP status codes (not WS frames). These tests exercise those
//! paths via plain HTTP oneshot calls so no real WS connection is required.
//!
//! Actual WS round-trip tests are deferred — marked `#[ignore]`.

use arena_core::session_status::SessionStatus;
use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use chrono::{Duration as ChronoDuration, Utc};
use http_body_util::BodyExt;
use jsonwebtoken::EncodingKey;
use migration::{Migrator, MigratorTrait};
use sea_orm::{ActiveModelTrait, Set};
use server::auth::jwt::issue_access_token;
use server::auth::pat::{generate_pat, hash_pat};
use server::entities::{cli_tokens, users};
use server::protocol::ArenaFrame;
use server::rate_limiter::NoOpRateLimiter;
use server::state::{SessionCacheInner, SessionEntry};
use server::{AppState, AuthConfig, build_router};
use std::sync::{Arc, RwLock};
use std::time::Duration;
use tokio::sync::broadcast;
use tower::ServiceExt;
use uuid::Uuid;

const ORIGIN: &str = "http://localhost:5173";
/// The JWT signing key used by `test_state()`.
const TEST_JWT_SECRET: &[u8] = b"integration-test-secret-32-bytes-or-more-xxxxxxx";

async fn test_state() -> AppState {
    let db = sea_orm::Database::connect("sqlite::memory:")
        .await
        .expect("sqlite connect");
    Migrator::up(&db, None).await.expect("migrate up");
    let cfg = AuthConfig {
        jwt_signing_key: b"integration-test-secret-32-bytes-or-more-xxxxxxx".to_vec(),
        frontend_origins: vec![ORIGIN.to_string()],
        access_ttl: Duration::from_secs(900),
        refresh_ttl: Duration::from_secs(30 * 86_400),
        max_agents_per_session: 16,
    };
    let mut state = AppState::new(db, cfg);
    state.rate_limiter = Arc::new(NoOpRateLimiter);
    state
}

async fn read_body(resp: axum::response::Response) -> (StatusCode, serde_json::Value) {
    let status = resp.status();
    let bytes = resp
        .into_body()
        .collect()
        .await
        .expect("collect body")
        .to_bytes();
    let value = if bytes.is_empty() {
        serde_json::Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null)
    };
    (status, value)
}

/// Insert a user row and a valid PAT into the DB.
/// Returns `(user_id, raw_token)`.
async fn insert_user_with_pat(state: &AppState) -> (Uuid, String) {
    let user_id = Uuid::new_v4();
    users::ActiveModel {
        id: Set(user_id),
        email: Set(format!("ws-test-{}@example.com", user_id)),
        password_hash: Set(None),
        display_name: Set("WS Test User".to_string()),
        created_at: Set(Utc::now()),
        updated_at: Set(Utc::now()),
        is_admin: Set(false),
        avatar_url: Set(None),
        email_verified: Set(false),
        username: Set(None),
        plan: Set(arena_core::quota::PLAN_PREMIUM.to_string()),
        judge_run_limit: Set(None),
        judge_run_credits: Set(0),
    }
    .insert(&state.db)
    .await
    .expect("insert user");

    let raw_token = generate_pat();
    let token_hash = hash_pat(&raw_token);
    cli_tokens::ActiveModel {
        id: Set(Uuid::new_v4()),
        token_hash: Set(token_hash),
        user_id: Set(user_id),
        created_at: Set(Utc::now().into()),
        expires_at: Set((Utc::now() + ChronoDuration::hours(1)).into()),
    }
    .insert(&state.db)
    .await
    .expect("insert cli_token");

    (user_id, raw_token)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn ws_session_rejects_missing_authorization_header() {
    let state = test_state().await;
    let app = build_router(state);

    let resp = app
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/ws/s/SOMECODE")
                .header(header::ORIGIN, ORIGIN)
                .body(Body::empty())
                .expect("build request"),
        )
        .await
        .expect("oneshot");

    let (status, body) = read_body(resp).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "expected 401: {body}");
    assert_eq!(body["error"].as_str().unwrap(), "unauthorized");
}

#[tokio::test]
async fn ws_session_rejects_invalid_bearer_token() {
    let state = test_state().await;
    let app = build_router(state);

    let resp = app
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/ws/s/SOMECODE")
                .header(header::ORIGIN, ORIGIN)
                .header(header::AUTHORIZATION, "Bearer not-a-real-pat")
                .body(Body::empty())
                .expect("build request"),
        )
        .await
        .expect("oneshot");

    let (status, body) = read_body(resp).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "expected 401: {body}");
    assert_eq!(body["error"].as_str().unwrap(), "unauthorized");
}

#[tokio::test]
async fn ws_session_rejects_unknown_join_code() {
    let state = test_state().await;
    // Insert a valid user + PAT so auth passes.
    let (_user_id, raw_token) = insert_user_with_pat(&state).await;
    let app = build_router(state);

    let resp = app
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/ws/s/XXXXXX")
                .header(header::ORIGIN, ORIGIN)
                .header("x-api-key", raw_token)
                .body(Body::empty())
                .expect("build request"),
        )
        .await
        .expect("oneshot");

    let (status, body) = read_body(resp).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "expected 404: {body}");
    assert_eq!(body["error"].as_str().unwrap(), "not_found");
}

#[tokio::test]
#[ignore = "requires a real WebSocket connection — deferred to manual/e2e testing"]
async fn ws_session_roundtrip_receives_broadcast_frames() {
    // Placeholder: full WS round-trip test deferred.
    // Would need tokio-tungstenite client, bind a real TCP listener,
    // and assert on received ArenaFrame JSON.
}

// ---------------------------------------------------------------------------
// JWT query-param auth tests (NFR-007 / FR-014)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn ws_session_rejects_bad_jwt_in_token_query_param() {
    let state = test_state().await;
    let app = build_router(state);

    // No Authorization header; pass an invalid JWT via ?token=
    let resp = app
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/ws/s/SOMECODE?token=not-a-real-jwt")
                .header(header::ORIGIN, ORIGIN)
                .body(Body::empty())
                .expect("build request"),
        )
        .await
        .expect("oneshot");

    let (status, body) = read_body(resp).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "expected 401: {body}");
    assert_eq!(body["error"].as_str().unwrap(), "unauthorized");
}

#[tokio::test]
async fn ws_session_rejects_unknown_join_code_with_valid_jwt_query_param() {
    let state = test_state().await;

    // Issue a valid JWT using the test signing key (no DB lookup required).
    let user_id = Uuid::new_v4();
    let enc_key = EncodingKey::from_secret(TEST_JWT_SECRET);
    let token = issue_access_token(
        &enc_key,
        user_id,
        "jwt-test@example.com",
        Duration::from_secs(900),
    )
    .expect("issue JWT");

    let app = build_router(state);

    let resp = app
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri(format!("/ws/s/XXXXXX?token={token}"))
                .header(header::ORIGIN, ORIGIN)
                .body(Body::empty())
                .expect("build request"),
        )
        .await
        .expect("oneshot");

    let (status, body) = read_body(resp).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "expected 404: {body}");
    assert_eq!(body["error"].as_str().unwrap(), "not_found");
}

// ---------------------------------------------------------------------------
// /observe endpoint tests (unauthenticated public spectator)
// ---------------------------------------------------------------------------

/// Inserts a fake session entry into `state.session_registry` and returns the join_code.
fn insert_fake_session(state: &AppState) -> String {
    let join_code = "TESTOB".to_string();
    let (tx, _rx) = broadcast::channel::<ArenaFrame>(16);
    state.session_registry.insert(
        join_code.clone(),
        SessionEntry {
            tx,
            cache: Arc::new(RwLock::new(SessionCacheInner {
                session_id: Uuid::new_v4(),
                phase: SessionStatus::Lobby,
                version: 1,
                participants: vec![],
                leaderboard: vec![],
                started_at: None,
            })),
        },
    );
    join_code
}

#[tokio::test]
async fn ws_observe_accepts_valid_join_code_without_auth() {
    let state = test_state().await;
    let join_code = insert_fake_session(&state);
    let app = build_router(state);

    // No credentials — the endpoint is intentionally unauthenticated.
    // Without WS upgrade headers the WS extractor returns 426 (not 401/404),
    // proving the session was found and no authentication was demanded.
    let resp = app
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri(format!("/ws/s/{join_code}/observe"))
                .header(header::ORIGIN, ORIGIN)
                .body(Body::empty())
                .expect("build request"),
        )
        .await
        .expect("oneshot");

    let status = resp.status();
    assert_ne!(
        status,
        StatusCode::UNAUTHORIZED,
        "observe must not require authentication (got: {status})"
    );
    assert_ne!(
        status,
        StatusCode::NOT_FOUND,
        "valid join code must not return 404 (got: {status})"
    );
}

#[tokio::test]
async fn ws_observe_rejects_unknown_join_code() {
    let state = test_state().await;
    let app = build_router(state);

    // No session in registry — no auth, no WS headers needed.
    let resp = app
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/ws/s/NOEXIST/observe")
                .header(header::ORIGIN, ORIGIN)
                .body(Body::empty())
                .expect("build request"),
        )
        .await
        .expect("oneshot");

    assert_eq!(
        resp.status(),
        StatusCode::NOT_FOUND,
        "expected 404 for unknown join code"
    );
}

#[tokio::test]
async fn ws_session_legacy_path_is_not_routable() {
    let state = test_state().await;
    let app = build_router(state);

    let resp = app
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/ws/session/SOMECODE")
                .header(header::ORIGIN, ORIGIN)
                .body(Body::empty())
                .expect("build request"),
        )
        .await
        .expect("oneshot");

    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}
