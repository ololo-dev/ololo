//! List paths must NOT carry the derived per-player completion status.
//!
//! The completion aggregate is computed only on single-session
//! detail/snapshot paths. Session LIST responses (`GET /api/sessions`,
//! `GET /api/projects/:id/sessions`) must stay wire-identical — no
//! `completion_status` key anywhere in the JSON, even when the listed
//! session has players with scheduler state that would yield a status.

use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use chrono::Utc;
use http_body_util::BodyExt;
use sea_orm::{ActiveModelTrait, Set};
use server::build_router;
use server::entities::{players, session_scheduler_state};
use tower::ServiceExt;
use uuid::Uuid;

use crate::common;
use crate::common::*;

/// Recursively assert no object in `value` carries a `completion_status` key.
fn assert_no_completion_status(value: &serde_json::Value, path: &str) {
    match value {
        serde_json::Value::Object(map) => {
            assert!(
                !map.contains_key("completion_status"),
                "list response must not contain a completion_status key (at {path})"
            );
            for (k, v) in map {
                assert_no_completion_status(v, &format!("{path}.{k}"));
            }
        }
        serde_json::Value::Array(items) => {
            for (i, v) in items.iter().enumerate() {
                assert_no_completion_status(v, &format!("{path}[{i}]"));
            }
        }
        _ => {}
    }
}

async fn get_json(
    app: &axum::Router,
    cookie: &str,
    uri: &str,
) -> (StatusCode, String, serde_json::Value) {
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri(uri)
                .header(header::ORIGIN, "http://localhost:5173")
                .header(header::COOKIE, cookie)
                .body(Body::empty())
                .expect("build request"),
        )
        .await
        .expect("oneshot");
    let status = resp.status();
    let bytes = resp
        .into_body()
        .collect()
        .await
        .expect("collect body")
        .to_bytes();
    let raw = String::from_utf8(bytes.to_vec()).expect("utf8 body");
    let value = serde_json::from_str(&raw).expect("json body");
    (status, raw, value)
}

#[tokio::test]
async fn session_list_responses_omit_completion_status() {
    let state = test_state().await;
    let db = state.db.clone();
    let app = build_router(state);
    let (user_id, cookie) =
        register_and_login(app.clone(), "lister@x.test", "password-12345").await;

    // Create a project and a session via the API.
    let resp = app
        .clone()
        .oneshot(req(
            Method::POST,
            "/api/projects",
            Some(&cookie),
            Some(serde_json::json!({ "name": "list-proj" })),
        ))
        .await
        .expect("create project");
    let (status, _, project) = read_body(resp).await;
    assert_eq!(status, StatusCode::CREATED, "project: {project}");
    let project_id = project["id"].as_str().expect("project id").to_string();

    let resp = app
        .clone()
        .oneshot(req(
            Method::POST,
            "/api/sessions",
            Some(&cookie),
            Some(serde_json::json!({ "name": "list-sess", "project_id": project_id })),
        ))
        .await
        .expect("create session");
    let (status, _, session) = read_body(resp).await;
    assert_eq!(status, StatusCode::CREATED, "session: {session}");
    let session_id: Uuid = session["id"]
        .as_str()
        .expect("session id")
        .parse()
        .expect("uuid");

    // Seed a player mid-tasks so a status WOULD be derivable — the list
    // must still skip the computation entirely.
    let player_id = Uuid::new_v4();
    players::ActiveModel {
        id: Set(player_id),
        session_id_fk: Set(session_id),
        user_id_fk: Set(Some(user_id)),
        display_name: Set("p".to_string()),
        fingerprint: Set(None),
        metadata_json: Set(None),
        joined_at: Set(Utc::now()),
        reconnected_at: Set(None),
        revoked_at: Set(None),
        agent_connected: Set(false),
        agent_last_seen_at: Set(None),
    }
    .insert(&db)
    .await
    .expect("insert player");
    session_scheduler_state::ActiveModel {
        id: Set(Uuid::new_v4()),
        session_id_fk: Set(session_id),
        player_id_fk: Set(player_id),
        task_id: Set(None),
        state: Set("active".to_string()),
        next_probe_at: Set(None),
        created_at: Set(Utc::now()),
        updated_at: Set(Utc::now()),
    }
    .insert(&db)
    .await
    .expect("insert scheduler state");

    // Sessions list.
    let (status, raw, body) = get_json(&app, &cookie, "/api/sessions").await;
    assert_eq!(status, StatusCode::OK, "list: {body}");
    assert!(
        !body["sessions"]
            .as_array()
            .expect("sessions array")
            .is_empty(),
        "list must contain the seeded session"
    );
    assert!(
        !raw.contains("completion_status"),
        "GET /api/sessions body must not mention completion_status: {raw}"
    );
    assert_no_completion_status(&body, "$");

    // Project sessions list.
    let (status, raw, body) = get_json(
        &app,
        &cookie,
        &format!("/api/projects/{project_id}/sessions"),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "project list: {body}");
    assert!(
        !body["sessions"]
            .as_array()
            .expect("sessions array")
            .is_empty(),
        "project list must contain the seeded session"
    );
    assert!(
        !raw.contains("completion_status"),
        "GET /api/projects/:id/sessions body must not mention completion_status: {raw}"
    );
    assert_no_completion_status(&body, "$");
}

#[tokio::test]
async fn public_project_sessions_are_visible_to_anonymous() {
    // A public project's sessions are public history — visible without login.
    let state = test_state().await;
    let app = build_router(state);
    // A non-admin owner: their project is created public.
    let (_owner, cookie) =
        register_and_login(app.clone(), "pubowner@x.test", "password-12345").await;

    let resp = app
        .clone()
        .oneshot(req(
            Method::POST,
            "/api/projects",
            Some(&cookie),
            Some(serde_json::json!({ "name": "public-proj" })),
        ))
        .await
        .expect("create project");
    let (status, _, project) = read_body(resp).await;
    assert_eq!(status, StatusCode::CREATED, "project: {project}");
    assert_eq!(
        project["public"],
        serde_json::json!(true),
        "non-admin project is public"
    );
    let project_id = project["id"].as_str().expect("project id").to_string();

    let resp = app
        .clone()
        .oneshot(req(
            Method::POST,
            "/api/sessions",
            Some(&cookie),
            Some(serde_json::json!({ "name": "pub-sess", "project_id": project_id })),
        ))
        .await
        .expect("create session");
    let (status, _, session) = read_body(resp).await;
    assert_eq!(status, StatusCode::CREATED, "session: {session}");

    // Anonymous (empty cookie) must still see the public project's sessions.
    let (status, _raw, body) =
        get_json(&app, "", &format!("/api/projects/{project_id}/sessions")).await;
    assert_eq!(status, StatusCode::OK, "anonymous project sessions: {body}");
    assert!(
        !body["sessions"]
            .as_array()
            .expect("sessions array")
            .is_empty(),
        "a public project's sessions must be visible to anonymous visitors"
    );
}
