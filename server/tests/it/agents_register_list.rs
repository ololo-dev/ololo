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
async fn register_agent_owner_lobby_201() {
    let state = test_state().await;
    let app = build_router(state);
    let (_, cookie_a) = register_and_login(app.clone(), "alice@x.test", "password-12345").await;
    let (_, s) = create_session(&app, &cookie_a, "alice-1").await;
    let sid = s["id"].as_str().expect("id");

    let (status, body) = register_agent(&app, &cookie_a, sid, default_register_body("a")).await;
    assert_eq!(status, StatusCode::CREATED, "body={body}");
    let agent = &body["agent"];
    let agent_id_str = agent["id"].as_str().expect("agent id");
    Uuid::parse_str(agent_id_str).expect("agent id is uuid");
    assert_eq!(agent["kind"], "claude-code");
    assert_eq!(agent["session_id"], serde_json::json!(sid));
    assert!(agent["registered_at"].is_string());
    assert!(agent["revoked_at"].is_null());
}

// 2
#[tokio::test]
async fn register_agent_member_running_201() {
    let state = test_state().await;
    let app = build_router(state);
    let (_, cookie_a) = register_and_login(app.clone(), "alice@x.test", "password-12345").await;
    let (uid_b, cookie_b) = register_and_login(app.clone(), "bob@x.test", "password-12345").await;
    let (_, s) = create_session(&app, &cookie_a, "alice-1").await;
    let sid = s["id"].as_str().expect("id");

    add_member(&app, &cookie_a, sid, uid_b, "participant").await;
    patch_status(&app, &cookie_a, sid, "running").await;

    let (status, body) = register_agent(&app, &cookie_b, sid, default_register_body("b")).await;
    assert_eq!(status, StatusCode::CREATED, "body={body}");
    assert!(body["agent"]["id"].as_str().is_some());
}

// 3
#[tokio::test]
async fn register_agent_invalid_kind_422() {
    let state = test_state().await;
    let app = build_router(state);
    let (_, cookie_a) = register_and_login(app.clone(), "alice@x.test", "password-12345").await;
    let (_, s) = create_session(&app, &cookie_a, "alice-1").await;
    let sid = s["id"].as_str().expect("id");

    let (status, body) = register_agent(
        &app,
        &cookie_a,
        sid,
        serde_json::json!({
            "kind": "gemini",
            "model": "g-1",
            "display_name": "x",
        }),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body, serde_json::json!({"error":"invalid_kind"}));
}

// 4
#[tokio::test]
async fn register_agent_outsider_404() {
    let state = test_state().await;
    let app = build_router(state);
    let (_, cookie_a) = register_and_login(app.clone(), "alice@x.test", "password-12345").await;
    let (_, cookie_c) = register_and_login(app.clone(), "carol@x.test", "password-12345").await;
    let (_, s) = create_session(&app, &cookie_a, "alice-1").await;
    let sid = s["id"].as_str().expect("id");

    let (status, _) = register_agent(&app, &cookie_c, sid, default_register_body("c")).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

// 5
#[tokio::test]
async fn register_agent_session_finished_409() {
    let state = test_state().await;
    let app = build_router(state);
    let (_, cookie_a) = register_and_login(app.clone(), "alice@x.test", "password-12345").await;
    let (_, s) = create_session(&app, &cookie_a, "alice-1").await;
    let sid = s["id"].as_str().expect("id");

    patch_status(&app, &cookie_a, sid, "running").await;
    patch_status(&app, &cookie_a, sid, "finished").await;

    let (status, body) = register_agent(&app, &cookie_a, sid, default_register_body("a")).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body, serde_json::json!({"error":"session_closed"}));
}

// 6
#[tokio::test]
async fn register_agent_cap_reached_409() {
    // Use constructor-injected cap to avoid env races (see module
    // rustdoc).
    let state = test_state_with_cap(2).await;
    let app = build_router(state);
    let (_, cookie_a) = register_and_login(app.clone(), "alice@x.test", "password-12345").await;
    let (_, s) = create_session(&app, &cookie_a, "alice-1").await;
    let sid = s["id"].as_str().expect("id");

    let (st, _) = register_agent(&app, &cookie_a, sid, default_register_body("1")).await;
    assert_eq!(st, StatusCode::CREATED);
    let (st, _) = register_agent(&app, &cookie_a, sid, default_register_body("2")).await;
    assert_eq!(st, StatusCode::CREATED);

    let (st, body) = register_agent(&app, &cookie_a, sid, default_register_body("3")).await;
    assert_eq!(st, StatusCode::CONFLICT);
    assert_eq!(body, serde_json::json!({"error":"agent_cap_reached"}));
}

// 7
#[tokio::test]
async fn list_agents_member_sees_session_agents_no_token() {
    let state = test_state().await;
    let app = build_router(state);
    let (_, cookie_a) = register_and_login(app.clone(), "alice@x.test", "password-12345").await;
    let (uid_b, cookie_b) = register_and_login(app.clone(), "bob@x.test", "password-12345").await;
    let (_, s) = create_session(&app, &cookie_a, "alice-1").await;
    let sid = s["id"].as_str().expect("id");
    add_member(&app, &cookie_a, sid, uid_b, "participant").await;

    let (st, _) = register_agent(&app, &cookie_a, sid, default_register_body("a")).await;
    assert_eq!(st, StatusCode::CREATED);

    let resp = app
        .oneshot(req(
            Method::GET,
            &format!("/api/sessions/{sid}/agents"),
            Some(&cookie_b),
            None,
        ))
        .await
        .expect("list resp");
    let (status, _, body) = read_body(resp).await;
    assert_eq!(status, StatusCode::OK);
    let arr = body["agents"].as_array().expect("agents array");
    assert_eq!(arr.len(), 1);
    assert!(
        arr[0].get("observer_token").is_none(),
        "list response must NOT contain observer_token"
    );
    assert!(
        body.get("observer_token").is_none(),
        "envelope must NOT contain observer_token"
    );
}

