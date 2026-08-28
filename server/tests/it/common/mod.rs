#![allow(dead_code)]

//! Shared harness for `server` integration tests.
//!
//! Every file under `tests/` is compiled as its own binary, so this module is
//! included via `mod common;` from each test file. It centralizes the
//! near-identical fixtures (in-memory SQLite, `AppState`, auth cookie helpers,
//! JSON request/response plumbing) that previously lived in every test file.

use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use http_body_util::BodyExt;
use migration::{Migrator, MigratorTrait};
use server::{AppState, AuthConfig};
use std::time::Duration;
use tower::ServiceExt;
use uuid::Uuid;

pub const ORIGIN: &str = "http://localhost:5173";
pub const DEFAULT_PASSWORD: &str = "password-12345";
pub const TEST_JWT_SECRET: &[u8] = b"integration-test-secret-32-bytes-or-more-xxxxxxx";

/// Standard `AuthConfig` used by the integration suite.
pub fn default_cfg() -> AuthConfig {
    AuthConfig {
        jwt_signing_key: TEST_JWT_SECRET.to_vec(),
        frontend_origins: vec![ORIGIN.to_string()],
        access_ttl: Duration::from_secs(900),
        refresh_ttl: Duration::from_secs(30 * 86_400),
        max_agents_per_session: 16,
    }
}

/// Fresh in-memory SQLite database with migrations applied and a default
/// `AppState`.
pub async fn test_state() -> AppState {
    let db = sea_orm::Database::connect("sqlite::memory:")
        .await
        .expect("sqlite connect");
    Migrator::up(&db, None).await.expect("migrate up");
    AppState::new(db, default_cfg())
}

/// Build an `AppState` from an explicit `AuthConfig`.
pub async fn test_state_with_cfg(cfg: AuthConfig) -> AppState {
    let db = sea_orm::Database::connect("sqlite::memory:")
        .await
        .expect("sqlite connect");
    Migrator::up(&db, None).await.expect("migrate up");
    AppState::new(db, cfg)
}

/// Serialize a `serde_json::Value` into a request body.
pub fn json_body(v: serde_json::Value) -> Body {
    Body::from(serde_json::to_vec(&v).expect("serialize body"))
}

/// Drain a response into `(status, set-cookie headers, parsed JSON body)`.
///
/// Most tests use the 3-tuple form. `read_body_json` is provided for the
/// smaller set of tests that only care about status + body.
pub async fn read_body(
    resp: axum::response::Response,
) -> (StatusCode, Vec<String>, serde_json::Value) {
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

/// 2-tuple variant of [`read_body`] (status + parsed body, no cookies).
pub async fn read_body_json(resp: axum::response::Response) -> (StatusCode, serde_json::Value) {
    let (status, _, value) = read_body(resp).await;
    (status, value)
}

/// Extract a single `name=value` cookie from a set-cookie header list.
pub fn cookie_pair(set_cookies: &[String], name: &str) -> Option<String> {
    for sc in set_cookies {
        if let Some(rest) = sc.strip_prefix(&format!("{name}=")) {
            let value = rest.split(';').next().unwrap_or("");
            if value.is_empty() {
                continue;
            }
            return Some(format!("{name}={value}"));
        }
    }
    None
}

/// Register + login, returning `(user_id, access-cookie)`.
pub async fn register_and_login(app: axum::Router, email: &str, password: &str) -> (Uuid, String) {
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
    let user_id: Uuid = serde_json::from_value(body["id"].clone()).expect("user id present");

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

/// `register_and_login` with the standard test password.
pub async fn register_and_login_default(app: axum::Router, email: &str) -> (Uuid, String) {
    register_and_login(app, email, DEFAULT_PASSWORD).await
}

/// Build a request with an optional auth cookie and optional JSON body.
pub fn req(
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
    b.body(body).expect("build req")
}

/// Build a request that always carries an auth cookie and optional JSON body.
pub fn req_with_cookie(
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

/// Promote a user to admin directly in the DB.
pub async fn make_user_admin(state: &AppState, user_id: Uuid) {
    use sea_orm::ActiveModelTrait;
    use sea_orm::EntityTrait;
    use sea_orm::Set;
    use server::entities::users;
    let user = users::Entity::find_by_id(user_id)
        .one(&state.db)
        .await
        .expect("find user")
        .expect("user exists");
    let mut am: users::ActiveModel = user.into();
    am.is_admin = Set(true);
    am.update(&state.db).await.expect("set is_admin");
}

/// Helper: set `allow_user_project_creation` via PUT /api/admin/settings (admin required).
pub async fn set_project_creation_setting(app: &axum::Router, admin_cookie: &str, value: &str) {
    let resp = app
        .clone()
        .oneshot(req(
            Method::PUT,
            "/api/admin/settings",
            Some(admin_cookie),
            Some(serde_json::json!({
                "key": "allow_user_project_creation",
                "value": value,
            })),
        ))
        .await
        .expect("set project creation resp");
    let (status, _, body) = read_body(resp).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "set project creation={value}: {body}"
    );
}
