//! Integration tests for the AI project-action endpoints.
//!
//! Uses `FakeLlmService` injected via `AppState::with_llm_service()` so no
//! real Ollama process is required.
//!
//! Covered:
//!   beautify-description: 200, 400-empty, 400-too-long, 401, 403, 404, 422-no-model
//!   generate-tasks:       200, 400-empty, 403, 502-parse-error

use async_trait::async_trait;
use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use http_body_util::BodyExt;
use migration::{Migrator, MigratorTrait};
use server::llm::{LlmError, LlmService};
use server::{AppState, AuthConfig, build_router};
use std::sync::Arc;
use std::time::Duration;
use tower::ServiceExt;
use uuid::Uuid;

const ORIGIN: &str = "http://localhost:5173";

// ─── Fake LLM implementation ──────────────────────────────────────────────────

/// Returns a fixed string for every `complete()` call.
struct FakeLlmService {
    response: String,
}

#[async_trait]
impl LlmService for FakeLlmService {
    async fn complete(
        &self,
        _cfg: &arena_core::llm::ModelConfig,
        _system: &str,
        _user: &str,
    ) -> Result<String, LlmError> {
        Ok(self.response.clone())
    }
}

fn fake_llm(response: impl Into<String>) -> Arc<FakeLlmService> {
    Arc::new(FakeLlmService {
        response: response.into(),
    })
}

const VALID_TASKS_JSON: &str = r#"[{"title":"Setup","description":"Set up the environment.","command_template":"```bash\nnpm install\n```"}]"#;

// ─── Shared test helpers ──────────────────────────────────────────────────────

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

/// State with fake LLM injected. `seed_llm_defaults` converts the
/// migration-seeded legacy settings (`ai_provider = "ollama"`,
/// `ai_model = "llama3.2"`) into a provider row + `llm_default` assignment,
/// so the operation-model chain resolves.
async fn test_state_with_llm(response: impl Into<String>) -> AppState {
    let state = test_state().await;
    server::seed::seed_llm_defaults(&state.db).await;
    state.with_llm_service(fake_llm(response))
}

/// State without any LLM provider/assignment seeded — the operation-model
/// chain resolves to `None`, triggering NoModelConfigured.
async fn test_state_no_model() -> AppState {
    test_state()
        .await
        .with_llm_service(fake_llm("should not be reached"))
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

async fn register_and_login(app: axum::Router, email: &str, password: &str) -> (Uuid, String) {
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
                .unwrap(),
        )
        .await
        .unwrap();
    let status = resp.status();
    let _cookies: Vec<String> = resp
        .headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .filter_map(|hv| hv.to_str().ok().map(|s| s.to_string()))
        .collect();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(status, StatusCode::CREATED, "register: {body}");
    let user_id: Uuid = serde_json::from_value(body["id"].clone()).expect("user id");

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
                .unwrap(),
        )
        .await
        .unwrap();
    let login_status = resp.status();
    let login_cookies: Vec<String> = resp
        .headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .filter_map(|hv| hv.to_str().ok().map(|s| s.to_string()))
        .collect();
    resp.into_body().collect().await.unwrap();
    assert_eq!(login_status, StatusCode::OK, "login");
    let access = cookie_pair(&login_cookies, "arena_access").expect("access cookie");
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
    b.body(body).expect("build req")
}

async fn create_project(app: &axum::Router, cookie: &str, name: &str) -> String {
    let resp = app
        .clone()
        .oneshot(req(
            Method::POST,
            "/api/projects",
            Some(cookie),
            Some(serde_json::json!({ "name": name, "description": "A project description for AI testing." })),
        ))
        .await
        .unwrap();
    let (status, body) = read_body(resp).await;
    assert_eq!(status, StatusCode::CREATED, "create project: {body}");
    body["id"].as_str().expect("project id").to_string()
}

// ─── beautify-description tests ───────────────────────────────────────────────

#[tokio::test]
async fn beautify_200_ok() {
    let state = test_state_with_llm("A polished project description.").await;
    let app = build_router(state);
    let (_, cookie) = register_and_login(app.clone(), "alice@ai.test", "pass-12345678").await;
    let pid = create_project(&app, &cookie, "AI Project").await;

    let resp = app
        .oneshot(req(
            Method::POST,
            &format!("/api/projects/{pid}/beautify-description"),
            Some(&cookie),
            Some(serde_json::json!({ "description": "some rough description" })),
        ))
        .await
        .unwrap();
    let (status, body) = read_body(resp).await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    assert_eq!(
        body["description"],
        serde_json::json!("A polished project description."),
        "returned description should match fake LLM output"
    );
}

#[tokio::test]
async fn beautify_400_empty_description() {
    let state = test_state_with_llm("ignored").await;
    let app = build_router(state);
    let (_, cookie) = register_and_login(app.clone(), "alice@ai.test", "pass-12345678").await;
    let pid = create_project(&app, &cookie, "AI Project").await;

    let resp = app
        .oneshot(req(
            Method::POST,
            &format!("/api/projects/{pid}/beautify-description"),
            Some(&cookie),
            Some(serde_json::json!({ "description": "   " })),
        ))
        .await
        .unwrap();
    let (status, body) = read_body(resp).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "body={body}");
    assert_eq!(body["error"], serde_json::json!("description_empty"));
}

#[tokio::test]
async fn beautify_400_description_too_long() {
    let state = test_state_with_llm("ignored").await;
    let app = build_router(state);
    let (_, cookie) = register_and_login(app.clone(), "alice@ai.test", "pass-12345678").await;
    let pid = create_project(&app, &cookie, "AI Project").await;

    let too_long = "x".repeat(10_001);
    let resp = app
        .oneshot(req(
            Method::POST,
            &format!("/api/projects/{pid}/beautify-description"),
            Some(&cookie),
            Some(serde_json::json!({ "description": too_long })),
        ))
        .await
        .unwrap();
    let (status, body) = read_body(resp).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "body={body}");
    assert_eq!(body["error"], serde_json::json!("description_too_long"));
}

