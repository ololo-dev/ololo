//! Admin judge-runs proxy integration tests.

use arena_core::session_status::SessionStatus;
use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use http_body_util::BodyExt;
use migration::{Migrator, MigratorTrait};
use sea_orm::{ActiveModelTrait, Set};
use server::entities::{players, projects, sessions, task_judges, tasks};
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

/// Insert a project + task directly into the DB. Returns (project_id, task_id).
async fn seed_project_and_task(state: &AppState, admin_id: Uuid) -> (Uuid, Uuid) {
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

    (project_id, task_id)
}

/// Insert a session (lobby status, no game_server by default) owned by `admin_id`.
async fn seed_session(state: &AppState, admin_id: Uuid, project_id: Uuid) -> Uuid {
    let session_id = Uuid::new_v4();
    let now = chrono::Utc::now();
    sessions::ActiveModel {
        id: Set(session_id),
        name: Set("sess".into()),
        created_at: Set(now),
        owner_id_fk: Set(Some(admin_id)),
        status: Set(SessionStatus::Lobby),
        join_code: Set("ABCD12".into()),
        started_at: Set(None),
        finished_at: Set(None),
        paused_at: Set(None),
        paused_duration_secs: Set(None),
        project_id_fk: Set(project_id),
        game_server_id: Set(None),
        cancel_reason: Set(None),
        cancelled_by: Set(None),
    }
    .insert(&state.db)
    .await
    .expect("insert session");
    session_id
}

/// Insert a player row for the given session. Returns player_id.
async fn seed_player(state: &AppState, session_id: Uuid) -> Uuid {
    let player_id = Uuid::new_v4();
    players::ActiveModel {
        id: Set(player_id),
        session_id_fk: Set(session_id),
        user_id_fk: Set(None),
        display_name: Set("p1".into()),
        fingerprint: Set(None),
        metadata_json: Set(None),
        joined_at: Set(chrono::Utc::now()),
        reconnected_at: Set(None),
        revoked_at: Set(None),
        agent_connected: Set(false),
        agent_last_seen_at: Set(None),
    }
    .insert(&state.db)
    .await
    .expect("insert player");
    player_id
}

/// Insert a task_judges row linking task_id and judge_id.
async fn seed_task_judge(state: &AppState, task_id: Uuid, judge_id: Uuid) {
    let now = chrono::Utc::now();
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
}

/// Insert a judge row directly. Returns judge_id.
async fn seed_judge(state: &AppState) -> Uuid {
    let judge_id = Uuid::new_v4();
    let now = chrono::Utc::now();
    server::entities::judges::ActiveModel {
        id: Set(judge_id),
        slug: Set("j1".into()),
        name: Set("Judge j1".into()),
        description: Set("desc".into()),
        prompt: Set("prompt".into()),
        rating_scale: Set(serde_json::json!({"min":0.0,"max":10.0,"step":1.0})),
        kind: Set("llm".to_string()),
        scope: Set("task".to_string()),
        evidence_mode: Set("tools".to_string()),
        evidence_needs: Set(None),
        created_at: Set(now),
        updated_at: Set(now),
        llm_provider_id_fk: Set(None),
        llm_model: Set(None),
        llm_pool_id_fk: Set(None),
        llm_source_order: Set(arena_core::llm::resolve::SOURCE_ORDER_POOL_FIRST.to_string()),
        criteria: Set(None),
        max_interactive: Set(None),
        avatar_url: Set(None),
        probes_config: Set(None),
        ignore_paths: Set(None),
    }
    .insert(&state.db)
    .await
    .expect("insert judge");
    judge_id
}

fn run_uri(session_code: &str, player_id: Uuid) -> String {
    format!("/api/admin/sessions/{session_code}/players/{player_id}/judge-runs")
}

// ─────────────────────────── Tests ───────────────────────────

