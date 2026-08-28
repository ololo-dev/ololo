//! `GET /api/admin/judges/usage` — where each judge is attached, and what it
//! has actually done.
//!
//! The settings page listed judges with no answer to either question: an
//! admin could not tell which projects a judge scores, nor whether it had
//! ever produced a verdict.

use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use http_body_util::BodyExt;
use migration::{Migrator, MigratorTrait};
use sea_orm::{ActiveModelTrait, Set};
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

// ─────────────────────────── fixtures ───────────────────────────

/// A project with two tasks, both with `judge_id` attached. Returns the
/// task-judge link ids, which is what `judge_results` rows point at.
async fn attach_to_two_tasks(
    db: &sea_orm::DatabaseConnection,
    owner: Uuid,
    judge_id: Uuid,
    project_name: &str,
) -> (Uuid, Vec<Uuid>) {
    use server::entities::{projects, task_judges, tasks};
    let now = chrono::Utc::now();
    let project_id = Uuid::new_v4();
    projects::ActiveModel {
        id: Set(project_id),
        name: Set(project_name.to_string()),
        slug: Set(Some(project_name.to_lowercase().replace(' ', "-"))),
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
        default_completion_bonus_points: Set(0),
        default_deadline_secs: Set(60),
        default_min_interval_secs: Set(10),
        default_interval_increment_secs: Set(10),
        default_max_interval_secs: Set(60),
        default_session_duration_secs: Set(1200),
        memory_schema: Set(None),
        parent_project_id_fk: Set(None),
        part_ordinal: Set(None),
        show_tasks: Set(true),
        idle_timeout_secs: Set(0),
    }
    .insert(db)
    .await
    .expect("project");

    let mut link_ids = Vec::new();
    for ordinal in 0..2 {
        let task_id = Uuid::new_v4();
        tasks::ActiveModel {
            id: Set(task_id),
            project_id_fk: Set(project_id),
            ordinal: Set(ordinal),
            title: Set(format!("Task {ordinal}")),
            content: Set("do it".to_string()),
            test_template: Set(serde_json::json!({"kind": "shell"})),
            created_at: Set(now),
            tags: Set(String::new()),
            point_value: Set(10),
            deadline_secs: Set(Some(60)),
            min_interval_secs: Set(Some(10)),
            interval_increment_secs: Set(Some(10)),
            max_interval_secs: Set(Some(60)),
            fail_points: Set(0),
            no_response_points: Set(0),
            completion_bonus_points: Set(0),
            evaluation: Set(None),
        }
        .insert(db)
        .await
        .expect("task");
        let link_id = Uuid::new_v4();
        task_judges::ActiveModel {
            id: Set(link_id),
            task_id: Set(task_id),
            judge_id: Set(judge_id),
            ordinal: Set(0),
            rating_scale_override: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
            weight: Set(None),
        }
        .insert(db)
        .await
        .expect("attach");
        link_ids.push(link_id);
    }
    (project_id, link_ids)
}

