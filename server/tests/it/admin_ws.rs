//! Integration tests for `GET /ws/admin` — pre-upgrade auth checks.
//!
//! Auth flow exercised here (all checks happen before the WS upgrade):
//!   1. `session_id` missing or empty  → 400
//!   2. Session not found in DB        → 404
//!   3. `token` missing or empty       → 403
//!   4. JWT invalid                    → 403
//!   5. User does not exist in DB      → 403
//!   6. User `is_admin = false`        → 403
//!   7. All checks pass, no WS headers → 400 (handler converts ws-upgrade rejection)
//!
//! Real WebSocket round-trip tests are deferred and marked `#[ignore]`.

use arena_core::session_status::SessionStatus;
use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use chrono::Utc;
use jsonwebtoken::EncodingKey;
use migration::{Migrator, MigratorTrait};
use sea_orm::{ActiveModelTrait, Set};
use server::auth::jwt::issue_access_token;
use server::entities::{projects, sessions, users};
use server::rate_limiter::NoOpRateLimiter;
use server::{AppState, AuthConfig, build_router};
use std::sync::Arc;
use std::time::Duration;
use tower::ServiceExt;
use uuid::Uuid;

const ORIGIN: &str = "http://localhost:5173";
const TEST_JWT_SECRET: &[u8] = b"integration-test-secret-32-bytes-or-more-xxxxxxx";

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

async fn test_state() -> AppState {
    let db = sea_orm::Database::connect("sqlite::memory:")
        .await
        .expect("sqlite connect");
    Migrator::up(&db, None).await.expect("migrate up");
    let cfg = AuthConfig {
        jwt_signing_key: TEST_JWT_SECRET.to_vec(),
        frontend_origins: vec![ORIGIN.to_string()],
        access_ttl: Duration::from_secs(900),
        refresh_ttl: Duration::from_secs(30 * 86_400),
        max_agents_per_session: 16,
    };
    let mut state = AppState::new(db, cfg);
    state.rate_limiter = Arc::new(NoOpRateLimiter);
    state
}

/// Insert a user row. `is_admin` controls the admin flag. Returns `user_id`.
async fn insert_user(state: &AppState, is_admin: bool) -> Uuid {
    let user_id = Uuid::new_v4();
    users::ActiveModel {
        id: Set(user_id),
        email: Set(format!("ws-admin-test-{}@example.com", user_id)),
        password_hash: Set(None),
        display_name: Set("WS Admin Test User".to_string()),
        created_at: Set(Utc::now()),
        updated_at: Set(Utc::now()),
        is_admin: Set(is_admin),
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
    user_id
}

/// Insert a minimal project owned by `owner_id`. Returns `project_id`.
async fn insert_project(state: &AppState, owner_id: Uuid) -> Uuid {
    let project_id = Uuid::new_v4();
    projects::ActiveModel {
        id: Set(project_id),
        name: Set("Test Project".to_string()),
        slug: Set(None),
        description: Set(String::new()),
        category: Set(None),
        tags: Set(String::new()),
        cover_image_url: Set(None),
        owner_user_id_fk: Set(owner_id),
        public: Set(true),
        archived_at: Set(None),
        created_at: Set(Utc::now()),
        updated_at: Set(Utc::now()),
        default_value_points: Set(10),
        default_fail_points: Set(-5),
        default_no_response_points: Set(-10),
        default_completion_bonus_points: Set(10),
        default_deadline_secs: Set(60),
        default_session_duration_secs: Set(3600),
        idle_timeout_secs: Set(300),
        default_min_interval_secs: Set(5),
        default_interval_increment_secs: Set(5),
        default_max_interval_secs: Set(60),
        memory_schema: Set(None),
        show_tasks: Set(true),
        parent_project_id_fk: Set(None),
        part_ordinal: Set(None),
    }
    .insert(&state.db)
    .await
    .expect("insert project");
    project_id
}

/// Insert a session with `project_id_fk`. Returns the session UUID.
async fn insert_session(state: &AppState, project_id: Uuid) -> Uuid {
    let session_id = Uuid::new_v4();
    sessions::ActiveModel {
        id: Set(session_id),
        name: Set("Admin WS Test Session".to_string()),
        created_at: Set(Utc::now()),
        owner_id_fk: Set(None),
        status: Set(SessionStatus::Lobby),
        join_code: Set(format!("TSTADM{}", &Uuid::new_v4().to_string()[..6])),
        started_at: Set(None),
        finished_at: Set(None),
        paused_at: Set(None),
        paused_duration_secs: Set(None),
        project_id_fk: Set(project_id),
        game_server_id: Set(None),
        cancel_reason: Set(None),
        cancelled_by: Set(None),
    }
    .insert(&state.db)
    .await
    .expect("insert session");
    session_id
}

/// Issue a valid JWT for `user_id` using the test signing key.
fn issue_jwt(user_id: Uuid) -> String {
    let enc_key = EncodingKey::from_secret(TEST_JWT_SECRET);
    issue_access_token(
        &enc_key,
        user_id,
        "ws-admin-test@example.com",
        Duration::from_secs(900),
    )
    .expect("issue JWT")
}

// ---------------------------------------------------------------------------
// Tests: session_id validation
// ---------------------------------------------------------------------------

#[tokio::test]
async fn ws_admin_rejects_missing_session_id() {
    let state = test_state().await;
    let app = build_router(state);

    let resp = app
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/ws/admin")
                .header("Origin", ORIGIN)
                .body(Body::empty())
                .expect("build request"),
        )
        .await
        .expect("oneshot");

    assert_eq!(
        resp.status(),
        StatusCode::BAD_REQUEST,
        "missing session_id must return 400"
    );
}

