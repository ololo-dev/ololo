//! WP-008 — pause / resume / cancel lifecycle + admin bypass.

use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use chrono::Utc;
use http_body_util::BodyExt;
use migration::{Migrator, MigratorTrait};
use sea_orm::{ActiveModelTrait, EntityTrait, Set};
use server::entities::{players, sessions, users};
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
    state.rate_limiter = Arc::new(server::rate_limiter::NoOpRateLimiter);
    state
}

fn json_body(v: serde_json::Value) -> Body {
    Body::from(serde_json::to_vec(&v).expect("serialize body"))
}

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

async fn read_body_with_cookies(
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
    b.body(body).expect("build req")
}

async fn login(app: axum::Router, email: &str, password: &str) -> String {
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
    let (status, cookies, _) = read_body_with_cookies(resp).await;
    assert_eq!(status, StatusCode::OK, "login failed");
    cookie_pair(&cookies, "arena_access").expect("access cookie present")
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
    let (status, _, body) = read_body_with_cookies(resp).await;
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
    let (status, cookies, _) = read_body_with_cookies(resp).await;
    assert_eq!(status, StatusCode::OK, "login failed");
    let access = cookie_pair(&cookies, "arena_access").expect("access cookie present");
    (user_id, access)
}

async fn create_project(
    app: &axum::Router,
    cookie: &str,
    name: &str,
) -> (StatusCode, serde_json::Value) {
    let resp = app
        .clone()
        .oneshot(req(
            Method::POST,
            "/api/projects",
            Some(cookie),
            Some(serde_json::json!({ "name": name })),
        ))
        .await
        .expect("create project resp");
    let (status, body) = read_body(resp).await;
    (status, body)
}

async fn create_session(
    app: &axum::Router,
    cookie: &str,
    name: &str,
) -> (StatusCode, serde_json::Value) {
    let (_, project) = create_project(app, cookie, "default-project").await;
    let project_id = project["id"]
        .as_str()
        .expect("project id from create_project");
    let resp = app
        .clone()
        .oneshot(req(
            Method::POST,
            "/api/sessions",
            Some(cookie),
            Some(serde_json::json!({ "name": name, "project_id": project_id })),
        ))
        .await
        .expect("create session resp");
    let (status, body) = read_body(resp).await;
    (status, body)
}

async fn patch_status(
    app: &axum::Router,
    cookie: &str,
    id: &str,
    new_status: &str,
) -> (StatusCode, serde_json::Value) {
    let resp = app
        .clone()
        .oneshot(req(
            Method::PATCH,
            &format!("/api/sessions/{id}"),
            Some(cookie),
            Some(serde_json::json!({ "status": new_status })),
        ))
        .await
        .expect("patch resp");
    let (s, b) = read_body(resp).await;
    (s, b)
}

async fn make_user_admin(state: &AppState, user_id: Uuid) {
    let user = users::Entity::find_by_id(user_id)
        .one(&state.db)
        .await
        .expect("find user")
        .expect("user exists");
    let mut am: users::ActiveModel = user.into();
    am.is_admin = Set(true);
    am.update(&state.db).await.expect("set is_admin");
}

async fn load_session(state: &AppState, id: Uuid) -> sessions::Model {
    sessions::Entity::find_by_id(id)
        .one(&state.db)
        .await
        .expect("load session")
        .expect("session exists")
}

// ---------------------------------------------------------------------------
// Transition edges
// ---------------------------------------------------------------------------

#[tokio::test]
async fn pause_running_session_ok() {
    let state = test_state().await;
    let app = build_router(state);
    let (_, cookie) = register_and_login(app.clone(), "alice@x.test", "password-12345").await;
    let (_, s) = create_session(&app, &cookie, "s1").await;
    let id = s["id"].as_str().expect("id");

    let (st, _) = patch_status(&app, &cookie, id, "running").await;
    assert_eq!(st, StatusCode::OK);
    let (st, body) = patch_status(&app, &cookie, id, "paused").await;
    assert_eq!(st, StatusCode::OK, "pause failed: {body}");
    assert_eq!(body["status"], serde_json::json!("paused"));
}

#[tokio::test]
async fn resume_paused_session_ok() {
    let state = test_state().await;
    let app = build_router(state);
    let (_, cookie) = register_and_login(app.clone(), "alice@x.test", "password-12345").await;
    let (_, s) = create_session(&app, &cookie, "s1").await;
    let id = s["id"].as_str().expect("id");

    patch_status(&app, &cookie, id, "running").await;
    patch_status(&app, &cookie, id, "paused").await;
    let (st, body) = patch_status(&app, &cookie, id, "running").await;
    assert_eq!(st, StatusCode::OK, "resume failed: {body}");
    assert_eq!(body["status"], serde_json::json!("running"));
}

