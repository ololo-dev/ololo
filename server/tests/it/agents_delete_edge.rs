#![allow(dead_code)]

//! WP-017 — agent registration + observer-token issuance.
//!
//! Spec: contract.md FR-015 (agent shape and member-registration
//! permissions), FR-016 (observer JWT delivered exactly once), FR-039
//! (advisory cap of 16 per session, configurable via
//! `ARENA_MAX_AGENTS_PER_SESSION`), FR-040 (revocation on agent delete
//! / member removal — wired in WP-018; here we set `revoked_at` on the
//! row and stash the `observer_token_jti`).
//!
//! Documented design decisions:
//!
//! - Agent cap source: `AppState::max_agents_per_session` is loaded
//!   from the env var at startup (default 16). Tests for the cap
//!   construct an `AppState` directly with a custom cap rather than
//!   mutating the env var, because env is process-global and races
//!   with parallel `cargo test` workers.
//!
//! - DELETE behavior: rows are preserved with `revoked_at = now()`
//!   (not hard-deleted). FR-040 mandates rebuilding the in-memory
//!   revocation set by scanning rows where `revoked_at IS NOT NULL`,
//!   which requires the audit row to remain. A second DELETE on a
//!   revoked row therefore returns 409 `already_revoked` (the row
//!   still exists), not 404. After DELETE the agent is still visible
//!   via GET with `revoked_at` populated.

use axum::http::{Method, StatusCode};
use migration::{Migrator, MigratorTrait};
use server::{AppState, AuthConfig, build_router};
use std::time::Duration;
use tower::ServiceExt;
use uuid::Uuid;

fn cfg(max_agents: u32) -> AuthConfig {
    AuthConfig {
        jwt_signing_key: b"integration-test-secret-32-bytes-or-more-xxxxxxx".to_vec(),
        frontend_origins: vec![ORIGIN.to_string()],
        access_ttl: Duration::from_secs(900),
        refresh_ttl: Duration::from_secs(30 * 86_400),
        max_agents_per_session: max_agents,
    }
}

async fn test_state_with_cap(max_agents: u32) -> AppState {
    let db = sea_orm::Database::connect("sqlite::memory:")
        .await
        .expect("sqlite connect");
    Migrator::up(&db, None).await.expect("migrate up");
    AppState::new(db, cfg(max_agents))
}

async fn create_project(
    app: &axum::Router,
    cookie: &str,
    name: &str,
) -> (StatusCode, serde_json::Value) {
    let resp = app
        .clone()
        .oneshot(req(
            Method::POST,
            "/api/projects",
            Some(cookie),
            Some(serde_json::json!({ "name": name })),
        ))
        .await
        .expect("create project resp");
    let (status, _, body) = read_body(resp).await;
    (status, body)
}

async fn create_session(
    app: &axum::Router,
    cookie: &str,
    name: &str,
) -> (StatusCode, serde_json::Value) {
    // Every session requires a project; create one implicitly so call sites
    // don't need to change.
    let (ps, pb) = create_project(app, cookie, &format!("proj-for-{name}")).await;
    assert_eq!(
        ps,
        StatusCode::CREATED,
        "implicit project creation failed: {pb}"
    );
    let project_id = pb["id"].as_str().expect("project id").to_string();

    let resp = app
        .clone()
        .oneshot(req(
            Method::POST,
            "/api/sessions",
            Some(cookie),
            Some(serde_json::json!({ "name": name, "project_id": project_id })),
        ))
        .await
        .expect("create session resp");
    let (status, _, body) = read_body(resp).await;
    (status, body)
}

async fn add_member(app: &axum::Router, cookie: &str, session_id: &str, user_id: Uuid, role: &str) {
    let resp = app
        .clone()
        .oneshot(req(
            Method::POST,
            &format!("/api/sessions/{session_id}/members"),
            Some(cookie),
            Some(serde_json::json!({ "user_id": user_id, "role": role })),
        ))
        .await
        .expect("add member resp");
    let (status, _, body) = read_body(resp).await;
    assert_eq!(status, StatusCode::CREATED, "add_member failed: {body}");
}

async fn patch_status(app: &axum::Router, cookie: &str, session_id: &str, status: &str) {
    let resp = app
        .clone()
        .oneshot(req(
            Method::PATCH,
            &format!("/api/sessions/{session_id}"),
            Some(cookie),
            Some(serde_json::json!({ "status": status })),
        ))
        .await
        .expect("patch resp");
    let (st, _, body) = read_body(resp).await;
    assert_eq!(st, StatusCode::OK, "patch status failed: {body}");
}

async fn register_agent(
    app: &axum::Router,
    cookie: &str,
    session_id: &str,
    body: serde_json::Value,
) -> (StatusCode, serde_json::Value) {
    let resp = app
        .clone()
        .oneshot(req(
            Method::POST,
            &format!("/api/sessions/{session_id}/agents"),
            Some(cookie),
            Some(body),
        ))
        .await
        .expect("register agent resp");
    let (status, _, body) = read_body(resp).await;
    (status, body)
}