#[tokio::test]
async fn ws_admin_rejects_empty_session_id() {
    let state = test_state().await;
    let app = build_router(state);

    let resp = app
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/ws/admin?session_id=")
                .header("Origin", ORIGIN)
                .body(Body::empty())
                .expect("build request"),
        )
        .await
        .expect("oneshot");

    assert_eq!(
        resp.status(),
        StatusCode::BAD_REQUEST,
        "empty session_id must return 400"
    );
}

#[tokio::test]
async fn ws_admin_rejects_unknown_session_id() {
    let state = test_state().await;
    let app = build_router(state);

    let resp = app
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri(format!("/ws/admin?session_id={}", Uuid::new_v4()))
                .header("Origin", ORIGIN)
                .body(Body::empty())
                .expect("build request"),
        )
        .await
        .expect("oneshot");

    assert_eq!(
        resp.status(),
        StatusCode::NOT_FOUND,
        "unknown session_id must return 404"
    );
}

// ---------------------------------------------------------------------------
// Tests: token validation
// ---------------------------------------------------------------------------

#[tokio::test]
async fn ws_admin_rejects_missing_token() {
    let state = test_state().await;
    let owner_id = insert_user(&state, false).await;
    let project_id = insert_project(&state, owner_id).await;
    let session_id = insert_session(&state, project_id).await;
    let app = build_router(state);

    let resp = app
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri(format!("/ws/admin?session_id={session_id}"))
                .header("Origin", ORIGIN)
                .body(Body::empty())
                .expect("build request"),
        )
        .await
        .expect("oneshot");

    assert_eq!(
        resp.status(),
        StatusCode::FORBIDDEN,
        "missing token must return 403"
    );
}

#[tokio::test]
async fn ws_admin_rejects_empty_token() {
    let state = test_state().await;
    let owner_id = insert_user(&state, false).await;
    let project_id = insert_project(&state, owner_id).await;
    let session_id = insert_session(&state, project_id).await;
    let app = build_router(state);

    let resp = app
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri(format!("/ws/admin?session_id={session_id}&token="))
                .header("Origin", ORIGIN)
                .body(Body::empty())
                .expect("build request"),
        )
        .await
        .expect("oneshot");

    assert_eq!(
        resp.status(),
        StatusCode::FORBIDDEN,
        "empty token must return 403"
    );
}

