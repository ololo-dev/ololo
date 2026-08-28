//! Regression: a `ZmqEvent::JudgeScored` must refresh the server's cached
//! session leaderboard. The probe-driven poll task only recomputes when a
//! probe resolves, so judge verdicts landing after the last probe (typical:
//! judges run after task/session completion) previously never reached the
//! cache — the player page's `score` kept showing the pre-judge total.

use arena_core::protocol::ZmqEvent;
use arena_core::session_status::SessionStatus;
use migration::{Migrator, MigratorTrait};
use sea_orm::{ActiveModelTrait, Set};
use server::entities::{
    judge_results, judges, players, projects, sessions, task_judges, task_results, tasks, users,
};
use server::protocol::ArenaFrame;
use server::state::{SessionCacheInner, SessionEntry};
use server::{AppState, AuthConfig};
use std::sync::{Arc, RwLock};
use std::time::Duration;
use tokio::sync::broadcast;
use uuid::Uuid;

const JOIN_CODE: &str = "JUDGE1";

async fn test_state() -> AppState {
    let db = sea_orm::Database::connect("sqlite::memory:")
        .await
        .expect("sqlite connect");
    Migrator::up(&db, None).await.expect("migrate up");
    let cfg = AuthConfig {
        jwt_signing_key: b"integration-test-secret-32-bytes-or-more-xxxxxxx".to_vec(),
        frontend_origins: vec!["http://localhost:5173".to_string()],
        access_ttl: Duration::from_secs(900),
        refresh_ttl: Duration::from_secs(30 * 86_400),
        max_agents_per_session: 16,
    };
    AppState::new(db, cfg)
}

