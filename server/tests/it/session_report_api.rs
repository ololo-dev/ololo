use arena_core::session_status::SessionStatus;
use axum::body::{Body, to_bytes};
use axum::http::{Method, Request, StatusCode};
use chrono::Utc;
use migration::{Migrator, MigratorTrait};
use sea_orm::{ActiveModelTrait, Set};
use server::rate_limiter::NoOpRateLimiter;
use server::{AppState, AuthConfig, build_router};
use std::sync::Arc;
use std::time::Duration;
use tower::ServiceExt;
use uuid::Uuid;

use server::entities::{players, projects, sessions, task_results, tasks, users};

const ORIGIN: &str = "http://localhost:5173";
const TEST_JWT_SECRET: &[u8] = b"integration-test-secret-32-bytes-or-more-xxxxxxx";

async fn test_state() -> AppState {
    let db = sea_orm::Database::connect("sqlite::memory:")
        .await
        .expect("sqlite connect");
    Migrator::up(&db, None).await.expect("migrate up");
    let cfg = AuthConfig {
        jwt_signing_key: TEST_JWT_SECRET.to_vec(),
        frontend_origins: vec![ORIGIN.to_string()],
        access_ttl: Duration::from_secs(900),
        refresh_ttl: Duration::from_secs(30 * 86_400),
        max_agents_per_session: 16,
    };
    let mut state = AppState::new(db, cfg);
    state.rate_limiter = Arc::new(NoOpRateLimiter);
    state
}

async fn insert_user(state: &AppState, is_admin: bool, name: &str) -> Uuid {
    let user_id = Uuid::new_v4();
    users::ActiveModel {
        id: Set(user_id),
        email: Set(format!("{name}-{user_id}@example.com")),
        password_hash: Set(None),
        display_name: Set(name.to_string()),
        created_at: Set(Utc::now()),
        updated_at: Set(Utc::now()),
        is_admin: Set(is_admin),
        avatar_url: Set(Some(format!("https://avatars.example/{name}.png"))),
        email_verified: Set(false),
        username: Set(Some(name.to_string())),
        plan: Set(arena_core::quota::PLAN_PREMIUM.to_string()),
        judge_run_limit: Set(None),
        judge_run_credits: Set(0),
    }
    .insert(&state.db)
    .await
    .expect("insert user");
    user_id
}

