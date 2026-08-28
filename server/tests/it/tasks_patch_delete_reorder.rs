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
async fn patch_task_project_frozen_409() {
    let state = test_state().await;
    let app = build_router(state);
    let (_, cookie_a) = register_and_login(app.clone(), "alice@x.test", "password-12345").await;
    let (_, project) = create_project(&app, &cookie_a, "p").await;
    let pid = project["id"].as_str().expect("pid");

    let (_, body) = create_task(
        &app,
        &cookie_a,
        pid,
        serde_json::json!({"title":"t","test_template":minimal_template()}),
    )
    .await;
    let tid = body["id"].as_str().expect("tid");

    // Create session referencing project (lobby → frozen).
    let (ss, s_body) = create_session(&app, &cookie_a, "arena", pid).await;
    assert_eq!(ss, StatusCode::CREATED, "session: {s_body}");

    let resp = app
        .oneshot(req(
            Method::PATCH,
            &format!("/api/projects/{pid}/tasks/{tid}"),
            Some(&cookie_a),
            Some(serde_json::json!({"title": "renamed"})),
        ))
        .await
        .expect("patch task");
    let (status, _, body) = read_body(resp).await;
    assert_eq!(status, StatusCode::CONFLICT, "body={body}");
    assert_eq!(body["error"], serde_json::json!("project_frozen"));
}

#[tokio::test]
async fn patch_task_changes_template_revalidates_422() {
    let state = test_state().await;
    let app = build_router(state);
    let (_, cookie_a) = register_and_login(app.clone(), "alice@x.test", "password-12345").await;
    let (_, project) = create_project(&app, &cookie_a, "p").await;
    let pid = project["id"].as_str().expect("pid");

    let (_, body) = create_task(
        &app,
        &cookie_a,
        pid,
        serde_json::json!({"title":"t","test_template":minimal_template()}),
    )
    .await;
    let tid = body["id"].as_str().expect("tid");

    let bad = serde_json::json!({"kind": "shell", "command_template": ""}); // empty
    let resp = app
        .oneshot(req(
            Method::PATCH,
            &format!("/api/projects/{pid}/tasks/{tid}"),
            Some(&cookie_a),
            Some(serde_json::json!({"test_template": bad})),
        ))
        .await
        .expect("patch resp");
    let (status, _, body) = read_body(resp).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "body={body}");
    assert_eq!(body["error"], serde_json::json!("invalid_template"));
}

#[tokio::test]
async fn patch_task_ordinal_collision_409() {
    let state = test_state().await;
    let app = build_router(state);
    let (_, cookie_a) = register_and_login(app.clone(), "alice@x.test", "password-12345").await;
    let (_, project) = create_project(&app, &cookie_a, "p").await;
    let pid = project["id"].as_str().expect("pid");

    let (_, b1) = create_task(
        &app,
        &cookie_a,
        pid,
        serde_json::json!({"title":"a","ordinal":0,"test_template":minimal_template()}),
    )
    .await;
    let _t1 = b1["id"].as_str().expect("id");
    let (_, b2) = create_task(
        &app,
        &cookie_a,
        pid,
        serde_json::json!({"title":"b","ordinal":1,"test_template":minimal_template()}),
    )
    .await;
    let t2 = b2["id"].as_str().expect("id");

    let resp = app
        .oneshot(req(
            Method::PATCH,
            &format!("/api/projects/{pid}/tasks/{t2}"),
            Some(&cookie_a),
            Some(serde_json::json!({"ordinal": 0})),
        ))
        .await
        .expect("patch resp");
    let (status, _, body) = read_body(resp).await;
    assert_eq!(status, StatusCode::CONFLICT, "body={body}");
    assert_eq!(body, serde_json::json!({"error":"ordinal_taken"}));
}

#[tokio::test]
async fn delete_task_owner_no_sessions_204() {
    let state = test_state().await;
    let app = build_router(state);
    let (_, cookie_a) = register_and_login(app.clone(), "alice@x.test", "password-12345").await;
    let (_, project) = create_project(&app, &cookie_a, "p").await;
    let pid = project["id"].as_str().expect("pid");

    let (_, body) = create_task(
        &app,
        &cookie_a,
        pid,
        serde_json::json!({"title":"t","test_template":minimal_template()}),
    )
    .await;
    let tid = body["id"].as_str().expect("tid");

    let resp = app
        .clone()
        .oneshot(req(
            Method::DELETE,
            &format!("/api/projects/{pid}/tasks/{tid}"),
            Some(&cookie_a),
            None,
        ))
        .await
        .expect("delete resp");
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);

    // Verify it is gone.
    let resp = app
        .oneshot(req(
            Method::GET,
            &format!("/api/projects/{pid}/tasks/{tid}"),
            Some(&cookie_a),
            None,
        ))
        .await
        .expect("get resp");
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

/// DELETE blocked when project is frozen (lobby session exists).
#[tokio::test]
async fn delete_task_project_frozen_409() {
    let state = test_state().await;
    let app = build_router(state);
    let (_, cookie_a) = register_and_login(app.clone(), "alice@x.test", "password-12345").await;
    let (_, project) = create_project(&app, &cookie_a, "p").await;
    let pid = project["id"].as_str().expect("pid");

    let (_, body) = create_task(
        &app,
        &cookie_a,
        pid,
        serde_json::json!({"title":"t","test_template":minimal_template()}),
    )
    .await;
    let tid = body["id"].as_str().expect("tid");

    // Create a session (lobby) → project frozen.
    let (ss, _) = create_session(&app, &cookie_a, "arena", pid).await;
    assert_eq!(ss, StatusCode::CREATED);

    let resp = app
        .oneshot(req(
            Method::DELETE,
            &format!("/api/projects/{pid}/tasks/{tid}"),
            Some(&cookie_a),
            None,
        ))
        .await
        .expect("delete resp");
    let (status, _, body) = read_body(resp).await;
    assert_eq!(status, StatusCode::CONFLICT, "body={body}");
    assert_eq!(body["error"], serde_json::json!("project_frozen"));
}