#[tokio::test]
async fn beautify_401_unauthenticated() {
    let state = test_state_with_llm("ignored").await;
    let app = build_router(state);
    let fake_id = Uuid::new_v4();

    let resp = app
        .oneshot(req(
            Method::POST,
            &format!("/api/projects/{fake_id}/beautify-description"),
            None, // no cookie
            Some(serde_json::json!({ "description": "some description" })),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn beautify_403_nonowner() {
    let state = test_state_with_llm("ignored").await;
    let app = build_router(state);
    let (_, cookie_a) = register_and_login(app.clone(), "alice@ai.test", "pass-12345678").await;
    let (_, cookie_b) = register_and_login(app.clone(), "bob@ai.test", "pass-12345678").await;
    let pid = create_project(&app, &cookie_a, "Alice's Project").await;

    let resp = app
        .oneshot(req(
            Method::POST,
            &format!("/api/projects/{pid}/beautify-description"),
            Some(&cookie_b),
            Some(serde_json::json!({ "description": "some description" })),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn beautify_404_project_not_found() {
    let state = test_state_with_llm("ignored").await;
    let app = build_router(state);
    let (_, cookie) = register_and_login(app.clone(), "alice@ai.test", "pass-12345678").await;
    let fake_id = Uuid::new_v4();

    let resp = app
        .oneshot(req(
            Method::POST,
            &format!("/api/projects/{fake_id}/beautify-description"),
            Some(&cookie),
            Some(serde_json::json!({ "description": "some description" })),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn beautify_422_no_model_configured() {
    let state = test_state_no_model().await;
    let app = build_router(state);
    let (_, cookie) = register_and_login(app.clone(), "alice@ai.test", "pass-12345678").await;
    let pid = create_project(&app, &cookie, "AI Project").await;

    let resp = app
        .oneshot(req(
            Method::POST,
            &format!("/api/projects/{pid}/beautify-description"),
            Some(&cookie),
            Some(serde_json::json!({ "description": "some description" })),
        ))
        .await
        .unwrap();
    let (status, body) = read_body(resp).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "body={body}");
    assert_eq!(body["error"], serde_json::json!("no_model_configured"));
}

// ─── generate-tasks tests ─────────────────────────────────────────────────────

#[tokio::test]
async fn generate_tasks_200_ok() {
    let state = test_state_with_llm(VALID_TASKS_JSON).await;
    let app = build_router(state);
    let (_, cookie) = register_and_login(app.clone(), "alice@ai.test", "pass-12345678").await;
    let pid = create_project(&app, &cookie, "AI Project").await;

    let resp = app
        .oneshot(req(
            Method::POST,
            &format!("/api/projects/{pid}/generate-tasks"),
            Some(&cookie),
            Some(serde_json::json!({ "description": "Build a web app with authentication." })),
        ))
        .await
        .unwrap();
    let (status, body) = read_body(resp).await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    let tasks = body["tasks"].as_array().expect("tasks array");
    assert_eq!(tasks.len(), 1, "expected 1 task, got {}", tasks.len());
    assert_eq!(tasks[0]["title"], serde_json::json!("Setup"));
}

#[tokio::test]
async fn generate_tasks_400_empty_description() {
    let state = test_state_with_llm("ignored").await;
    let app = build_router(state);
    let (_, cookie) = register_and_login(app.clone(), "alice@ai.test", "pass-12345678").await;
    let pid = create_project(&app, &cookie, "AI Project").await;

    let resp = app
        .oneshot(req(
            Method::POST,
            &format!("/api/projects/{pid}/generate-tasks"),
            Some(&cookie),
            Some(serde_json::json!({ "description": "" })),
        ))
        .await
        .unwrap();
    let (status, body) = read_body(resp).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "body={body}");
    assert_eq!(body["error"], serde_json::json!("description_empty"));
}

#[tokio::test]
async fn generate_tasks_403_nonowner() {
    let state = test_state_with_llm("ignored").await;
    let app = build_router(state);
    let (_, cookie_a) = register_and_login(app.clone(), "alice@ai.test", "pass-12345678").await;
    let (_, cookie_b) = register_and_login(app.clone(), "bob@ai.test", "pass-12345678").await;
    let pid = create_project(&app, &cookie_a, "Alice's Project").await;

    let resp = app
        .oneshot(req(
            Method::POST,
            &format!("/api/projects/{pid}/generate-tasks"),
            Some(&cookie_b),
            Some(serde_json::json!({ "description": "Build something." })),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn generate_tasks_502_ai_parse_error() {
    // Fake LLM returns non-JSON — handler must respond 502 ai_parse_error.
    let state = test_state_with_llm("this is not valid json at all").await;
    let app = build_router(state);
    let (_, cookie) = register_and_login(app.clone(), "alice@ai.test", "pass-12345678").await;
    let pid = create_project(&app, &cookie, "AI Project").await;

    let resp = app
        .oneshot(req(
            Method::POST,
            &format!("/api/projects/{pid}/generate-tasks"),
            Some(&cookie),
            Some(serde_json::json!({ "description": "Build a web app." })),
        ))
        .await
        .unwrap();
    let (status, body) = read_body(resp).await;
    assert_eq!(status, StatusCode::BAD_GATEWAY, "body={body}");
    assert_eq!(body["error"], serde_json::json!("ai_parse_error"));
}