#[tokio::test]
async fn no_auth_401() {
    let state = test_state().await;
    let app = build_router(state);
    let player_id = Uuid::new_v4();

    let resp = app
        .oneshot(req(
            Method::POST,
            &run_uri("ABCD12", player_id),
            None,
            Some(serde_json::json!({
                "task_id": Uuid::new_v4(),
                "judge_id": Uuid::new_v4(),
            })),
        ))
        .await
        .expect("resp");
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn non_admin_forbidden() {
    let state = test_state().await;
    let app = build_router(state);
    // First user = auto-admin; second user = non-admin.
    let _ = register_and_login(app.clone(), "admin@jr.test", "password-12345").await;
    let (_, cookie_b) = register_and_login(app.clone(), "peasant@jr.test", "password-12345").await;

    let player_id = Uuid::new_v4();
    let resp = app
        .oneshot(req(
            Method::POST,
            &run_uri("ABCD12", player_id),
            Some(&cookie_b),
            Some(serde_json::json!({
                "task_id": Uuid::new_v4(),
                "judge_id": Uuid::new_v4(),
            })),
        ))
        .await
        .expect("resp");
    let (s, _, b) = read_body(resp).await;
    assert!(
        s == StatusCode::FORBIDDEN || s == StatusCode::UNAUTHORIZED,
        "non-admin: {s} body={b}"
    );
}

#[tokio::test]
async fn task_judge_not_found_404() {
    let state = test_state().await;
    let app = build_router(state.clone());
    let (admin_id, cookie) =
        register_and_login(app.clone(), "admin@jr1.test", "password-12345").await;
    let (project_id, _task_id) = seed_project_and_task(&state, admin_id).await;
    let _session_id = seed_session(&state, admin_id, project_id).await;
    let player_id = seed_player(&state, _session_id).await;

    // No task_judges row: POST with arbitrary task_id + judge_id → 404.
    let resp = app
        .oneshot(req(
            Method::POST,
            &run_uri("ABCD12", player_id),
            Some(&cookie),
            Some(serde_json::json!({
                "task_id": Uuid::new_v4(),
                "judge_id": Uuid::new_v4(),
            })),
        ))
        .await
        .expect("resp");
    let (s, _, b) = read_body(resp).await;
    assert_eq!(s, StatusCode::NOT_FOUND, "task_judge_not_found: {b}");
    assert_eq!(b["error"], serde_json::json!("task_judge_not_found"));
}

#[tokio::test]
async fn malformed_body_422() {
    let state = test_state().await;
    let app = build_router(state.clone());
    let (admin_id, cookie) =
        register_and_login(app.clone(), "admin@jr2.test", "password-12345").await;
    let (project_id, _task_id) = seed_project_and_task(&state, admin_id).await;
    let _session_id = seed_session(&state, admin_id, project_id).await;
    let player_id = seed_player(&state, _session_id).await;

    // Missing task_id → Json deserialize rejection (missing field).
    let resp = app
        .oneshot(req(
            Method::POST,
            &run_uri("ABCD12", player_id),
            Some(&cookie),
            Some(serde_json::json!({ "judge_id": Uuid::new_v4() })),
        ))
        .await
        .expect("resp");
    let (s, _, _b) = read_body(resp).await;
    assert!(
        s == StatusCode::UNPROCESSABLE_ENTITY || s == StatusCode::BAD_REQUEST,
        "malformed missing task_id: {s}"
    );
}

#[tokio::test]
async fn deny_unknown_fields_422() {
    let state = test_state().await;
    let app = build_router(state.clone());
    let (admin_id, cookie) =
        register_and_login(app.clone(), "admin@jr3.test", "password-12345").await;
    let (project_id, _task_id) = seed_project_and_task(&state, admin_id).await;
    let _session_id = seed_session(&state, admin_id, project_id).await;
    let player_id = seed_player(&state, _session_id).await;

    let resp = app
        .oneshot(req(
            Method::POST,
            &run_uri("ABCD12", player_id),
            Some(&cookie),
            Some(serde_json::json!({
                "task_id": Uuid::new_v4(),
                "judge_id": Uuid::new_v4(),
                "extra": "bad",
            })),
        ))
        .await
        .expect("resp");
    let (s, _, _b) = read_body(resp).await;
    assert!(
        s == StatusCode::UNPROCESSABLE_ENTITY || s == StatusCode::BAD_REQUEST,
        "deny_unknown_fields: {s}"
    );
}

#[tokio::test]
async fn game_server_unavailable_503_no_game_server_id() {
    let state = test_state().await;
    let app = build_router(state.clone());
    let (admin_id, cookie) =
        register_and_login(app.clone(), "admin@jr4.test", "password-12345").await;
    let (project_id, task_id) = seed_project_and_task(&state, admin_id).await;
    let _session_id = seed_session(&state, admin_id, project_id).await;
    let player_id = seed_player(&state, _session_id).await;
    let judge_id = seed_judge(&state).await;
    seed_task_judge(&state, task_id, judge_id).await;

    // Session has game_server_id = None → 503.
    let resp = app
        .oneshot(req(
            Method::POST,
            &run_uri("ABCD12", player_id),
            Some(&cookie),
            Some(serde_json::json!({
                "task_id": task_id,
                "judge_id": judge_id,
            })),
        ))
        .await
        .expect("resp");
    let (s, _, b) = read_body(resp).await;
    assert_eq!(s, StatusCode::SERVICE_UNAVAILABLE, "no game_server_id: {b}");
    assert_eq!(b["error"], serde_json::json!("game_server_unavailable"));
}

#[tokio::test]
async fn game_server_unavailable_503_row_not_found() {
    let state = test_state().await;
    let app = build_router(state.clone());
    let (admin_id, cookie) =
        register_and_login(app.clone(), "admin@jr5.test", "password-12345").await;
    let (project_id, task_id) = seed_project_and_task(&state, admin_id).await;

    // Bypass the sessions.game_server_id FK so the session can reference a
    // game-server row that doesn't exist. The handler's `find_by_id` then
    // returns None → 503 `game_server_unavailable`.
    use sea_orm::ConnectionTrait;
    state
        .db
        .execute_unprepared("PRAGMA foreign_keys = OFF")
        .await
        .expect("disable FK");

    let orphan_gs_id = Uuid::new_v4();
    let session_id = Uuid::new_v4();
    let now = chrono::Utc::now();
    sessions::ActiveModel {
        id: Set(session_id),
        name: Set("sess".into()),
        created_at: Set(now),
        owner_id_fk: Set(Some(admin_id)),
        status: Set(SessionStatus::Lobby),
        join_code: Set("ZZZ999".into()),
        started_at: Set(None),
        finished_at: Set(None),
        paused_at: Set(None),
        paused_duration_secs: Set(None),
        project_id_fk: Set(project_id),
        game_server_id: Set(Some(orphan_gs_id)),
        cancel_reason: Set(None),
        cancelled_by: Set(None),
    }
    .insert(&state.db)
    .await
    .expect("insert session");

    state
        .db
        .execute_unprepared("PRAGMA foreign_keys = ON")
        .await
        .expect("re-enable FK");

    let player_id = seed_player(&state, session_id).await;
    let judge_id = seed_judge(&state).await;
    seed_task_judge(&state, task_id, judge_id).await;

    let resp = app
        .oneshot(req(
            Method::POST,
            &run_uri("ZZZ999", player_id),
            Some(&cookie),
            Some(serde_json::json!({
                "task_id": task_id,
                "judge_id": judge_id,
            })),
        ))
        .await
        .expect("resp");
    let (s, _, b) = read_body(resp).await;
    assert_eq!(s, StatusCode::SERVICE_UNAVAILABLE, "row missing: {b}");
    assert_eq!(b["error"], serde_json::json!("game_server_unavailable"));
}