#[tokio::test]
async fn ws_admin_rejects_invalid_jwt() {
    let state = test_state().await;
    let owner_id = insert_user(&state, false).await;
    let project_id = insert_project(&state, owner_id).await;
    let session_id = insert_session(&state, project_id).await;
    let app = build_router(state);

    let resp = app
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri(format!("/ws/admin?session_id={session_id}&token=not-a-jwt"))
                .header("Origin", ORIGIN)
                .body(Body::empty())
                .expect("build request"),
        )
        .await
        .expect("oneshot");

    assert_eq!(
        resp.status(),
        StatusCode::FORBIDDEN,
        "invalid JWT must return 403"
    );
}

// ---------------------------------------------------------------------------
// Tests: admin check
// ---------------------------------------------------------------------------

#[tokio::test]
async fn ws_admin_rejects_non_admin_user() {
    let state = test_state().await;
    let user_id = insert_user(&state, false).await;
    let project_id = insert_project(&state, user_id).await;
    let session_id = insert_session(&state, project_id).await;
    let token = issue_jwt(user_id);
    let app = build_router(state);

    let resp = app
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri(format!("/ws/admin?session_id={session_id}&token={token}"))
                .header("Origin", ORIGIN)
                .body(Body::empty())
                .expect("build request"),
        )
        .await
        .expect("oneshot");

    assert_eq!(
        resp.status(),
        StatusCode::FORBIDDEN,
        "non-admin user must return 403"
    );
}

#[tokio::test]
async fn ws_admin_rejects_jwt_for_unknown_user() {
    // JWT is valid (correct signature) but the user_id is not in the DB.
    let state = test_state().await;
    let owner_id = insert_user(&state, false).await;
    let project_id = insert_project(&state, owner_id).await;
    let session_id = insert_session(&state, project_id).await;

    // Issue a JWT for a user_id that was never inserted.
    let ghost_user_id = Uuid::new_v4();
    let token = issue_jwt(ghost_user_id);
    let app = build_router(state);

    let resp = app
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri(format!("/ws/admin?session_id={session_id}&token={token}"))
                .header("Origin", ORIGIN)
                .body(Body::empty())
                .expect("build request"),
        )
        .await
        .expect("oneshot");

    assert_eq!(
        resp.status(),
        StatusCode::FORBIDDEN,
        "JWT for unknown user must return 403"
    );
}

// ---------------------------------------------------------------------------
// Test: all auth checks pass → upgrade rejected (no WS headers) → 400
// ---------------------------------------------------------------------------

#[tokio::test]
async fn ws_admin_passes_auth_for_admin_user() {
    // All pre-upgrade checks pass.  Without real WebSocket upgrade headers the
    // handler converts the ws-extractor rejection into a 400 response.
    // The key assertion is that we do NOT get 403/404 — auth was accepted.
    let state = test_state().await;
    let admin_id = insert_user(&state, true).await;
    let project_id = insert_project(&state, admin_id).await;
    let session_id = insert_session(&state, project_id).await;
    let token = issue_jwt(admin_id);
    let app = build_router(state);

    let resp = app
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri(format!("/ws/admin?session_id={session_id}&token={token}"))
                .header("Origin", ORIGIN)
                .body(Body::empty())
                .expect("build request"),
        )
        .await
        .expect("oneshot");

    let status = resp.status();
    assert_ne!(status, StatusCode::UNAUTHORIZED, "auth must pass: {status}");
    assert_ne!(status, StatusCode::FORBIDDEN, "auth must pass: {status}");
    assert_ne!(status, StatusCode::NOT_FOUND, "auth must pass: {status}");
    // Handler converts ws-upgrade rejection to 400.
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "expected 400 (ws upgrade rejected) after auth passes: {status}"
    );
}

// ---------------------------------------------------------------------------
// Deferred: real WS round-trip
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore = "requires a real WebSocket connection — deferred to manual/e2e testing"]
async fn ws_admin_roundtrip_receives_snapshot_frame() {
    // Placeholder: full WS round-trip deferred.
    // Would need tokio-tungstenite client, a real TCP listener,
    // and assertions on the received AdminAdaptedTasksSnapshot JSON.
}
