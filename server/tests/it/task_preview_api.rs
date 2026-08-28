//! `GET /api/projects/:id/tasks/preview` — the public task arc (audit
//! UI-H2). The preview shows only what a player may read before playing
//! (ordinal, title, brief, points), and only when the project opted in:
//! quiz-shaped projects hide their ladder with `show_tasks: false`.

use axum::http::{Method, StatusCode};
use sea_orm::{ActiveModelTrait, ActiveValue::Set, EntityTrait};
use server::build_router;
use tower::ServiceExt;

use crate::common;
use crate::common::*;

async fn create_project_with_task(app: &axum::Router, cookie: &str, name: &str) -> String {
    let resp = app
        .clone()
        .oneshot(req(
            Method::POST,
            "/api/projects",
            Some(cookie),
            Some(serde_json::json!({ "name": name })),
        ))
        .await
        .expect("create project");
    let (s, _, body) = read_body(resp).await;
    assert_eq!(s, StatusCode::CREATED, "{body}");
    let project_id = body["id"].as_str().expect("id").to_string();

    let resp = app
        .clone()
        .oneshot(req(
            Method::POST,
            &format!("/api/projects/{project_id}/tasks"),
            Some(cookie),
            Some(serde_json::json!({
                "title": "Build the widget",
                "description": "Make it show the weather.",
                "test_template": {
                    "kind": "shell",
                    "command_template": "echo secret-fixture-machinery",
                    "answer_template": "result.trim() === 'ok'",
                },
                "points": { "value": 40 },
            })),
        ))
        .await
        .expect("create task");
    let (s, _, body) = read_body(resp).await;
    assert_eq!(s, StatusCode::CREATED, "{body}");
    project_id
}

async fn preview(
    app: &axum::Router,
    cookie: Option<&str>,
    project_id: &str,
) -> (StatusCode, serde_json::Value) {
    let resp = app
        .clone()
        .oneshot(req(
            Method::GET,
            &format!("/api/projects/{project_id}/tasks/preview"),
            cookie,
            None,
        ))
        .await
        .expect("preview resp");
    let (s, _, b) = read_body(resp).await;
    (s, b)
}

#[tokio::test]
async fn preview_shows_briefs_and_never_the_grading_machinery() {
    let state = test_state().await;
    let app = build_router(state);
    // First registrant is the admin/owner.
    let (_, owner) = register_and_login(app.clone(), "owner@x.test", "password-12345").await;
    let project_id = create_project_with_task(&app, &owner, "Widget").await;

    // Anonymous read of a public, shown ladder.
    let (s, body) = preview(&app, None, &project_id).await;
    assert_eq!(s, StatusCode::OK, "{body}");
    let tasks = body["tasks"].as_array().expect("tasks array");
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0]["title"], "Build the widget");
    assert_eq!(tasks[0]["description"], "Make it show the weather.");
    assert_eq!(tasks[0]["points"], 40);
    // Scoring semantics ride along so the UI can say what `points` means:
    // a classic task pays per passing check, an open-ended one hands the
    // budget to its judge panel.
    assert_eq!(tasks[0]["open_ended"], false);
    assert!(tasks[0]["completion_bonus"].is_number());
    assert_eq!(tasks[0]["judges"].as_array().expect("judges").len(), 0);
    // The grading machinery must never ride along.
    let raw = body.to_string();
    assert!(
        !raw.contains("secret-fixture-machinery") && !raw.contains("test_template"),
        "preview leaked the template: {raw}"
    );
}

#[tokio::test]
async fn hidden_ladder_is_not_found_for_players_but_open_to_the_owner() {
    let state = test_state().await;
    let app = build_router(state.clone());
    let (_, owner) = register_and_login(app.clone(), "owner@x.test", "password-12345").await;
    let (_, player) = register_and_login(app.clone(), "player@x.test", "password-12345").await;
    let project_id = create_project_with_task(&app, &owner, "Quiz").await;

    // Flip the seed-controlled flag the way a quiz project ships it.
    use arena_core::entities::projects;
    let row = projects::Entity::find_by_id(project_id.parse::<uuid::Uuid>().unwrap())
        .one(&state.db)
        .await
        .unwrap()
        .unwrap();
    let mut am: projects::ActiveModel = row.into();
    am.show_tasks = Set(false);
    am.update(&state.db).await.unwrap();

    // Anonymous and ordinary players get the same NotFound a private
    // project would give — the endpoint confirms nothing it hides.
    let (s, _) = preview(&app, None, &project_id).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let (s, _) = preview(&app, Some(&player), &project_id).await;
    assert_eq!(s, StatusCode::NOT_FOUND);

    // The owner still sees their own ladder.
    let (s, body) = preview(&app, Some(&owner), &project_id).await;
    assert_eq!(s, StatusCode::OK, "{body}");
    assert_eq!(body["tasks"].as_array().unwrap().len(), 1);
}
