//! `GET /api/public/active-sessions` — the landing page's live-sessions
//! block. Unauthenticated; must list lobby/running sessions of PUBLIC
//! projects only (the join code is an invite — listing it publishes it),
//! and drop sessions once they finish.

use crate::common;

use crate::common::{read_body_json, register_and_login, req, test_state};
use axum::http::{Method, StatusCode};
use sea_orm::{ActiveModelTrait, ActiveValue::Set, EntityTrait};
use server::build_router;
use tower::ServiceExt;

use arena_core::entities::{projects, sessions};
use arena_core::session_status::SessionStatus;

async fn list_active(app: axum::Router) -> serde_json::Value {
    let resp = app
        .oneshot(req(Method::GET, "/api/public/active-sessions", None, None))
        .await
        .expect("active-sessions resp");
    let (status, body) = read_body_json(resp).await;
    assert_eq!(status, StatusCode::OK, "active-sessions list: {body}");
    body
}

#[tokio::test]
async fn lists_only_public_project_sessions_while_active() {
    let state = test_state().await;
    let db = state.db.clone();
    let app = build_router(state);

    let (_user_id, access) =
        register_and_login(app.clone(), "owner@example.com", "Pass1234!").await;

    // A non-admin's project is public by construction (write.rs), so its
    // lobby session is landing material from the start.
    let resp = app
        .clone()
        .oneshot(req(
            Method::POST,
            "/api/projects",
            Some(&access),
            Some(serde_json::json!({ "name": "Landing Project" })),
        ))
        .await
        .expect("create project resp");
    let (status, project) = read_body_json(resp).await;
    assert_eq!(status, StatusCode::CREATED, "create project: {project}");
    let project_id: uuid::Uuid = project["id"].as_str().unwrap().parse().unwrap();

    let resp = app
        .clone()
        .oneshot(req(
            Method::POST,
            "/api/sessions",
            Some(&access),
            Some(serde_json::json!({ "name": "Friday night run", "project_id": project_id })),
        ))
        .await
        .expect("create session resp");
    let (status, session) = read_body_json(resp).await;
    assert_eq!(status, StatusCode::CREATED, "create session: {session}");
    let session_id: uuid::Uuid = session["id"].as_str().unwrap().parse().unwrap();
    let join_code = session["join_code"].as_str().unwrap().to_string();

    let body = list_active(app.clone()).await;
    let listed = body["sessions"].as_array().unwrap();
    assert_eq!(listed.len(), 1, "public lobby session listed: {body}");
    let s = &listed[0];
    assert_eq!(s["id"].as_str().unwrap(), session_id.to_string());
    assert_eq!(s["join_code"].as_str().unwrap(), join_code);
    assert_eq!(s["name"].as_str().unwrap(), "Friday night run");
    assert_eq!(s["status"].as_str().unwrap(), "lobby");
    assert_eq!(s["project_name"].as_str().unwrap(), "Landing Project");
    assert_eq!(s["players"].as_i64().unwrap(), 0);
    // Lobby countdown snapshot: freshly created, so the full lobby window
    // (minus at most a second or two of test latency) must remain.
    let secs = s["seconds_remaining"].as_i64().unwrap();
    assert!(
        secs > 0,
        "lobby seconds_remaining should be positive: {body}"
    );

    // Running phase: the countdown switches to the project session duration.
    let mut sm: sessions::ActiveModel = sessions::Entity::find_by_id(session_id)
        .one(&db)
        .await
        .unwrap()
        .unwrap()
        .into();
    sm.status = Set(SessionStatus::Running);
    sm.started_at = Set(Some(chrono::Utc::now()));
    sm.update(&db).await.unwrap();

    let body = list_active(app.clone()).await;
    let s = &body["sessions"].as_array().unwrap()[0];
    assert_eq!(s["status"].as_str().unwrap(), "running");
    let secs = s["seconds_remaining"].as_i64().unwrap();
    assert!(
        secs > 0,
        "running seconds_remaining should be positive: {body}"
    );

    // A private project's session must vanish — the join code is an invite,
    // and listing it here would publish it.
    let mut pm: projects::ActiveModel = projects::Entity::find_by_id(project_id)
        .one(&db)
        .await
        .unwrap()
        .unwrap()
        .into();
    pm.public = Set(false);
    pm.update(&db).await.unwrap();

    let body = list_active(app.clone()).await;
    assert_eq!(
        body["sessions"].as_array().unwrap().len(),
        0,
        "a private project's session must not be listed: {body}"
    );

    // Back to public, then finish: a finished session disappears too.
    let mut pm: projects::ActiveModel = projects::Entity::find_by_id(project_id)
        .one(&db)
        .await
        .unwrap()
        .unwrap()
        .into();
    pm.public = Set(true);
    pm.update(&db).await.unwrap();
    let mut sm: sessions::ActiveModel = sessions::Entity::find_by_id(session_id)
        .one(&db)
        .await
        .unwrap()
        .unwrap()
        .into();
    sm.status = Set(SessionStatus::Finished);
    sm.update(&db).await.unwrap();

    let body = list_active(app.clone()).await;
    assert_eq!(
        body["sessions"].as_array().unwrap().len(),
        0,
        "finished session must not be listed: {body}"
    );
}
