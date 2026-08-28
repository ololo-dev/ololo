#![allow(dead_code)]

//! Task-attach judges API integration tests.

use axum::http::{Method, StatusCode};
use sea_orm::{ActiveModelTrait, Set};
use server::entities::{projects, tasks};
use server::{AppState, build_router};
use tower::ServiceExt;
use uuid::Uuid;

fn valid_scale() -> serde_json::Value {
    serde_json::json!({ "min": 0.0, "max": 10.0, "step": 1.0 })
}

fn alt_scale() -> serde_json::Value {
    serde_json::json!({ "min": 0.0, "max": 1.0, "step": 1.0 })
}

fn judge_body(slug: &str) -> serde_json::Value {
    serde_json::json!({
        "slug": slug,
        "name": format!("Judge {slug}"),
        "description": "desc",
        "prompt": "prompt",
        "rating_scale": valid_scale(),
    })
}

async fn create_judge(app: &axum::Router, cookie: &str, slug: &str) -> Uuid {
    let resp = app
        .clone()
        .oneshot(req(
            Method::POST,
            "/api/admin/judges",
            Some(cookie),
            Some(judge_body(slug)),
        ))
        .await
        .expect("create judge");
    let (s, _, b) = read_body(resp).await;
    assert_eq!(s, StatusCode::CREATED, "create_judge: {b}");
    b["id"].as_str().expect("id").parse().unwrap()
}

async fn create_project_and_task(state: &AppState, admin_id: Uuid) -> (Uuid, Uuid) {
    let project_id = Uuid::new_v4();
    let now = chrono::Utc::now();
    projects::ActiveModel {
        id: Set(project_id),
        name: Set("proj".into()),
        slug: Set(None),
        description: Set("".into()),
        category: Set(None),
        tags: Set("[]".into()),
        cover_image_url: Set(None),
        owner_user_id_fk: Set(admin_id),
        public: Set(true),
        archived_at: Set(None),
        created_at: Set(now),
        updated_at: Set(now),
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

    let task_id = Uuid::new_v4();
    tasks::ActiveModel {
        id: Set(task_id),
        project_id_fk: Set(project_id),
        ordinal: Set(0),
        title: Set("t".into()),
        content: Set("".into()),
        test_template: Set(serde_json::json!({"kind":"shell","command_template":"echo hi"})),
        created_at: Set(now),
        tags: Set("[]".into()),
        point_value: Set(10),
        deadline_secs: Set(None),
        min_interval_secs: Set(None),
        interval_increment_secs: Set(None),
        max_interval_secs: Set(None),
        fail_points: Set(-5),
        no_response_points: Set(-10),
        completion_bonus_points: Set(10),
        evaluation: Set(None),
    }
    .insert(&state.db)
    .await
    .expect("insert task");

    (project_id, task_id)
}

async fn setup() -> (axum::Router, AppState, String, Uuid, Uuid, Uuid) {
    let state = test_state().await;
    let app = build_router(state.clone());
    let (admin_id, cookie) =
        register_and_login(app.clone(), "admin@tj.test", "password-12345").await;
    let (project_id, task_id) = create_project_and_task(&state, admin_id).await;
    let judge_id = create_judge(&app, &cookie, "j1").await;
    (app, state, cookie, project_id, task_id, judge_id)
}

fn attach_uri(project_id: Uuid, task_id: Uuid) -> String {
    format!("/api/projects/{project_id}/tasks/{task_id}/judges")
}

fn item_uri(project_id: Uuid, task_id: Uuid, judge_id: Uuid) -> String {
    format!("/api/projects/{project_id}/tasks/{task_id}/judges/{judge_id}")
}

// ─────────────────────────── Tests ───────────────────────────

use crate::common;
use crate::common::*;

#[tokio::test]
async fn update_invalid_scale_400() {
    let (app, _state, cookie, project_id, task_id, judge_id) = setup().await;

    let _ = app
        .clone()
        .oneshot(req(
            Method::POST,
            &attach_uri(project_id, task_id),
            Some(&cookie),
            Some(serde_json::json!({
                "judge_id": judge_id,
                "ordinal": 1,
                "rating_scale_override": serde_json::Value::Null,
            })),
        ))
        .await
        .expect("attach");

    let bad = serde_json::json!({ "min": 5.0, "max": 0.0, "step": 1.0 });
    let resp = app
        .clone()
        .oneshot(req(
            Method::PUT,
            &item_uri(project_id, task_id, judge_id),
            Some(&cookie),
            Some(serde_json::json!({ "rating_scale_override": bad })),
        ))
        .await
        .expect("update bad");
    let (s, _, b) = read_body(resp).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "update bad: {b}");
}

#[tokio::test]
async fn detach_happy_204() {
    let (app, _state, cookie, project_id, task_id, judge_id) = setup().await;

    let _ = app
        .clone()
        .oneshot(req(
            Method::POST,
            &attach_uri(project_id, task_id),
            Some(&cookie),
            Some(serde_json::json!({
                "judge_id": judge_id,
                "ordinal": 1,
                "rating_scale_override": serde_json::Value::Null,
            })),
        ))
        .await
        .expect("attach");

    let resp = app
        .clone()
        .oneshot(req(
            Method::DELETE,
            &item_uri(project_id, task_id, judge_id),
            Some(&cookie),
            None,
        ))
        .await
        .expect("detach");
    let (s, _, b) = read_body(resp).await;
    assert!(
        s == StatusCode::NO_CONTENT || s == StatusCode::OK,
        "detach: {b}"
    );
}

#[tokio::test]
async fn detach_missing_404() {
    let (app, _state, cookie, project_id, task_id, judge_id) = setup().await;

    let resp = app
        .clone()
        .oneshot(req(
            Method::DELETE,
            &item_uri(project_id, task_id, judge_id),
            Some(&cookie),
            None,
        ))
        .await
        .expect("detach missing");
    let (s, _, b) = read_body(resp).await;
    assert_eq!(s, StatusCode::NOT_FOUND, "detach missing: {b}");
}

#[tokio::test]
async fn attach_unknown_field_400() {
    let (app, _state, cookie, project_id, task_id, judge_id) = setup().await;

    let resp = app
        .clone()
        .oneshot(req(
            Method::POST,
            &attach_uri(project_id, task_id),
            Some(&cookie),
            Some(serde_json::json!({
                "judge_id": judge_id,
                "ordinal": 1,
                "rating_scale_override": serde_json::Value::Null,
                "bogus": true,
            })),
        ))
        .await
        .expect("bogus");
    let (s, _, _b) = read_body(resp).await;
    assert!(
        s == StatusCode::BAD_REQUEST || s == StatusCode::UNPROCESSABLE_ENTITY,
        "deny_unknown_fields: {s}"
    );
}

#[tokio::test]
async fn list_missing_task_404() {
    let (app, _state, cookie, project_id, _task_id, _judge_id) = setup().await;
    let missing_task = Uuid::new_v4();

    let resp = app
        .clone()
        .oneshot(req(
            Method::GET,
            &attach_uri(project_id, missing_task),
            Some(&cookie),
            None,
        ))
        .await
        .expect("list missing");
    let (s, _, b) = read_body(resp).await;
    assert_eq!(s, StatusCode::NOT_FOUND, "list missing task: {b}");
}
