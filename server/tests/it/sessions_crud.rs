//! WP-011 — sessions CRUD with ownership.
//!
//! Spec: contract.md §"Sessions and invites" — FR-008 (owner-only edit),
//! FR-009 (`status` lifecycle: `lobby` → `running` → `finished` | `cancelled`,
//! no transitions out of terminal states), FR-038 (cascade rules),
//! NFR-005 (audit timestamps).
//!
//! Notes for reviewers (WP-011 deviation from the work-package brief):
//! the brief described the lifecycle as `draft → active → ended`. The
//! signed contract (FR-009) and the live schema in
//! `m20260502_000004_alter_sessions_owner_state` use `status` with
//! `lobby | running | finished | cancelled`. We follow the contract +
//! schema (the source of truth) and surface the column under the JSON
//! key `status` to stay consistent with downstream WPs (invites,
//! scheduler) that already key off these values.

use axum::http::{Method, StatusCode};
use chrono::Utc;
use sea_orm::{ActiveModelTrait, Set};
use server::build_router;
use server::entities::players;
use tower::ServiceExt;
use uuid::Uuid;

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
    // Sessions require a project; create one per call to keep tests isolated.
    let (_, project) = create_project(app, cookie, "default-project").await;
    let project_id = project["id"]
        .as_str()
        .expect("project id from create_project");
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

use crate::common;
use crate::common::*;

#[tokio::test]
async fn create_session_returns_201_with_owner() {
    let state = test_state().await;
    let app = build_router(state);
    let (uid_a, cookie_a) = register_and_login(app.clone(), "alice@x.test", "password-12345").await;

    let (status, body) = create_session(&app, &cookie_a, "My Arena").await;
    assert_eq!(status, StatusCode::CREATED, "body={body}");
    assert_eq!(body["name"], serde_json::json!("My Arena"));
    assert_eq!(body["status"], serde_json::json!("lobby"));
    assert_eq!(body["owner_id"], serde_json::json!(uid_a.to_string()));
    assert!(body["id"].is_string(), "id present");
    assert!(body["created_at"].is_string(), "created_at present");
}

#[tokio::test]
async fn create_session_rejects_empty_name() {
    let state = test_state().await;
    let app = build_router(state);
    let (_, cookie_a) = register_and_login(app.clone(), "alice@x.test", "password-12345").await;

    for bad in ["", "   ", "\t\n"] {
        let (status, body) = create_session(&app, &cookie_a, bad).await;
        assert_eq!(
            status,
            StatusCode::UNPROCESSABLE_ENTITY,
            "name={bad:?} body={body}"
        );
    }
}

#[tokio::test]
async fn create_session_rejects_long_name() {
    let state = test_state().await;
    let app = build_router(state);
    let (_, cookie_a) = register_and_login(app.clone(), "alice@x.test", "password-12345").await;

    let long = "x".repeat(121);
    let (status, _) = create_session(&app, &cookie_a, &long).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn list_sessions_returns_only_user_visible() {
    let state = test_state().await;
    let app = build_router(state);
    let (_, cookie_a) = register_and_login(app.clone(), "alice@x.test", "password-12345").await;
    let (_, cookie_b) = register_and_login(app.clone(), "bob@x.test", "password-12345").await;

    let (_, s1) = create_session(&app, &cookie_a, "alice-1").await;
    let (_, _s2) = create_session(&app, &cookie_b, "bob-1").await;

    let resp = app
        .clone()
        .oneshot(req(Method::GET, "/api/sessions", Some(&cookie_a), None))
        .await
        .expect("list resp");
    let (status, _, body) = read_body(resp).await;
    assert_eq!(status, StatusCode::OK);
    let sessions = body["sessions"].as_array().expect("sessions array");
    assert_eq!(sessions.len(), 1, "alice should only see her session");
    assert_eq!(sessions[0]["id"], s1["id"]);
}

#[tokio::test]
async fn get_session_owner_succeeds() {
    let state = test_state().await;
    let app = build_router(state);
    let (_, cookie_a) = register_and_login(app.clone(), "alice@x.test", "password-12345").await;
    let (_, s1) = create_session(&app, &cookie_a, "alice-1").await;
    let id = s1["id"].as_str().expect("id");

    let resp = app
        .oneshot(req(
            Method::GET,
            &format!("/api/sessions/{id}"),
            Some(&cookie_a),
            None,
        ))
        .await
        .expect("get resp");
    let (status, _, body) = read_body(resp).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["id"], s1["id"]);
}