fn default_register_body(suffix: &str) -> serde_json::Value {
    serde_json::json!({
        "kind": "claude-code",
        "model": "claude-sonnet-4",
        "display_name": format!("agent-{suffix}"),
    })
}

// 1

use crate::common;
use crate::common::*;

#[tokio::test]
async fn delete_agent_sets_revoked_at() {
    // Per FR-040 we preserve the row and flip `revoked_at`.
    let state = test_state().await;
    let app = build_router(state);
    let (_, cookie_a) = register_and_login(app.clone(), "alice@x.test", "password-12345").await;
    let (_, s) = create_session(&app, &cookie_a, "alice-1").await;
    let sid = s["id"].as_str().expect("id");

    let (_, body) = register_agent(&app, &cookie_a, sid, default_register_body("a")).await;
    let agent_id = body["agent"]["id"].as_str().expect("agent id");

    let resp = app
        .clone()
        .oneshot(req(
            Method::DELETE,
            &format!("/api/sessions/{sid}/agents/{agent_id}"),
            Some(&cookie_a),
            None,
        ))
        .await
        .expect("delete resp");
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);

    let resp = app
        .oneshot(req(
            Method::GET,
            &format!("/api/sessions/{sid}/agents/{agent_id}"),
            Some(&cookie_a),
            None,
        ))
        .await
        .expect("get resp");
    let (status, _, body) = read_body(resp).await;
    assert_eq!(status, StatusCode::OK, "row preserved for audit");
    assert!(
        body["revoked_at"].is_string(),
        "revoked_at must be set: {body}"
    );
}

// 14
#[tokio::test]
async fn delete_agent_idempotent_already_revoked_409() {
    // Documented choice: row preservation + 409 on second delete.
    let state = test_state().await;
    let app = build_router(state);
    let (_, cookie_a) = register_and_login(app.clone(), "alice@x.test", "password-12345").await;
    let (_, s) = create_session(&app, &cookie_a, "alice-1").await;
    let sid = s["id"].as_str().expect("id");

    let (_, body) = register_agent(&app, &cookie_a, sid, default_register_body("a")).await;
    let agent_id = body["agent"]["id"].as_str().expect("agent id");

    let resp = app
        .clone()
        .oneshot(req(
            Method::DELETE,
            &format!("/api/sessions/{sid}/agents/{agent_id}"),
            Some(&cookie_a),
            None,
        ))
        .await
        .expect("delete resp 1");
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);

    let resp = app
        .oneshot(req(
            Method::DELETE,
            &format!("/api/sessions/{sid}/agents/{agent_id}"),
            Some(&cookie_a),
            None,
        ))
        .await
        .expect("delete resp 2");
    let (status, _, body) = read_body(resp).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body, serde_json::json!({"error":"already_revoked"}));
}

// 15
#[tokio::test]
async fn unauth_request_401() {
    let state = test_state().await;
    let app = build_router(state);
    let (_, cookie_a) = register_and_login(app.clone(), "alice@x.test", "password-12345").await;
    let (_, s) = create_session(&app, &cookie_a, "alice-1").await;
    let sid = s["id"].as_str().expect("id");

    // No cookie on POST.
    let resp = app
        .clone()
        .oneshot(req(
            Method::POST,
            &format!("/api/sessions/{sid}/agents"),
            None,
            Some(default_register_body("a")),
        ))
        .await
        .expect("post resp");
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

    // No cookie on GET list.
    let resp = app
        .clone()
        .oneshot(req(
            Method::GET,
            &format!("/api/sessions/{sid}/agents"),
            None,
            None,
        ))
        .await
        .expect("get list resp");
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

    // No cookie on DELETE.
    let some_uuid = Uuid::new_v4();
    let resp = app
        .oneshot(req(
            Method::DELETE,
            &format!("/api/sessions/{sid}/agents/{some_uuid}"),
            None,
            None,
        ))
        .await
        .expect("delete resp");
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

// Bonus: deny_unknown_fields enforcement on the request DTO.
#[tokio::test]
async fn register_agent_unknown_field_422() {
    let state = test_state().await;
    let app = build_router(state);
    let (_, cookie_a) = register_and_login(app.clone(), "alice@x.test", "password-12345").await;
    let (_, s) = create_session(&app, &cookie_a, "alice-1").await;
    let sid = s["id"].as_str().expect("id");

    let (status, _) = register_agent(
        &app,
        &cookie_a,
        sid,
        serde_json::json!({
            "kind": "claude-code",
            "model": "x",
            "display_name": "y",
            "extra": "nope",
        }),
    )
    .await;
    // axum rejects malformed JSON with 422 (or 400) before our handler runs.
    assert!(
        status == StatusCode::UNPROCESSABLE_ENTITY || status == StatusCode::BAD_REQUEST,
        "expected 4xx for unknown field, got {status}"
    );
}
