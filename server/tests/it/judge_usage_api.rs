//! `GET /api/admin/sessions/:code/judge-usage` — per-player judge LLM
//! token spend for a session. Multiple judges per task produce multiple
//! judge_results rows per player; the endpoint must sum them all
//! (scored AND failed — failed runs still burned tokens).

use arena_core::session_status::SessionStatus;
use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use http_body_util::BodyExt;
use migration::{Migrator, MigratorTrait};
use sea_orm::{ActiveModelTrait, EntityTrait, Set};
use server::entities::{
    judge_results, judges, players, projects, sessions, task_judges, tasks, users,
};
use server::{AppState, AuthConfig, build_router};
use std::time::Duration;
use tower::ServiceExt;
use uuid::Uuid;

const ORIGIN: &str = "http://localhost:5173";
const JOIN_CODE: &str = "USAGE1";

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
            let val = rest.split(';').next().unwrap_or("");
            return Some(format!("{name}={val}"));
        }
    }
    None
}

async fn register_and_login(
    app: axum::Router,
    db: &sea_orm::DatabaseConnection,
    email: &str,
) -> String {
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/auth/register")
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::ORIGIN, ORIGIN)
                .body(json_body(
                    serde_json::json!({"email": email, "password": "password-12345"}),
                ))
                .expect("register req"),
        )
        .await
        .expect("register resp");
    let (status, _, body) = read_body(resp).await;
    assert_eq!(status, StatusCode::CREATED, "register: {body}");
    // The seed inserts users directly, so the registered user is not the
    // first row — promote to admin explicitly.
    let user_id: Uuid = serde_json::from_value(body["id"].clone()).expect("user id");
    let user = users::Entity::find_by_id(user_id)
        .one(db)
        .await
        .expect("find user")
        .expect("user row");
    let mut am: users::ActiveModel = user.into();
    am.is_admin = Set(true);
    am.update(db).await.expect("promote admin");

    let resp = app
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/auth/login")
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::ORIGIN, ORIGIN)
                .body(json_body(
                    serde_json::json!({"email": email, "password": "password-12345"}),
                ))
                .expect("login req"),
        )
        .await
        .expect("login resp");
    let (status, cookies, _) = read_body(resp).await;
    assert_eq!(status, StatusCode::OK, "login");
    cookie_pair(&cookies, "arena_access").expect("access cookie")
}

