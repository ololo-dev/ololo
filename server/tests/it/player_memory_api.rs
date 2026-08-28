//! `GET /api/sessions/:code/players/:player_id/memory` — session-memory view:
//! disabled without a project schema, defaults before extraction, extracted
//! values overlaid after, owner-only access (session owner ≠ player owner).

use axum::http::{Method, StatusCode};
use sea_orm::{ActiveModelTrait, ActiveValue::Set};
use server::build_router;
use tower::ServiceExt;
use uuid::Uuid;

use crate::common;
use crate::common::{read_body_json, register_and_login_default, req_with_cookie, test_state};

async fn create_session_with_memory(
    app: &axum::Router,
    cookie: &str,
    memory_schema: Option<serde_json::Value>,
) -> String {
    let mut body = serde_json::json!({ "name": format!("proj-{}", Uuid::new_v4()) });
    if let Some(ms) = memory_schema {
        body["memory_schema"] = ms;
    }
    let resp = app
        .clone()
        .oneshot(req_with_cookie(
            Method::POST,
            "/api/projects",
            cookie,
            Some(body),
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

#[tokio::test]
async fn memory_disabled_without_project_schema() {
    let state = test_state().await;
    let app = build_router(state.clone());
    let (_oid, cookie_owner) = register_and_login_default(app.clone(), "own-mem0@x.test").await;
    let (_jid, cookie_joiner) = register_and_login_default(app.clone(), "join-mem0@x.test").await;

    let code = create_session_with_memory(&app, &cookie_owner, None).await;
    let player_id = join_session(&app, &cookie_joiner, &code).await;

    let uri = format!("/api/sessions/{code}/players/{player_id}/memory");
    let resp = app
        .clone()
        .oneshot(req_with_cookie(Method::GET, &uri, &cookie_joiner, None))
        .await
        .expect("get memory");
    let (sc, body) = read_body_json(resp).await;
    assert_eq!(sc, StatusCode::OK, "{body}");
    assert_eq!(body["enabled"], false);
    assert_eq!(body["entries"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn memory_defaults_then_extracted_overlay() {
    let state = test_state().await;
    let app = build_router(state.clone());
    let (_oid, cookie_owner) = register_and_login_default(app.clone(), "own-mem1@x.test").await;
    let (_jid, cookie_joiner) = register_and_login_default(app.clone(), "join-mem1@x.test").await;

    let schema = serde_json::json!({"command": "sh answer.sh", "port": 8080});
    let code = create_session_with_memory(&app, &cookie_owner, Some(schema)).await;
    let player_id = join_session(&app, &cookie_joiner, &code).await;
    let uri = format!("/api/sessions/{code}/players/{player_id}/memory");

    // Before any extraction: defaults only.
    let resp = app
        .clone()
        .oneshot(req_with_cookie(Method::GET, &uri, &cookie_joiner, None))
        .await
        .expect("get memory");
    let (sc, body) = read_body_json(resp).await;
    assert_eq!(sc, StatusCode::OK, "{body}");
    assert_eq!(body["enabled"], true);
    assert!(body["updated_at"].is_null());
    let entries = body["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 2);
    let cmd = entries.iter().find(|e| e["key"] == "command").unwrap();
    assert_eq!(cmd["value"], "sh answer.sh");
    assert_eq!(cmd["extracted"], false);

    // Simulate a stored extraction (what game-server's extractor writes).
    // The session id must be resolved from the join code.
    let session = {
        use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
        arena_core::entities::sessions::Entity::find()
            .filter(arena_core::entities::sessions::Column::JoinCode.eq(code.clone()))
            .one(&state.db)
            .await
            .unwrap()
            .unwrap()
    };
    arena_core::entities::player_memory::ActiveModel {
        id: Set(Uuid::new_v4()),
        session_id_fk: Set(session.id),
        player_id_fk: Set(Uuid::parse_str(&player_id).unwrap()),
        values_json: Set(r#"{"command": "bun run answer.ts", "invented": "x"}"#.to_string()),
        source_hash: Set(Some("abc".to_string())),
        created_at: Set(chrono::Utc::now()),
        updated_at: Set(chrono::Utc::now()),
    }
    .insert(&state.db)
    .await
    .expect("insert player_memory");

    let resp = app
        .clone()
        .oneshot(req_with_cookie(Method::GET, &uri, &cookie_joiner, None))
        .await
        .expect("get memory 2");
    let (sc, body) = read_body_json(resp).await;
    assert_eq!(sc, StatusCode::OK, "{body}");
    assert!(body["updated_at"].is_string());
    let entries = body["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 2, "schema keys only — no invented keys");
    let cmd = entries.iter().find(|e| e["key"] == "command").unwrap();
    assert_eq!(cmd["value"], "bun run answer.ts");
    assert_eq!(cmd["extracted"], true);
    assert_eq!(cmd["default"], "sh answer.sh");
    let port = entries.iter().find(|e| e["key"] == "port").unwrap();
    assert_eq!(port["value"], "8080");
    assert_eq!(port["extracted"], false);
}

#[tokio::test]
async fn memory_is_owner_only() {
    let state = test_state().await;
    let app = build_router(state.clone());
    // First registrant becomes admin — keep that role for the session owner
    // and assert with a later-registered non-admin stranger.
    let (_oid, cookie_owner) = register_and_login_default(app.clone(), "own-mem2@x.test").await;
    let (_jid, cookie_joiner) = register_and_login_default(app.clone(), "join-mem2@x.test").await;
    let (_sid, cookie_stranger) =
        register_and_login_default(app.clone(), "stranger-mem2@x.test").await;

    let schema = serde_json::json!({"command": "sh answer.sh"});
    let code = create_session_with_memory(&app, &cookie_owner, Some(schema)).await;
    let player_id = join_session(&app, &cookie_joiner, &code).await;
    let uri = format!("/api/sessions/{code}/players/{player_id}/memory");

    // A non-admin user who does not own the player row → 403.
    let resp = app
        .clone()
        .oneshot(req_with_cookie(Method::GET, &uri, &cookie_stranger, None))
        .await
        .expect("get memory as stranger");
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);

    // The admin session owner may view it (admin bypass).
    let resp = app
        .clone()
        .oneshot(req_with_cookie(Method::GET, &uri, &cookie_owner, None))
        .await
        .expect("get memory as admin owner");
    assert_eq!(resp.status(), StatusCode::OK);
}