#[tokio::test]
async fn get_session_member_succeeds() {
    let state = test_state().await;
    let app = build_router(state.clone());
    let (_, cookie_a) = register_and_login(app.clone(), "alice@x.test", "password-12345").await;
    let (uid_b, cookie_b) = register_and_login(app.clone(), "bob@x.test", "password-12345").await;

    let (_, s1) = create_session(&app, &cookie_a, "alice-1").await;
    let id = s1["id"].as_str().expect("id").to_string();
    let session_id = Uuid::parse_str(&id).expect("uuid");

    // Insert membership row for bob directly (members CRUD lands in WP-012).
    let am = players::ActiveModel {
        id: Set(Uuid::new_v4()),
        session_id_fk: Set(session_id),
        user_id_fk: Set(Some(uid_b)),
        display_name: Set("bob".into()),
        fingerprint: Set(None),
        metadata_json: Set(None),
        joined_at: Set(Utc::now()),
        reconnected_at: Set(None),
        revoked_at: Set(None),
        agent_connected: Set(false),
        agent_last_seen_at: Set(None),
    };
    am.insert(&state.db).await.expect("insert member");

    let resp = app
        .oneshot(req(
            Method::GET,
            &format!("/api/sessions/{id}"),
            Some(&cookie_b),
            None,
        ))
        .await
        .expect("get resp");
    let (status, _, body) = read_body(resp).await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    assert_eq!(body["id"], serde_json::json!(id));
}

#[tokio::test]
async fn get_session_outsider_returns_404() {
    let state = test_state().await;
    let app = build_router(state);
    let (_, cookie_a) = register_and_login(app.clone(), "alice@x.test", "password-12345").await;
    let (_, cookie_c) = register_and_login(app.clone(), "carol@x.test", "password-12345").await;

    let (_, s1) = create_session(&app, &cookie_a, "alice-1").await;
    let id = s1["id"].as_str().expect("id");

    let resp = app
        .oneshot(req(
            Method::GET,
            &format!("/api/sessions/{id}"),
            Some(&cookie_c),
            None,
        ))
        .await
        .expect("get resp");
    let (status, _, body) = read_body(resp).await;
    // 404 not 403: outsiders must not learn the session exists.
    assert_eq!(status, StatusCode::NOT_FOUND, "body={body}");
}

#[tokio::test]
async fn patch_session_owner_can_rename() {
    let state = test_state().await;
    let app = build_router(state);
    let (_, cookie_a) = register_and_login(app.clone(), "alice@x.test", "password-12345").await;
    let (_, s1) = create_session(&app, &cookie_a, "alice-1").await;
    let id = s1["id"].as_str().expect("id");

    let resp = app
        .oneshot(req(
            Method::PATCH,
            &format!("/api/sessions/{id}"),
            Some(&cookie_a),
            Some(serde_json::json!({ "name": "renamed" })),
        ))
        .await
        .expect("patch resp");
    let (status, _, body) = read_body(resp).await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    assert_eq!(body["name"], serde_json::json!("renamed"));
}

#[tokio::test]
async fn patch_session_member_forbidden() {
    let state = test_state().await;
    let app = build_router(state.clone());
    let (_, cookie_a) = register_and_login(app.clone(), "alice@x.test", "password-12345").await;
    let (uid_b, cookie_b) = register_and_login(app.clone(), "bob@x.test", "password-12345").await;

    let (_, s1) = create_session(&app, &cookie_a, "alice-1").await;
    let id = s1["id"].as_str().expect("id").to_string();
    let session_id = Uuid::parse_str(&id).expect("uuid");

    let am = players::ActiveModel {
        id: Set(Uuid::new_v4()),
        session_id_fk: Set(session_id),
        user_id_fk: Set(Some(uid_b)),
        display_name: Set("bob".into()),
        fingerprint: Set(None),
        metadata_json: Set(None),
        joined_at: Set(Utc::now()),
        reconnected_at: Set(None),
        revoked_at: Set(None),
        agent_connected: Set(false),
        agent_last_seen_at: Set(None),
    };
    am.insert(&state.db).await.expect("insert member");

    let resp = app
        .oneshot(req(
            Method::PATCH,
            &format!("/api/sessions/{id}"),
            Some(&cookie_b),
            Some(serde_json::json!({ "name": "hijacked" })),
        ))
        .await
        .expect("patch resp");
    let (status, _, body) = read_body(resp).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "body={body}");
    assert_eq!(body, serde_json::json!({"error":"forbidden"}));
}

async fn patch_status(
    app: &axum::Router,
    cookie: &str,
    id: &str,
    new_status: &str,
) -> (StatusCode, serde_json::Value) {
    let resp = app
        .clone()
        .oneshot(req(
            Method::PATCH,
            &format!("/api/sessions/{id}"),
            Some(cookie),
            Some(serde_json::json!({ "status": new_status })),
        ))
        .await
        .expect("patch resp");
    let (s, _, b) = read_body(resp).await;
    (s, b)
}

#[tokio::test]
async fn patch_session_status_lobby_to_running_ok() {
    let state = test_state().await;
    let app = build_router(state);
    let (_, cookie_a) = register_and_login(app.clone(), "alice@x.test", "password-12345").await;
    let (_, s1) = create_session(&app, &cookie_a, "alice-1").await;
    let id = s1["id"].as_str().expect("id");

    let (status, body) = patch_status(&app, &cookie_a, id, "running").await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    assert_eq!(body["status"], serde_json::json!("running"));
}

