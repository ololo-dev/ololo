#![allow(dead_code)]

//! Projects CRUD integration tests.
//!
//! Spec: contract.md FR-001 (project ownership), FR-002 (project list),
//! FR-005 (project delete RESTRICT), FR-006 (freeze-while-running),
//! FR-007 (archive via soft-retire), FR-008 (public flag).

use axum::http::{Method, StatusCode};
use server::build_router;
use tower::ServiceExt;

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

async fn create_project_private(
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
            Some(serde_json::json!({ "name": name, "public": false })),
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

async fn create_project_with_slug(
    app: &axum::Router,
    cookie: &str,
    name: &str,
    slug: Option<&str>,
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
    let (post_status, _, post_body) = read_body(resp).await;
    let slug = match slug {
        None => return (post_status, post_body),
        Some(_) if post_status != StatusCode::CREATED => return (post_status, post_body),
        Some(s) => s,
    };
    let pid = post_body["id"]
        .as_str()
        .expect("project id for slug patch")
        .to_string();
    let resp = app
        .clone()
        .oneshot(req(
            Method::PATCH,
            &format!("/api/projects/{pid}"),
            Some(cookie),
            Some(serde_json::json!({ "slug": slug })),
        ))
        .await
        .expect("patch slug resp");
    let (status, _, body) = read_body(resp).await;
    (status, body)
}

// ─────────────────────────── Tests ───────────────────────────

use crate::common;
use crate::common::*;

#[tokio::test]
async fn create_project_partial_points_uses_baseline_for_absent_fields() {
    let state = test_state().await;
    let app = build_router(state);
    let (_, cookie) = register_and_login(app.clone(), "alice@pts3.test", "password-12345").await;

    // Only `value` is overridden; the other three fall back to baseline.
    let resp = app
        .clone()
        .oneshot(req(
            Method::POST,
            "/api/projects",
            Some(&cookie),
            Some(serde_json::json!({
                "name": "pts-partial",
                "points": { "value": 99 }
            })),
        ))
        .await
        .expect("resp");
    let (status, _, body) = read_body(resp).await;
    assert_eq!(status, StatusCode::CREATED, "body={body}");
    assert_eq!(body["points"]["value"], serde_json::json!(99));
    assert_eq!(body["points"]["fail"], serde_json::json!(-5));
    assert_eq!(body["points"]["no_response"], serde_json::json!(-10));
    assert_eq!(body["points"]["completion_bonus"], serde_json::json!(10));
}

#[tokio::test]
async fn patch_project_updates_only_some_points_fields() {
    let state = test_state().await;
    let app = build_router(state);
    let (_, cookie) = register_and_login(app.clone(), "alice@pts4.test", "password-12345").await;

    let (_, body) = create_project(&app, &cookie, "pts-patch").await;
    let pid = body["id"].as_str().expect("project id");

    // Patch only `fail` and `completion_bonus`; value/no_response must be preserved.
    let resp = app
        .clone()
        .oneshot(req(
            Method::PATCH,
            &format!("/api/projects/{pid}"),
            Some(&cookie),
            Some(serde_json::json!({
                "points": { "fail": -99, "completion_bonus": 50 }
            })),
        ))
        .await
        .expect("resp");
    let (status, _, body) = read_body(resp).await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    assert_eq!(
        body["points"]["value"],
        serde_json::json!(10),
        "value preserved"
    );
    assert_eq!(
        body["points"]["fail"],
        serde_json::json!(-99),
        "fail updated"
    );
    assert_eq!(
        body["points"]["no_response"],
        serde_json::json!(-10),
        "no_response preserved"
    );
    assert_eq!(
        body["points"]["completion_bonus"],
        serde_json::json!(50),
        "completion_bonus updated"
    );
}

#[tokio::test]
async fn list_projects_omits_points_range() {
    let state = test_state().await;
    let app = build_router(state);
    let (_, cookie) = register_and_login(app.clone(), "alice@pts5.test", "password-12345").await;

    let (_, body) = create_project(&app, &cookie, "pts-list").await;
    let pid = body["id"].as_str().expect("project id");

    // Add a task so a range would exist if computed.
    let _ = app
        .clone()
        .oneshot(req(
            Method::POST,
            &format!("/api/projects/{pid}/tasks"),
            Some(&cookie),
            Some(serde_json::json!({
                "title": "t",
                "test_template": { "kind": "shell", "command_template": "echo hi" },
                "points": { "value": 25 }
            })),
        ))
        .await
        .expect("create task");

    let resp = app
        .clone()
        .oneshot(req(Method::GET, "/api/projects", Some(&cookie), None))
        .await
        .expect("list resp");
    let (_, _, body) = read_body(resp).await;
    let projects = body["projects"].as_array().expect("projects array");
    let mine = projects
        .iter()
        .find(|p| p["id"].as_str() == Some(pid))
        .expect("own project in list");
    // List endpoint must NOT compute points_range (N+1 avoidance).
    assert!(
        mine["points_range"].is_null(),
        "list must omit points_range; got {}",
        mine["points_range"]
    );
    // points (defaults) are still present.
    assert_eq!(mine["points"]["value"], serde_json::json!(10));
}

#[tokio::test]
async fn get_project_returns_points_range_from_tasks() {
    let state = test_state().await;
    let app = build_router(state);
    let (_, cookie) = register_and_login(app.clone(), "alice@pts6.test", "password-12345").await;

    let (_, body) = create_project(&app, &cookie, "pts-range").await;
    let pid = body["id"].as_str().expect("project id");

    // Add three tasks with point_value 10, 40, 60 → range 10..60.
    for pv in [10, 40, 60] {
        let _ = app
            .clone()
            .oneshot(req(
                Method::POST,
                &format!("/api/projects/{pid}/tasks"),
                Some(&cookie),
                Some(serde_json::json!({
                    "title": format!("t-{pv}"),
                    "test_template": { "kind": "shell", "command_template": "echo hi" },
                    "points": { "value": pv }
                })),
            ))
            .await
            .expect("create task");
    }

    let resp = app
        .clone()
        .oneshot(req(
            Method::GET,
            &format!("/api/projects/{pid}"),
            Some(&cookie),
            None,
        ))
        .await
        .expect("get resp");
    let (status, _, body) = read_body(resp).await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    assert_eq!(body["points_range"]["min"], serde_json::json!(10));
    assert_eq!(body["points_range"]["max"], serde_json::json!(60));
}
