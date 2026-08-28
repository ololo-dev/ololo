//! Integration tests for the admin email template management API.
//!
//! Routes exercised:
//!   GET /api/admin/email-templates       — list all templates
//!   PUT /api/admin/email-templates/:type — update a template

use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use http_body_util::BodyExt;
use migration::{Migrator, MigratorTrait};
use server::{AppState, AuthConfig, build_router};
use std::time::Duration;
use tower::ServiceExt;

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
    AppState::new(db, cfg)
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
        if let Some(rest) = sc.strip_prefix(&format!("{name}=")) {
            let value = rest.split(';').next().unwrap_or("");
            if !value.is_empty() {
                return Some(format!("{name}={value}"));
            }
        }
    }
    None
}

/// Register + login, return access cookie header.
/// First registered user becomes admin.
async fn register_and_login(app: axum::Router, email: &str, password: &str) -> String {
    let register_resp = app
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
                .expect("build register req"),
        )
        .await
        .expect("register resp");
    let status = register_resp.status();
    let bytes = register_resp
        .into_body()
        .collect()
        .await
        .expect("collect register body")
        .to_bytes();
    let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
    assert_eq!(status, StatusCode::CREATED, "register: {body}");

    let login_resp = app
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/auth/login")
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::ORIGIN, ORIGIN)
                .body(json_body(
                    serde_json::json!({ "email": email, "password": password }),
                ))
                .expect("build login req"),
        )
        .await
        .expect("login resp");
    let login_status = login_resp.status();
    let cookies: Vec<String> = login_resp
        .headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .filter_map(|hv| hv.to_str().ok().map(String::from))
        .collect();
    let _ = login_resp.into_body().collect().await;
    assert_eq!(login_status, StatusCode::OK, "login");
    cookie_pair(&cookies, "arena_access").expect("access cookie")
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// Admin GET /api/admin/email-templates returns 200 with every seeded template.
#[tokio::test]
async fn test_get_email_templates_admin() {
    let app = build_router(test_state().await);
    let cookie = register_and_login(app.clone(), "admin@etpl1.test", "password-12345").await;

    let (status, body) = read_body(
        app.oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/api/admin/email-templates")
                .header(header::ORIGIN, ORIGIN)
                .header(header::COOKIE, &cookie)
                .body(Body::empty())
                .expect("build req"),
        )
        .await
        .expect("resp"),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "GET templates: {body}");
    let arr = body.as_array().expect("response should be array");
    // Assert the set, not a bare count: a missing type is the failure that
    // matters, and the count alone never said which one.
    let mut types: Vec<&str> = arr
        .iter()
        .map(|t| t["type"].as_str().expect("type"))
        .collect();
    types.sort_unstable();
    assert_eq!(
        types,
        ["magic_link", "reset_password", "verify"],
        "seeded templates; body={body}"
    );
}

/// Non-admin user gets 403 on GET /api/admin/email-templates.
#[tokio::test]
async fn test_get_email_templates_non_admin() {
    let app = build_router(test_state().await);

    // Register admin first (first user = admin).
    let _admin_cookie =
        register_and_login(app.clone(), "admin@etpl2a.test", "password-12345").await;

    // Register a second (non-admin) user.
    let non_admin_cookie =
        register_and_login(app.clone(), "user@etpl2b.test", "password-12345").await;

    let (status, _body) = read_body(
        app.oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/api/admin/email-templates")
                .header(header::ORIGIN, ORIGIN)
                .header(header::COOKIE, &non_admin_cookie)
                .body(Body::empty())
                .expect("build req"),
        )
        .await
        .expect("resp"),
    )
    .await;

    assert_eq!(status, StatusCode::FORBIDDEN, "non-admin should get 403");
}

/// PUT valid body with required placeholder → 200.
#[tokio::test]
async fn test_put_email_template_valid() {
    let app = build_router(test_state().await);
    let cookie = register_and_login(app.clone(), "admin@etpl3.test", "password-12345").await;

    let (status, body) = read_body(
        app.oneshot(
            Request::builder()
                .method(Method::PUT)
                .uri("/api/admin/email-templates/verify")
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::ORIGIN, ORIGIN)
                .header(header::COOKIE, &cookie)
                .body(json_body(serde_json::json!({
                    "subject": "Verify your account",
                    "body_html": "<p>Click here: {{VERIFY_URL}}</p>",
                    "body_text": "Click here: {{VERIFY_URL}}"
                })))
                .expect("build req"),
        )
        .await
        .expect("resp"),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "PUT valid template: {body}");
}

/// PUT body missing required placeholder → 422.
#[tokio::test]
async fn test_put_email_template_missing_placeholder() {
    let app = build_router(test_state().await);
    let cookie = register_and_login(app.clone(), "admin@etpl4.test", "password-12345").await;

    let (status, body) = read_body(
        app.oneshot(
            Request::builder()
                .method(Method::PUT)
                .uri("/api/admin/email-templates/verify")
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::ORIGIN, ORIGIN)
                .header(header::COOKIE, &cookie)
                .body(json_body(serde_json::json!({
                    "subject": "Verify your account",
                    "body_html": "<p>Click here</p>",
                    "body_text": "Click here"
                })))
                .expect("build req"),
        )
        .await
        .expect("resp"),
    )
    .await;

    assert_eq!(
        status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "missing placeholder should be 422; body={body}"
    );
    assert!(
        body["error"]
            .as_str()
            .unwrap_or("")
            .contains("{{VERIFY_URL}}"),
        "error should mention the placeholder; body={body}"
    );
}

/// PUT with invalid template type → 400.
#[tokio::test]
async fn test_put_email_template_invalid_type() {
    let app = build_router(test_state().await);
    let cookie = register_and_login(app.clone(), "admin@etpl5.test", "password-12345").await;

    let (status, body) = read_body(
        app.oneshot(
            Request::builder()
                .method(Method::PUT)
                .uri("/api/admin/email-templates/nonexistent")
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::ORIGIN, ORIGIN)
                .header(header::COOKIE, &cookie)
                .body(json_body(serde_json::json!({
                    "subject": "Test",
                    "body_html": "<p>Test</p>",
                    "body_text": "Test"
                })))
                .expect("build req"),
        )
        .await
        .expect("resp"),
    )
    .await;

    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "invalid type should be 400; body={body}"
    );
}
