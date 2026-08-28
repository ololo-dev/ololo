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
async fn attach_happy_default_scale_201() {
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
            })),
        ))
        .await
        .expect("attach");
    let (s, _, b) = read_body(resp).await;
    assert_eq!(s, StatusCode::CREATED, "attach: {b}");
    assert_eq!(b["judge_slug"], serde_json::json!("j1"));
    assert_eq!(b["ordinal"], serde_json::json!(1));
    assert!(b["rating_scale_override"].is_null());
    assert_eq!(b["effective_rating_scale"]["max"], serde_json::json!(10.0));
}

#[tokio::test]
async fn attach_with_override_201() {
    let (app, _state, cookie, project_id, task_id, judge_id) = setup().await;

    let resp = app
        .clone()
        .oneshot(req(
            Method::POST,
            &attach_uri(project_id, task_id),
            Some(&cookie),
            Some(serde_json::json!({
                "judge_id": judge_id,
                "ordinal": 2,
                "rating_scale_override": alt_scale(),
            })),
        ))
        .await
        .expect("attach");
    let (s, _, b) = read_body(resp).await;
    assert_eq!(s, StatusCode::CREATED, "attach: {b}");
    assert_eq!(b["effective_rating_scale"]["max"], serde_json::json!(1.0));
    assert_eq!(b["rating_scale_override"]["max"], serde_json::json!(1.0));
}

#[tokio::test]
async fn attach_duplicate_judge_409() {
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
        .expect("first attach");

    let resp = app
        .clone()
        .oneshot(req(
            Method::POST,
            &attach_uri(project_id, task_id),
            Some(&cookie),
            Some(serde_json::json!({
                "judge_id": judge_id,
                "ordinal": 2,
                "rating_scale_override": serde_json::Value::Null,
            })),
        ))
        .await
        .expect("dup judge");
    let (s, _, b) = read_body(resp).await;
    assert_eq!(s, StatusCode::CONFLICT, "dup judge: {b}");
}

#[tokio::test]
async fn attach_duplicate_ordinal_409() {
    let (app, _state, cookie, project_id, task_id, judge_id) = setup().await;
    let judge2 = create_judge(&app, &cookie, "j2").await;

    let _ = app
        .clone()
        .oneshot(req(
            Method::POST,
            &attach_uri(project_id, task_id),
            Some(&cookie),
            Some(serde_json::json!({
                "judge_id": judge_id,
                "ordinal": 5,
                "rating_scale_override": serde_json::Value::Null,
            })),
        ))
        .await
        .expect("first attach");

    let resp = app
        .clone()
        .oneshot(req(
            Method::POST,
            &attach_uri(project_id, task_id),
            Some(&cookie),
            Some(serde_json::json!({
                "judge_id": judge2,
                "ordinal": 5,
                "rating_scale_override": serde_json::Value::Null,
            })),
        ))
        .await
        .expect("dup ord");
    let (s, _, b) = read_body(resp).await;
    assert_eq!(s, StatusCode::CONFLICT, "dup ord: {b}");
}

#[tokio::test]
async fn attach_missing_judge_404() {
    let (app, _state, cookie, project_id, task_id, _judge_id) = setup().await;
    let missing = Uuid::new_v4();

    let resp = app
        .clone()
        .oneshot(req(
            Method::POST,
            &attach_uri(project_id, task_id),
            Some(&cookie),
            Some(serde_json::json!({
                "judge_id": missing,
                "ordinal": 1,
                "rating_scale_override": serde_json::Value::Null,
            })),
        ))
        .await
        .expect("missing judge");
    let (s, _, b) = read_body(resp).await;
    assert_eq!(s, StatusCode::NOT_FOUND, "missing judge: {b}");
}

#[tokio::test]
async fn attach_missing_task_404() {
    let (app, _state, cookie, project_id, _task_id, judge_id) = setup().await;
    let missing_task = Uuid::new_v4();

    let resp = app
        .clone()
        .oneshot(req(
            Method::POST,
            &attach_uri(project_id, missing_task),
            Some(&cookie),
            Some(serde_json::json!({
                "judge_id": judge_id,
                "ordinal": 1,
                "rating_scale_override": serde_json::Value::Null,
            })),
        ))
        .await
        .expect("missing task");
    let (s, _, b) = read_body(resp).await;
    assert_eq!(s, StatusCode::NOT_FOUND, "missing task: {b}");
}

#[tokio::test]
async fn attach_invalid_scale_override_400() {
    let (app, _state, cookie, project_id, task_id, judge_id) = setup().await;
    let bad = serde_json::json!({ "min": 5.0, "max": 0.0, "step": 1.0 });

    let resp = app
        .clone()
        .oneshot(req(
            Method::POST,
            &attach_uri(project_id, task_id),
            Some(&cookie),
            Some(serde_json::json!({
                "judge_id": judge_id,
                "ordinal": 1,
                "rating_scale_override": bad,
            })),
        ))
        .await
        .expect("bad scale");
    let (s, _, b) = read_body(resp).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "bad scale: {b}");
}

