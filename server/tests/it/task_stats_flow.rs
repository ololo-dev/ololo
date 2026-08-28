//! E2E integration tests for the task-stats flow:
//! POST /api/sessions/:code/players/:player_id/task-stats (CLI report)
//! GET  /api/sessions/:code/players/:player_id/task-stats (player page)

use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use http_body_util::BodyExt;
use server::build_router;
use tower::ServiceExt;
use uuid::Uuid;

use crate::common;
use crate::common::{ORIGIN, json_body, test_state};

async fn read_body(resp: axum::response::Response) -> (StatusCode, serde_json::Value) {
    let status = resp.status();
    let bytes = resp
        .into_body()
        .collect()
        .await
        .expect("collect body")
        .to_bytes();
    let value = if bytes.is_empty() {
        serde_json::Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null)
    };
    (status, value)
}

fn req_with_cookie(
    method: Method,
    uri: &str,
    cookie: &str,
    body: Option<serde_json::Value>,
) -> Request<Body> {
    let mut b = Request::builder()
        .method(method)
        .uri(uri)
        .header(header::ORIGIN, ORIGIN)
        .header(header::COOKIE, cookie);
    let body = match body {
        Some(v) => {
            b = b.header(header::CONTENT_TYPE, "application/json");
            json_body(v)
        }
        None => Body::empty(),
    };
    b.body(body).expect("build req")
}

async fn register_and_login(app: axum::Router, email: &str) -> String {
    let password = "password-12345";
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/auth/register")
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::ORIGIN, ORIGIN)
                .body(json_body(
                    serde_json::json!({ "email": email, "password": password }),
                ))
                .expect("register req"),
        )
        .await
        .expect("register resp");
    assert_eq!(resp.status(), StatusCode::CREATED, "register failed");

    let resp = app
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/auth/login")
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::ORIGIN, ORIGIN)
                .body(json_body(
                    serde_json::json!({ "email": email, "password": password }),
                ))
                .expect("login req"),
        )
        .await
        .expect("login resp");
    assert_eq!(resp.status(), StatusCode::OK, "login failed");
    resp.headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .filter_map(|hv| hv.to_str().ok())
        .find(|c| c.starts_with("arena_access="))
        .expect("access cookie")
        .split(';')
        .next()
        .expect("cookie value")
        .to_string()
}

async fn create_session(app: &axum::Router, cookie: &str) -> (String, String) {
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
    let (sc, pb) = read_body(resp).await;
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
    let (sc, sb) = read_body(resp).await;
    assert_eq!(sc, StatusCode::CREATED, "session: {sb}");
    (
        sb["id"].as_str().expect("session id").to_string(),
        sb["join_code"].as_str().expect("join_code").to_string(),
    )
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
    let (sc, body) = read_body(resp).await;
    assert_eq!(sc, StatusCode::CREATED, "join failed: {body}");
    body["player_id"].as_str().expect("player_id").to_string()
}

fn stats_report(ordinal: i32, input_tokens: u64) -> serde_json::Value {
    serde_json::json!({
        "task_ordinal": ordinal,
        "window_started_at_ms": 1_770_000_000_000i64,
        "window_ended_at_ms": 1_770_000_120_000i64,
        "agents": [{
            "agent": "claude",
            "agent_session_id": "sess-1",
            "model": "claude-fable-5",
            "input_tokens": input_tokens,
            "output_tokens": 200,
            "cache_read_tokens": 50,
            "cache_write_tokens": 10,
            "reasoning_tokens": 0,
            "cost": 0.05,
            "user_messages": 1,
            "assistant_messages": 4,
            "tool_calls": 6,
            "tools": {"Bash": 3, "Edit": 2, "Read": 1},
            "skills": {"drill-tdd": 1}
        }]
    })
}

#[tokio::test]
async fn post_then_get_round_trip() {
    let state = test_state().await;
    let app = build_router(state);

    let cookie_owner = register_and_login(app.clone(), "owner-ts1@x.test").await;
    let cookie_joiner = register_and_login(app.clone(), "joiner-ts1@x.test").await;
    let (_session_id, join_code) = create_session(&app, &cookie_owner).await;
    let player_id = join_session(&app, &cookie_joiner, &join_code).await;

    // POST by join code (the ololo path).
    let uri = format!("/api/sessions/{join_code}/players/{player_id}/task-stats");
    let resp = app
        .clone()
        .oneshot(req_with_cookie(
            Method::POST,
            &uri,
            &cookie_joiner,
            Some(stats_report(0, 1000)),
        ))
        .await
        .expect("post resp");
    let (status, body) = read_body(resp).await;
    assert_eq!(status, StatusCode::OK, "post failed: {body}");

    // GET returns the stored entry with aggregates and agent detail.
    let resp = app
        .clone()
        .oneshot(req_with_cookie(Method::GET, &uri, &cookie_joiner, None))
        .await
        .expect("get resp");
    let (status, body) = read_body(resp).await;
    assert_eq!(status, StatusCode::OK, "get failed: {body}");
    let entries = body["entries"].as_array().expect("entries array");
    assert_eq!(entries.len(), 1);
    let e = &entries[0];
    assert_eq!(e["task_ordinal"], 0);
    assert_eq!(e["input_tokens"], 1000);
    assert_eq!(e["tool_calls"], 6);
    assert_eq!(e["agents"][0]["agent"], "claude");
    assert_eq!(e["agents"][0]["tools"]["Bash"], 3);
    assert_eq!(e["agents"][0]["skills"]["drill-tdd"], 1);
}

