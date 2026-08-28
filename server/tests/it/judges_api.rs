//! Admin judges CRUD integration tests.

use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use http_body_util::BodyExt;
use migration::{Migrator, MigratorTrait};
use sea_orm::{ActiveModelTrait, Set};
use server::entities::{projects, task_judges, tasks};
use server::{AppState, AuthConfig, build_router};
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
    AppState::new(db, cfg)
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
        .filter_map(|hv| hv.to_str().ok().map(String::from))
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
    assert_eq!(status, StatusCode::CREATED, "register: {body}");
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
    let (status, cookies, _) = read_body(resp).await;
    assert_eq!(status, StatusCode::OK, "login");
    let access = cookie_pair(&cookies, "arena_access").expect("access cookie");
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

fn valid_scale() -> serde_json::Value {
    serde_json::json!({ "min": 0.0, "max": 10.0, "step": 1.0 })
}

fn create_body(slug: &str, scale: serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "slug": slug,
        "name": format!("Judge {slug}"),
        "description": "desc",
        "prompt": "prompt",
        "rating_scale": scale,
    })
}

async fn create_judge(
    app: &axum::Router,
    cookie: &str,
    slug: &str,
) -> (StatusCode, serde_json::Value) {
    let resp = app
        .clone()
        .oneshot(req(
            Method::POST,
            "/api/admin/judges",
            Some(cookie),
            Some(create_body(slug, valid_scale())),
        ))
        .await
        .expect("create judge resp");
    let (s, _, b) = read_body(resp).await;
    (s, b)
}

// ─────────────────────────── Tests ───────────────────────────

#[tokio::test]
async fn create_judge_happy_path_201() {
    let app = build_router(test_state().await);
    let (_, cookie) = register_and_login(app.clone(), "admin@j1.test", "password-12345").await;

    let (status, body) = create_judge(&app, &cookie, "binary").await;
    assert_eq!(status, StatusCode::CREATED, "create: {body}");
    assert_eq!(body["slug"], serde_json::json!("binary"));
    assert_eq!(body["name"], serde_json::json!("Judge binary"));
    assert_eq!(body["rating_scale"]["min"], serde_json::json!(0.0));
}

#[tokio::test]
async fn create_judge_duplicate_slug_409() {
    let app = build_router(test_state().await);
    let (_, cookie) = register_and_login(app.clone(), "admin@j2.test", "password-12345").await;

    let (s1, _) = create_judge(&app, &cookie, "dup").await;
    assert_eq!(s1, StatusCode::CREATED);
    let (s2, body) = create_judge(&app, &cookie, "dup").await;
    assert_eq!(s2, StatusCode::CONFLICT, "dup: {body}");
}

#[tokio::test]
async fn create_judge_invalid_rating_scale_400() {
    let app = build_router(test_state().await);
    let (_, cookie) = register_and_login(app.clone(), "admin@j3.test", "password-12345").await;

    let bad = serde_json::json!({ "min": 5.0, "max": 0.0, "step": 1.0 });
    let resp = app
        .clone()
        .oneshot(req(
            Method::POST,
            "/api/admin/judges",
            Some(&cookie),
            Some(create_body("bad-scale", bad)),
        ))
        .await
        .expect("create resp");
    let (s, _, b) = read_body(resp).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "bad scale: {b}");
}

#[tokio::test]
async fn create_judge_invalid_slug_400() {
    let app = build_router(test_state().await);
    let (_, cookie) = register_and_login(app.clone(), "admin@j4.test", "password-12345").await;

    let resp = app
        .clone()
        .oneshot(req(
            Method::POST,
            "/api/admin/judges",
            Some(&cookie),
            Some(create_body("UPPER CASE", valid_scale())),
        ))
        .await
        .expect("create resp");
    let (s, _, b) = read_body(resp).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "bad slug: {b}");
}