#[tokio::test]
async fn cancel_paused_session_ok() {
    let state = test_state().await;
    let app = build_router(state);
    let (_, cookie) = register_and_login(app.clone(), "alice@x.test", "password-12345").await;
    let (_, s) = create_session(&app, &cookie, "s1").await;
    let id = s["id"].as_str().expect("id");

    patch_status(&app, &cookie, id, "running").await;
    patch_status(&app, &cookie, id, "paused").await;
    let (st, body) = patch_status(&app, &cookie, id, "cancelled").await;
    assert_eq!(st, StatusCode::OK, "cancel from paused failed: {body}");
    assert_eq!(body["status"], serde_json::json!("cancelled"));
}

#[tokio::test]
async fn paused_to_finished_rejected() {
    let state = test_state().await;
    let app = build_router(state);
    let (_, cookie) = register_and_login(app.clone(), "alice@x.test", "password-12345").await;
    let (_, s) = create_session(&app, &cookie, "s1").await;
    let id = s["id"].as_str().expect("id");

    patch_status(&app, &cookie, id, "running").await;
    patch_status(&app, &cookie, id, "paused").await;
    let (st, body) = patch_status(&app, &cookie, id, "finished").await;
    assert_eq!(st, StatusCode::CONFLICT, "must resume first: {body}");
}

// ---------------------------------------------------------------------------
// Timestamp logic
// ---------------------------------------------------------------------------

#[tokio::test]
async fn pause_sets_paused_at() {
    let state = test_state().await;
    let app = build_router(state.clone());
    let (_, cookie) = register_and_login(app.clone(), "alice@x.test", "password-12345").await;
    let (_, s) = create_session(&app, &cookie, "s1").await;
    let id = s["id"].as_str().expect("id");
    let sid = Uuid::parse_str(id).expect("uuid");

    patch_status(&app, &cookie, id, "running").await;
    let before = Utc::now();
    patch_status(&app, &cookie, id, "paused").await;
    let after = Utc::now();

    let row = load_session(&state, sid).await;
    let paused_at = row.paused_at.expect("paused_at set");
    assert!(
        paused_at >= before && paused_at <= after,
        "paused_at in window"
    );
}

#[tokio::test]
async fn resume_accumulates_and_clears_paused_at() {
    let state = test_state().await;
    let app = build_router(state.clone());
    let (_, cookie) = register_and_login(app.clone(), "alice@x.test", "password-12345").await;
    let (_, s) = create_session(&app, &cookie, "s1").await;
    let id = s["id"].as_str().expect("id");
    let sid = Uuid::parse_str(id).expect("uuid");

    patch_status(&app, &cookie, id, "running").await;
    patch_status(&app, &cookie, id, "paused").await;
    // sleep so num_seconds is at least 1.
    tokio::time::sleep(Duration::from_secs(2)).await;
    patch_status(&app, &cookie, id, "running").await;

    let row = load_session(&state, sid).await;
    assert!(row.paused_at.is_none(), "paused_at cleared on resume");
    let acc = row.paused_duration_secs.unwrap_or(0);
    assert!(acc >= 1, "paused_duration_secs accumulated: {acc}");
}

#[tokio::test]
async fn re_pause_sets_paused_at_again() {
    let state = test_state().await;
    let app = build_router(state.clone());
    let (_, cookie) = register_and_login(app.clone(), "alice@x.test", "password-12345").await;
    let (_, s) = create_session(&app, &cookie, "s1").await;
    let id = s["id"].as_str().expect("id");
    let sid = Uuid::parse_str(id).expect("uuid");

    patch_status(&app, &cookie, id, "running").await;
    patch_status(&app, &cookie, id, "paused").await;
    tokio::time::sleep(Duration::from_secs(1)).await;
    patch_status(&app, &cookie, id, "running").await;
    tokio::time::sleep(Duration::from_secs(1)).await;
    patch_status(&app, &cookie, id, "paused").await;

    let row = load_session(&state, sid).await;
    assert!(
        row.paused_at.is_some(),
        "paused_at set again after re-pause"
    );
    assert!(
        row.paused_duration_secs.unwrap_or(0) >= 1,
        "prior cycle retained"
    );
}

