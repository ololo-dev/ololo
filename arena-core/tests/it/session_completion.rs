//! DB-backed tests for `arena_core::session_completion` — per-player
//! task-completion facts and the all-eligible-players-done aggregate.

use crate::common;
use crate::common::*;

use arena_core::entities::{
    judge_results, judges, players, session_scheduler_state, task_judges, tasks,
};
use arena_core::protocol::player::PlayerCompletionStatus;
use arena_core::session_completion::{
    SCHEDULER_STATE_COMPLETED, all_eligible_players_done, session_player_completion,
    session_player_statuses,
};
use chrono::Utc;
use sea_orm::{ActiveModelTrait, DatabaseConnection, EntityTrait, Set};
use uuid::Uuid;

async fn insert_task(db: &DatabaseConnection, project_id: Uuid, ordinal: i32) -> Uuid {
    let id = Uuid::new_v4();
    tasks::ActiveModel {
        id: Set(id),
        project_id_fk: Set(project_id),
        ordinal: Set(ordinal),
        title: Set(format!("Task {ordinal}")),
        content: Set("do the thing".to_string()),
        test_template: Set(serde_json::json!({"kind":"shell","command_template":"echo ok"})),
        created_at: Set(Utc::now()),
        tags: Set("[]".to_string()),
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
    .insert(db)
    .await
    .expect("insert task");
    id
}

async fn insert_scheduler_row(
    db: &DatabaseConnection,
    session_id: Uuid,
    player_id: Uuid,
    state: &str,
) {
    session_scheduler_state::ActiveModel {
        id: Set(Uuid::new_v4()),
        session_id_fk: Set(session_id),
        player_id_fk: Set(player_id),
        task_id: Set(None),
        state: Set(state.to_string()),
        next_probe_at: Set(None),
        created_at: Set(Utc::now()),
        updated_at: Set(Utc::now()),
    }
    .insert(db)
    .await
    .expect("insert scheduler row");
}

async fn revoke_player(db: &DatabaseConnection, player_id: Uuid) {
    let row = players::Entity::find_by_id(player_id)
        .one(db)
        .await
        .expect("query player")
        .expect("player exists");
    let mut am: players::ActiveModel = row.into();
    am.revoked_at = Set(Some(Utc::now()));
    am.update(db).await.expect("revoke player");
}

#[tokio::test]
async fn player_without_scheduler_row_is_not_done() {
    let db = setup_db().await;
    let user_id = insert_user(&db).await;
    let project_id = insert_project(&db, user_id).await;
    let session_id = insert_session(&db, project_id).await;
    insert_task(&db, project_id, 0).await;
    let player_id = insert_player(&db, session_id).await;

    let facts = session_player_completion(&db, session_id)
        .await
        .expect("completion facts");
    assert_eq!(facts.len(), 1);
    assert_eq!(facts[0].player_id, player_id);
    assert!(
        !facts[0].done,
        "a joined player who has not started must not count as done"
    );
    assert!(
        !all_eligible_players_done(&db, session_id)
            .await
            .expect("all done")
    );
}

#[tokio::test]
async fn completed_scheduler_state_marks_player_done() {
    let db = setup_db().await;
    let user_id = insert_user(&db).await;
    let project_id = insert_project(&db, user_id).await;
    let session_id = insert_session(&db, project_id).await;
    insert_task(&db, project_id, 0).await;
    let done_player = insert_player(&db, session_id).await;
    let pending_player = insert_player(&db, session_id).await;
    insert_scheduler_row(&db, session_id, done_player, SCHEDULER_STATE_COMPLETED).await;
    insert_scheduler_row(&db, session_id, pending_player, "idle").await;

    let facts = session_player_completion(&db, session_id)
        .await
        .expect("completion facts");
    let done = facts.iter().find(|f| f.player_id == done_player).unwrap();
    let pending = facts
        .iter()
        .find(|f| f.player_id == pending_player)
        .unwrap();
    assert!(done.done);
    assert!(!pending.done);
    assert!(
        !all_eligible_players_done(&db, session_id)
            .await
            .expect("all done")
    );

    // The pending player completes too — now the whole session is done.
    mark_scheduler_completed(&db, session_id, pending_player).await;
    assert!(
        all_eligible_players_done(&db, session_id)
            .await
            .expect("all done")
    );
}

async fn mark_scheduler_completed(db: &DatabaseConnection, session_id: Uuid, player_id: Uuid) {
    use sea_orm::{ColumnTrait, QueryFilter};
    let row = session_scheduler_state::Entity::find()
        .filter(session_scheduler_state::Column::SessionIdFk.eq(session_id))
        .filter(session_scheduler_state::Column::PlayerIdFk.eq(player_id))
        .one(db)
        .await
        .expect("query scheduler row")
        .expect("scheduler row exists");
    let mut am: session_scheduler_state::ActiveModel = row.into();
    am.state = Set(SCHEDULER_STATE_COMPLETED.to_string());
    am.update(db).await.expect("update scheduler row");
}

#[tokio::test]
async fn revoked_player_is_excluded_from_the_facts() {
    let db = setup_db().await;
    let user_id = insert_user(&db).await;
    let project_id = insert_project(&db, user_id).await;
    let session_id = insert_session(&db, project_id).await;
    insert_task(&db, project_id, 0).await;
    let done_player = insert_player(&db, session_id).await;
    let revoked = insert_player(&db, session_id).await;
    insert_scheduler_row(&db, session_id, done_player, SCHEDULER_STATE_COMPLETED).await;
    revoke_player(&db, revoked).await;

    let facts = session_player_completion(&db, session_id)
        .await
        .expect("completion facts");
    assert_eq!(facts.len(), 1, "revoked players are not eligible");
    assert_eq!(facts[0].player_id, done_player);
    assert!(
        all_eligible_players_done(&db, session_id)
            .await
            .expect("all done"),
        "a revoked player must never block completion"
    );
}

#[tokio::test]
async fn zero_tasks_makes_every_player_trivially_done() {
    let db = setup_db().await;
    let user_id = insert_user(&db).await;
    let project_id = insert_project(&db, user_id).await;
    let session_id = insert_session(&db, project_id).await;
    let player_id = insert_player(&db, session_id).await;

    let facts = session_player_completion(&db, session_id)
        .await
        .expect("completion facts");
    assert_eq!(facts.len(), 1);
    assert_eq!(facts[0].player_id, player_id);
    assert!(
        facts[0].done,
        "with zero tasks every player is trivially done"
    );
    assert!(
        all_eligible_players_done(&db, session_id)
            .await
            .expect("all done")
    );
}

#[tokio::test]
async fn zero_players_is_never_all_done() {
    let db = setup_db().await;
    let user_id = insert_user(&db).await;
    let project_id = insert_project(&db, user_id).await;
    let session_id = insert_session(&db, project_id).await;
    insert_task(&db, project_id, 0).await;

    assert!(
        session_player_completion(&db, session_id)
            .await
            .expect("completion facts")
            .is_empty()
    );
    assert!(
        !all_eligible_players_done(&db, session_id)
            .await
            .expect("all done"),
        "an empty session must only end via time expiry, never all-done"
    );
}

#[tokio::test]
async fn missing_session_is_an_error() {
    let db = setup_db().await;
    let err = session_player_completion(&db, Uuid::new_v4()).await;
    assert!(err.is_err(), "unknown session must surface as DbErr");
}

// ---------------------------------------------------------------------------
// Judge-aware derived player status (session_player_statuses).
// ---------------------------------------------------------------------------

async fn attach_judge(db: &DatabaseConnection, task_id: Uuid, ordinal: i32) -> Uuid {
    let judge_id = Uuid::new_v4();
    judges::ActiveModel {
        id: Set(judge_id),
        slug: Set(format!("judge-{judge_id}")),
        name: Set("Judge".to_string()),
        description: Set(String::new()),
        prompt: Set("Evaluate.".to_string()),
        rating_scale: Set(serde_json::json!({"min": 0.0, "max": 10.0, "step": 0.5})),
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

    let task_judge_id = Uuid::new_v4();
    task_judges::ActiveModel {
        id: Set(task_judge_id),
        task_id: Set(task_id),
        judge_id: Set(judge_id),
        ordinal: Set(ordinal),
        rating_scale_override: Set(None),
        created_at: Set(Utc::now()),
        updated_at: Set(Utc::now()),
        weight: Set(None),
    }
    .insert(db)
    .await
    .expect("insert task_judge");
    task_judge_id
}

async fn insert_judge_result(
    db: &DatabaseConnection,
    session_id: Uuid,
    player_id: Uuid,
    task_judge_id: Uuid,
    status: &str,
) {
    judge_results::ActiveModel {
        id: Set(Uuid::new_v4()),
        session_id_fk: Set(session_id),
        player_id_fk: Set(player_id),
        task_judge_id: Set(task_judge_id),
        rating: Set(serde_json::json!(5.0)),
        point_delta: Set(0),
        feedback: Set(String::new()),
        model: Set("test-model".to_string()),
        provider: Set("test".to_string()),
        raw_output: Set(String::new()),
        duration_ms: Set(None),
        run_log: Set(None),
        tokens_input: Set(None),
        tokens_output: Set(None),
        tokens_cache_read: Set(None),
        tokens_cache_write: Set(None),
        status: Set(status.to_string()),
        error: Set(None),
        created_at: Set(Utc::now()),
        updated_at: Set(Utc::now()),
        verdict_kind: Set(None),
    }
    .insert(db)
    .await
    .expect("insert judge result");
}

async fn status_of(
    db: &DatabaseConnection,
    session_id: Uuid,
    player_id: Uuid,
) -> PlayerCompletionStatus {
    session_player_statuses(db, session_id)
        .await
        .expect("player statuses")
        .into_iter()
        .find(|s| s.player_id == player_id)
        .expect("player present in statuses")
        .status
}

#[tokio::test]
async fn player_with_tasks_remaining_is_in_progress_regardless_of_judges() {
    let db = setup_db().await;
    let user_id = insert_user(&db).await;
    let project_id = insert_project(&db, user_id).await;
    let session_id = insert_session(&db, project_id).await;
    let task_id = insert_task(&db, project_id, 0).await;
    attach_judge(&db, task_id, 0).await;
    let player_id = insert_player(&db, session_id).await;
    insert_scheduler_row(&db, session_id, player_id, "idle").await;

    assert_eq!(
        status_of(&db, session_id, player_id).await,
        PlayerCompletionStatus::InProgress
    );
}

#[tokio::test]
async fn done_player_with_no_judge_result_row_awaits_judges() {
    let db = setup_db().await;
    let user_id = insert_user(&db).await;
    let project_id = insert_project(&db, user_id).await;
    let session_id = insert_session(&db, project_id).await;
    let task_id = insert_task(&db, project_id, 0).await;
    attach_judge(&db, task_id, 0).await;
    let player_id = insert_player(&db, session_id).await;
    insert_scheduler_row(&db, session_id, player_id, SCHEDULER_STATE_COMPLETED).await;

    assert_eq!(
        status_of(&db, session_id, player_id).await,
        PlayerCompletionStatus::AwaitingJudges,
        "an expected judge with no result row yet (enqueue-wait window) must hold the player in awaiting-judges"
    );
}

#[tokio::test]
async fn done_player_with_running_judge_awaits_judges() {
    let db = setup_db().await;
    let user_id = insert_user(&db).await;
    let project_id = insert_project(&db, user_id).await;
    let session_id = insert_session(&db, project_id).await;
    let task_id = insert_task(&db, project_id, 0).await;
    let task_judge_id = attach_judge(&db, task_id, 0).await;
    let player_id = insert_player(&db, session_id).await;
    insert_scheduler_row(&db, session_id, player_id, SCHEDULER_STATE_COMPLETED).await;
    insert_judge_result(&db, session_id, player_id, task_judge_id, "running").await;

    assert_eq!(
        status_of(&db, session_id, player_id).await,
        PlayerCompletionStatus::AwaitingJudges
    );
}

#[tokio::test]
async fn done_player_with_all_judges_scored_is_completed() {
    let db = setup_db().await;
    let user_id = insert_user(&db).await;
    let project_id = insert_project(&db, user_id).await;
    let session_id = insert_session(&db, project_id).await;
    let task_a = insert_task(&db, project_id, 0).await;
    let task_b = insert_task(&db, project_id, 1).await;
    let tj_a = attach_judge(&db, task_a, 0).await;
    let tj_b = attach_judge(&db, task_b, 0).await;
    let player_id = insert_player(&db, session_id).await;
    insert_scheduler_row(&db, session_id, player_id, SCHEDULER_STATE_COMPLETED).await;
    insert_judge_result(&db, session_id, player_id, tj_a, "scored").await;
    insert_judge_result(&db, session_id, player_id, tj_b, "scored").await;

    assert_eq!(
        status_of(&db, session_id, player_id).await,
        PlayerCompletionStatus::Completed
    );
}

#[tokio::test]
async fn failed_judge_counts_as_terminal_and_never_blocks_completion() {
    let db = setup_db().await;
    let user_id = insert_user(&db).await;
    let project_id = insert_project(&db, user_id).await;
    let session_id = insert_session(&db, project_id).await;
    let task_id = insert_task(&db, project_id, 0).await;
    let tj_scored = attach_judge(&db, task_id, 0).await;
    let tj_failed = attach_judge(&db, task_id, 1).await;
    let player_id = insert_player(&db, session_id).await;
    insert_scheduler_row(&db, session_id, player_id, SCHEDULER_STATE_COMPLETED).await;
    insert_judge_result(&db, session_id, player_id, tj_scored, "scored").await;
    insert_judge_result(&db, session_id, player_id, tj_failed, "failed").await;

    assert_eq!(
        status_of(&db, session_id, player_id).await,
        PlayerCompletionStatus::Completed,
        "a failed judge run is terminal and must never block completion"
    );
}

#[tokio::test]
async fn done_player_with_no_attached_judges_is_immediately_completed() {
    let db = setup_db().await;
    let user_id = insert_user(&db).await;
    let project_id = insert_project(&db, user_id).await;
    let session_id = insert_session(&db, project_id).await;
    insert_task(&db, project_id, 0).await;
    let player_id = insert_player(&db, session_id).await;
    insert_scheduler_row(&db, session_id, player_id, SCHEDULER_STATE_COMPLETED).await;

    assert_eq!(
        status_of(&db, session_id, player_id).await,
        PlayerCompletionStatus::Completed
    );
}

#[tokio::test]
async fn revoked_player_is_absent_from_statuses() {
    let db = setup_db().await;
    let user_id = insert_user(&db).await;
    let project_id = insert_project(&db, user_id).await;
    let session_id = insert_session(&db, project_id).await;
    let task_id = insert_task(&db, project_id, 0).await;
    attach_judge(&db, task_id, 0).await;
    let active = insert_player(&db, session_id).await;
    let revoked = insert_player(&db, session_id).await;
    insert_scheduler_row(&db, session_id, active, SCHEDULER_STATE_COMPLETED).await;
    revoke_player(&db, revoked).await;

    let statuses = session_player_statuses(&db, session_id)
        .await
        .expect("player statuses");
    assert_eq!(statuses.len(), 1, "revoked players are not eligible");
    assert_eq!(statuses[0].player_id, active);
}

// ---------------------------------------------------------------------------
// Time-expiry accounting: interrupted tasks + pending judge runs before AP.
// ---------------------------------------------------------------------------

async fn insert_scheduler_row_at_task(
    db: &DatabaseConnection,
    session_id: Uuid,
    player_id: Uuid,
    state: &str,
    task_id: Option<Uuid>,
) {
    session_scheduler_state::ActiveModel {
        id: Set(Uuid::new_v4()),
        session_id_fk: Set(session_id),
        player_id_fk: Set(player_id),
        task_id: Set(task_id),
        state: Set(state.to_string()),
        next_probe_at: Set(None),
        created_at: Set(Utc::now()),
        updated_at: Set(Utc::now()),
    }
    .insert(db)
    .await
    .expect("insert scheduler row");
}

#[tokio::test]
async fn interrupted_tasks_reports_mid_task_players_only() {
    let db = setup_db().await;
    let user_id = insert_user(&db).await;
    let project_id = insert_project(&db, user_id).await;
    let session_id = insert_session(&db, project_id).await;
    let t0 = insert_task(&db, project_id, 0).await;
    let t1 = insert_task(&db, project_id, 1).await;

    let mid_task = insert_player(&db, session_id).await;
    let done = insert_player(&db, session_id).await;
    let never_started = insert_player(&db, session_id).await;
    let revoked = insert_player(&db, session_id).await;
    insert_scheduler_row_at_task(&db, session_id, mid_task, "waiting", Some(t1)).await;
    insert_scheduler_row_at_task(&db, session_id, done, SCHEDULER_STATE_COMPLETED, None).await;
    insert_scheduler_row_at_task(&db, session_id, revoked, "waiting", Some(t0)).await;
    revoke_player(&db, revoked).await;
    let _ = never_started;

    let interrupted = arena_core::session_completion::interrupted_tasks(&db, session_id)
        .await
        .expect("interrupted tasks");
    assert_eq!(
        interrupted.len(),
        1,
        "only the eligible mid-task player counts"
    );
    assert_eq!(interrupted[0].player_id, mid_task);
    assert_eq!(interrupted[0].task_id, t1);
}

#[tokio::test]
async fn expired_pending_counts_judges_up_to_and_including_current_task() {
    let db = setup_db().await;
    let user_id = insert_user(&db).await;
    let project_id = insert_project(&db, user_id).await;
    let session_id = insert_session(&db, project_id).await;
    let t0 = insert_task(&db, project_id, 0).await;
    let t1 = insert_task(&db, project_id, 1).await;
    let t2 = insert_task(&db, project_id, 2).await;
    let tj0 = attach_judge(&db, t0, 0).await;
    let tj1 = attach_judge(&db, t1, 0).await;
    let tj2 = attach_judge(&db, t2, 0).await;

    // Player interrupted on t1: judges of t0 (completed) and t1 (current)
    // are expected; t2's judge is not.
    let player = insert_player(&db, session_id).await;
    insert_scheduler_row_at_task(&db, session_id, player, "waiting", Some(t1)).await;

    let pending = arena_core::session_completion::expired_session_pending_judges(&db, session_id)
        .await
        .expect("pending");
    assert_eq!(pending, 2, "t0 + t1 judges pending, t2 not expected");

    // t0's run scored, t1's run failed — both terminal; nothing pending.
    insert_judge_result(&db, session_id, player, tj0, "scored").await;
    insert_judge_result(&db, session_id, player, tj1, "failed").await;
    let pending = arena_core::session_completion::expired_session_pending_judges(&db, session_id)
        .await
        .expect("pending");
    assert_eq!(pending, 0, "scored and failed both count as terminal");
    let _ = tj2;
}

#[tokio::test]
async fn expired_pending_counts_all_judges_for_completed_players() {
    let db = setup_db().await;
    let user_id = insert_user(&db).await;
    let project_id = insert_project(&db, user_id).await;
    let session_id = insert_session(&db, project_id).await;
    let t0 = insert_task(&db, project_id, 0).await;
    let tj0 = attach_judge(&db, t0, 0).await;

    let done = insert_player(&db, session_id).await;
    insert_scheduler_row_at_task(&db, session_id, done, SCHEDULER_STATE_COMPLETED, None).await;
    let never_started = insert_player(&db, session_id).await;
    let _ = never_started;

    let pending = arena_core::session_completion::expired_session_pending_judges(&db, session_id)
        .await
        .expect("pending");
    assert_eq!(
        pending, 1,
        "completed player expects every project judge; never-started player expects none"
    );

    insert_judge_result(&db, session_id, done, tj0, "running").await;
    let pending = arena_core::session_completion::expired_session_pending_judges(&db, session_id)
        .await
        .expect("pending");
    assert_eq!(pending, 1, "a running row is not terminal");

    // The run lifecycle upserts one row per (player, judge): flip it to
    // scored rather than inserting a second row (UNIQUE constraint).
    set_judge_result_status(&db, done, tj0, "scored").await;
    let pending = arena_core::session_completion::expired_session_pending_judges(&db, session_id)
        .await
        .expect("pending");
    assert_eq!(pending, 0);
}

async fn set_judge_result_status(
    db: &DatabaseConnection,
    player_id: Uuid,
    task_judge_id: Uuid,
    status: &str,
) {
    use sea_orm::{ColumnTrait, QueryFilter};
    let row = judge_results::Entity::find()
        .filter(judge_results::Column::PlayerIdFk.eq(player_id))
        .filter(judge_results::Column::TaskJudgeId.eq(task_judge_id))
        .one(db)
        .await
        .expect("query judge result")
        .expect("judge result exists");
    let mut am: judge_results::ActiveModel = row.into();
    am.status = Set(status.to_string());
    am.update(db).await.expect("update judge result");
}

#[tokio::test]
async fn expired_pending_is_zero_without_judges_or_players() {
    let db = setup_db().await;
    let user_id = insert_user(&db).await;
    let project_id = insert_project(&db, user_id).await;
    let session_id = insert_session(&db, project_id).await;
    insert_task(&db, project_id, 0).await;

    // No players at all.
    assert_eq!(
        arena_core::session_completion::expired_session_pending_judges(&db, session_id)
            .await
            .expect("pending"),
        0
    );

    // A player but no judges attached anywhere.
    let player = insert_player(&db, session_id).await;
    insert_scheduler_row_at_task(&db, session_id, player, SCHEDULER_STATE_COMPLETED, None).await;
    assert_eq!(
        arena_core::session_completion::expired_session_pending_judges(&db, session_id)
            .await
            .expect("pending"),
        0
    );
}

// The settle poll waits for a terminal judge_results row per (player,
// task_judge) over the tasks a player reached; a session-scoped judge must
// write rows for exactly that set. Both now read `reached_task_ids`, and this
// pins them together: if the two ever diverge, Arena Points stall until the
// settle deadline fires instead of awarding on time.
#[tokio::test]
async fn reached_tasks_matches_what_the_settle_poll_waits_for() {
    let db = setup_db().await;
    let user_id = insert_user(&db).await;
    let project_id = insert_project(&db, user_id).await;
    let session_id = insert_session(&db, project_id).await;
    let t0 = insert_task(&db, project_id, 0).await;
    let t1 = insert_task(&db, project_id, 1).await;
    let _t2 = insert_task(&db, project_id, 2).await;
    attach_judge(&db, t0, 0).await;
    attach_judge(&db, t1, 0).await;
    attach_judge(&db, _t2, 0).await;

    // Interrupted on t1 → reached t0 and t1, not t2.
    let player = insert_player(&db, session_id).await;
    insert_scheduler_row_at_task(&db, session_id, player, "waiting", Some(t1)).await;

    let reached = arena_core::session_completion::reached_tasks_for_player(&db, session_id, player)
        .await
        .expect("reached");
    let pending = arena_core::session_completion::expired_session_pending_judges(&db, session_id)
        .await
        .expect("pending");
    assert_eq!(reached.len(), 2, "t0 + t1 reached, t2 not: {reached:?}");
    assert!(reached.contains(&t0) && reached.contains(&t1));
    assert_eq!(
        pending,
        reached.len(),
        "one judge per task: the settle poll waits for exactly the reached set"
    );

    // A completed player reaches every task.
    let done = insert_player(&db, session_id).await;
    insert_scheduler_row_at_task(&db, session_id, done, SCHEDULER_STATE_COMPLETED, None).await;
    let reached_done =
        arena_core::session_completion::reached_tasks_for_player(&db, session_id, done)
            .await
            .expect("reached");
    assert_eq!(reached_done.len(), 3, "completed → all tasks");

    // A player with no scheduler row reaches nothing.
    let never = insert_player(&db, session_id).await;
    let reached_never =
        arena_core::session_completion::reached_tasks_for_player(&db, session_id, never)
            .await
            .expect("reached");
    assert!(
        reached_never.is_empty(),
        "no scheduler row → no tasks judged"
    );
}

// The report judge is anchored on the project's first task precisely because
// that task is in every non-empty reached set. This pins the property that
// choice rests on: a player who never got past task 0 — failed it, or was
// still on it when time ran out — has still *reached* it, so the report's
// (player, task_judge) pair exists and the debrief is written. Anchored on the
// last task, the same player got nothing (session IL3T44).
#[tokio::test]
async fn a_player_stuck_on_the_first_task_has_still_reached_it() {
    let db = setup_db().await;
    let user_id = insert_user(&db).await;
    let project_id = insert_project(&db, user_id).await;
    let session_id = insert_session(&db, project_id).await;
    let t0 = insert_task(&db, project_id, 0).await;
    let _t1 = insert_task(&db, project_id, 1).await;

    let stuck = insert_player(&db, session_id).await;
    insert_scheduler_row_at_task(&db, session_id, stuck, "waiting", Some(t0)).await;

    let reached = arena_core::session_completion::reached_tasks_for_player(&db, session_id, stuck)
        .await
        .expect("reached");
    assert_eq!(
        reached,
        vec![t0],
        "the task they were on is reached, done or not"
    );

    // The one case that genuinely has nothing to report on: a row exists but no
    // task was ever assigned, so the session never started for this player.
    let idle = insert_player(&db, session_id).await;
    insert_scheduler_row_at_task(&db, session_id, idle, "waiting", None).await;
    let reached_idle =
        arena_core::session_completion::reached_tasks_for_player(&db, session_id, idle)
            .await
            .expect("reached");
    assert!(
        reached_idle.is_empty(),
        "no task assigned → nothing was played, and nothing is judged"
    );
}

// ---------------------------------------------------------------------------
// Tasks seeded into the project AFTER the session finished (apply-seed on a
// live deployment) never ran in that session — they must not appear in the
// reached set, hold completion in awaiting-judges, or count as pending judge
// runs. Session WQ2WOQ froze on "Judges scoring — 7 left" exactly this way.
// ---------------------------------------------------------------------------

async fn finish_session_at(
    db: &DatabaseConnection,
    session_id: Uuid,
    finished_at: chrono::DateTime<Utc>,
) {
    use arena_core::entities::sessions;
    use arena_core::session_status::SessionStatus;
    let row = sessions::Entity::find_by_id(session_id)
        .one(db)
        .await
        .expect("query session")
        .expect("session exists");
    let mut am: sessions::ActiveModel = row.into();
    am.status = Set(SessionStatus::Finished);
    am.finished_at = Set(Some(finished_at));
    am.update(db).await.expect("finish session");
}

async fn insert_task_at(
    db: &DatabaseConnection,
    project_id: Uuid,
    ordinal: i32,
    created_at: chrono::DateTime<Utc>,
) -> Uuid {
    let id = insert_task(db, project_id, ordinal).await;
    let row = tasks::Entity::find_by_id(id)
        .one(db)
        .await
        .expect("query task")
        .expect("task exists");
    let mut am: tasks::ActiveModel = row.into();
    am.created_at = Set(created_at);
    am.update(db).await.expect("backdate task");
    id
}

#[tokio::test]
async fn task_seeded_after_finish_is_not_expected_of_a_finished_session() {
    let db = setup_db().await;
    let user_id = insert_user(&db).await;
    let project_id = insert_project(&db, user_id).await;
    let session_id = insert_session(&db, project_id).await;
    let finished_at = Utc::now() - chrono::Duration::hours(1);

    // The session ran with one judged task; the player completed it and the
    // judge scored — a fully settled session.
    let task_a = insert_task_at(&db, project_id, 0, finished_at - chrono::Duration::hours(1)).await;
    let tj_a = attach_judge(&db, task_a, 0).await;
    let player_id = insert_player(&db, session_id).await;
    insert_scheduler_row(&db, session_id, player_id, SCHEDULER_STATE_COMPLETED).await;
    insert_judge_result(&db, session_id, player_id, tj_a, "scored").await;
    finish_session_at(&db, session_id, finished_at).await;

    assert_eq!(
        status_of(&db, session_id, player_id).await,
        PlayerCompletionStatus::Completed
    );

    // A later apply-seed adds a second task with its own judge.
    let task_b = insert_task_at(&db, project_id, 1, Utc::now()).await;
    attach_judge(&db, task_b, 0).await;

    assert_eq!(
        status_of(&db, session_id, player_id).await,
        PlayerCompletionStatus::Completed,
        "a task seeded after finish must not drag a settled session back to awaiting-judges"
    );
    let reached =
        arena_core::session_completion::reached_tasks_for_player(&db, session_id, player_id)
            .await
            .expect("reached");
    assert_eq!(
        reached,
        vec![task_a],
        "reached tasks of a finished session are the tasks it ran with"
    );
    let pending = arena_core::session_completion::expired_session_pending_judges(&db, session_id)
        .await
        .expect("pending");
    assert_eq!(
        pending, 0,
        "no judge run is expected for a task the session never had"
    );
}

#[tokio::test]
async fn task_added_while_the_session_runs_still_counts() {
    let db = setup_db().await;
    let user_id = insert_user(&db).await;
    let project_id = insert_project(&db, user_id).await;
    let session_id = insert_session(&db, project_id).await;

    let task_a = insert_task(&db, project_id, 0).await;
    attach_judge(&db, task_a, 0).await;
    let player_id = insert_player(&db, session_id).await;
    insert_scheduler_row(&db, session_id, player_id, SCHEDULER_STATE_COMPLETED).await;

    // No finished_at: the session is live, the full current task list applies.
    let task_b = insert_task(&db, project_id, 1).await;
    attach_judge(&db, task_b, 0).await;

    let reached =
        arena_core::session_completion::reached_tasks_for_player(&db, session_id, player_id)
            .await
            .expect("reached");
    assert_eq!(
        reached.len(),
        2,
        "a live session sees every current task: {reached:?}"
    );
}