// 8
#[tokio::test]
async fn list_agents_outsider_404() {
    let state = test_state().await;
    let app = build_router(state);
    let (_, cookie_a) = register_and_login(app.clone(), "alice@x.test", "password-12345").await;
    let (_, cookie_c) = register_and_login(app.clone(), "carol@x.test", "password-12345").await;
    let (_, s) = create_session(&app, &cookie_a, "alice-1").await;
    let sid = s["id"].as_str().expect("id");

    let resp = app
        .oneshot(req(
            Method::GET,
            &format!("/api/sessions/{sid}/agents"),
            Some(&cookie_c),
            None,
        ))
        .await
        .expect("list resp");
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

// 9
#[tokio::test]
async fn get_agent_member_no_token() {
    let state = test_state().await;
    let app = build_router(state);
    let (_, cookie_a) = register_and_login(app.clone(), "alice@x.test", "password-12345").await;
    let (uid_b, cookie_b) = register_and_login(app.clone(), "bob@x.test", "password-12345").await;
    let (_, s) = create_session(&app, &cookie_a, "alice-1").await;
    let sid = s["id"].as_str().expect("id");
    add_member(&app, &cookie_a, sid, uid_b, "participant").await;

    let (_, body) = register_agent(&app, &cookie_a, sid, default_register_body("a")).await;
    let agent_id = body["agent"]["id"].as_str().expect("agent id");

    let resp = app
        .oneshot(req(
            Method::GET,
            &format!("/api/sessions/{sid}/agents/{agent_id}"),
            Some(&cookie_b),
            None,
        ))
        .await
        .expect("get resp");
    let (status, _, body) = read_body(resp).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.get("observer_token").is_none());
    assert_eq!(body["id"], serde_json::json!(agent_id));
}

// 10
#[tokio::test]
async fn delete_agent_self_204() {
    let state = test_state().await;
    let app = build_router(state);
    let (_, cookie_a) = register_and_login(app.clone(), "alice@x.test", "password-12345").await;
    let (uid_b, cookie_b) = register_and_login(app.clone(), "bob@x.test", "password-12345").await;
    let (_, s) = create_session(&app, &cookie_a, "alice-1").await;
    let sid = s["id"].as_str().expect("id");
    add_member(&app, &cookie_a, sid, uid_b, "participant").await;

    let (_, body) = register_agent(&app, &cookie_b, sid, default_register_body("b")).await;
    let agent_id = body["agent"]["id"].as_str().expect("agent id");

    let resp = app
        .oneshot(req(
            Method::DELETE,
            &format!("/api/sessions/{sid}/agents/{agent_id}"),
            Some(&cookie_b),
            None,
        ))
        .await
        .expect("delete resp");
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);
}

// 11
#[tokio::test]
async fn delete_agent_session_owner_204() {
    let state = test_state().await;
    let app = build_router(state);
    let (_, cookie_a) = register_and_login(app.clone(), "alice@x.test", "password-12345").await;
    let (uid_b, cookie_b) = register_and_login(app.clone(), "bob@x.test", "password-12345").await;
    let (_, s) = create_session(&app, &cookie_a, "alice-1").await;
    let sid = s["id"].as_str().expect("id");
    add_member(&app, &cookie_a, sid, uid_b, "participant").await;

    let (_, body) = register_agent(&app, &cookie_b, sid, default_register_body("b")).await;
    let agent_id = body["agent"]["id"].as_str().expect("agent id");

    let resp = app
        .oneshot(req(
            Method::DELETE,
            &format!("/api/sessions/{sid}/agents/{agent_id}"),
            Some(&cookie_a),
            None,
        ))
        .await
        .expect("delete resp");
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);
}

// 12
#[tokio::test]
async fn delete_agent_other_member_403() {
    let state = test_state().await;
    let app = build_router(state);
    let (_, cookie_a) = register_and_login(app.clone(), "alice@x.test", "password-12345").await;
    let (uid_b, cookie_b) = register_and_login(app.clone(), "bob@x.test", "password-12345").await;
    let (uid_c, cookie_c) = register_and_login(app.clone(), "carol@x.test", "password-12345").await;
    let (_, s) = create_session(&app, &cookie_a, "alice-1").await;
    let sid = s["id"].as_str().expect("id");
    add_member(&app, &cookie_a, sid, uid_b, "participant").await;
    add_member(&app, &cookie_a, sid, uid_c, "participant").await;

    // bob registers an agent. carol (different member, not owner) tries to delete.
    let (_, body) = register_agent(&app, &cookie_b, sid, default_register_body("b")).await;
    let agent_id = body["agent"]["id"].as_str().expect("agent id");

    let resp = app
        .oneshot(req(
            Method::DELETE,
            &format!("/api/sessions/{sid}/agents/{agent_id}"),
            Some(&cookie_c),
            None,
        ))
        .await
        .expect("delete resp");
    let (status, _, body) = read_body(resp).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(body, serde_json::json!({"error":"forbidden"}));
}

// 13
