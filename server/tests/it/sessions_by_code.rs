use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use http_body_util::BodyExt;
use migration::{Migrator, MigratorTrait};
use server::rate_limiter::NoOpRateLimiter;
use server::{AppState, AuthConfig, build_router};
use std::sync::Arc;
use std::time::Duration;
use tower::ServiceExt;
use uuid::Uuid;

const ORIGIN: &str = "http://localhost:5173";

async fn test_state() -> AppState {
    let db = sea_orm::Database::connect("sqlite::memory:")
        .await
        .expect("sqlite connect");
    Migrator::up(&db, None).await.expect("migrate up");
    let cfg = AuthConfig {
        jwt_signing_key: b"integration-test-secret-32-bytes-or-more-xxxxxxx".to_vec(),
        frontend_origins: vec![ORIGIN.to_string()],
        access_ttl: Duration::from_secs(900),
        refresh_ttl: Duration::from_secs(30 * 86_400),
        max_agents_per_session: 16,
    };
    let mut state = AppState::new(db, cfg);
    state.rate_limiter = Arc::new(NoOpRateLimiter);
    state
}

fn json_body(v: serde_json::Value) -> Body {
    Body::from(serde_json::to_vec(&v).expect("serialize body"))
}

async fn read_body(resp: axum::response::Response) -> (StatusCode, Vec<String>, serde_json::Value) {
    let status = resp.status();
    let cookies: Vec<String> = resp
        .headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .filter_map(|hv| hv.to_str().ok().map(|s| s.to_string()))
        .collect();
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
    (status, cookies, value)
}

fn cookie_pair(set_cookies: &[String], name: &str) -> Option<String> {
    for sc in set_cookies {
        if let Some(rest) = sc.strip_prefix(&format!("{}=", name)) {
            let value = rest.split(';').next().unwrap_or("");
            if value.is_empty() {
                continue;
            }
            return Some(format!("{}={}", name, value));
        }
    }
    None
}

async fn register_and_login(app: axum::Router, email: &str, password: &str) -> (Uuid, String) {
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
    let (status, _, body) = read_body(resp).await;
    assert_eq!(status, StatusCode::CREATED, "register failed: {body}");
    let user_id: Uuid =
        serde_json::from_value(body["id"].clone()).expect("user id present in register response");

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
    let (status, cookies, _) = read_body(resp).await;
    assert_eq!(status, StatusCode::OK, "login failed");
    let access = cookie_pair(&cookies, "arena_access").expect("access cookie present");
    (user_id, access)
}

fn req(
    method: Method,
    uri: &str,
    cookie: Option<&str>,
    body: Option<serde_json::Value>,
) -> Request<Body> {
    let mut b = Request::builder()
        .method(method)
        .uri(uri)
        .header(header::ORIGIN, ORIGIN);
    if let Some(c) = cookie {
        b = b.header(header::COOKIE, c);
    }
    let body = match body {
        Some(v) => {
            b = b.header(header::CONTENT_TYPE, "application/json");
            json_body(v)
        }
        None => Body::empty(),
    };
    b.body(body).expect("build request")
}

async fn create_project(app: axum::Router, access: &str) -> String {
    let resp = app
        .oneshot(req(
            Method::POST,
            "/api/projects",
            Some(access),
            Some(serde_json::json!({ "name": "Test Project" })),
        ))
        .await
        .expect("create project resp");
    let (status, _, body) = read_body(resp).await;
    assert_eq!(status, StatusCode::CREATED, "create project failed: {body}");
    body["id"].as_str().expect("project id").to_string()
}

#[tokio::test]
async fn get_by_code_returns_session_on_hit() {
    let state = test_state().await;
    let app = build_router(state);

    let (user_id, access) = register_and_login(app.clone(), "owner@example.com", "Pass1234!").await;
    let project_id = create_project(app.clone(), &access).await;

    // Create a session.
    let resp = app
        .clone()
        .oneshot(req(
            Method::POST,
            "/api/sessions",
            Some(&access),
            Some(serde_json::json!({
                "name": "Test Session",
                "project_id": project_id,
            })),
        ))
        .await
        .expect("create session resp");
    let (status, _, body) = read_body(resp).await;
    assert_eq!(status, StatusCode::CREATED, "create session failed: {body}");
    let session_id = body["id"].as_str().expect("session id").to_string();
    let join_code = body["join_code"].as_str().expect("join_code").to_string();

    // Look up by code — unauthenticated.
    let resp = app
        .clone()
        .oneshot(req(
            Method::GET,
            &format!("/api/sessions/by-code/{join_code}"),
            None,
            None,
        ))
        .await
        .expect("get by code resp");
    let (status, _, body) = read_body(resp).await;
    assert_eq!(status, StatusCode::OK, "expected 200: {body}");
    assert_eq!(body["id"].as_str().unwrap(), session_id);
    assert_eq!(body["join_code"].as_str().unwrap(), join_code);
    assert_eq!(body["state"].as_str().unwrap(), "lobby");
    assert_eq!(
        body["owner_id"].as_str().unwrap(),
        user_id.to_string(),
        "owner_id must match session owner"
    );

    // Confirm no extra fields.
    assert!(body.get("name").is_none(), "should not expose name field");
    assert!(
        body.get("project_title").is_none(),
        "should not expose project_title field"
    );
}

#[tokio::test]
async fn get_by_code_returns_404_on_miss() {
    let state = test_state().await;
    let app = build_router(state);

    let resp = app
        .oneshot(req(Method::GET, "/api/sessions/by-code/XXXXXX", None, None))
        .await
        .expect("get by code resp");
    let (status, _, body) = read_body(resp).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "expected 404: {body}");
    assert_eq!(body["error"].as_str().unwrap(), "not_found");
}