#[tokio::test]
async fn update_judge_partial_name_200() {
    let app = build_router(test_state().await);
    let (_, cookie) = register_and_login(app.clone(), "admin@j5.test", "password-12345").await;

    let (_, body) = create_judge(&app, &cookie, "up").await;
    let id = body["id"].as_str().expect("judge id");

    let resp = app
        .clone()
        .oneshot(req(
            Method::PUT,
            &format!("/api/admin/judges/{id}"),
            Some(&cookie),
            Some(serde_json::json!({ "name": "Renamed" })),
        ))
        .await
        .expect("update resp");
    let (s, _, b) = read_body(resp).await;
    assert_eq!(s, StatusCode::OK, "update: {b}");
    assert_eq!(b["name"], serde_json::json!("Renamed"));
    assert_eq!(b["description"], serde_json::json!("desc"));
    assert_eq!(b["prompt"], serde_json::json!("prompt"));
    assert_eq!(b["slug"], serde_json::json!("up"));
}

#[tokio::test]
async fn update_judge_invalid_rating_scale_400() {
    let app = build_router(test_state().await);
    let (_, cookie) = register_and_login(app.clone(), "admin@j6.test", "password-12345").await;

    let (_, body) = create_judge(&app, &cookie, "up-bad").await;
    let id = body["id"].as_str().expect("judge id");

    let bad = serde_json::json!({ "min": 10.0, "max": 0.0, "step": 1.0 });
    let resp = app
        .clone()
        .oneshot(req(
            Method::PUT,
            &format!("/api/admin/judges/{id}"),
            Some(&cookie),
            Some(serde_json::json!({ "rating_scale": bad })),
        ))
        .await
        .expect("update resp");
    let (s, _, b) = read_body(resp).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "update bad scale: {b}");
}

#[tokio::test]
async fn get_one_happy_200() {
    let app = build_router(test_state().await);
    let (_, cookie) = register_and_login(app.clone(), "admin@j7.test", "password-12345").await;

    let (_, body) = create_judge(&app, &cookie, "get").await;
    let id = body["id"].as_str().expect("judge id");

    let resp = app
        .clone()
        .oneshot(req(
            Method::GET,
            &format!("/api/admin/judges/{id}"),
            Some(&cookie),
            None,
        ))
        .await
        .expect("get resp");
    let (s, _, b) = read_body(resp).await;
    assert_eq!(s, StatusCode::OK, "get: {b}");
    assert_eq!(b["slug"], serde_json::json!("get"));
}

#[tokio::test]
async fn get_one_missing_404() {
    let app = build_router(test_state().await);
    let (_, cookie) = register_and_login(app.clone(), "admin@j8.test", "password-12345").await;

    let missing = Uuid::new_v4();
    let resp = app
        .clone()
        .oneshot(req(
            Method::GET,
            &format!("/api/admin/judges/{missing}"),
            Some(&cookie),
            None,
        ))
        .await
        .expect("get resp");
    let (s, _, b) = read_body(resp).await;
    assert_eq!(s, StatusCode::NOT_FOUND, "get missing: {b}");
}

#[tokio::test]
async fn delete_judge_happy_204() {
    let app = build_router(test_state().await);
    let (_, cookie) = register_and_login(app.clone(), "admin@j9.test", "password-12345").await;

    let (_, body) = create_judge(&app, &cookie, "del").await;
    let id = body["id"].as_str().expect("judge id");

    let resp = app
        .clone()
        .oneshot(req(
            Method::DELETE,
            &format!("/api/admin/judges/{id}"),
            Some(&cookie),
            None,
        ))
        .await
        .expect("delete resp");
    let (s, _, b) = read_body(resp).await;
    assert_eq!(s, StatusCode::NO_CONTENT, "delete: {b}");
}

