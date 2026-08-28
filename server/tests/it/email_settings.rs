//! Integration tests for WP-013: email settings allowlist, redaction, sentinel rejection,
//! and encryption-on-write for `email.secret_access_key`.

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

/// Register + login as the first user (who becomes admin). Returns access cookie header.
async fn register_and_login_admin(app: axum::Router) -> String {
    let email = "admin@example.com";
    let password = "hunter2";

    let register_resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/auth/register")
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::ORIGIN, ORIGIN)
                .body(json_body(serde_json::json!({
                    "email": email,
                    "password": password
                })))
                .expect("build register req"),
        )
        .await
        .expect("register resp");
    let reg_status = register_resp.status();
    let reg_bytes = register_resp
        .into_body()
        .collect()
        .await
        .expect("collect")
        .to_bytes();
    let reg_body: serde_json::Value = serde_json::from_slice(&reg_bytes).unwrap_or_default();
    assert_eq!(reg_status, StatusCode::CREATED, "register: {reg_body}");

    let login_resp = app
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/auth/login")
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::ORIGIN, ORIGIN)
                .body(json_body(serde_json::json!({
                    "email": email,
                    "password": password
                })))
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
    cookie_pair(&cookies, "arena_access").expect("access cookie after login")
}

async fn put_setting(
    app: axum::Router,
    cookie: &str,
    key: &str,
    value: &str,
) -> (StatusCode, serde_json::Value) {
    let resp = app
        .oneshot(
            Request::builder()
                .method(Method::PUT)
                .uri("/api/admin/settings")
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::ORIGIN, ORIGIN)
                .header(header::COOKIE, cookie)
                .body(json_body(serde_json::json!({ "key": key, "value": value })))
                .unwrap(),
        )
        .await
        .unwrap();
    read_body(resp).await
}

async fn get_settings(app: axum::Router, cookie: &str) -> (StatusCode, serde_json::Value) {
    let resp = app
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/api/admin/settings")
                .header(header::ORIGIN, ORIGIN)
                .header(header::COOKIE, cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    read_body(resp).await
}

/// PUT secret_access_key with real value → 200, then GET → value is "[redacted]".
#[tokio::test]
async fn test_get_settings_redacts_secret_access_key() {
    let state = test_state().await;
    let app = build_router(state);

    let cookie = register_and_login_admin(app.clone()).await;

    let (status, body) = put_setting(
        app.clone(),
        &cookie,
        "email.secret_access_key",
        "AKIAIOSFODNN7EXAMPLE",
    )
    .await;
    assert_eq!(status, StatusCode::OK, "PUT failed: {body}");

    let (status, body) = get_settings(app.clone(), &cookie).await;
    assert_eq!(status, StatusCode::OK, "GET failed: {body}");

    let returned = body
        .get("email.secret_access_key")
        .and_then(|v| v.as_str())
        .expect("key present in response");
    assert_eq!(
        returned, "[redacted]",
        "secret_access_key must be redacted in GET response"
    );
}

/// PUT with sentinel value "[redacted]" → 400 sentinel_value_not_allowed.
#[tokio::test]
async fn test_put_settings_sentinel_rejected() {
    let state = test_state().await;
    let app = build_router(state);

    let cookie = register_and_login_admin(app.clone()).await;

    let (status, body) = put_setting(
        app.clone(),
        &cookie,
        "email.secret_access_key",
        "[redacted]",
    )
    .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "Expected 400, got {status}: {body}"
    );
    assert_eq!(
        body.get("error").and_then(|v| v.as_str()),
        Some("sentinel_value_not_allowed")
    );
}

/// PUT real secret → 200; stored DB value must differ from input (it was encrypted).
#[tokio::test]
async fn test_put_settings_secret_encrypted() {
    use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
    use server::entities::app_settings;

    let state = test_state().await;
    let db = state.db.clone();
    let app = build_router(state);

    let cookie = register_and_login_admin(app.clone()).await;

    let plaintext = "AKIAIOSFODNN7EXAMPLE";
    let (status, body) =
        put_setting(app.clone(), &cookie, "email.secret_access_key", plaintext).await;
    assert_eq!(status, StatusCode::OK, "PUT failed: {body}");

    // Read raw value from DB.
    let row = app_settings::Entity::find()
        .filter(app_settings::Column::Key.eq("email.secret_access_key"))
        .one(&db)
        .await
        .expect("db query")
        .expect("row must exist");

    assert_ne!(
        row.value, plaintext,
        "stored value must be encrypted, not plaintext"
    );
}

/// The Cloudflare API token is a secret: encrypted at rest, redacted on read.
#[tokio::test]
async fn test_cloudflare_token_encrypted_and_redacted() {
    use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
    use server::entities::app_settings;

    let state = test_state().await;
    let db = state.db.clone();
    let app = build_router(state);

    let cookie = register_and_login_admin(app.clone()).await;

    let plaintext = "cf-token-plaintext-value";
    let (status, body) = put_setting(
        app.clone(),
        &cookie,
        "email.cloudflare_api_token",
        plaintext,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "PUT failed: {body}");

    let row = app_settings::Entity::find()
        .filter(app_settings::Column::Key.eq("email.cloudflare_api_token"))
        .one(&db)
        .await
        .expect("db query")
        .expect("row must exist");
    assert_ne!(row.value, plaintext, "token must be encrypted at rest");

    let (status, body) = get_settings(app.clone(), &cookie).await;
    assert_eq!(status, StatusCode::OK, "GET failed: {body}");
    assert_eq!(
        body.get("email.cloudflare_api_token")
            .and_then(|v| v.as_str()),
        Some("[redacted]"),
        "token must be redacted in GET response"
    );
}

/// `email.provider` accepts ses/cloudflare (case-insensitive) and rejects others.
#[tokio::test]
async fn test_email_provider_validated() {
    let state = test_state().await;
    let app = build_router(state);

    let cookie = register_and_login_admin(app.clone()).await;

    let (status, body) = put_setting(app.clone(), &cookie, "email.provider", "cloudflare").await;
    assert_eq!(status, StatusCode::OK, "cloudflare rejected: {body}");

    let (status, body) = put_setting(app.clone(), &cookie, "email.provider", "SES").await;
    assert_eq!(status, StatusCode::OK, "uppercase SES rejected: {body}");

    let (status, body) = put_setting(app.clone(), &cookie, "email.provider", "sendgrid").await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "sendgrid accepted: {body}");
    assert_eq!(
        body.get("error").and_then(|v| v.as_str()),
        Some("invalid_email_provider")
    );

    // Cloudflare account id is a plain (non-secret) allowlisted key.
    let (status, body) = put_setting(
        app.clone(),
        &cookie,
        "email.cloudflare_account_id",
        "abc123",
    )
    .await;
    assert_eq!(status, StatusCode::OK, "account id rejected: {body}");
}

/// PUT with unknown key → 400 unknown_key.
#[tokio::test]
async fn test_put_settings_invalid_key() {
    let state = test_state().await;
    let app = build_router(state);

    let cookie = register_and_login_admin(app.clone()).await;

    let (status, body) =
        put_setting(app.clone(), &cookie, "email.totally_unknown_key", "value").await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "Expected 400, got {status}: {body}"
    );
    assert_eq!(
        body.get("error").and_then(|v| v.as_str()),
        Some("unknown_key")
    );
}
