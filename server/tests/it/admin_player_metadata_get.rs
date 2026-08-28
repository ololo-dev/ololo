//! Integration tests for GET /api/sessions/:code/players/metadata

use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use http_body_util::BodyExt;

use server::build_router;
use tower::ServiceExt;
use uuid::Uuid;

const VALID_FINGERPRINT: &str = "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789";

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

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

use crate::common;
use crate::common::{ORIGIN, json_body, make_user_admin, test_state};

#[tokio::test]
async fn get_metadata_200_with_populated_data() {
    let state = test_state().await;
    let app = build_router(state.clone());

    let (admin_id, _cookie_admin) = register_and_login(app.clone(), "admin-meta-pop@x.test").await;
    make_user_admin(&state, admin_id).await;
    // Re-login to get a fresh token after admin promotion
    let (_, cookie_admin) = {
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/auth/login")
                    .header(header::CONTENT_TYPE, "application/json")
                    .header(header::ORIGIN, ORIGIN)
                    .body(json_body(serde_json::json!({
                        "email": "admin-meta-pop@x.test",
                        "password": "password-12345",
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
        assert_eq!(status, StatusCode::OK);
        let access = cookies
            .iter()
            .find(|c| c.starts_with("arena_access="))
            .expect("access cookie")
            .split(';')
            .next()
            .expect("cookie value")
            .to_string();
        (admin_id, access)
    };

    let (_session_id, join_code) = create_session(&app, &cookie_admin).await;

    let (_, cookie_joiner) = register_and_login(app.clone(), "joiner-meta-pop@x.test").await;
    let player_id = join_session(&app, &cookie_joiner, &join_code).await;

    // Patch metadata for the joiner's player
    let resp = app
        .clone()
        .oneshot(req_with_cookie(
            Method::PATCH,
            &format!("/api/sessions/{join_code}/players/{player_id}"),
            &cookie_joiner,
            Some(serde_json::json!({
                "fingerprint": VALID_FINGERPRINT,
                "metadata": {
                    "ai_agents": [{"name": "gpt-4"}],
                    "build_tools": [{"name": "cargo"}],
                    "languages": [{"name": "rust"}],
                    "probe_duration_ms": 999
                }
            })),
        ))
        .await
        .expect("patch resp");
    let (sc, _) = read_body(resp).await;
    assert_eq!(sc, StatusCode::OK);

    // GET metadata as admin
    let resp = app
        .oneshot(req_with_cookie(
            Method::GET,
            &format!("/api/sessions/{join_code}/players/metadata"),
            &cookie_admin,
            None,
        ))
        .await
        .expect("get metadata resp");
    let (status, body) = read_body(resp).await;
    assert_eq!(status, StatusCode::OK, "body={body}");

    let obj = body.as_object().expect("response is object");
    assert!(obj.contains_key(&player_id), "player_id key missing");
    let entry = &obj[&player_id];
    assert_eq!(entry["ai_agents"], serde_json::json!([{"name": "gpt-4"}]));
    assert_eq!(entry["build_tools"], serde_json::json!([{"name": "cargo"}]));
    assert_eq!(entry["languages"], serde_json::json!([{"name": "rust"}]));
}

#[tokio::test]
async fn get_metadata_200_null_metadata() {
    let state = test_state().await;
    let app = build_router(state.clone());

    let (admin_id, _) = register_and_login(app.clone(), "admin-meta-null@x.test").await;
    make_user_admin(&state, admin_id).await;
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/auth/login")
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::ORIGIN, ORIGIN)
                .body(json_body(serde_json::json!({
                    "email": "admin-meta-null@x.test",
                    "password": "password-12345",
                })))
                .expect("login"),
        )
        .await
        .expect("login resp");
    let cookies: Vec<String> = resp
        .headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .filter_map(|hv| hv.to_str().ok().map(|s| s.to_string()))
        .collect();
    let cookie_admin = cookies
        .iter()
        .find(|c| c.starts_with("arena_access="))
        .expect("access cookie")
        .split(';')
        .next()
        .unwrap()
        .to_string();

    let (_, join_code) = create_session(&app, &cookie_admin).await;

    let (_, cookie_joiner) = register_and_login(app.clone(), "joiner-meta-null@x.test").await;
    let player_id = join_session(&app, &cookie_joiner, &join_code).await;

    let resp = app
        .oneshot(req_with_cookie(
            Method::GET,
            &format!("/api/sessions/{join_code}/players/metadata"),
            &cookie_admin,
            None,
        ))
        .await
        .expect("get metadata resp");
    let (status, body) = read_body(resp).await;
    assert_eq!(status, StatusCode::OK, "body={body}");

    let obj = body.as_object().expect("response is object");
    assert!(obj.contains_key(&player_id));
    let entry = &obj[&player_id];
    assert_eq!(entry["ai_agents"], serde_json::json!([]));
    assert_eq!(entry["build_tools"], serde_json::json!([]));
    assert_eq!(entry["languages"], serde_json::json!([]));
    assert!(entry["fingerprint"].is_null());
    assert!(entry["probe_duration_ms"].is_null());
}

#[tokio::test]
async fn get_metadata_200_empty_session() {
    let state = test_state().await;
    let app = build_router(state.clone());

    let (admin_id, _) = register_and_login(app.clone(), "admin-meta-empty@x.test").await;
    make_user_admin(&state, admin_id).await;
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/auth/login")
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::ORIGIN, ORIGIN)
                .body(json_body(serde_json::json!({
                    "email": "admin-meta-empty@x.test",
                    "password": "password-12345",
                })))
                .expect("login"),
        )
        .await
        .expect("login resp");
    let cookies: Vec<String> = resp
        .headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .filter_map(|hv| hv.to_str().ok().map(|s| s.to_string()))
        .collect();
    let cookie_admin = cookies
        .iter()
        .find(|c| c.starts_with("arena_access="))
        .expect("access cookie")
        .split(';')
        .next()
        .unwrap()
        .to_string();

    let (_, join_code) = create_session(&app, &cookie_admin).await;

    let resp = app
        .oneshot(req_with_cookie(
            Method::GET,
            &format!("/api/sessions/{join_code}/players/metadata"),
            &cookie_admin,
            None,
        ))
        .await
        .expect("get metadata resp");
    let (status, body) = read_body(resp).await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    let obj = body.as_object().expect("response is object");
    assert!(obj.is_empty(), "expected empty object, got {obj:?}");
}