#[tokio::test]
async fn delete_judge_referenced_by_task_judges_409() {
    let state = test_state().await;
    let app = build_router(state.clone());
    let (admin_id, cookie) =
        register_and_login(app.clone(), "admin@j10.test", "password-12345").await;

    let (_, body) = create_judge(&app, &cookie, "ref").await;
    let judge_id: Uuid = body["id"].as_str().expect("judge id").parse().unwrap();

    let project_id = Uuid::new_v4();
    let now = chrono::Utc::now();
    projects::ActiveModel {
        id: Set(project_id),
        name: Set("proj".into()),
        slug: Set(None),
        description: Set("".into()),
        category: Set(None),
        tags: Set("[]".into()),
        cover_image_url: Set(None),
        owner_user_id_fk: Set(admin_id),
        public: Set(true),
        archived_at: Set(None),
        created_at: Set(now),
        updated_at: Set(now),
        default_value_points: Set(10),
        default_fail_points: Set(-5),
        default_no_response_points: Set(-10),
        default_completion_bonus_points: Set(10),
        default_deadline_secs: Set(60),
        default_session_duration_secs: Set(3600),
        idle_timeout_secs: Set(300),
        default_min_interval_secs: Set(5),
        default_interval_increment_secs: Set(5),
        default_max_interval_secs: Set(60),
        memory_schema: Set(None),
        show_tasks: Set(true),
        parent_project_id_fk: Set(None),
        part_ordinal: Set(None),
    }
    .insert(&state.db)
    .await
    .expect("insert project");

    let task_id = Uuid::new_v4();
    tasks::ActiveModel {
        id: Set(task_id),
        project_id_fk: Set(project_id),
        ordinal: Set(0),
        title: Set("t".into()),
        content: Set("".into()),
        test_template: Set(serde_json::json!({"kind":"shell","command_template":"echo hi"})),
        created_at: Set(now),
        tags: Set("[]".into()),
        point_value: Set(10),
        deadline_secs: Set(None),
        min_interval_secs: Set(None),
        interval_increment_secs: Set(None),
        max_interval_secs: Set(None),
        fail_points: Set(-5),
        no_response_points: Set(-10),
        completion_bonus_points: Set(10),
        evaluation: Set(None),
    }
    .insert(&state.db)
    .await
    .expect("insert task");

    task_judges::ActiveModel {
        id: Set(Uuid::new_v4()),
        task_id: Set(task_id),
        judge_id: Set(judge_id),
        ordinal: Set(0),
        rating_scale_override: Set(None),
        created_at: Set(now),
        updated_at: Set(now),
        weight: Set(None),
    }
    .insert(&state.db)
    .await
    .expect("insert task_judges");

    let resp = app
        .clone()
        .oneshot(req(
            Method::DELETE,
            &format!("/api/admin/judges/{judge_id}"),
            Some(&cookie),
            None,
        ))
        .await
        .expect("delete resp");
    let (s, _, b) = read_body(resp).await;
    assert_eq!(s, StatusCode::CONFLICT, "delete ref: {b}");
}

#[tokio::test]
async fn list_judges_ordered_by_slug() {
    let app = build_router(test_state().await);
    let (_, cookie) = register_and_login(app.clone(), "admin@j11.test", "password-12345").await;

    let _ = create_judge(&app, &cookie, "zeta").await;
    let _ = create_judge(&app, &cookie, "alpha").await;
    let _ = create_judge(&app, &cookie, "mid").await;

    let resp = app
        .clone()
        .oneshot(req(Method::GET, "/api/admin/judges", Some(&cookie), None))
        .await
        .expect("list resp");
    let (s, _, b) = read_body(resp).await;
    assert_eq!(s, StatusCode::OK, "list: {b}");
    let arr = b.as_array().expect("array");
    assert_eq!(arr.len(), 3);
    let slugs: Vec<&str> = arr
        .iter()
        .map(|j| j["slug"].as_str().expect("slug"))
        .collect();
    assert_eq!(slugs, vec!["alpha", "mid", "zeta"]);
}

