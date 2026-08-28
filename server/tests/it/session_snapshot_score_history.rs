//! Integration tests for `arena_core::scoring::build_score_history`.
//!
//! Covers: empty sessions, interleaved task_results + judge_results merge,
//! cumulative-sum parity with `aggregate_scores`, ordered/non-negative `t`,
//! re-judge delta replacement at the original timestamp, and `None` when
//! `started_at` is absent.

use arena_core::scoring::{aggregate_scores, build_score_history};
use arena_core::session_status::SessionStatus;
use migration::{Migrator, MigratorTrait};
use sea_orm::{ActiveModelTrait, Set};
use server::entities::{
    judge_results, judges, players, projects, sessions, task_judges, task_results, tasks, users,
};
use server::{AppState, AuthConfig};
use std::time::Duration;
use uuid::Uuid;

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
    AppState::new(db, cfg)
}

async fn insert_user(state: &AppState, is_admin: bool, name: &str) -> Uuid {
    let user_id = Uuid::new_v4();
    users::ActiveModel {
        id: Set(user_id),
        email: Set(format!("{name}-{user_id}@example.com")),
        password_hash: Set(None),
        display_name: Set(name.to_string()),
        created_at: Set(chrono::Utc::now()),
        updated_at: Set(chrono::Utc::now()),
        is_admin: Set(is_admin),
        avatar_url: Set(None),
        email_verified: Set(false),
        username: Set(None),
        plan: Set(arena_core::quota::PLAN_PREMIUM.to_string()),
        judge_run_limit: Set(None),
        judge_run_credits: Set(0),
    }
    .insert(&state.db)
    .await
    .expect("insert user");
    user_id
}