#[tokio::test]
async fn patch_session_status_running_to_finished_ok() {
    let state = test_state().await;
    let app = build_router(state);
    let (_, cookie_a) = register_and_login(app.clone(), "alice@x.test", "password-12345").await;
    let (_, s1) = create_session(&app, &cookie_a, "alice-1").await;
    let id = s1["id"].as_str().expect("id");

    let (status, _) = patch_status(&app, &cookie_a, id, "running").await;
    assert_eq!(status, StatusCode::OK);
    let (status, body) = patch_status(&app, &cookie_a, id, "finished").await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    assert_eq!(body["status"], serde_json::json!("finished"));
}

#[tokio::test]
async fn patch_session_status_finished_to_running_rejected() {
    let state = test_state().await;
    let app = build_router(state);
    let (_, cookie_a) = register_and_login(app.clone(), "alice@x.test", "password-12345").await;
    let (_, s1) = create_session(&app, &cookie_a, "alice-1").await;
    let id = s1["id"].as_str().expect("id");

    let (s, _) = patch_status(&app, &cookie_a, id, "running").await;
    assert_eq!(s, StatusCode::OK);
    let (s, _) = patch_status(&app, &cookie_a, id, "finished").await;
    assert_eq!(s, StatusCode::OK);
    let (status, body) = patch_status(&app, &cookie_a, id, "running").await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body, serde_json::json!({"error":"invalid_transition"}));
}

#[tokio::test]
async fn patch_session_status_running_to_lobby_rejected() {
    let state = test_state().await;
    let app = build_router(state);
    let (_, cookie_a) = register_and_login(app.clone(), "alice@x.test", "password-12345").await;
    let (_, s1) = create_session(&app, &cookie_a, "alice-1").await;
    let id = s1["id"].as_str().expect("id");

    let (s, _) = patch_status(&app, &cookie_a, id, "running").await;
    assert_eq!(s, StatusCode::OK);
    let (status, body) = patch_status(&app, &cookie_a, id, "lobby").await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body, serde_json::json!({"error":"invalid_transition"}));
}

#[tokio::test]
async fn patch_session_status_unknown_value_422() {
    let state = test_state().await;
    let app = build_router(state);
    let (_, cookie_a) = register_and_login(app.clone(), "alice@x.test", "password-12345").await;
    let (_, s1) = create_session(&app, &cookie_a, "alice-1").await;
    let id = s1["id"].as_str().expect("id");

    let (status, _) = patch_status(&app, &cookie_a, id, "weird").await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn delete_session_owner_204() {
    let state = test_state().await;
    let app = build_router(state);
    let (_, cookie_a) = register_and_login(app.clone(), "alice@x.test", "password-12345").await;
    let (_, s1) = create_session(&app, &cookie_a, "alice-1").await;
    let id = s1["id"].as_str().expect("id").to_string();

    let resp = app
        .clone()
        .oneshot(req(
            Method::DELETE,
            &format!("/api/sessions/{id}"),
            Some(&cookie_a),
            None,
        ))
        .await
        .expect("delete resp");
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);

    let resp = app
        .oneshot(req(
            Method::GET,
            &format!("/api/sessions/{id}"),
            Some(&cookie_a),
            None,
        ))
        .await
        .expect("get resp");
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn delete_session_member_forbidden() {
    let state = test_state().await;
    let app = build_router(state.clone());
    let (_, cookie_a) = register_and_login(app.clone(), "alice@x.test", "password-12345").await;
    let (uid_b, cookie_b) = register_and_login(app.clone(), "bob@x.test", "password-12345").await;

    let (_, s1) = create_session(&app, &cookie_a, "alice-1").await;
    let id = s1["id"].as_str().expect("id").to_string();
    let session_id = Uuid::parse_str(&id).expect("uuid");

    let am = players::ActiveModel {
        id: Set(Uuid::new_v4()),
        session_id_fk: Set(session_id),
        user_id_fk: Set(Some(uid_b)),
        display_name: Set("bob".into()),
        fingerprint: Set(None),
        metadata_json: Set(None),
        joined_at: Set(Utc::now()),
        reconnected_at: Set(None),
        revoked_at: Set(None),
        agent_connected: Set(false),
        agent_last_seen_at: Set(None),
    };
    am.insert(&state.db).await.expect("insert member");

    let resp = app
        .oneshot(req(
            Method::DELETE,
            &format!("/api/sessions/{id}"),
            Some(&cookie_b),
            None,
        ))
        .await
        .expect("delete resp");
    let (status, _, body) = read_body(resp).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(body, serde_json::json!({"error":"forbidden"}));
}

#[tokio::test]
async fn unauth_request_401() {
    let state = test_state().await;
    let app = build_router(state);

    // POST without auth cookie — origin still required for state-changing.
    let resp = app
        .oneshot(req(
            Method::POST,
            "/api/sessions",
            None,
            Some(serde_json::json!({ "name": "x" })),
        ))
        .await
        .expect("post resp");
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}