/// Minimal fixture chain: user → project → session(JOIN_CODE) → player →
/// task → task_result(+100) → judge chain → judge_result(scored, −30).
async fn seed(state: &AppState) -> (Uuid, Uuid, Uuid) {
    let now = chrono::Utc::now();
    let user_id = Uuid::new_v4();
    users::ActiveModel {
        id: Set(user_id),
        email: Set(format!("u-{user_id}@example.com")),
        password_hash: Set(None),
        display_name: Set("Player".to_string()),
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
    .expect("user");

    let project_id = Uuid::new_v4();
    projects::ActiveModel {
        id: Set(project_id),
        name: Set("P".to_string()),
        slug: Set(None),
        description: Set(String::new()),
        category: Set(None),
        tags: Set(String::new()),
        cover_image_url: Set(None),
        owner_user_id_fk: Set(user_id),
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

    let player_id = Uuid::new_v4();
    players::ActiveModel {
        id: Set(player_id),
        session_id_fk: Set(session_id),
        user_id_fk: Set(Some(user_id)),
        display_name: Set("Player".to_string()),
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

    task_results::ActiveModel {
        id: Set(Uuid::new_v4()),
        session_id_fk: Set(session_id),
        player_id_fk: Set(player_id),
        task_id: Set(Some(task_id)),
        answer: Set(String::new()),
        created_at: Set(now),
        point_delta: Set(100),
        is_bonus: Set(false),
    }
    .insert(&state.db)
    .await
    .expect("task_result");

    let judge_id = Uuid::new_v4();
    judges::ActiveModel {
        id: Set(judge_id),
        slug: Set("anti-cheater".to_string()),
        name: Set("Anti Cheater".to_string()),
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

    let task_judge_id = Uuid::new_v4();
    task_judges::ActiveModel {
        id: Set(task_judge_id),
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
    .expect("task_judge");

    judge_results::ActiveModel {
        id: Set(Uuid::new_v4()),
        session_id_fk: Set(session_id),
        player_id_fk: Set(player_id),
        task_judge_id: Set(task_judge_id),
        rating: Set(serde_json::json!(2.0)),
        point_delta: Set(-30),
        feedback: Set("suspicious".to_string()),
        model: Set("m".to_string()),
        provider: Set("ollama".to_string()),
        raw_output: Set(String::new()),
        duration_ms: Set(Some(1000)),
        run_log: Set(None),
        tokens_input: Set(None),
        tokens_output: Set(None),
        tokens_cache_read: Set(None),
        tokens_cache_write: Set(None),
        status: Set("scored".to_string()),
        error: Set(None),
        created_at: Set(now),
        updated_at: Set(now),
        verdict_kind: Set(None),
    }
    .insert(&state.db)
    .await
    .expect("judge_result");

    (session_id, player_id, task_id)
}

fn insert_registry_entry(state: &AppState, session_id: Uuid, stale_total: i64, player_id: Uuid) {
    let (tx, _rx) = broadcast::channel::<ArenaFrame>(16);
    state.session_registry.insert(
        JOIN_CODE.to_string(),
        SessionEntry {
            tx,
            cache: Arc::new(RwLock::new(SessionCacheInner {
                session_id,
                phase: SessionStatus::Running,
                version: 1,
                participants: vec![],
                // Stale pre-judge leaderboard: task points only.
                leaderboard: vec![arena_core::protocol::LeaderboardEntry {
                    player_id: arena_core::protocol::PlayerId(player_id),
                    display_name: "Player".to_string(),
                    agent_display_name: None,
                    total_points: stale_total,
                    tests_passed: 1,
                    total_wall_ms: 0,
                }],
                started_at: None,
            })),
        },
    );
}

fn cached_total(state: &AppState, player_id: Uuid) -> i64 {
    let entry = state.session_registry.get(JOIN_CODE).expect("entry");
    let cache = entry.cache.read().expect("cache read");
    cache
        .leaderboard
        .iter()
        .find(|e| e.player_id.as_uuid() == player_id)
        .map(|e| e.total_points)
        .expect("player in leaderboard")
}

#[tokio::test]
async fn judge_scored_event_refreshes_cached_leaderboard() {
    let state = test_state().await;
    let (session_id, player_id, task_id) = seed(&state).await;
    insert_registry_entry(&state, session_id, 100, player_id);
    assert_eq!(
        cached_total(&state, player_id),
        100,
        "stale pre-judge cache"
    );

    let event = ZmqEvent::JudgeScored {
        join_code: JOIN_CODE.to_string(),
        player_id,
        player_display_name: "Player".to_string(),
        task_id,
        task_ordinal: 0,
        task_title: "T".to_string(),
        point_delta: -30,
        judge_slug: "anti-cheater".to_string(),
        judge_name: "Anti Cheater".to_string(),
        rating: 2.0,
        feedback: "suspicious".to_string(),
        duration_ms: Some(1000),
        timestamp: chrono::Utc::now(),
        version: 2,
        detail: None,
    };
    server::zmq_sub::route_event(&state, &event)
        .await
        .expect("route_event");

    assert_eq!(
        cached_total(&state, player_id),
        70,
        "cache must include the judge delta (100 task − 30 judge)"
    );
}

/// Bridged frames must carry the SERVER's per-session version counter, not
/// the game-server's: browser clients gate on monotonic versions per
/// channel, and two independent counters interleaved on one channel freeze
/// the dashboard (whichever stream is behind gets dropped).
#[tokio::test]
async fn bridged_frames_use_server_version_counter() {
    let state = test_state().await;
    let (session_id, player_id, task_id) = seed(&state).await;
    insert_registry_entry(&state, session_id, 100, player_id);
    let mut rx = state
        .session_registry
        .get(JOIN_CODE)
        .expect("entry")
        .tx
        .subscribe();

    // Incoming zmq versions are wildly off in both directions; outgoing
    // frames must ignore them and continue the server counter (seeded at 1).
    let timer = ZmqEvent::SessionTimer {
        join_code: JOIN_CODE.to_string(),
        phase: "running".to_string(),
        seconds_remaining: 42,
        version: 50_000,
    };
    server::zmq_sub::route_event(&state, &timer)
        .await
        .expect("timer");

    let started = ZmqEvent::TaskStarted {
        join_code: JOIN_CODE.to_string(),
        player_id,
        player_display_name: "Player".to_string(),
        task_id,
        task_ordinal: 0,
        task_title: "T".to_string(),
        timestamp: chrono::Utc::now(),
        version: 3,
    };
    server::zmq_sub::route_event(&state, &started)
        .await
        .expect("started");

    let scored = ZmqEvent::JudgeScored {
        join_code: JOIN_CODE.to_string(),
        player_id,
        player_display_name: "Player".to_string(),
        task_id,
        task_ordinal: 0,
        task_title: "T".to_string(),
        point_delta: -30,
        judge_slug: "anti-cheater".to_string(),
        judge_name: "Anti Cheater".to_string(),
        rating: 2.0,
        feedback: "suspicious".to_string(),
        duration_ms: Some(1000),
        timestamp: chrono::Utc::now(),
        version: 99_999,
        detail: None,
    };
    server::zmq_sub::route_event(&state, &scored)
        .await
        .expect("scored");

    let mut versions = Vec::new();
    while let Ok(frame) = rx.try_recv() {
        match frame {
            ArenaFrame::RunningCountdown {
                version,
                seconds_remaining,
                ..
            } => {
                assert_eq!(seconds_remaining, 42);
                versions.push(version);
            }
            ArenaFrame::TaskStarted { version, .. } => versions.push(version),
            ArenaFrame::TaskScored { version, .. } => versions.push(version),
            _ => {}
        }
    }
    assert_eq!(
        versions,
        vec![2, 3, 4],
        "bridged frames must continue the server's counter, not echo zmq versions"
    );
}