async fn insert_project(state: &AppState, owner_id: Uuid) -> Uuid {
    let project_id = Uuid::new_v4();
    projects::ActiveModel {
        id: Set(project_id),
        name: Set("Score History Project".to_string()),
        slug: Set(None),
        description: Set(String::new()),
        category: Set(None),
        tags: Set(String::new()),
        cover_image_url: Set(None),
        owner_user_id_fk: Set(owner_id),
        public: Set(true),
        archived_at: Set(None),
        created_at: Set(chrono::Utc::now()),
        updated_at: Set(chrono::Utc::now()),
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

async fn insert_session(
    state: &AppState,
    project_id: Uuid,
    started_at: Option<chrono::DateTime<chrono::Utc>>,
) -> Uuid {
    let session_id = Uuid::new_v4();
    sessions::ActiveModel {
        id: Set(session_id),
        name: Set("Score History Session".to_string()),
        created_at: Set(chrono::Utc::now()),
        owner_id_fk: Set(None),
        status: Set(SessionStatus::Running),
        join_code: Set(format!("SH{}", &Uuid::new_v4().to_string()[..4]).to_uppercase()),
        game_server_id: Set(None),
        cancel_reason: Set(None),
        cancelled_by: Set(None),
        started_at: Set(started_at),
        finished_at: Set(None),
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
        created_at: Set(chrono::Utc::now()),
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

async fn insert_task_result(
    state: &AppState,
    session_id: Uuid,
    player_id: Uuid,
    task_id: Option<Uuid>,
    point_delta: i32,
    created_at: chrono::DateTime<chrono::Utc>,
) -> Uuid {
    let id = Uuid::new_v4();
    task_results::ActiveModel {
        id: Set(id),
        session_id_fk: Set(session_id),
        player_id_fk: Set(player_id),
        task_id: Set(task_id),
        answer: Set(String::new()),
        created_at: Set(created_at),
        point_delta: Set(point_delta),
        is_bonus: Set(false),
    }
    .insert(&state.db)
    .await
    .expect("insert task result");
    id
}

async fn insert_judge(state: &AppState, slug: &str, name: &str) -> Uuid {
    let judge_id = Uuid::new_v4();
    let now = chrono::Utc::now();
    judges::ActiveModel {
        id: Set(judge_id),
        slug: Set(slug.to_string()),
        name: Set(name.to_string()),
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

async fn insert_task_judge(state: &AppState, task_id: Uuid, judge_id: Uuid) -> Uuid {
    let id = Uuid::new_v4();
    let now = chrono::Utc::now();
    task_judges::ActiveModel {
        id: Set(id),
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
    id
}

async fn insert_judge_result(
    state: &AppState,
    session_id: Uuid,
    player_id: Uuid,
    task_judge_id: Uuid,
    point_delta: i32,
    created_at: chrono::DateTime<chrono::Utc>,
) -> Uuid {
    let id = Uuid::new_v4();
    judge_results::ActiveModel {
        id: Set(id),
        session_id_fk: Set(session_id),
        player_id_fk: Set(player_id),
        task_judge_id: Set(task_judge_id),
        rating: Set(serde_json::json!({"score": 5})),
        point_delta: Set(point_delta),
        feedback: Set(String::new()),
        model: Set("test-model".into()),
        provider: Set(String::new()),
        raw_output: Set(String::new()),
        duration_ms: Set(None),
        run_log: Set(None),
        tokens_input: Set(None),
        tokens_output: Set(None),
        tokens_cache_read: Set(None),
        tokens_cache_write: Set(None),
        status: Set("scored".to_string()),
        error: Set(None),
        created_at: Set(created_at),
        updated_at: Set(created_at),
        verdict_kind: Set(None),
    }
    .insert(&state.db)
    .await
    .expect("insert judge result");
    id
}

async fn update_judge_result_delta(state: &AppState, id: Uuid, new_delta: i32) {
    use sea_orm::EntityTrait;
    let row = judge_results::Entity::find_by_id(id)
        .one(&state.db)
        .await
        .expect("find judge result")
        .expect("judge result exists");
    let mut am: judge_results::ActiveModel = row.into();
    am.point_delta = Set(new_delta);
    am.update(&state.db).await.expect("update judge result");
}

#[tokio::test]
async fn build_score_history_none_when_started_at_none() {
    let state = test_state().await;
    let owner = insert_user(&state, true, "owner").await;
    let project_id = insert_project(&state, owner).await;
    let session_id = insert_session(&state, project_id, None).await;

    let out = build_score_history(&state.db, session_id, None)
        .await
        .expect("ok");
    assert!(out.is_none(), "expected None when started_at is None");
}

#[tokio::test]
async fn build_score_history_none_when_no_rows() {
    let state = test_state().await;
    let owner = insert_user(&state, true, "owner").await;
    let project_id = insert_project(&state, owner).await;
    let started = chrono::Utc::now();
    let session_id = insert_session(&state, project_id, Some(started)).await;

    let out = build_score_history(&state.db, session_id, Some(started))
        .await
        .expect("ok");
    assert!(out.is_none(), "expected None when no rows exist");
}

#[tokio::test]
async fn build_score_history_coalesces_same_second_events() {
    // PERF-M1: events within one second collapse to a single sample carrying
    // the combined cumulative, rather than one full snapshot per row.
    let state = test_state().await;
    let owner = insert_user(&state, true, "owner").await;
    let p1_user = insert_user(&state, false, "alpha").await;
    let p2_user = insert_user(&state, false, "beta").await;
    let project_id = insert_project(&state, owner).await;
    let started = chrono::Utc::now();
    let session_id = insert_session(&state, project_id, Some(started)).await;
    let task_id = insert_task(&state, project_id, 1, "T1").await;
    let p1 = insert_player(&state, session_id, p1_user, "Alpha").await;
    let p2 = insert_player(&state, session_id, p2_user, "Beta").await;

    // Three events all in the same second (t = 2).
    let t = started + chrono::Duration::seconds(2);
    insert_task_result(&state, session_id, p1, Some(task_id), 10, t).await;
    insert_task_result(
        &state,
        session_id,
        p2,
        Some(task_id),
        4,
        t + chrono::Duration::milliseconds(300),
    )
    .await;
    insert_task_result(
        &state,
        session_id,
        p1,
        Some(task_id),
        6,
        t + chrono::Duration::milliseconds(600),
    )
    .await;

    let samples = build_score_history(&state.db, session_id, Some(started))
        .await
        .expect("ok")
        .expect("some samples");
    assert_eq!(
        samples.len(),
        1,
        "same-second events coalesce to one sample"
    );
    assert_eq!(samples[0].t, 2.0);
    let p1_total = *samples[0]
        .scores
        .iter()
        .find(|(k, _)| k.as_uuid() == p1)
        .unwrap()
        .1;
    let p2_total = *samples[0]
        .scores
        .iter()
        .find(|(k, _)| k.as_uuid() == p2)
        .unwrap()
        .1;
    assert_eq!(p1_total, 16, "p1 cumulative folds both same-second deltas");
    assert_eq!(p2_total, 4);
}

#[tokio::test]
async fn build_score_history_cumulative_matches_aggregate_and_is_ordered() {
    let state = test_state().await;
    let owner = insert_user(&state, true, "owner").await;
    let p1_user = insert_user(&state, false, "alpha").await;
    let p2_user = insert_user(&state, false, "beta").await;
    let project_id = insert_project(&state, owner).await;
    let started = chrono::Utc::now();
    let session_id = insert_session(&state, project_id, Some(started)).await;
    let task_id = insert_task(&state, project_id, 1, "T1").await;
    let p1 = insert_player(&state, session_id, p1_user, "Alpha").await;
    let p2 = insert_player(&state, session_id, p2_user, "Beta").await;
    let judge_id = insert_judge(&state, "j1", "Judge 1").await;
    let task_judge_id = insert_task_judge(&state, task_id, judge_id).await;

    // Interleaved timestamps relative to `started`.
    let t0 = started + chrono::Duration::seconds(1);
    let t1 = started + chrono::Duration::seconds(3);
    let t2 = started + chrono::Duration::seconds(5);
    let t3 = started + chrono::Duration::seconds(7);

    // task_result p1 +10 @ t0
    insert_task_result(&state, session_id, p1, Some(task_id), 10, t0).await;
    // judge_result p2 +4 @ t1
    insert_judge_result(&state, session_id, p2, task_judge_id, 4, t1).await;
    // task_result p1 -2 @ t2
    insert_task_result(&state, session_id, p1, Some(task_id), -2, t2).await;
    // judge_result p1 +6 @ t3
    insert_judge_result(&state, session_id, p1, task_judge_id, 6, t3).await;

    let samples = build_score_history(&state.db, session_id, Some(started))
        .await
        .expect("ok")
        .expect("some samples");

    // (d) one sample per row.
    assert_eq!(
        samples.len(),
        4,
        "one sample per distinct second (all 4 are)"
    );

    // (c) t values non-negative and ordered.
    let ts: Vec<f64> = samples.iter().map(|s| s.t).collect();
    assert!(ts.iter().all(|t| *t >= 0.0), "all t >= 0");
    let mut sorted = ts.clone();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
    assert_eq!(ts, sorted, "t values ordered ascending");

    // Expected cumulative sequence:
    //   t0: p1=10            -> scores {p1:10}
    //   t1: p2=+4            -> scores {p1:10, p2:4}
    //   t2: p1=-2            -> scores {p1:8,  p2:4}
    //   t3: p1=+6            -> scores {p1:14, p2:4}
    assert_eq!(samples[0].scores.len(), 1);
    assert_eq!(samples[0].scores.iter().next().unwrap().1, &10i64);

    assert_eq!(samples[1].scores.len(), 2);

    assert_eq!(samples[2].scores.len(), 2);
    let p1_after_t2 = *samples[2]
        .scores
        .iter()
        .find(|(k, _)| k.as_uuid() == p1)
        .unwrap()
        .1;
    assert_eq!(p1_after_t2, 8);

    // (b) cumulative at the last sample matches aggregate_scores totals.
    let last = samples.last().unwrap();
    let aggregates = aggregate_scores(&state.db, session_id)
        .await
        .expect("aggregate");
    let p1_total = aggregates.get(&p1).map(|d| d.total_points).unwrap_or(0);
    let p2_total = aggregates.get(&p2).map(|d| d.total_points).unwrap_or(0);
    let last_p1 = *last
        .scores
        .iter()
        .find(|(k, _)| k.as_uuid() == p1)
        .unwrap()
        .1;
    let last_p2 = *last
        .scores
        .iter()
        .find(|(k, _)| k.as_uuid() == p2)
        .unwrap()
        .1;
    assert_eq!(last_p1, p1_total, "p1 cumulative matches aggregate");
    assert_eq!(last_p2, p2_total, "p2 cumulative matches aggregate");
    assert_eq!(p1_total, 14);
    assert_eq!(p2_total, 4);

    // (e) BTreeMap keys are sorted — iterate and verify ascending UUID order.
    let keys: Vec<_> = last.scores.keys().collect();
    let mut sorted_keys = keys.clone();
    sorted_keys.sort();
    assert_eq!(keys, sorted_keys, "BTreeMap keys sorted");
}

#[tokio::test]
async fn build_score_history_re_judge_replaces_delta_at_same_t() {
    let state = test_state().await;
    let owner = insert_user(&state, true, "owner").await;
    let p1_user = insert_user(&state, false, "alpha").await;
    let project_id = insert_project(&state, owner).await;
    let started = chrono::Utc::now();
    let session_id = insert_session(&state, project_id, Some(started)).await;
    let task_id = insert_task(&state, project_id, 1, "T1").await;
    let p1 = insert_player(&state, session_id, p1_user, "Alpha").await;
    let judge_id = insert_judge(&state, "j1", "Judge 1").await;
    let task_judge_id = insert_task_judge(&state, task_id, judge_id).await;

    let t0 = started + chrono::Duration::seconds(2);
    let jr_id = insert_judge_result(&state, session_id, p1, task_judge_id, 3, t0).await;

    let samples_before = build_score_history(&state.db, session_id, Some(started))
        .await
        .expect("ok")
        .expect("some samples");
    let t_before = samples_before[0].t;
    let total_before = *samples_before[0].scores.values().next().unwrap();
    assert_eq!(total_before, 3);

    // Re-judge: UPDATE point_delta without changing created_at.
    update_judge_result_delta(&state, jr_id, 9).await;

    let samples_after = build_score_history(&state.db, session_id, Some(started))
        .await
        .expect("ok")
        .expect("some samples");
    assert_eq!(samples_after.len(), 1, "still one sample");
    let t_after = samples_after[0].t;
    let total_after = *samples_after[0].scores.values().next().unwrap();
    assert_eq!(t_after, t_before, "t unchanged after re-judge");
    assert_eq!(total_after, 9, "revised delta appears at original t");
}
