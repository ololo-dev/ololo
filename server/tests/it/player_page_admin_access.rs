//! Admin access to player pages:
//! admins may read any player's snapshot, history, and task-stats.
//! The snapshot itself is readable by any signed-in visitor on a public
//! project (the docs promise a spectator the report and the verdicts), but
//! the inspection surface — history — stays owner-and-admin, and stat
//! reporting (POST) stays owner-only even for admins.

use axum::http::{Method, StatusCode};
use server::build_router;
use tower::ServiceExt;
use uuid::Uuid;

use crate::common;
use crate::common::{
    make_user_admin, read_body_json, register_and_login_default, req_with_cookie, test_state,
};

async fn create_session(app: &axum::Router, cookie: &str) -> String {
    let resp = app
        .clone()
        .oneshot(req_with_cookie(
            Method::POST,
            "/api/projects",
            cookie,
            Some(serde_json::json!({ "name": format!("proj-{}", Uuid::new_v4()) })),
        ))
        .await
        .expect("create project");
    let (sc, pb) = read_body_json(resp).await;
    assert_eq!(sc, StatusCode::CREATED, "project: {pb}");
    let project_id = pb["id"].as_str().expect("project id").to_string();

    let resp = app
        .clone()
        .oneshot(req_with_cookie(
            Method::POST,
            "/api/sessions",
            cookie,
            Some(serde_json::json!({
                "name": format!("sess-{}", Uuid::new_v4()),
                "project_id": project_id
            })),
        ))
        .await
        .expect("create session");
    let (sc, sb) = read_body_json(resp).await;
    assert_eq!(sc, StatusCode::CREATED, "session: {sb}");
    sb["join_code"].as_str().expect("join_code").to_string()
}

async fn join_session(app: &axum::Router, cookie: &str, code: &str) -> String {
    let resp = app
        .clone()
        .oneshot(req_with_cookie(
            Method::POST,
            "/api/sessions/join",
            cookie,
            Some(serde_json::json!({ "code": code })),
        ))
        .await
        .expect("join resp");
    let (sc, body) = read_body_json(resp).await;
    assert_eq!(sc, StatusCode::CREATED, "join failed: {body}");
    body["player_id"].as_str().expect("player_id").to_string()
}

fn stats_report() -> serde_json::Value {
    serde_json::json!({
        "task_ordinal": 0,
        "window_started_at_ms": 1_770_000_000_000i64,
        "window_ended_at_ms": 1_770_000_120_000i64,
        "agents": [{
            "agent": "claude",
            "agent_session_id": "sess-1",
            "model": "claude-fable-5",
            "input_tokens": 1000,
            "output_tokens": 200,
            "cache_read_tokens": 0,
            "cache_write_tokens": 0,
            "reasoning_tokens": 0,
            "cost": 0.05,
            "user_messages": 1,
            "assistant_messages": 2,
            "tool_calls": 3,
            "tools": {"Bash": 3},
            "skills": {}
        }]
    })
}