/// Seed a session with one task judged by TWO judges and two players.
async fn seed(state: &AppState) {
    let now = chrono::Utc::now();
    let owner = Uuid::new_v4();
    users::ActiveModel {
        id: Set(owner),
        email: Set(format!("o-{owner}@example.com")),
        password_hash: Set(None),
        display_name: Set("Owner".to_string()),
        created_at: Set(now),
        updated_at: Set(now),
        is_admin: Set(false),
        avatar_url: Set(None),
        email_verified: Set(false),
        username: Set(None),
        plan: Set(arena_core::quota::PLAN_PREMIUM.to_string()),
        judge_run_limit: Set(None),
        judge_run_credits: Set(0),
    }
    .insert(&state.db)
    .await
    .expect("owner");

    let project_id = Uuid::new_v4();
    projects::ActiveModel {
        id: Set(project_id),
        name: Set("P".to_string()),
        slug: Set(None),
        description: Set(String::new()),
        category: Set(None),
        tags: Set(String::new()),
        cover_image_url: Set(None),
        owner_user_id_fk: Set(owner),
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
    .expect("project");

    let session_id = Uuid::new_v4();
    sessions::ActiveModel {
        id: Set(session_id),
        name: Set("S".to_string()),
        created_at: Set(now),
        owner_id_fk: Set(None),
        status: Set(SessionStatus::Running),
        join_code: Set(JOIN_CODE.to_string()),
        game_server_id: Set(None),
        cancel_reason: Set(None),
        cancelled_by: Set(None),
        started_at: Set(Some(now)),
        finished_at: Set(None),
        paused_at: Set(None),
        paused_duration_secs: Set(None),
        project_id_fk: Set(project_id),
    }
    .insert(&state.db)
    .await
    .expect("session");

    let task_id = Uuid::new_v4();
    tasks::ActiveModel {
        id: Set(task_id),
        project_id_fk: Set(project_id),
        ordinal: Set(0),
        title: Set("T".to_string()),
        content: Set(String::new()),
        test_template: Set(serde_json::json!({ "kind": "shell", "command_template": "echo ok" })),
        created_at: Set(now),
        tags: Set(String::new()),
        point_value: Set(10),
        deadline_secs: Set(Some(300)),
        min_interval_secs: Set(Some(5)),
        interval_increment_secs: Set(Some(0)),
        max_interval_secs: Set(Some(300)),
        fail_points: Set(0),
        no_response_points: Set(0),
        completion_bonus_points: Set(10),
        evaluation: Set(None),
    }
    .insert(&state.db)
    .await
    .expect("task");

    let mut task_judge_ids = Vec::new();
    for slug in ["anti-cheater", "code-cleanliness"] {
        let judge_id = Uuid::new_v4();
        judges::ActiveModel {
            id: Set(judge_id),
            slug: Set(slug.to_string()),
            name: Set(slug.to_string()),
            description: Set("d".into()),
            prompt: Set("p".into()),
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
        .expect("judge");
        let tj_id = Uuid::new_v4();
        task_judges::ActiveModel {
            id: Set(tj_id),
            task_id: Set(task_id),
            judge_id: Set(judge_id),
            ordinal: Set(task_judge_ids.len() as i32),
            rating_scale_override: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
            weight: Set(None),
        }
        .insert(&state.db)
        .await
        .expect("task_judge");
        task_judge_ids.push(tj_id);
    }

    let mk_player = async |name: &str| -> Uuid {
        let pid = Uuid::new_v4();
        players::ActiveModel {
            id: Set(pid),
            session_id_fk: Set(session_id),
            user_id_fk: Set(None),
            display_name: Set(name.to_string()),
            fingerprint: Set(None),
            metadata_json: Set(None),
            joined_at: Set(now),
            reconnected_at: Set(None),
            revoked_at: Set(None),
            agent_connected: Set(false),
            agent_last_seen_at: Set(None),
        }
        .insert(&state.db)
        .await
        .expect("player");
        pid
    };
    let alice = mk_player("alice").await;
    let bob = mk_player("bob").await;

    // (player, task_judge, status, in, out, cache_r, cache_w)
    let rows = [
        (
            alice,
            task_judge_ids[0],
            "scored",
            1000i64,
            100i64,
            200i64,
            10i64,
        ),
        (alice, task_judge_ids[1], "failed", 500, 0, 0, 0),
        (bob, task_judge_ids[0], "scored", 300, 30, 50, 0),
    ];
    for (pid, tj, status, ti, to, cr, cw) in rows {
        judge_results::ActiveModel {
            id: Set(Uuid::new_v4()),
            session_id_fk: Set(session_id),
            player_id_fk: Set(pid),
            task_judge_id: Set(tj),
            rating: Set(serde_json::json!(5.0)),
            point_delta: Set(0),
            feedback: Set(String::new()),
            model: Set("m".to_string()),
            provider: Set("openrouter".to_string()),
            raw_output: Set(String::new()),
            duration_ms: Set(Some(1000)),
            run_log: Set(None),
            tokens_input: Set(Some(ti)),
            tokens_output: Set(Some(to)),
            tokens_cache_read: Set(Some(cr)),
            tokens_cache_write: Set(Some(cw)),
            status: Set(status.to_string()),
            error: Set((status == "failed").then(|| "429".to_string())),
            created_at: Set(now),
            updated_at: Set(now),
            verdict_kind: Set(None),
        }
        .insert(&state.db)
        .await
        .expect("judge_result");
    }
}

#[tokio::test]
async fn judge_usage_sums_all_judges_per_player() {
    let state = test_state().await;
    seed(&state).await;
    let db = state.db.clone();
    let app = build_router(state);
    let cookie = register_and_login(app.clone(), &db, "admin@usage.test").await;

    let resp = app
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri(format!("/api/admin/sessions/{JOIN_CODE}/judge-usage"))
                .header(header::ORIGIN, ORIGIN)
                .header(header::COOKIE, cookie)
                .body(Body::empty())
                .expect("req"),
        )
        .await
        .expect("resp");
    let (status, _, body) = read_body(resp).await;
    assert_eq!(status, StatusCode::OK, "usage: {body}");

    let players = body["players"].as_array().expect("players array");
    assert_eq!(players.len(), 2);
    // Sorted by total tokens desc: alice (1600) before bob (330).
    assert_eq!(players[0]["display_name"], "alice");
    assert_eq!(players[0]["runs"], 2, "both judges' runs counted");
    assert_eq!(players[0]["scored"], 1);
    assert_eq!(players[0]["failed"], 1);
    assert_eq!(
        players[0]["tokens_input"], 1500,
        "failed run tokens included"
    );
    assert_eq!(players[0]["tokens_output"], 100);
    assert_eq!(players[0]["tokens_cache_read"], 200);
    assert_eq!(players[0]["tokens_cache_write"], 10);
    assert_eq!(players[1]["display_name"], "bob");
    assert_eq!(players[1]["tokens_input"], 300);

    assert_eq!(body["totals"]["runs"], 3);
    assert_eq!(body["totals"]["tokens_input"], 1800);
    assert_eq!(body["totals"]["tokens_output"], 130);
}

#[tokio::test]
async fn judge_usage_requires_admin() {
    let state = test_state().await;
    seed(&state).await;
    let app = build_router(state);
    let resp = app
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri(format!("/api/admin/sessions/{JOIN_CODE}/judge-usage"))
                .header(header::ORIGIN, ORIGIN)
                .body(Body::empty())
                .expect("req"),
        )
        .await
        .expect("resp");
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn raw_judge_log_requires_admin() {
    let state = test_state().await;
    seed(&state).await;
    let app = build_router(state);
    let resp = app
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri(format!(
                    "/api/admin/sessions/{JOIN_CODE}/players/{}/tasks/{}/judge-log",
                    Uuid::new_v4(),
                    Uuid::new_v4()
                ))
                .header(header::ORIGIN, ORIGIN)
                .body(Body::empty())
                .expect("req"),
        )
        .await
        .expect("resp");
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn raw_judge_log_without_game_server_is_unavailable() {
    // The seeded session has no game_server_id — the proxy target is
    // unknown, so the endpoint must answer 503, not 500 or a hang.
    let state = test_state().await;
    seed(&state).await;
    let db = state.db.clone();
    let app = build_router(state);
    let cookie = register_and_login(app.clone(), &db, "admin@rawlog.test").await;

    let resp = app
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri(format!(
                    "/api/admin/sessions/{JOIN_CODE}/players/{}/tasks/{}/judge-log",
                    Uuid::new_v4(),
                    Uuid::new_v4()
                ))
                .header(header::ORIGIN, ORIGIN)
                .header(header::COOKIE, cookie)
                .body(Body::empty())
                .expect("req"),
        )
        .await
        .expect("resp");
    assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
}