#[tokio::test]
async fn get_metadata_401_unauthenticated() {
    let state = test_state().await;
    let app = build_router(state.clone());

    let (admin_id, cookie_admin) = register_and_login(app.clone(), "admin-meta-401@x.test").await;
    make_user_admin(&state, admin_id).await;

    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/auth/login")
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::ORIGIN, ORIGIN)
                .body(json_body(serde_json::json!({
                    "email": "admin-meta-401@x.test",
                    "password": "password-12345",
                })))
                .expect("login"),
        )
        .await
        .expect("login resp");
    let cookies: Vec<String> = resp
        .headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .filter_map(|hv| hv.to_str().ok().map(|s| s.to_string()))
        .collect();
    let cookie_admin_fresh = cookies
        .iter()
        .find(|c| c.starts_with("arena_access="))
        .map(|c| c.split(';').next().unwrap().to_string())
        .unwrap_or(cookie_admin);

    let (_, join_code) = create_session(&app, &cookie_admin_fresh).await;

    // Request without any cookie
    let resp = app
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri(format!("/api/sessions/{join_code}/players/metadata"))
                .header(header::ORIGIN, ORIGIN)
                .body(Body::empty())
                .expect("req"),
        )
        .await
        .expect("resp");
    let (status, _) = read_body(resp).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn get_metadata_403_non_admin() {
    let state = test_state().await;
    let app = build_router(state.clone());

    // First registered user becomes admin automatically; register them first.
    let (admin_id, cookie_admin) =
        register_and_login(app.clone(), "admin-meta-403-first@x.test").await;
    let _ = (admin_id, cookie_admin);

    // Second user is non-admin.
    let (_, cookie_user) = register_and_login(app.clone(), "nonadmin-meta-403@x.test").await;
    let (_, join_code) = create_session(&app, &cookie_user).await;

    let resp = app
        .oneshot(req_with_cookie(
            Method::GET,
            &format!("/api/sessions/{join_code}/players/metadata"),
            &cookie_user,
            None,
        ))
        .await
        .expect("resp");
    let (status, _) = read_body(resp).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}
