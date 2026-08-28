#![allow(dead_code)]

//! Tasks CRUD integration tests.
//!
//! Spec: contract.md FR-003 (tasks are project-owned), FR-006 (freeze while
//! any non-terminal session — lobby or running — exists), FR-007 (ordinal
//! ordering), FR-020 (TestTemplate schema), FR-021 (placeholder rules).
//!
//! Write operations target `POST/PATCH/DELETE /api/projects/:id/tasks`.
//! Read operations use both project and session (read-through) endpoints.
//! Old `/api/sessions/:id/tasks` write endpoints return 410 (tombstone).

use axum::http::{Method, StatusCode};
use chrono::Utc;
use sea_orm::{ActiveModelTrait, Set};
use server::entities::players;
use server::{AppState, build_router};
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
    let (s, _, b) = read_body(resp).await;
    (s, b)
}

async fn create_session(
    app: &axum::Router,
    cookie: &str,
    name: &str,
    project_id: &str,
) -> (StatusCode, serde_json::Value) {
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
    let (s, _, b) = read_body(resp).await;
    (s, b)
}

/// Create a task via `POST /api/projects/:project_id/tasks`.
async fn create_task(
    app: &axum::Router,
    cookie: &str,
    project_id: &str,
    body: serde_json::Value,
) -> (StatusCode, serde_json::Value) {
    let resp = app
        .clone()
        .oneshot(req(
            Method::POST,
            &format!("/api/projects/{project_id}/tasks"),
            Some(cookie),
            Some(body),
        ))
        .await
        .expect("create task resp");
    let (s, _, b) = read_body(resp).await;
    (s, b)
}

async fn add_member(state: &AppState, sid: &str, user_id: Uuid) {
    let session_id = Uuid::parse_str(sid).expect("valid session uuid");
    let am = players::ActiveModel {
        id: Set(Uuid::new_v4()),
        session_id_fk: Set(session_id),
        user_id_fk: Set(Some(user_id)),
        display_name: Set("member".into()),
        fingerprint: Set(None),
        metadata_json: Set(None),
        joined_at: Set(Utc::now()),
        reconnected_at: Set(None),
        revoked_at: Set(None),
        agent_connected: Set(false),
        agent_last_seen_at: Set(None),
    };
    am.insert(&state.db).await.expect("insert member");
}

/// A minimal valid `TestTemplate` JSON suitable for happy-path tests.
fn minimal_template() -> serde_json::Value {
    serde_json::json!({
        "kind": "shell",
        "command_template": "echo hi",
    })
}

// ─────────────────────────── Tests ───────────────────────────

use crate::common;
use crate::common::*;

#[tokio::test]
async fn session_task_write_endpoints_return_410() {
    let state = test_state().await;
    let app = build_router(state);
    let (_, cookie_a) = register_and_login(app.clone(), "alice@x.test", "password-12345").await;
    let (_, project) = create_project(&app, &cookie_a, "p").await;
    let pid = project["id"].as_str().expect("pid");
    let (_, sess) = create_session(&app, &cookie_a, "arena", pid).await;
    let sid = sess["id"].as_str().expect("sid");
    let fake_tid = Uuid::new_v4();

    // POST (create)
    let resp = app
        .clone()
        .oneshot(req(
            Method::POST,
            &format!("/api/sessions/{sid}/tasks"),
            Some(&cookie_a),
            Some(serde_json::json!({"title":"t","test_template":minimal_template()})),
        ))
        .await
        .expect("post resp");
    let (status, _, body) = read_body(resp).await;
    assert_eq!(status, StatusCode::GONE, "post: {body}");
    assert_eq!(body["error"], serde_json::json!("route_gone"));
    assert!(body["moved_to"].as_str().unwrap_or("").contains(pid));

    // PATCH (update)
    let resp = app
        .clone()
        .oneshot(req(
            Method::PATCH,
            &format!("/api/sessions/{sid}/tasks/{fake_tid}"),
            Some(&cookie_a),
            Some(serde_json::json!({"title":"n"})),
        ))
        .await
        .expect("patch resp");
    assert_eq!(resp.status(), StatusCode::GONE);

    // DELETE
    let resp = app
        .oneshot(req(
            Method::DELETE,
            &format!("/api/sessions/{sid}/tasks/{fake_tid}"),
            Some(&cookie_a),
            None,
        ))
        .await
        .expect("delete resp");
    assert_eq!(resp.status(), StatusCode::GONE);
}

#[tokio::test]
async fn unauth_request_401() {
    let state = test_state().await;
    let app = build_router(state);
    let fake_pid = Uuid::new_v4();
    let fake_tid = Uuid::new_v4();

    let cases: &[(Method, String, Option<serde_json::Value>)] = &[
        (
            Method::POST,
            format!("/api/projects/{fake_pid}/tasks"),
            Some(serde_json::json!({"title":"t","test_template":minimal_template()})),
        ),
        (Method::GET, format!("/api/projects/{fake_pid}/tasks"), None),
        (
            Method::PATCH,
            format!("/api/projects/{fake_pid}/tasks/{fake_tid}"),
            Some(serde_json::json!({"title":"n"})),
        ),
        (
            Method::DELETE,
            format!("/api/projects/{fake_pid}/tasks/{fake_tid}"),
            None,
        ),
    ];
    for (m, uri, body) in cases {
        let resp = app
            .clone()
            .oneshot(req(m.clone(), uri, None, body.clone()))
            .await
            .expect("req");
        assert_eq!(
            resp.status(),
            StatusCode::UNAUTHORIZED,
            "method={m} uri={uri}"
        );
    }
}

// ─────────────────────────── Tag tests (AC-005 – AC-009) ─────────────────────