#[tokio::test]
async fn sync_judges_from_disk_inserts_then_refreshes() {
    // Point the sync endpoint at a private temp dir. The env var is process
    // global, but this is the only test in the binary that reads it.
    let dir = std::env::temp_dir().join(format!("arena-judges-sync-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&dir).expect("mkdir");
    unsafe { std::env::set_var("ARENA_JUDGES_DIR", &dir) };
    std::fs::write(
        dir.join("disk-judge.md"),
        "---\nname: Disk Judge\nrating_scale: {min: 0.0, max: 10.0, step: 0.5}\n---\nPrompt v1.\n",
    )
    .unwrap();

    let app = build_router(test_state().await);
    let (_, cookie) = register_and_login(app.clone(), "admin@jsync.test", "password-12345").await;

    let resp = app
        .clone()
        .oneshot(req(
            Method::POST,
            "/api/admin/judges/sync",
            Some(&cookie),
            None,
        ))
        .await
        .expect("sync resp");
    let (s, _, b) = read_body(resp).await;
    assert_eq!(s, StatusCode::OK, "sync: {b}");
    assert_eq!(b["inserted"], 1, "first sync inserts: {b}");
    assert_eq!(b["updated"], 0);

    // The judge is now listable.
    let resp = app
        .clone()
        .oneshot(req(Method::GET, "/api/admin/judges", Some(&cookie), None))
        .await
        .expect("list resp");
    let (_, _, b) = read_body(resp).await;
    let slugs: Vec<&str> = b
        .as_array()
        .expect("array")
        .iter()
        .map(|j| j["slug"].as_str().unwrap())
        .collect();
    assert!(slugs.contains(&"disk-judge"), "listed: {slugs:?}");

    // Change the file → second sync refreshes the row.
    std::fs::write(
        dir.join("disk-judge.md"),
        "---\nname: Disk Judge\nrating_scale: {min: 0.0, max: 10.0, step: 0.5}\n---\nPrompt v2.\n",
    )
    .unwrap();
    let resp = app
        .clone()
        .oneshot(req(
            Method::POST,
            "/api/admin/judges/sync",
            Some(&cookie),
            None,
        ))
        .await
        .expect("resync resp");
    let (s, _, b) = read_body(resp).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(b["updated"], 1, "changed file refreshes: {b}");

    std::fs::remove_dir_all(&dir).ok();
}

// ───────────────────── LLM override (pool + model) ─────────────────────

/// Create a provider and a pool, returning `(provider_id, pool_id)`.
async fn provider_and_pool(app: &axum::Router, cookie: &str) -> (String, String) {
    let resp = app
        .clone()
        .oneshot(req(
            Method::POST,
            "/api/admin/llm/providers",
            Some(cookie),
            Some(serde_json::json!({ "name": "P", "kind": "ollama" })),
        ))
        .await
        .unwrap();
    let (s, _, provider) = read_body(resp).await;
    assert_eq!(s, StatusCode::CREATED, "{provider}");

    let resp = app
        .clone()
        .oneshot(req(
            Method::POST,
            "/api/admin/llm/pools",
            Some(cookie),
            Some(serde_json::json!({ "name": "Pool" })),
        ))
        .await
        .unwrap();
    let (s, _, pool) = read_body(resp).await;
    assert_eq!(s, StatusCode::CREATED, "{pool}");

    (
        provider["id"].as_str().unwrap().to_string(),
        pool["id"].as_str().unwrap().to_string(),
    )
}

async fn post_judge(
    app: &axum::Router,
    cookie: &str,
    body: serde_json::Value,
) -> (StatusCode, serde_json::Value) {
    let resp = app
        .clone()
        .oneshot(req(
            Method::POST,
            "/api/admin/judges",
            Some(cookie),
            Some(body),
        ))
        .await
        .unwrap();
    let (s, _, b) = read_body(resp).await;
    (s, b)
}

#[tokio::test]
async fn judge_override_accepts_pool_and_model_together() {
    let app = build_router(test_state().await);
    let (_, cookie) = register_and_login(app.clone(), "admin@j20.test", "password-12345").await;
    let (provider_id, pool_id) = provider_and_pool(&app, &cookie).await;

    let mut body = create_body("both", valid_scale());
    body["llm_provider_id"] = provider_id.clone().into();
    body["llm_model"] = "pinned".into();
    body["llm_pool_id"] = pool_id.clone().into();
    body["llm_source_order"] = "model_first".into();

    let (s, b) = post_judge(&app, &cookie, body).await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    assert_eq!(b["llm_pool_id"].as_str(), Some(pool_id.as_str()));
    assert_eq!(b["llm_provider_id"].as_str(), Some(provider_id.as_str()));
    assert_eq!(b["llm_model"].as_str(), Some("pinned"));
    assert_eq!(b["llm_source_order"].as_str(), Some("model_first"));

    // The order defaults to pool_first when the field is omitted.
    let (s, b) = post_judge(&app, &cookie, create_body("plain", valid_scale())).await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    assert_eq!(b["llm_source_order"].as_str(), Some("pool_first"));
    assert!(b["llm_pool_id"].is_null());
}

#[tokio::test]
async fn judge_override_rejects_half_a_model_pair() {
    let app = build_router(test_state().await);
    let (_, cookie) = register_and_login(app.clone(), "admin@j21.test", "password-12345").await;
    let (provider_id, _pool_id) = provider_and_pool(&app, &cookie).await;

    // A provider with no model, and a model with no provider, are both
    // half-overrides — neither can resolve to anything on its own.
    let mut body = create_body("only-provider", valid_scale());
    body["llm_provider_id"] = provider_id.clone().into();
    let (s, _) = post_judge(&app, &cookie, body).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);

    let mut body = create_body("only-model", valid_scale());
    body["llm_model"] = "m".into();
    let (s, _) = post_judge(&app, &cookie, body).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);

    // A whitespace-only model does not count as the other half either.
    let mut body = create_body("blank-model", valid_scale());
    body["llm_provider_id"] = provider_id.into();
    body["llm_model"] = "   ".into();
    let (s, _) = post_judge(&app, &cookie, body).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn judge_override_rejects_unknown_pool_and_bad_order() {
    let app = build_router(test_state().await);
    let (_, cookie) = register_and_login(app.clone(), "admin@j22.test", "password-12345").await;

    let mut body = create_body("ghost-pool", valid_scale());
    body["llm_pool_id"] = Uuid::new_v4().to_string().into();
    let (s, _) = post_judge(&app, &cookie, body).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "unknown pool must be refused");

    let mut body = create_body("bad-order", valid_scale());
    body["llm_source_order"] = "sideways".into();
    let (s, _) = post_judge(&app, &cookie, body).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn update_judge_validates_the_state_the_patch_lands_on() {
    let app = build_router(test_state().await);
    let (_, cookie) = register_and_login(app.clone(), "admin@j23.test", "password-12345").await;
    let (provider_id, pool_id) = provider_and_pool(&app, &cookie).await;

    let mut body = create_body("patchme", valid_scale());
    body["llm_provider_id"] = provider_id.clone().into();
    body["llm_model"] = "pinned".into();
    let (s, b) = post_judge(&app, &cookie, body).await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    let id = b["id"].as_str().unwrap().to_string();

    let patch = |body: serde_json::Value| {
        let app = app.clone();
        let cookie = cookie.clone();
        let id = id.clone();
        async move {
            let resp = app
                .oneshot(req(
                    Method::PUT,
                    &format!("/api/admin/judges/{id}"),
                    Some(&cookie),
                    Some(body),
                ))
                .await
                .unwrap();
            read_body(resp).await
        }
    };

    // Clearing only the provider would leave a model with nothing to run on.
    let (s, _, _) = patch(serde_json::json!({ "llm_provider_id": null })).await;
    assert_eq!(
        s,
        StatusCode::BAD_REQUEST,
        "clearing half the pair must be refused"
    );

    // Clearing both together is fine.
    let (s, _, b) = patch(serde_json::json!({ "llm_provider_id": null, "llm_model": null })).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert!(b["llm_provider_id"].is_null());
    assert!(b["llm_model"].is_null());

    // Attaching a pool afterwards works and leaves the cleared pair alone.
    let (s, _, b) = patch(serde_json::json!({ "llm_pool_id": pool_id })).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b["llm_pool_id"].as_str(), Some(pool_id.as_str()));
}
