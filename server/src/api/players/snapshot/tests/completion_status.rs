//! Derived per-player completion status on the player snapshot payload.
//!
//! The snapshot (detail) path must carry the judge-aware status computed by
//! `arena_core::session_completion::session_player_statuses`: a tasks-done
//! player with a pending judge run is `awaiting_judges`, a player still
//! mid-tasks is `in_progress`.

use super::*;
use arena_core::protocol::PlayerCompletionStatus;
use arena_core::session_completion::SCHEDULER_STATE_COMPLETED;

/// Seed a running session with one judged task and two players:
/// player A has exhausted their task list (scheduler state "completed"),
/// player B is still mid-task. The task has one attached judge and no
/// `judge_results` row yet, so A's judge run is pending.
///
/// Returns `(session_id, player_a, player_b)`.
async fn seed_two_player_judged_session(db: &sea_orm::DatabaseConnection) -> (Uuid, Uuid, Uuid) {
    let user_id = Uuid::new_v4();
    crate::entities::users::ActiveModel {
        id: Set(user_id),
        email: Set("cs@test.local".to_string()),
        password_hash: Set(None),
        display_name: Set("User".to_string()),
        created_at: Set(Utc::now()),
        updated_at: Set(Utc::now()),
        is_admin: Set(false),
        avatar_url: Set(None),
        email_verified: Set(false),
        username: Set(None),
        plan: Set(arena_core::quota::PLAN_PREMIUM.to_string()),
        judge_run_limit: Set(None),
        judge_run_credits: Set(0),
    }
    .insert(db)
    .await
    .expect("insert user");

    let project_id = Uuid::new_v4();
    crate::entities::projects::ActiveModel {
        id: Set(project_id),
        name: Set("proj-cs".to_string()),
        slug: Set(None),
        description: Set("".to_string()),
        category: Set(None),
        tags: Set("[]".to_string()),
        cover_image_url: Set(None),
        owner_user_id_fk: Set(user_id),
        public: Set(false),
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
    .insert(db)
    .await
    .expect("insert project");

    let session_id = Uuid::new_v4();
    crate::entities::sessions::ActiveModel {
        id: Set(session_id),
        name: Set("s-cs".to_string()),
        created_at: Set(Utc::now()),
        owner_id_fk: Set(Some(user_id)),
        status: Set(SessionStatus::Running),
        join_code: Set("CSCODE".to_string()),
        started_at: Set(Some(Utc::now())),
        finished_at: Set(None),
        paused_at: Set(None),
        paused_duration_secs: Set(None),
        project_id_fk: Set(project_id),
        game_server_id: Set(None),
        cancel_reason: Set(None),
        cancelled_by: Set(None),
    }
    .insert(db)
    .await
    .expect("insert session");

    let mut player_ids = Vec::new();
    for name in ["a", "b"] {
        let player_id = Uuid::new_v4();
        crate::entities::players::ActiveModel {
            id: Set(player_id),
            session_id_fk: Set(session_id),
            user_id_fk: Set(Some(user_id)),
            display_name: Set(name.to_string()),
            fingerprint: Set(None),
            metadata_json: Set(None),
            joined_at: Set(Utc::now()),
            reconnected_at: Set(None),
            revoked_at: Set(None),
            agent_connected: Set(false),
            agent_last_seen_at: Set(None),
        }
        .insert(db)
        .await
        .expect("insert player");
        player_ids.push(player_id);
    }
    let (player_a, player_b) = (player_ids[0], player_ids[1]);

    let task_id = Uuid::new_v4();
    let template = crate::task_template::TestTemplate {
        kind: crate::task_template::TestKind::Shell,
        command_template: "echo hi".to_string(),
        placeholders: vec![],
        matchers: crate::task_template::Matchers::default(),
        backoff: crate::task_template::Backoff::default(),
        fixtures: vec![],
        answer_template: Some("".to_string()),
    };
    crate::entities::tasks::ActiveModel {
        id: Set(task_id),
        project_id_fk: Set(project_id),
        ordinal: Set(1),
        title: Set("t1".to_string()),
        content: Set("d".to_string()),
        test_template: Set(serde_json::to_value(template).expect("serialize")),
        created_at: Set(Utc::now()),
        tags: Set("[]".to_string()),
        point_value: Set(10),
        completion_bonus_points: Set(10),
        deadline_secs: Set(Some(30)),
        min_interval_secs: Set(Some(1)),
        interval_increment_secs: Set(Some(1)),
        max_interval_secs: Set(Some(10)),
        fail_points: Set(0),
        no_response_points: Set(0),
        evaluation: Set(None),
    }
    .insert(db)
    .await
    .expect("insert task");

    // One judge attached to the task; no judge_results row → run pending.
    let judge_id = Uuid::new_v4();
    crate::entities::judges::ActiveModel {
        id: Set(judge_id),
        slug: Set("code-cleanliness".to_string()),
        name: Set("Code Cleanliness".to_string()),
        description: Set("".to_string()),
        prompt: Set("judge it".to_string()),
        rating_scale: Set(serde_json::json!({"min": 1, "max": 10})),
        kind: Set("llm".to_string()),
        scope: Set("task".to_string()),
        evidence_mode: Set("tools".to_string()),
        evidence_needs: Set(None),
        created_at: Set(Utc::now()),
        updated_at: Set(Utc::now()),
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
    .insert(db)
    .await
    .expect("insert judge");
    crate::entities::task_judges::ActiveModel {
        id: Set(Uuid::new_v4()),
        task_id: Set(task_id),
        judge_id: Set(judge_id),
        ordinal: Set(0),
        rating_scale_override: Set(None),
        created_at: Set(Utc::now()),
        updated_at: Set(Utc::now()),
        weight: Set(None),
    }
    .insert(db)
    .await
    .expect("insert task_judge");

    // Player A: task list exhausted. Player B: still working the task.
    crate::entities::session_scheduler_state::ActiveModel {
        id: Set(Uuid::new_v4()),
        session_id_fk: Set(session_id),
        player_id_fk: Set(player_a),
        task_id: Set(None),
        state: Set(SCHEDULER_STATE_COMPLETED.to_string()),
        next_probe_at: Set(None),
        created_at: Set(Utc::now()),
        updated_at: Set(Utc::now()),
    }
    .insert(db)
    .await
    .expect("insert scheduler state a");
    crate::entities::session_scheduler_state::ActiveModel {
        id: Set(Uuid::new_v4()),
        session_id_fk: Set(session_id),
        player_id_fk: Set(player_b),
        task_id: Set(Some(task_id)),
        state: Set("active".to_string()),
        next_probe_at: Set(None),
        created_at: Set(Utc::now()),
        updated_at: Set(Utc::now()),
    }
    .insert(db)
    .await
    .expect("insert scheduler state b");

    (session_id, player_a, player_b)
}

#[tokio::test]
async fn snapshot_reports_awaiting_judges_for_tasks_done_player_with_pending_judge() {
    let state = fresh_state().await;
    let (session_id, player_a, _player_b) = seed_two_player_judged_session(&state.db).await;

    let snap = build_snapshot(&state, player_a, session_id, "a", 0).await;
    assert_eq!(
        snap.completion_status,
        Some(PlayerCompletionStatus::AwaitingJudges),
        "tasks-done player with a pending judge run must be awaiting_judges"
    );
}

#[tokio::test]
async fn snapshot_reports_in_progress_for_mid_task_player() {
    let state = fresh_state().await;
    let (session_id, _player_a, player_b) = seed_two_player_judged_session(&state.db).await;

    let snap = build_snapshot(&state, player_b, session_id, "b", 0).await;
    assert_eq!(
        snap.completion_status,
        Some(PlayerCompletionStatus::InProgress),
        "player with tasks remaining must be in_progress"
    );
}