/// A running session with one player, so `judge_results` has real rows to
/// point at (its FKs are enforced).
async fn session_with_player(
    db: &sea_orm::DatabaseConnection,
    project_id: Uuid,
    join_code: &str,
    name: &str,
) -> (Uuid, Uuid) {
    use arena_core::session_status::SessionStatus;
    use server::entities::{players, sessions};
    let now = chrono::Utc::now();
    let session_id = Uuid::new_v4();
    sessions::ActiveModel {
        id: Set(session_id),
        name: Set("S".to_string()),
        created_at: Set(now),
        owner_id_fk: Set(None),
        status: Set(SessionStatus::Running),
        join_code: Set(join_code.to_string()),
        game_server_id: Set(None),
        cancel_reason: Set(None),
        cancelled_by: Set(None),
        started_at: Set(Some(now)),
        finished_at: Set(None),
        paused_at: Set(None),
        paused_duration_secs: Set(None),
        project_id_fk: Set(project_id),
    }
    .insert(db)
    .await
    .expect("session");

    let player_id = Uuid::new_v4();
    players::ActiveModel {
        id: Set(player_id),
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
    .insert(db)
    .await
    .expect("player");
    (session_id, player_id)
}

async fn record_verdict(
    db: &sea_orm::DatabaseConnection,
    session_id: Uuid,
    player_id: Uuid,
    task_judge_id: Uuid,
    point_delta: i32,
    status: &str,
) {
    use server::entities::judge_results;
    let now = chrono::Utc::now();
    judge_results::ActiveModel {
        id: Set(Uuid::new_v4()),
        session_id_fk: Set(session_id),
        player_id_fk: Set(player_id),
        task_judge_id: Set(task_judge_id),
        rating: Set(serde_json::json!(5.0)),
        point_delta: Set(point_delta),
        feedback: Set("ok".to_string()),
        model: Set("m".to_string()),
        provider: Set("p".to_string()),
        raw_output: Set(String::new()),
        duration_ms: Set(Some(10)),
        run_log: Set(None),
        tokens_input: Set(None),
        tokens_output: Set(None),
        tokens_cache_read: Set(None),
        tokens_cache_write: Set(None),
        status: Set(status.to_string()),
        error: Set(None),
        created_at: Set(now),
        updated_at: Set(now),
        verdict_kind: Set(None),
    }
    .insert(db)
    .await
    .expect("judge result");
}

async fn usage_of(app: axum::Router, cookie: &str, slug: &str) -> serde_json::Value {
    let (status, _, judges) = read_body(
        app.clone()
            .oneshot(req(Method::GET, "/api/admin/judges", Some(cookie), None))
            .await
            .expect("list resp"),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "list: {judges}");
    let judge_id = judges
        .as_array()
        .expect("array")
        .iter()
        .find(|j| j["slug"] == serde_json::json!(slug))
        .unwrap_or_else(|| panic!("judge {slug} not in list: {judges}"))["id"]
        .clone();

    let (status, _, usage) = read_body(
        app.oneshot(req(
            Method::GET,
            "/api/admin/judges/usage",
            Some(cookie),
            None,
        ))
        .await
        .expect("usage resp"),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "usage: {usage}");
    usage
        .as_array()
        .expect("array")
        .iter()
        .find(|u| u["judge_id"] == judge_id)
        .cloned()
        .unwrap_or_else(|| panic!("judge {slug} missing from usage: {usage}"))
}

// ─────────────────────────── Tests ───────────────────────────

/// Where does this judge run? Every task it is attached to, named, with the
/// project that owns it.
#[tokio::test]
async fn usage_lists_every_task_a_judge_is_attached_to() {
    let state = test_state().await;
    let db = state.db.clone();
    let app = build_router(state);
    let (admin, cookie) =
        register_and_login(app.clone(), "admin@usage-a.test", "password-12345").await;

    let (status, created) = create_judge(&app, &cookie, "attached-one").await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let judge_id: Uuid = created["id"].as_str().unwrap().parse().unwrap();
    attach_to_two_tasks(&db, admin, judge_id, "Weather Widget").await;

    let usage = usage_of(app, &cookie, "attached-one").await;
    let attachments = usage["attachments"].as_array().expect("attachments");
    assert_eq!(attachments.len(), 2, "{usage}");
    assert_eq!(attachments[0]["project_name"], "Weather Widget");
    assert_eq!(attachments[0]["task_ordinal"], 0);
    assert_eq!(attachments[0]["task_title"], "Task 0");
    assert_eq!(
        attachments[1]["task_ordinal"], 1,
        "ordered by task: {usage}"
    );
}

/// What has it done? Verdicts counted, points summed, and the two directions
/// kept apart — a judge that hands out 40 and takes 15 is not the same as one
/// that quietly moved 25.
#[tokio::test]
async fn usage_counts_verdicts_and_the_points_they_moved() {
    let state = test_state().await;
    let db = state.db.clone();
    let app = build_router(state);
    let (admin, cookie) =
        register_and_login(app.clone(), "admin@usage-b.test", "password-12345").await;

    let (_, created) = create_judge(&app, &cookie, "worked-hard").await;
    let judge_id: Uuid = created["id"].as_str().unwrap().parse().unwrap();
    let (project_id, links) = attach_to_two_tasks(&db, admin, judge_id, "Money Tracker").await;

    // One verdict per (attachment, player) — the table enforces it, which is
    // why a third player carries the in-flight row.
    let (session_a, player_a) = session_with_player(&db, project_id, "USAGEA", "Ada").await;
    let (session_b, player_b) = session_with_player(&db, project_id, "USAGEB", "Grace").await;
    let (session_c, player_c) = session_with_player(&db, project_id, "USAGEC", "Linus").await;
    record_verdict(&db, session_a, player_a, links[0], 25, "scored").await;
    record_verdict(&db, session_a, player_a, links[1], 15, "scored").await;
    record_verdict(&db, session_b, player_b, links[0], -15, "scored").await;
    // A run that never produced a verdict is counted apart, not as a zero.
    record_verdict(&db, session_b, player_b, links[1], 0, "failed").await;
    // And one still in flight is neither.
    record_verdict(&db, session_c, player_c, links[1], 0, "running").await;

    let stats = usage_of(app, &cookie, "worked-hard").await["stats"].clone();
    assert_eq!(stats["verdicts"], 3, "{stats}");
    assert_eq!(stats["failed_runs"], 1, "{stats}");
    assert_eq!(stats["points_total"], 25, "25 + 15 - 15: {stats}");
    assert_eq!(stats["points_awarded"], 40, "{stats}");
    assert_eq!(stats["points_withdrawn"], 15, "{stats}");
    assert_eq!(stats["sessions"], 2, "{stats}");
    assert_eq!(stats["players"], 2, "{stats}");
    assert!(stats["last_verdict_at"].is_string(), "{stats}");
}

/// A judge nobody has attached still appears, with an honest set of zeroes —
/// "never used" is the answer an admin came for.
#[tokio::test]
async fn a_judge_that_never_ran_reports_zeroes() {
    let state = test_state().await;
    let app = build_router(state);
    let (_, cookie) = register_and_login(app.clone(), "admin@usage-c.test", "password-12345").await;
    create_judge(&app, &cookie, "never-used").await;

    let usage = usage_of(app, &cookie, "never-used").await;
    assert!(usage["attachments"].as_array().expect("array").is_empty());
    assert_eq!(usage["stats"]["verdicts"], 0);
    assert_eq!(usage["stats"]["points_total"], 0);
    assert!(usage["stats"]["last_verdict_at"].is_null());
}

/// Admin-only, like everything else under `/api/admin`.
#[tokio::test]
async fn usage_is_refused_without_an_admin_cookie() {
    let state = test_state().await;
    let app = build_router(state);
    let (status, _, _) = read_body(
        app.oneshot(req(Method::GET, "/api/admin/judges/usage", None, None))
            .await
            .expect("resp"),
    )
    .await;
    assert!(
        status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN,
        "anonymous got {status}"
    );
}