// ─────────────────────────── Reorder tests ───────────────────────────────

/// Create 3 tasks, reorder them in reverse, verify ordinals match new order.
#[tokio::test]
async fn test_reorder_tasks() {
    let state = test_state().await;
    let app = build_router(state);
    let (_, cookie_a) = register_and_login(app.clone(), "alice@x.test", "password-12345").await;
    let (_, project) = create_project(&app, &cookie_a, "p-reorder").await;
    let pid = project["id"].as_str().expect("pid");

    // Create 3 tasks with explicit ordinals 0, 1, 2.
    let mut task_ids = Vec::new();
    for i in 0..3_i32 {
        let (s, body) = create_task(
            &app,
            &cookie_a,
            pid,
            serde_json::json!({
                "title": format!("task-{i}"),
                "ordinal": i,
                "test_template": minimal_template(),
            }),
        )
        .await;
        assert_eq!(s, StatusCode::CREATED, "create task {i}: {body}");
        task_ids.push(body["id"].as_str().expect("task id").to_string());
    }

    // Reorder in reverse: [2, 1, 0].
    let reversed: Vec<&str> = task_ids.iter().rev().map(|s| s.as_str()).collect();
    let resp = app
        .clone()
        .oneshot(req(
            Method::PATCH,
            &format!("/api/projects/{pid}/tasks/reorder"),
            Some(&cookie_a),
            Some(serde_json::json!({ "task_ids": reversed })),
        ))
        .await
        .expect("reorder resp");
    let (status, _, body) = read_body(resp).await;
    assert_eq!(status, StatusCode::OK, "reorder body={body}");
    assert_eq!(body["ok"], serde_json::json!(true));

    // Verify ordinals by fetching task list.
    let resp = app
        .oneshot(req(
            Method::GET,
            &format!("/api/projects/{pid}/tasks"),
            Some(&cookie_a),
            None,
        ))
        .await
        .expect("list resp");
    let (status, _, list_body) = read_body(resp).await;
    assert_eq!(status, StatusCode::OK, "list body={list_body}");

    let tasks = list_body["tasks"].as_array().expect("tasks array");
    // After reorder, tasks ordered by ordinal: reversed[0] has ordinal 1,
    // reversed[1] has ordinal 2, reversed[2] has ordinal 3 (1-based).
    // Actually ordinals are 1..=3 per the implementation (index+1).
    // Check that the first task in ordered list is the last original task.
    assert_eq!(tasks[0]["id"], serde_json::json!(reversed[0]));
    assert_eq!(tasks[1]["id"], serde_json::json!(reversed[1]));
    assert_eq!(tasks[2]["id"], serde_json::json!(reversed[2]));
}

/// Reorder with a task_id that doesn't belong to the project → 422.
#[tokio::test]
async fn test_reorder_invalid_task_ids() {
    let state = test_state().await;
    let app = build_router(state);
    let (_, cookie_a) = register_and_login(app.clone(), "alice@x.test", "password-12345").await;
    let (_, project) = create_project(&app, &cookie_a, "p-invalid-reorder").await;
    let pid = project["id"].as_str().expect("pid");

    let (s, body) = create_task(
        &app,
        &cookie_a,
        pid,
        serde_json::json!({"title":"t","test_template":minimal_template()}),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED);
    let _tid = body["id"].as_str().expect("tid");

    // Use a foreign task id.
    let foreign_id = Uuid::new_v4().to_string();
    let resp = app
        .oneshot(req(
            Method::PATCH,
            &format!("/api/projects/{pid}/tasks/reorder"),
            Some(&cookie_a),
            Some(serde_json::json!({ "task_ids": [foreign_id] })),
        ))
        .await
        .expect("reorder resp");
    let (status, _, body) = read_body(resp).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "body={body}");
    assert_eq!(body["error"], serde_json::json!("invalid_task_ids"));
}

/// Reorder on a frozen project → 409 project_frozen.
#[tokio::test]
async fn test_reorder_frozen_project() {
    let state = test_state().await;
    let app = build_router(state);
    let (_, cookie_a) = register_and_login(app.clone(), "alice@x.test", "password-12345").await;
    let (_, project) = create_project(&app, &cookie_a, "p-frozen-reorder").await;
    let pid = project["id"].as_str().expect("pid");

    let (s, body) = create_task(
        &app,
        &cookie_a,
        pid,
        serde_json::json!({"title":"t","test_template":minimal_template()}),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED);
    let tid = body["id"].as_str().expect("tid").to_string();

    // Freeze the project by creating a session (lobby).
    let (ss, _) = create_session(&app, &cookie_a, "arena", pid).await;
    assert_eq!(ss, StatusCode::CREATED);

    let resp = app
        .oneshot(req(
            Method::PATCH,
            &format!("/api/projects/{pid}/tasks/reorder"),
            Some(&cookie_a),
            Some(serde_json::json!({ "task_ids": [tid] })),
        ))
        .await
        .expect("reorder resp");
    let (status, _, body) = read_body(resp).await;
    assert_eq!(status, StatusCode::CONFLICT, "body={body}");
    assert_eq!(body["error"], serde_json::json!("project_frozen"));
}

// Old session-scoped write endpoints return 410 Gone with a `moved_to` pointer.
