#![allow(dead_code)]

//! Integration tests for PATCH /api/sessions/:session_id/players/:player_id

use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use http_body_util::BodyExt;
use server::build_router;
use tower::ServiceExt;
use uuid::Uuid;

const VALID_FINGERPRINT: &str = "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789";
const VALID_FINGERPRINT_B: &str =
    "1111110123456789abcdef0123456789abcdef0123456789abcdef0123456789";

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

async fn register_and_login(app: axum::Router, email: &str) -> (Uuid, String) {
    let password = "password-12345";
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/auth/register")
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::ORIGIN, ORIGIN)
                .body(json_body(serde_json::json!({
                    "email": email,
                    "password": password,
                })))
                .expect("register req"),
        )
        .await
        .expect("register resp");
    let (status, body) = read_body(resp).await;
    assert_eq!(status, StatusCode::CREATED, "register failed: {body}");
    let user_id: Uuid = serde_json::from_value(body["id"].clone()).expect("user id");

    let resp = app
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/auth/login")
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::ORIGIN, ORIGIN)
                .body(json_body(serde_json::json!({
                    "email": email,
                    "password": password,
                })))
                .expect("login req"),
        )
        .await
        .expect("login resp");
    let status = resp.status();
    let cookies: Vec<String> = resp
        .headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .filter_map(|hv| hv.to_str().ok().map(|s| s.to_string()))
        .collect();
    assert_eq!(status, StatusCode::OK, "login failed");
    let access = cookies
        .iter()
        .find(|c| c.starts_with("arena_access="))
        .expect("access cookie")
        .split(';')
        .next()
        .expect("cookie value")
        .to_string();
    (user_id, access)
}

/// Create a project + session, return (session_id, join_code).
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
    let session_id = sb["id"].as_str().expect("session id").to_string();
    let join_code = sb["join_code"].as_str().expect("join_code").to_string();
    (session_id, join_code)
}

/// Join a session, return player_id.
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
    body["player_id"]
        .as_str()
        .expect("player_id in join response")
        .to_string()
}

fn valid_metadata() -> serde_json::Value {
    serde_json::json!({
        "ai_agents": [{"name": "gpt-4"}],
        "build_tools": [{"name": "cargo"}],
        "languages": [{"name": "rust"}],
        "probe_duration_ms": 1234
    })
}

fn patch_body(fingerprint: &str) -> serde_json::Value {
    serde_json::json!({
        "fingerprint": fingerprint,
        "metadata": valid_metadata()
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

use crate::common;
use crate::common::{ORIGIN, json_body, test_state};

#[tokio::test]
async fn patch_player_metadata_400_item_too_long() {
    let state = test_state().await;
    let app = build_router(state);

    let (_, cookie_owner) = register_and_login(app.clone(), "owner-meta10@x.test").await;
    let (_, cookie_joiner) = register_and_login(app.clone(), "joiner-meta10@x.test").await;

    let (session_id, join_code) = create_session(&app, &cookie_owner).await;
    let player_id = join_session(&app, &cookie_joiner, &join_code).await;

    // A single 65-character name exceeds the 64-char per-item limit
    let long_name = "a".repeat(65);
    let resp = app
        .oneshot(req_with_cookie(
            Method::PATCH,
            &format!("/api/sessions/{session_id}/players/{player_id}"),
            &cookie_joiner,
            Some(serde_json::json!({
                "fingerprint": VALID_FINGERPRINT,
                "metadata": {
                    "ai_agents": [{"name": long_name}],
                    "build_tools": [],
                    "languages": [],
                    "probe_duration_ms": 0
                }
            })),
        ))
        .await
        .expect("patch resp");

    let (status, body) = read_body(resp).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "body={body}");
}

#[tokio::test]
async fn patch_player_metadata_400_unknown_fields() {
    let state = test_state().await;
    let app = build_router(state);

    let (_, cookie_owner) = register_and_login(app.clone(), "owner-meta8@x.test").await;
    let (_, cookie_joiner) = register_and_login(app.clone(), "joiner-meta8@x.test").await;

    let (session_id, join_code) = create_session(&app, &cookie_owner).await;
    let player_id = join_session(&app, &cookie_joiner, &join_code).await;

    let resp = app
        .oneshot(req_with_cookie(
            Method::PATCH,
            &format!("/api/sessions/{session_id}/players/{player_id}"),
            &cookie_joiner,
            Some(serde_json::json!({
                "fingerprint": VALID_FINGERPRINT,
                "metadata": valid_metadata(),
                "unknown_extra_field": "should_fail"
            })),
        ))
        .await
        .expect("patch resp");

    let (status, body) = read_body(resp).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "body={body}");
}