#[tokio::test]
async fn list_returns_attachments() {
    let (app, _state, cookie, project_id, task_id, judge_id) = setup().await;
    let judge2 = create_judge(&app, &cookie, "j2").await;

    let _ = app
        .clone()
        .oneshot(req(
            Method::POST,
            &attach_uri(project_id, task_id),
            Some(&cookie),
            Some(serde_json::json!({
                "judge_id": judge_id,
                "ordinal": 2,
                "rating_scale_override": serde_json::Value::Null,
            })),
        ))
        .await
        .expect("attach1");

    let _ = app
        .clone()
        .oneshot(req(
            Method::POST,
            &attach_uri(project_id, task_id),
            Some(&cookie),
            Some(serde_json::json!({
                "judge_id": judge2,
                "ordinal": 1,
                "rating_scale_override": alt_scale(),
            })),
        ))
        .await
        .expect("attach2");

    let resp = app
        .clone()
        .oneshot(req(
            Method::GET,
            &attach_uri(project_id, task_id),
            Some(&cookie),
            None,
        ))
        .await
        .expect("list");
    let (s, _, b) = read_body(resp).await;
    assert_eq!(s, StatusCode::OK, "list: {b}");
    let arr = b.as_array().expect("array");
    assert_eq!(arr.len(), 2);
    let ordinals: Vec<i64> = arr
        .iter()
        .map(|x| x["ordinal"].as_i64().expect("ord"))
        .collect();
    assert_eq!(ordinals, vec![1, 2]);
}

#[tokio::test]
async fn update_ordinal_200() {
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
            })),
        ))
        .await
        .expect("attach");
    let (_, _, _b) = read_body(resp).await;

    let resp = app
        .clone()
        .oneshot(req(
            Method::PUT,
            &item_uri(project_id, task_id, judge_id),
            Some(&cookie),
            Some(serde_json::json!({ "ordinal": 9 })),
        ))
        .await
        .expect("update");
    let (s, _, b) = read_body(resp).await;
    assert_eq!(s, StatusCode::OK, "update: {b}");
    assert_eq!(b["ordinal"], serde_json::json!(9));
}

#[tokio::test]
async fn update_override_to_null_reverts_default() {
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
                "rating_scale_override": alt_scale(),
            })),
        ))
        .await
        .expect("attach");

    let resp = app
        .clone()
        .oneshot(req(
            Method::PUT,
            &item_uri(project_id, task_id, judge_id),
            Some(&cookie),
            Some(serde_json::json!({
                "rating_scale_override": serde_json::Value::Null,
            })),
        ))
        .await
        .expect("update null");
    let (s, _, b) = read_body(resp).await;
    assert_eq!(s, StatusCode::OK, "update null: {b}");
    assert!(b["rating_scale_override"].is_null());
    assert_eq!(b["effective_rating_scale"]["max"], serde_json::json!(10.0));
}

/// The project summary reports `judge_review_count` = total judges attached
/// across the project's tasks (the review cost a full session incurs). Zero
/// before any attach; rises with each attachment. Present on both the list
/// and the single-project (slug) endpoints.
#[tokio::test]
async fn judge_review_count_reflects_attachments() {
    let (app, _state, cookie, project_id, task_id, judge_id) = setup().await;

    // No judges yet — count is 0.
    let resp = app
        .clone()
        .oneshot(req(Method::GET, "/api/projects", Some(&cookie), None))
        .await
        .expect("list");
    let (s, _, b) = read_body(resp).await;
    assert_eq!(s, StatusCode::OK, "list: {b}");
    let before = b["projects"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["id"] == serde_json::json!(project_id.to_string()))
        .expect("project in list");
    assert_eq!(before["judge_review_count"], serde_json::json!(0));

    // Attach one judge.
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
            })),
        ))
        .await
        .expect("attach");
    let (s, _, b) = read_body(resp).await;
    assert_eq!(s, StatusCode::CREATED, "attach: {b}");

    // List now reports 1.
    let resp = app
        .clone()
        .oneshot(req(Method::GET, "/api/projects", Some(&cookie), None))
        .await
        .expect("list");
    let (_, _, b) = read_body(resp).await;
    let after = b["projects"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["id"] == serde_json::json!(project_id.to_string()))
        .expect("project in list");
    assert_eq!(after["judge_review_count"], serde_json::json!(1));

    // Single-project endpoint also carries it.
    let resp = app
        .clone()
        .oneshot(req(
            Method::GET,
            &format!("/api/projects/{project_id}"),
            Some(&cookie),
            None,
        ))
        .await
        .expect("get one");
    let (s, _, b) = read_body(resp).await;
    assert_eq!(s, StatusCode::OK, "get one: {b}");
    assert_eq!(b["judge_review_count"], serde_json::json!(1));
}