#[tokio::test]
async fn multi_cycle_paused_duration_accumulates() {
    let state = test_state().await;
    let app = build_router(state.clone());
    let (_, cookie) = register_and_login(app.clone(), "alice@x.test", "password-12345").await;
    let (_, s) = create_session(&app, &cookie, "s1").await;
    let id = s["id"].as_str().expect("id");
    let sid = Uuid::parse_str(id).expect("uuid");

    patch_status(&app, &cookie, id, "running").await;
    for _ in 0..3 {
        patch_status(&app, &cookie, id, "paused").await;
        tokio::time::sleep(Duration::from_secs(1)).await;
        patch_status(&app, &cookie, id, "running").await;
    }
    let row = load_session(&state, sid).await;
    let acc = row.paused_duration_secs.unwrap_or(0);
    assert!(acc >= 2, "multi-cycle accumulated: {acc}");
}

#[tokio::test]
async fn cancel_from_paused_clears_paused_at() {
    let state = test_state().await;
    let app = build_router(state.clone());
    let (_, cookie) = register_and_login(app.clone(), "alice@x.test", "password-12345").await;
    let (_, s) = create_session(&app, &cookie, "s1").await;
    let id = s["id"].as_str().expect("id");
    let sid = Uuid::parse_str(id).expect("uuid");

    patch_status(&app, &cookie, id, "running").await;
    patch_status(&app, &cookie, id, "paused").await;
    patch_status(&app, &cookie, id, "cancelled").await;

    let row = load_session(&state, sid).await;
    assert!(row.paused_at.is_none(), "paused_at cleared on cancel");
    assert!(row.finished_at.is_some(), "finished_at set on cancel");
}

// ---------------------------------------------------------------------------
// Admin bypass (C10)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn admin_can_pause_session_not_owned() {
    let state = test_state().await;
    let app = build_router(state.clone());
    let (admin_id, _) = register_and_login(app.clone(), "admin@x.test", "password-12345").await;
    make_user_admin(&state, admin_id).await;
    // Re-login admin to get a fresh cookie (admin flag is DB-only, no JWT claim).
    let admin_cookie = login(app.clone(), "admin@x.test", "password-12345").await;

    let (_, owner_cookie) = register_and_login(app.clone(), "owner@x.test", "password-12345").await;
    let (_, s) = create_session(&app, &owner_cookie, "owner-session").await;
    let id = s["id"].as_str().expect("id");
    let sid = Uuid::parse_str(id).expect("uuid");

    // Owner starts; admin pauses.
    patch_status(&app, &owner_cookie, id, "running").await;
    let (st, body) = patch_status(&app, &admin_cookie, id, "paused").await;
    assert_eq!(st, StatusCode::OK, "admin pause failed: {body}");

    let row = load_session(&state, sid).await;
    assert!(row.paused_at.is_some(), "admin pause set paused_at");
}

#[tokio::test]
async fn non_admin_member_403_on_pause() {
    let state = test_state().await;
    let app = build_router(state.clone());
    let (_, owner_cookie) = register_and_login(app.clone(), "owner@x.test", "password-12345").await;
    let (member_id, member_cookie) =
        register_and_login(app.clone(), "member@x.test", "password-12345").await;

    let (_, s) = create_session(&app, &owner_cookie, "owner-session").await;
    let id = s["id"].as_str().expect("id").to_string();
    let session_id = Uuid::parse_str(&id).expect("uuid");

    // Insert membership row for member.
    let am = players::ActiveModel {
        id: Set(Uuid::new_v4()),
        session_id_fk: Set(session_id),
        user_id_fk: Set(Some(member_id)),
        display_name: Set("member".into()),
        fingerprint: Set(None),
        metadata_json: Set(None),
        joined_at: Set(Utc::now()),
        reconnected_at: Set(None),
        revoked_at: Set(None),
        agent_connected: Set(false),
        agent_last_seen_at: Set(None),
    };
    am.insert(&state.db).await.expect("insert member");

    patch_status(&app, &owner_cookie, &id, "running").await;
    let (st, body) = patch_status(&app, &member_cookie, &id, "paused").await;
    assert_eq!(st, StatusCode::FORBIDDEN, "member should get 403: {body}");
    assert_eq!(body, serde_json::json!({"error":"forbidden"}));
}

#[tokio::test]
async fn non_admin_outsider_404_on_pause() {
    let state = test_state().await;
    let app = build_router(state);
    let (_, owner_cookie) = register_and_login(app.clone(), "owner@x.test", "password-12345").await;
    let (_, outsider_cookie) =
        register_and_login(app.clone(), "outsider@x.test", "password-12345").await;

    let (_, s) = create_session(&app, &owner_cookie, "owner-session").await;
    let id = s["id"].as_str().expect("id");

    patch_status(&app, &owner_cookie, id, "running").await;
    let (st, body) = patch_status(&app, &outsider_cookie, id, "paused").await;
    assert_eq!(st, StatusCode::NOT_FOUND, "outsider should get 404: {body}");
}