#[tokio::test]
async fn admin_can_read_any_players_page() {
    let state = test_state().await;
    let app = build_router(state.clone());

    let (_owner_id, cookie_owner) =
        register_and_login_default(app.clone(), "owner-adm1@x.test").await;
    let (_joiner_id, cookie_joiner) =
        register_and_login_default(app.clone(), "joiner-adm1@x.test").await;
    let (admin_id, cookie_admin) =
        register_and_login_default(app.clone(), "admin-adm1@x.test").await;
    make_user_admin(&state, admin_id).await;

    let join_code = create_session(&app, &cookie_owner).await;
    let player_id = join_session(&app, &cookie_joiner, &join_code).await;

    // Joiner reports stats so the admin GET below has an entry to read.
    let stats_uri = format!("/api/sessions/{join_code}/players/{player_id}/task-stats");
    let resp = app
        .clone()
        .oneshot(req_with_cookie(
            Method::POST,
            &stats_uri,
            &cookie_joiner,
            Some(stats_report()),
        ))
        .await
        .expect("post stats");
    let (sc, body) = read_body_json(resp).await;
    assert_eq!(sc, StatusCode::OK, "post stats: {body}");

    // Snapshot: admin reads another user's player page.
    let resp = app
        .clone()
        .oneshot(req_with_cookie(
            Method::GET,
            &format!("/api/sessions/{join_code}/players/{player_id}"),
            &cookie_admin,
            None,
        ))
        .await
        .expect("admin snapshot");
    let (sc, body) = read_body_json(resp).await;
    assert_eq!(sc, StatusCode::OK, "admin snapshot: {body}");
    assert_eq!(body["player_id"].as_str(), Some(player_id.as_str()));

    // History: admin allowed (empty repo → empty list).
    let resp = app
        .clone()
        .oneshot(req_with_cookie(
            Method::GET,
            &format!("/api/sessions/{join_code}/players/{player_id}/history"),
            &cookie_admin,
            None,
        ))
        .await
        .expect("admin history");
    let (sc, body) = read_body_json(resp).await;
    assert_eq!(sc, StatusCode::OK, "admin history: {body}");

    // Task-stats GET: admin allowed.
    let resp = app
        .clone()
        .oneshot(req_with_cookie(
            Method::GET,
            &stats_uri,
            &cookie_admin,
            None,
        ))
        .await
        .expect("admin stats get");
    let (sc, body) = read_body_json(resp).await;
    assert_eq!(sc, StatusCode::OK, "admin stats get: {body}");
    assert_eq!(body["entries"].as_array().map(|a| a.len()), Some(1));

    // Task-stats POST stays owner-only — admin reporting for another
    // player is rejected.
    let resp = app
        .oneshot(req_with_cookie(
            Method::POST,
            &stats_uri,
            &cookie_admin,
            Some(stats_report()),
        ))
        .await
        .expect("admin stats post");
    let (sc, _) = read_body_json(resp).await;
    assert_eq!(sc, StatusCode::NOT_FOUND, "admin POST must not be widened");
}

#[tokio::test]
async fn a_spectator_reads_the_run_but_not_its_inspection_surface() {
    let state = test_state().await;
    let app = build_router(state);

    let (_owner_id, cookie_owner) =
        register_and_login_default(app.clone(), "owner-adm2@x.test").await;
    let (_joiner_id, cookie_joiner) =
        register_and_login_default(app.clone(), "joiner-adm2@x.test").await;
    let (_other_id, cookie_other) =
        register_and_login_default(app.clone(), "other-adm2@x.test").await;

    let join_code = create_session(&app, &cookie_owner).await;
    let player_id = join_session(&app, &cookie_joiner, &join_code).await;

    let resp = app
        .clone()
        .oneshot(req_with_cookie(
            Method::GET,
            &format!("/api/sessions/{join_code}/players/{player_id}"),
            &cookie_other,
            None,
        ))
        .await
        .expect("other snapshot");
    let (sc, body) = read_body_json(resp).await;
    // Fixture projects are public (non-admin creators cannot make private
    // ones), so a signed-in stranger reads the run page.
    assert_eq!(sc, StatusCode::OK, "spectator snapshot: {body}");

    let resp = app
        .clone()
        .oneshot(req_with_cookie(
            Method::GET,
            &format!("/api/sessions/{join_code}/players/{player_id}/history"),
            &cookie_other,
            None,
        ))
        .await
        .expect("other history");
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);

    let resp = app
        .oneshot(req_with_cookie(
            Method::GET,
            &format!("/api/sessions/{join_code}/players/{player_id}/task-stats"),
            &cookie_other,
            None,
        ))
        .await
        .expect("other stats get");
    let (sc, _) = read_body_json(resp).await;
    assert_eq!(sc, StatusCode::NOT_FOUND);
}