#[tokio::test]
async fn post_twice_upserts_single_row() {
    let state = test_state().await;
    let app = build_router(state);

    let cookie_owner = register_and_login(app.clone(), "owner-ts2@x.test").await;
    let cookie_joiner = register_and_login(app.clone(), "joiner-ts2@x.test").await;
    let (_session_id, join_code) = create_session(&app, &cookie_owner).await;
    let player_id = join_session(&app, &cookie_joiner, &join_code).await;

    let uri = format!("/api/sessions/{join_code}/players/{player_id}/task-stats");
    for tokens in [500u64, 900u64] {
        let resp = app
            .clone()
            .oneshot(req_with_cookie(
                Method::POST,
                &uri,
                &cookie_joiner,
                Some(stats_report(3, tokens)),
            ))
            .await
            .expect("post resp");
        let (status, body) = read_body(resp).await;
        assert_eq!(status, StatusCode::OK, "post failed: {body}");
    }

    let resp = app
        .clone()
        .oneshot(req_with_cookie(Method::GET, &uri, &cookie_joiner, None))
        .await
        .expect("get resp");
    let (status, body) = read_body(resp).await;
    assert_eq!(status, StatusCode::OK);
    let entries = body["entries"].as_array().expect("entries");
    assert_eq!(entries.len(), 1, "resend upserts, no duplicate rows");
    assert_eq!(entries[0]["input_tokens"], 900, "latest report wins");
}

#[tokio::test]
async fn player_page_urls_accept_username() {
    let state = test_state().await;
    let db = state.db.clone();
    let app = build_router(state);

    let cookie_owner = register_and_login(app.clone(), "owner-ts4@x.test").await;
    let cookie_joiner = register_and_login(app.clone(), "joiner-ts4@x.test").await;
    let (_session_id, join_code) = create_session(&app, &cookie_owner).await;
    let player_id = join_session(&app, &cookie_joiner, &join_code).await;

    // Give the joiner an account username (the /u/<username> identifier).
    {
        use arena_core::entities::users;
        use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, Set};
        let user = users::Entity::find()
            .filter(users::Column::Email.eq("joiner-ts4@x.test"))
            .one(&db)
            .await
            .expect("query user")
            .expect("joiner user");
        let mut am: users::ActiveModel = user.into();
        am.username = Set(Some("anod".to_string()));
        am.update(&db).await.expect("set username");
    }
    let encoded = "anod";

    // Snapshot by username resolves to the same player.
    let resp = app
        .clone()
        .oneshot(req_with_cookie(
            Method::GET,
            &format!("/api/sessions/{join_code}/players/{encoded}"),
            &cookie_joiner,
            None,
        ))
        .await
        .expect("snapshot by name");
    let (status, body) = read_body(resp).await;
    assert_eq!(status, StatusCode::OK, "snapshot by username: {body}");
    assert_eq!(body["player_id"].as_str(), Some(player_id.as_str()));

    // Task-stats POST by UUID (ololo path), GET by display name (page path).
    let resp = app
        .clone()
        .oneshot(req_with_cookie(
            Method::POST,
            &format!("/api/sessions/{join_code}/players/{player_id}/task-stats"),
            &cookie_joiner,
            Some(stats_report(0, 700)),
        ))
        .await
        .expect("post stats");
    let (status, body) = read_body(resp).await;
    assert_eq!(status, StatusCode::OK, "post stats: {body}");

    let resp = app
        .clone()
        .oneshot(req_with_cookie(
            Method::GET,
            &format!("/api/sessions/{join_code}/players/{encoded}/task-stats"),
            &cookie_joiner,
            None,
        ))
        .await
        .expect("get stats by name");
    let (status, body) = read_body(resp).await;
    assert_eq!(status, StatusCode::OK, "get stats by name: {body}");
    assert_eq!(body["entries"].as_array().map(|a| a.len()), Some(1));

    // History by display name also resolves (empty repo → empty list).
    let resp = app
        .oneshot(req_with_cookie(
            Method::GET,
            &format!("/api/sessions/{join_code}/players/{encoded}/history"),
            &cookie_joiner,
            None,
        ))
        .await
        .expect("history by name");
    let (status, body) = read_body(resp).await;
    assert_eq!(status, StatusCode::OK, "history by name: {body}");
}

#[tokio::test]
async fn post_wrong_player_is_404_and_invalid_report_is_400() {
    let state = test_state().await;
    let app = build_router(state);

    let cookie_owner = register_and_login(app.clone(), "owner-ts3@x.test").await;
    let cookie_a = register_and_login(app.clone(), "joinera-ts3@x.test").await;
    let cookie_b = register_and_login(app.clone(), "joinerb-ts3@x.test").await;
    let (_session_id, join_code) = create_session(&app, &cookie_owner).await;
    let player_a = join_session(&app, &cookie_a, &join_code).await;
    join_session(&app, &cookie_b, &join_code).await;

    // B posting to A's player → 404.
    let uri = format!("/api/sessions/{join_code}/players/{player_a}/task-stats");
    let resp = app
        .clone()
        .oneshot(req_with_cookie(
            Method::POST,
            &uri,
            &cookie_b,
            Some(stats_report(0, 100)),
        ))
        .await
        .expect("post resp");
    let (status, _) = read_body(resp).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // Negative ordinal → 400.
    let resp = app
        .clone()
        .oneshot(req_with_cookie(
            Method::POST,
            &uri,
            &cookie_a,
            Some(stats_report(-1, 100)),
        ))
        .await
        .expect("post resp");
    let (status, body) = read_body(resp).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "body={body}");

    // Unknown field → 400 (deny_unknown_fields).
    let mut bad = stats_report(0, 100);
    bad["bogus"] = serde_json::json!(true);
    let resp = app
        .oneshot(req_with_cookie(Method::POST, &uri, &cookie_a, Some(bad)))
        .await
        .expect("post resp");
    let (status, _) = read_body(resp).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
}
