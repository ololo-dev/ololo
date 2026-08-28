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
async fn create_task_owner_no_sessions_201() {
    let state = test_state().await;
    let app = build_router(state);
    let (_, cookie_a) = register_and_login(app.clone(), "alice@x.test", "password-12345").await;
    let (_, project) = create_project(&app, &cookie_a, "my-project").await;
    let pid = project["id"].as_str().expect("project id");

    let (status, body) = create_task(
        &app,
        &cookie_a,
        pid,
        serde_json::json!({
            "title": "task-1",
            "description": "first",
            "test_template": minimal_template(),
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "body={body}");
    assert_eq!(body["ordinal"], serde_json::json!(0));
    assert_eq!(body["title"], serde_json::json!("task-1"));
    assert_eq!(body["project_id"], serde_json::json!(pid));
    assert!(body["id"].is_string());
    assert_eq!(body["test_template"]["kind"], serde_json::json!("shell"));
}

#[tokio::test]
async fn create_task_auto_increments_ordinal() {
    let state = test_state().await;
    let app = build_router(state);
    let (_, cookie_a) = register_and_login(app.clone(), "alice@x.test", "password-12345").await;
    let (_, project) = create_project(&app, &cookie_a, "p").await;
    let pid = project["id"].as_str().expect("pid");

    let mut ords = Vec::new();
    for i in 0..3 {
        let (status, body) = create_task(
            &app,
            &cookie_a,
            pid,
            serde_json::json!({
                "title": format!("t-{i}"),
                "test_template": minimal_template(),
            }),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "i={i} body={body}");
        ords.push(body["ordinal"].as_i64().expect("ord"));
    }
    assert_eq!(ords, vec![0, 1, 2]);
}

#[tokio::test]
async fn create_task_explicit_ordinal_ok() {
    let state = test_state().await;
    let app = build_router(state);
    let (_, cookie_a) = register_and_login(app.clone(), "alice@x.test", "password-12345").await;
    let (_, project) = create_project(&app, &cookie_a, "p").await;
    let pid = project["id"].as_str().expect("pid");

    let (status, body) = create_task(
        &app,
        &cookie_a,
        pid,
        serde_json::json!({
            "title": "t",
            "ordinal": 7,
            "test_template": minimal_template(),
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "body={body}");
    assert_eq!(body["ordinal"], serde_json::json!(7));
}

#[tokio::test]
async fn create_task_ordinal_collision_409() {
    let state = test_state().await;
    let app = build_router(state);
    let (_, cookie_a) = register_and_login(app.clone(), "alice@x.test", "password-12345").await;
    let (_, project) = create_project(&app, &cookie_a, "p").await;
    let pid = project["id"].as_str().expect("pid");

    let (s1, _) = create_task(
        &app,
        &cookie_a,
        pid,
        serde_json::json!({"title":"first","ordinal":5,"test_template":minimal_template()}),
    )
    .await;
    assert_eq!(s1, StatusCode::CREATED);

    let (s2, body) = create_task(
        &app,
        &cookie_a,
        pid,
        serde_json::json!({"title":"second","ordinal":5,"test_template":minimal_template()}),
    )
    .await;
    assert_eq!(s2, StatusCode::CONFLICT);
    assert_eq!(body, serde_json::json!({"error": "ordinal_taken"}));
}

#[tokio::test]
async fn create_task_invalid_template_422_with_detail() {
    let state = test_state().await;
    let app = build_router(state);
    let (_, cookie_a) = register_and_login(app.clone(), "alice@x.test", "password-12345").await;
    let (_, project) = create_project(&app, &cookie_a, "p").await;
    let pid = project["id"].as_str().expect("pid");

    // declared placeholder `ghost` not referenced in command_template.
    let bad = serde_json::json!({
        "kind": "shell",
        "command_template": "echo hi",
        "placeholders": [
            {"name": "ghost", "description": "d", "required": true, "secret": false}
        ]
    });
    let (status, body) = create_task(
        &app,
        &cookie_a,
        pid,
        serde_json::json!({"title": "t", "test_template": bad}),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "body={body}");
    assert_eq!(body["error"], serde_json::json!("invalid_template"));
    let detail = body["detail"].as_str().expect("detail string");
    assert!(
        detail.contains("not referenced") || detail.contains("unreferenced"),
        "detail should mention unreferenced; got {detail}"
    );
}

#[tokio::test]
async fn create_task_title_too_long_422() {
    let state = test_state().await;
    let app = build_router(state);
    let (_, cookie_a) = register_and_login(app.clone(), "alice@x.test", "password-12345").await;
    let (_, project) = create_project(&app, &cookie_a, "p").await;
    let pid = project["id"].as_str().expect("pid");

    let long = "x".repeat(201);
    let (status, _) = create_task(
        &app,
        &cookie_a,
        pid,
        serde_json::json!({"title": long, "test_template": minimal_template()}),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
}

/// Project is frozen (lobby session exists) → cannot add tasks.
#[tokio::test]
async fn create_task_project_frozen_409() {
    let state = test_state().await;
    let app = build_router(state);
    let (_, cookie_a) = register_and_login(app.clone(), "alice@x.test", "password-12345").await;
    let (_, project) = create_project(&app, &cookie_a, "p").await;
    let pid = project["id"].as_str().expect("pid");

    // Create one task while project is unfrozen.
    let (s1, _) = create_task(
        &app,
        &cookie_a,
        pid,
        serde_json::json!({"title":"t","test_template":minimal_template()}),
    )
    .await;
    assert_eq!(s1, StatusCode::CREATED);

    // Create a session (lobby) — project is now frozen.
    let (ss, _) = create_session(&app, &cookie_a, "arena", pid).await;
    assert_eq!(ss, StatusCode::CREATED);

    // Try to add another task — must be blocked.
    let (status, body) = create_task(
        &app,
        &cookie_a,
        pid,
        serde_json::json!({"title":"late","test_template":minimal_template()}),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "body={body}");
    assert_eq!(body["error"], serde_json::json!("project_frozen"));
}

/// Non-owner of a private project → 403 on write.
#[tokio::test]
async fn create_task_nonowner_private_project_403() {
    let state = test_state().await;
    let app = build_router(state);
    let (_, cookie_a) = register_and_login(app.clone(), "alice@x.test", "password-12345").await;
    let (_, cookie_b) = register_and_login(app.clone(), "bob@x.test", "password-12345").await;
    let (_, project) = create_project(&app, &cookie_a, "alice-private").await;
    let pid = project["id"].as_str().expect("pid");

    let (status, _) = create_task(
        &app,
        &cookie_b,
        pid,
        serde_json::json!({"title":"t","test_template":minimal_template()}),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

/// Session member of an alice session can read project tasks via the session
/// read-through endpoint.
#[tokio::test]
async fn list_tasks_member_sees_ordered() {
    let state = test_state().await;
    let app = build_router(state.clone());
    let (_, cookie_a) = register_and_login(app.clone(), "alice@x.test", "password-12345").await;
    let (uid_b, cookie_b) = register_and_login(app.clone(), "bob@x.test", "password-12345").await;

    // Create project and tasks (no sessions yet → not frozen).
    let (_, project) = create_project(&app, &cookie_a, "p").await;
    let pid = project["id"].as_str().expect("pid");
    for ord in [2_i32, 0, 1] {
        let (s, _) = create_task(
            &app,
            &cookie_a,
            pid,
            serde_json::json!({
                "title": format!("t-{ord}"),
                "ordinal": ord,
                "test_template": minimal_template(),
            }),
        )
        .await;
        assert_eq!(s, StatusCode::CREATED);
    }

    // Now create a session referencing the project and add bob as member.
    let (_, sess) = create_session(&app, &cookie_a, "arena", pid).await;
    let sid = sess["id"].as_str().expect("sid").to_string();
    add_member(&state, &sid, uid_b).await;

    // Bob reads tasks via the session read-through endpoint.
    let resp = app
        .oneshot(req(
            Method::GET,
            &format!("/api/sessions/{sid}/tasks"),
            Some(&cookie_b),
            None,
        ))
        .await
        .expect("list resp");
    let (status, _, body) = read_body(resp).await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    let arr = body["tasks"].as_array().expect("tasks array");
    let ords: Vec<i64> = arr.iter().map(|t| t["ordinal"].as_i64().unwrap()).collect();
    assert_eq!(ords, vec![0, 1, 2]);
}

#[tokio::test]
async fn get_task_member_200() {
    let state = test_state().await;
    let app = build_router(state.clone());
    let (_, cookie_a) = register_and_login(app.clone(), "alice@x.test", "password-12345").await;
    let (uid_b, cookie_b) = register_and_login(app.clone(), "bob@x.test", "password-12345").await;

    // Create project + task (no sessions yet).
    let (_, project) = create_project(&app, &cookie_a, "p").await;
    let pid = project["id"].as_str().expect("pid");
    let (_, task_body) = create_task(
        &app,
        &cookie_a,
        pid,
        serde_json::json!({"title":"t","test_template":minimal_template()}),
    )
    .await;
    let tid = task_body["id"].as_str().expect("tid");

    // Create a session referencing the project; add bob as member.
    let (_, sess) = create_session(&app, &cookie_a, "arena", pid).await;
    let sid = sess["id"].as_str().expect("sid").to_string();
    add_member(&state, &sid, uid_b).await;

    // Bob reads the task via the session read-through endpoint.
    let resp = app
        .oneshot(req(
            Method::GET,
            &format!("/api/sessions/{sid}/tasks/{tid}"),
            Some(&cookie_b),
            None,
        ))
        .await
        .expect("get task resp");
    let (status, _, body) = read_body(resp).await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    assert_eq!(body["id"], serde_json::json!(tid));
}

#[tokio::test]
async fn patch_task_owner_renames_200() {
    let state = test_state().await;
    let app = build_router(state);
    let (_, cookie_a) = register_and_login(app.clone(), "alice@x.test", "password-12345").await;
    let (_, project) = create_project(&app, &cookie_a, "p").await;
    let pid = project["id"].as_str().expect("pid");

    let (_, body) = create_task(
        &app,
        &cookie_a,
        pid,
        serde_json::json!({"title":"old","test_template":minimal_template()}),
    )
    .await;
    let tid = body["id"].as_str().expect("tid");

    let resp = app
        .oneshot(req(
            Method::PATCH,
            &format!("/api/projects/{pid}/tasks/{tid}"),
            Some(&cookie_a),
            Some(serde_json::json!({"title": "new"})),
        ))
        .await
        .expect("patch resp");
    let (status, _, body) = read_body(resp).await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    assert_eq!(body["title"], serde_json::json!("new"));
}

// PATCH blocked when a non-terminal session exists (project frozen).