async fn insert_project(state: &AppState, owner_id: Uuid, public: bool) -> Uuid {
    let project_id = Uuid::new_v4();
    projects::ActiveModel {
        id: Set(project_id),
        name: Set("Session Report Project".to_string()),
        slug: Set(None),
        description: Set(String::new()),
        category: Set(None),
        tags: Set(String::new()),
        cover_image_url: Set(None),
        owner_user_id_fk: Set(owner_id),
        public: Set(public),
        archived_at: Set(None),
        created_at: Set(Utc::now()),
        updated_at: Set(Utc::now()),
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
    project_id
}

async fn insert_session(state: &AppState, project_id: Uuid, status: SessionStatus) -> Uuid {
    let session_id = Uuid::new_v4();
    sessions::ActiveModel {
        id: Set(session_id),
        name: Set("Session Report Test Session".to_string()),
        created_at: Set(Utc::now()),
        owner_id_fk: Set(None),
        status: Set(status),
        join_code: Set(format!("REP{}", &Uuid::new_v4().to_string()[..4]).to_uppercase()),
        game_server_id: Set(None),
        cancel_reason: Set(None),
        cancelled_by: Set(None),
        started_at: Set(Some(Utc::now())),
        finished_at: Set(Some(Utc::now())),
        paused_at: Set(None),
        paused_duration_secs: Set(None),
        project_id_fk: Set(project_id),
    }
    .insert(&state.db)
    .await
    .expect("insert session");
    session_id
}

async fn insert_task(state: &AppState, project_id: Uuid, ordinal: i32, title: &str) -> Uuid {
    let task_id = Uuid::new_v4();
    tasks::ActiveModel {
        id: Set(task_id),
        project_id_fk: Set(project_id),
        ordinal: Set(ordinal),
        title: Set(title.to_string()),
        content: Set(String::new()),
        test_template: Set(serde_json::json!({ "kind": "shell", "command_template": "echo ok" })),
        created_at: Set(Utc::now()),
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
    .expect("insert task");
    task_id
}

async fn insert_player(
    state: &AppState,
    session_id: Uuid,
    user_id: Uuid,
    display_name: &str,
) -> Uuid {
    let player_id = Uuid::new_v4();
    players::ActiveModel {
        id: Set(player_id),
        session_id_fk: Set(session_id),
        user_id_fk: Set(Some(user_id)),
        display_name: Set(display_name.to_string()),
        fingerprint: Set(None),
        metadata_json: Set(None),
        joined_at: Set(Utc::now()),
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

#[allow(clippy::too_many_arguments)]
async fn insert_result(
    state: &AppState,
    session_id: Uuid,
    player_id: Uuid,
    task_id: Option<Uuid>,
    point_delta: i32,
    answer: &str,
    created_at: chrono::DateTime<Utc>,
) {
    task_results::ActiveModel {
        id: Set(Uuid::new_v4()),
        session_id_fk: Set(session_id),
        player_id_fk: Set(player_id),
        task_id: Set(task_id),
        answer: Set(answer.to_string()),
        created_at: Set(created_at),
        point_delta: Set(point_delta),
        is_bonus: Set(false),
    }
    .insert(&state.db)
    .await
    .expect("insert task result");
}

#[tokio::test]
async fn session_report_returns_timeline_and_leaderboard_for_finished_session() {
    let state = test_state().await;
    let owner = insert_user(&state, true, "owner").await;
    let p1_user = insert_user(&state, false, "alpha").await;
    let p2_user = insert_user(&state, false, "beta").await;
    let project_id = insert_project(&state, owner, true).await;
    let session_id = insert_session(&state, project_id, SessionStatus::Finished).await;
    let t1 = insert_task(&state, project_id, 1, "Task One").await;
    let t2 = insert_task(&state, project_id, 2, "Task Two").await;
    let p1 = insert_player(&state, session_id, p1_user, "Alpha").await;
    let p2 = insert_player(&state, session_id, p2_user, "Beta").await;
    // Distinct-second timestamps: score_history coalesces samples by second
    // (PERF-M1), so three same-second inserts would collapse to one sample and
    // make the count depend on sub-second insert timing. Space them one second
    // apart to deterministically get one sample per scored result.
    let base = Utc::now();
    insert_result(&state, session_id, p1, Some(t1), 1, "ok-alpha-1", base).await;
    insert_result(
        &state,
        session_id,
        p1,
        Some(t2),
        1,
        "ok-alpha-2",
        base + chrono::Duration::seconds(1),
    )
    .await;
    insert_result(
        &state,
        session_id,
        p2,
        Some(t1),
        1,
        "ok-beta-1",
        base + chrono::Duration::seconds(2),
    )
    .await;

    let app = build_router(state);

    let resp = app
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri(format!("/api/sessions/{session_id}/report"))
                .header("Origin", ORIGIN)
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("oneshot");

    assert_eq!(resp.status(), StatusCode::OK);
    let body = to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .expect("read body");
    let json: serde_json::Value = serde_json::from_slice(&body).expect("json body");

    let leaderboard = json["leaderboard"].as_array().expect("leaderboard array");
    assert_eq!(leaderboard.len(), 2);
    assert_eq!(leaderboard[0]["display_name"], "Alpha");
    assert_eq!(leaderboard[0]["tests_passed"], 2);
    assert_eq!(leaderboard[1]["display_name"], "Beta");
    assert_eq!(leaderboard[1]["tests_passed"], 1);

    let timeline = json["timeline"].as_array().expect("timeline array");
    assert_eq!(timeline.len(), 3);
    assert_eq!(timeline[0]["player_display_name"], "Alpha");
    assert_eq!(timeline[0]["task_title"], "Task One");

    // No judge runs were owed in this fixture, so the finish is final and the
    // dashboard may celebrate right away.
    assert_eq!(json["judges_pending"], 0);

    let players_arr = json["players"].as_array().expect("players array");
    assert_eq!(players_arr.len(), 2);
    let alpha = players_arr
        .iter()
        .find(|p| p["display_name"] == "Alpha")
        .expect("alpha player entry");
    assert_eq!(alpha["avatar_url"], "https://avatars.example/alpha.png");
    assert_eq!(alpha["username"], "alpha");
    assert_eq!(alpha["user_id"], p1_user.to_string());

    let score_history = json["score_history"]
        .as_array()
        .expect("score_history array");
    assert_eq!(score_history.len(), 3, "one sample per scored result");
    let last = score_history.last().expect("last sample");
    assert!(last["t"].as_f64().expect("t is a number") >= 0.0);
    assert!(last["scores"].is_object());
}

#[tokio::test]
async fn session_report_requires_terminal_session() {
    let state = test_state().await;
    let owner = insert_user(&state, true, "owner").await;
    let project_id = insert_project(&state, owner, true).await;
    let session_id = insert_session(&state, project_id, SessionStatus::Running).await;
    let app = build_router(state);

    let resp = app
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri(format!("/api/sessions/{session_id}/report"))
                .header("Origin", ORIGIN)
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("oneshot");

    assert_eq!(resp.status(), StatusCode::CONFLICT);
}

#[tokio::test]
async fn private_project_report_denies_anonymous_but_allows_member() {
    // SEC-M3: a session under a NON-public project must not be readable by
    // anyone with the id/join code. Anonymous → 404 (existence hidden); an
    // authenticated member → 200.
    let state = test_state().await;
    let owner = insert_user(&state, true, "owner").await;
    let member_user = insert_user(&state, false, "member").await;
    let project_id = insert_project(&state, owner, false).await; // private
    let session_id = insert_session(&state, project_id, SessionStatus::Finished).await;
    let _member = insert_player(&state, session_id, member_user, "Member").await;

    let access = server::auth::jwt::issue_access_token(
        &state.jwt_encoding_key,
        member_user,
        "member@example.com",
        Duration::from_secs(900),
    )
    .expect("issue access token");

    let app = build_router(state);

    // Anonymous → 404.
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri(format!("/api/sessions/{session_id}/report"))
                .header("Origin", ORIGIN)
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("oneshot");
    assert_eq!(
        resp.status(),
        StatusCode::NOT_FOUND,
        "anonymous must not read a private session report"
    );

    // Authenticated member → 200.
    let resp = app
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri(format!("/api/sessions/{session_id}/report"))
                .header("Origin", ORIGIN)
                .header("Authorization", format!("Bearer {access}"))
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("oneshot");
    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "member must read the private session report"
    );
}
