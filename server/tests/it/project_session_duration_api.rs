//! Project REST API surface for `session_duration_secs`.
//!
//! The project's session duration is settable through the existing project
//! create/edit API alongside the sibling `default_*` fields, with bounds
//! validation (60..=86400 inclusive) delegated to arena-core validation.

use axum::http::{Method, StatusCode};
use server::build_router;
use tower::ServiceExt;

use crate::common;
use crate::common::*;

async fn create_project(
    app: &axum::Router,
    cookie: &str,
    body: serde_json::Value,
) -> (StatusCode, serde_json::Value) {
    let resp = app
        .clone()
        .oneshot(req(Method::POST, "/api/projects", Some(cookie), Some(body)))
        .await
        .expect("create project resp");
    read_body_json(resp).await
}

#[tokio::test]
async fn create_project_with_explicit_session_duration_round_trips() {
    let state = test_state().await;
    let app = build_router(state);
    let (_, cookie) = register_and_login(app.clone(), "alice@sd1.test", "password-12345").await;

    let (status, body) = create_project(
        &app,
        &cookie,
        serde_json::json!({ "name": "sd-explicit", "session_duration_secs": 7200 }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "body={body}");
    assert_eq!(body["session_duration_secs"], serde_json::json!(7200));

    // Detail read returns the stored value.
    let pid = body["id"].as_str().expect("project id");
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
    let (status, body) = read_body_json(resp).await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    assert_eq!(body["session_duration_secs"], serde_json::json!(7200));
}

#[tokio::test]
async fn create_project_without_session_duration_defaults_to_3600() {
    let state = test_state().await;
    let app = build_router(state);
    let (_, cookie) = register_and_login(app.clone(), "alice@sd2.test", "password-12345").await;

    let (status, body) =
        create_project(&app, &cookie, serde_json::json!({ "name": "sd-default" })).await;
    assert_eq!(status, StatusCode::CREATED, "body={body}");
    assert_eq!(body["session_duration_secs"], serde_json::json!(3600));
}

#[tokio::test]
async fn patch_project_updates_session_duration() {
    let state = test_state().await;
    let app = build_router(state);
    let (_, cookie) = register_and_login(app.clone(), "alice@sd3.test", "password-12345").await;

    let (_, body) = create_project(&app, &cookie, serde_json::json!({ "name": "sd-patch" })).await;
    let pid = body["id"].as_str().expect("project id");

    let resp = app
        .clone()
        .oneshot(req(
            Method::PATCH,
            &format!("/api/projects/{pid}"),
            Some(&cookie),
            Some(serde_json::json!({ "session_duration_secs": 5400 })),
        ))
        .await
        .expect("patch resp");
    let (status, body) = read_body_json(resp).await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    assert_eq!(body["session_duration_secs"], serde_json::json!(5400));

    // Persisted: detail read returns the updated value.
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
    let (status, body) = read_body_json(resp).await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    assert_eq!(body["session_duration_secs"], serde_json::json!(5400));
}

/// POST with duration 59 (below minimum 60) → 422 invalid_session_duration.
#[tokio::test]
async fn create_project_session_duration_below_min_422() {
    let state = test_state().await;
    let app = build_router(state);
    let (_, cookie) = register_and_login(app.clone(), "alice@sd4.test", "password-12345").await;

    let (status, body) = create_project(
        &app,
        &cookie,
        serde_json::json!({ "name": "sd-too-short", "session_duration_secs": 59 }),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "body={body}");
    assert_eq!(body["error"], serde_json::json!("invalid_session_duration"));
}

/// POST with duration 86401 (above maximum 86400) → 422 invalid_session_duration.
#[tokio::test]
async fn create_project_session_duration_above_max_422() {
    let state = test_state().await;
    let app = build_router(state);
    let (_, cookie) = register_and_login(app.clone(), "alice@sd5.test", "password-12345").await;

    let (status, body) = create_project(
        &app,
        &cookie,
        serde_json::json!({ "name": "sd-too-long", "session_duration_secs": 86401 }),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "body={body}");
    assert_eq!(body["error"], serde_json::json!("invalid_session_duration"));
}

/// Bounds are inclusive: 60 and 86400 are both accepted.
#[tokio::test]
async fn create_project_session_duration_accepts_inclusive_bounds() {
    let state = test_state().await;
    let app = build_router(state);
    let (_, cookie) = register_and_login(app.clone(), "alice@sd6.test", "password-12345").await;

    let (status, body) = create_project(
        &app,
        &cookie,
        serde_json::json!({ "name": "sd-min", "session_duration_secs": 60 }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "body={body}");
    assert_eq!(body["session_duration_secs"], serde_json::json!(60));

    let (status, body) = create_project(
        &app,
        &cookie,
        serde_json::json!({ "name": "sd-max", "session_duration_secs": 86400 }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "body={body}");
    assert_eq!(body["session_duration_secs"], serde_json::json!(86400));
}

/// PATCH with an out-of-bounds duration → 422; stored value is unchanged.
#[tokio::test]
async fn patch_project_session_duration_invalid_422_leaves_value_unchanged() {
    let state = test_state().await;
    let app = build_router(state);
    let (_, cookie) = register_and_login(app.clone(), "alice@sd7.test", "password-12345").await;

    let (_, body) =
        create_project(&app, &cookie, serde_json::json!({ "name": "sd-patch-bad" })).await;
    let pid = body["id"].as_str().expect("project id");

    let resp = app
        .clone()
        .oneshot(req(
            Method::PATCH,
            &format!("/api/projects/{pid}"),
            Some(&cookie),
            Some(serde_json::json!({ "session_duration_secs": 59 })),
        ))
        .await
        .expect("patch resp");
    let (status, body) = read_body_json(resp).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "body={body}");
    assert_eq!(body["error"], serde_json::json!("invalid_session_duration"));

    // Value untouched by the rejected patch.
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
    let (status, body) = read_body_json(resp).await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    assert_eq!(body["session_duration_secs"], serde_json::json!(3600));
}

/// The list endpoint carries the field on each summary (same DTO as detail).
#[tokio::test]
async fn list_projects_includes_session_duration() {
    let state = test_state().await;
    let app = build_router(state);
    let (_, cookie) = register_and_login(app.clone(), "alice@sd8.test", "password-12345").await;

    let (_, body) = create_project(
        &app,
        &cookie,
        serde_json::json!({ "name": "sd-list", "session_duration_secs": 7200 }),
    )
    .await;
    let pid = body["id"].as_str().expect("project id");

    let resp = app
        .clone()
        .oneshot(req(Method::GET, "/api/projects", Some(&cookie), None))
        .await
        .expect("list resp");
    let (_, body) = read_body_json(resp).await;
    let projects = body["projects"].as_array().expect("projects array");
    let mine = projects
        .iter()
        .find(|p| p["id"].as_str() == Some(pid))
        .expect("own project in list");
    assert_eq!(mine["session_duration_secs"], serde_json::json!(7200));
}