/// AC-005: POST with tags returns them in the created resource.
#[tokio::test]
async fn create_task_with_tags_roundtrip() {
    let state = test_state().await;
    let app = build_router(state);
    let (_, cookie) = register_and_login(app.clone(), "alice@x.test", "password-12345").await;
    let (_, project) = create_project(&app, &cookie, "p-tags").await;
    let pid = project["id"].as_str().expect("pid");

    let (status, body) = create_task(
        &app,
        &cookie,
        pid,
        serde_json::json!({
            "title": "tagged-task",
            "tags": ["rust", "async"],
            "test_template": minimal_template(),
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "body={body}");
    let tags = body["tags"].as_array().expect("tags array");
    let tag_strs: Vec<&str> = tags.iter().filter_map(|v| v.as_str()).collect();
    assert_eq!(tag_strs, vec!["rust", "async"]);
}

/// AC-006: POST without tags field returns tags: [].
#[tokio::test]
async fn create_task_no_tags_defaults_empty() {
    let state = test_state().await;
    let app = build_router(state);
    let (_, cookie) = register_and_login(app.clone(), "alice@x.test", "password-12345").await;
    let (_, project) = create_project(&app, &cookie, "p-no-tags").await;
    let pid = project["id"].as_str().expect("pid");

    let (status, body) = create_task(
        &app,
        &cookie,
        pid,
        serde_json::json!({
            "title": "untagged",
            "test_template": minimal_template(),
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "body={body}");
    assert_eq!(body["tags"], serde_json::json!([]));
}

/// AC-007: PATCH with tags: [] clears all existing tags.
#[tokio::test]
async fn patch_task_clear_tags() {
    let state = test_state().await;
    let app = build_router(state);
    let (_, cookie) = register_and_login(app.clone(), "alice@x.test", "password-12345").await;
    let (_, project) = create_project(&app, &cookie, "p-clear-tags").await;
    let pid = project["id"].as_str().expect("pid");

    // Create with tags.
    let (s, body) = create_task(
        &app,
        &cookie,
        pid,
        serde_json::json!({
            "title": "t",
            "tags": ["backend", "api"],
            "test_template": minimal_template(),
        }),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "create: {body}");
    let tid = body["id"].as_str().expect("tid");

    // PATCH with empty tags.
    let resp = app
        .clone()
        .oneshot(req(
            Method::PATCH,
            &format!("/api/projects/{pid}/tasks/{tid}"),
            Some(&cookie),
            Some(serde_json::json!({"tags": []})),
        ))
        .await
        .expect("patch resp");
    let (status, _, patch_body) = read_body(resp).await;
    assert_eq!(status, StatusCode::OK, "patch: {patch_body}");
    assert_eq!(patch_body["tags"], serde_json::json!([]));

    // GET to confirm persisted.
    let resp = app
        .oneshot(req(
            Method::GET,
            &format!("/api/projects/{pid}/tasks/{tid}"),
            Some(&cookie),
            None,
        ))
        .await
        .expect("get resp");
    let (status, _, get_body) = read_body(resp).await;
    assert_eq!(status, StatusCode::OK, "get: {get_body}");
    assert_eq!(get_body["tags"], serde_json::json!([]));
}

/// AC-008: PATCH without tags field preserves existing tags.
#[tokio::test]
async fn patch_task_omit_tags_preserves_them() {
    let state = test_state().await;
    let app = build_router(state);
    let (_, cookie) = register_and_login(app.clone(), "alice@x.test", "password-12345").await;
    let (_, project) = create_project(&app, &cookie, "p-preserve-tags").await;
    let pid = project["id"].as_str().expect("pid");

    let (s, body) = create_task(
        &app,
        &cookie,
        pid,
        serde_json::json!({
            "title": "t",
            "tags": ["keep-me"],
            "test_template": minimal_template(),
        }),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED);
    let tid = body["id"].as_str().expect("tid");

    // Patch only the title — no tags field in body.
    let resp = app
        .clone()
        .oneshot(req(
            Method::PATCH,
            &format!("/api/projects/{pid}/tasks/{tid}"),
            Some(&cookie),
            Some(serde_json::json!({"title": "renamed"})),
        ))
        .await
        .expect("patch resp");
    let (status, _, patch_body) = read_body(resp).await;
    assert_eq!(status, StatusCode::OK, "patch: {patch_body}");
    assert_eq!(patch_body["tags"], serde_json::json!(["keep-me"]));

    // GET to confirm.
    let resp = app
        .oneshot(req(
            Method::GET,
            &format!("/api/projects/{pid}/tasks/{tid}"),
            Some(&cookie),
            None,
        ))
        .await
        .expect("get resp");
    let (status, _, get_body) = read_body(resp).await;
    assert_eq!(status, StatusCode::OK, "get: {get_body}");
    assert_eq!(get_body["tags"], serde_json::json!(["keep-me"]));
}

/// AC-009: POST with 21 tags returns 422.
#[tokio::test]
async fn create_task_too_many_tags_422() {
    let state = test_state().await;
    let app = build_router(state);
    let (_, cookie) = register_and_login(app.clone(), "alice@x.test", "password-12345").await;
    let (_, project) = create_project(&app, &cookie, "p-too-many").await;
    let pid = project["id"].as_str().expect("pid");

    let tags: Vec<String> = (0..21).map(|i| format!("tag-{i}")).collect();
    let (status, body) = create_task(
        &app,
        &cookie,
        pid,
        serde_json::json!({
            "title": "t",
            "tags": tags,
            "test_template": minimal_template(),
        }),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "body={body}");
    assert_eq!(body["error"], serde_json::json!("invalid_tags"));
}
