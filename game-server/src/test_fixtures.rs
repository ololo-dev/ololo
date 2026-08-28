//! Shared fixtures for the crate's `#[cfg(test)]` unit tests: an in-memory
//! migrated DB, a minimal `GameServerState`, and the FK chain
//! (user → project → session → player → task → test → probe) that
//! sqlite's enforced foreign keys demand before any scoring row can land.

use crate::state::GameServerState;
use arena_core::entities::{players, probes, projects, sessions, tasks, tests, users};
use chrono::Utc;
use migration::MigratorTrait;
use sea_orm::{ActiveModelTrait, DatabaseConnection, Set};
use std::sync::Arc;
use uuid::Uuid;

pub(crate) async fn mem_db() -> DatabaseConnection {
    let db = sea_orm::Database::connect("sqlite::memory:")
        .await
        .expect("connect");
    migration::Migrator::up(&db, None).await.expect("migrate");
    db
}

pub(crate) fn test_state(db: DatabaseConnection) -> GameServerState {
    let secret = b"test-secret-32-bytes-or-more-xxxxxxx".to_vec();
    GameServerState {
        db,
        server_id: Uuid::new_v4(),
        advertise_url: "ws://localhost:8081".to_string(),
        jwt_encoding_key: Arc::new(jsonwebtoken::EncodingKey::from_secret(&secret)),
        jwt_decoding_key: Arc::new(jsonwebtoken::DecodingKey::from_secret(&secret)),
        jwt_signing_secret: Arc::new(secret),
        session_registry: Arc::new(dashmap::DashMap::new()),
        player_agent_registry: Arc::new(dashmap::DashMap::new()),
        lobby_timer_secs: 60,
        event_publisher: Arc::new(crate::zmq_pub::NoopEventPublisher),
        judge_semaphore: Arc::new(tokio::sync::Semaphore::new(3)),
        settings_encryption: Arc::new(arena_core::settings_encryption::SettingsEncryption::new(
            b"test-secret-key-for-settings-enc",
        )),
    }
}

pub(crate) struct SessionFixture {
    pub project_id: Uuid,
    pub session_id: Uuid,
    pub player_id: Uuid,
}

/// A running session with one player, FK chain and all.
pub(crate) async fn session_with_player(db: &DatabaseConnection) -> SessionFixture {
    let now = Utc::now();
    let owner = users::ActiveModel {
        id: Set(Uuid::new_v4()),
        email: Set(format!("u{}@example.com", Uuid::new_v4())),
        password_hash: Set(None),
        display_name: Set("tester".to_string()),
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
    .insert(db)
    .await
    .expect("insert user")
    .id;
    let project_id = projects::ActiveModel {
        id: Set(Uuid::new_v4()),
        name: Set("proj".to_string()),
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
    .insert(db)
    .await
    .expect("insert project")
    .id;
    let session_id = sessions::ActiveModel {
        id: Set(Uuid::new_v4()),
        name: Set("s".to_string()),
        created_at: Set(now),
        owner_id_fk: Set(None),
        status: Set(arena_core::session_status::SessionStatus::Running),
        join_code: Set(format!("T{}", &Uuid::new_v4().simple().to_string()[..5]).to_uppercase()),
        started_at: Set(Some(now)),
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
    .expect("insert session")
    .id;
    let player_id = players::ActiveModel {
        id: Set(Uuid::new_v4()),
        session_id_fk: Set(session_id),
        user_id_fk: Set(None),
        display_name: Set("bob".to_string()),
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
    .expect("insert player")
    .id;
    SessionFixture {
        project_id,
        session_id,
        player_id,
    }
}

/// One task with one adapted test — the parents a probe row needs.
pub(crate) async fn task_with_test(db: &DatabaseConnection, fx: &SessionFixture) -> (Uuid, Uuid) {
    let task_id = tasks::ActiveModel {
        id: Set(Uuid::new_v4()),
        project_id_fk: Set(fx.project_id),
        ordinal: Set(0),
        title: Set("t".to_string()),
        content: Set("c".to_string()),
        test_template: Set(serde_json::json!({"kind": "shell", "command_template": "echo hi"})),
        created_at: Set(Utc::now()),
        tags: Set(String::new()),
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
    .expect("insert task")
    .id;
    let test_id = tests::ActiveModel {
        id: Set(Uuid::new_v4()),
        command_template: Set("echo hi".to_string()),
        answer_template: Set(String::new()),
        fixture_definitions: Set(String::new()),
        created_at: Set(Utc::now()),
        session_id: Set(fx.session_id),
        task_id: Set(task_id),
        ordinal: Set(0),
        prompt: Set(String::new()),
        description: Set(None),
        probe_config: Set(None),
        initiator: Set("system".to_string()),
        registered_by_judge_id: Set(None),
    }
    .insert(db)
    .await
    .expect("insert test")
    .id;
    (task_id, test_id)
}

/// A dispatched probe, ungraded unless `outcome` says otherwise.
pub(crate) async fn insert_probe(
    db: &DatabaseConnection,
    fx: &SessionFixture,
    test_id: Uuid,
    outcome: Option<&str>,
) -> Uuid {
    let id = Uuid::new_v4();
    probes::ActiveModel {
        id: Set(id),
        test_id: Set(test_id),
        player_id: Set(fx.player_id),
        session_id: Set(fx.session_id),
        attempt: Set(1),
        rendered_command: Set("echo hi".to_string()),
        fixture_values: Set("{}".to_string()),
        expected_answer: Set(None),
        resolved_answer: Set(None),
        secret_meta: Set(None),
        outcome: Set(outcome.map(str::to_string)),
        dispatched_at: Set(Utc::now()),
        deadline_at: Set(Utc::now()),
        resolved_at: Set(None),
        updated_at: Set(None),
        output: Set(None),
        exit_code: Set(None),
        duration_ms: Set(None),
        point_delta: Set(None),
        result_json: Set(None),
        artifact_path: Set(None),
    }
    .insert(db)
    .await
    .expect("insert probe");
    id
}

/// Attach a judge to a task. `scope` is `"task"` or `"session"` — the
/// judge-phase gate treats session-scoped judges as never holding a task.
pub(crate) async fn attach_judge(
    db: &DatabaseConnection,
    task_id: Uuid,
    slug: &str,
    scope: &str,
) -> (Uuid, Uuid) {
    use arena_core::entities::{judges, task_judges};
    let now = Utc::now();
    let judge_id = judges::ActiveModel {
        id: Set(Uuid::new_v4()),
        slug: Set(slug.to_string()),
        name: Set(slug.to_string()),
        description: Set(String::new()),
        prompt: Set("judge".to_string()),
        rating_scale: Set(serde_json::json!({"min": 0.0, "max": 10.0, "step": 0.5})),
        kind: Set("llm".to_string()),
        scope: Set(scope.to_string()),
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
    .insert(db)
    .await
    .expect("insert judge")
    .id;
    let task_judge_id = task_judges::ActiveModel {
        id: Set(Uuid::new_v4()),
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
    .expect("insert task_judge")
    .id;
    (judge_id, task_judge_id)
}

/// A judge run row in a given lifecycle `status` (`pending`, `running`,
/// `scored`, `failed`, `waiting`).
pub(crate) async fn insert_judge_result(
    db: &DatabaseConnection,
    fx: &SessionFixture,
    task_judge_id: Uuid,
    status: &str,
) {
    use arena_core::entities::judge_results;
    let now = Utc::now();
    judge_results::ActiveModel {
        id: Set(Uuid::new_v4()),
        session_id_fk: Set(fx.session_id),
        player_id_fk: Set(fx.player_id),
        task_judge_id: Set(task_judge_id),
        rating: Set(serde_json::json!(0.0)),
        point_delta: Set(0),
        feedback: Set(String::new()),
        model: Set("m".to_string()),
        provider: Set("p".to_string()),
        raw_output: Set(String::new()),
        duration_ms: Set(None),
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
    .expect("insert judge_result");
}

/// A judge-registered extra check (`registered_by_judge_id` set), created
/// `age_secs` ago so tests can drive the judge-phase cap.
pub(crate) async fn insert_judge_test(
    db: &DatabaseConnection,
    fx: &SessionFixture,
    task_id: Uuid,
    judge_id: Uuid,
    age_secs: i64,
) -> Uuid {
    use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
    // (session, task, ordinal) is unique — take the next free slot.
    let ordinal = tests::Entity::find()
        .filter(tests::Column::SessionId.eq(fx.session_id))
        .filter(tests::Column::TaskId.eq(task_id))
        .all(db)
        .await
        .expect("count tests")
        .len() as i32;
    tests::ActiveModel {
        id: Set(Uuid::new_v4()),
        command_template: Set("node --test".to_string()),
        answer_template: Set(String::new()),
        fixture_definitions: Set(String::new()),
        created_at: Set(Utc::now() - chrono::Duration::seconds(age_secs)),
        session_id: Set(fx.session_id),
        task_id: Set(task_id),
        ordinal: Set(ordinal),
        prompt: Set(String::new()),
        description: Set(None),
        probe_config: Set(None),
        initiator: Set("judge".to_string()),
        registered_by_judge_id: Set(Some(judge_id)),
    }
    .insert(db)
    .await
    .expect("insert judge test")
    .id
}

/// The player's scheduler row, parked on `task_id` in `state`.
pub(crate) async fn insert_scheduler_state(
    db: &DatabaseConnection,
    fx: &SessionFixture,
    task_id: Option<Uuid>,
    state: &str,
) {
    use arena_core::entities::session_scheduler_state;
    let now = Utc::now();
    session_scheduler_state::ActiveModel {
        id: Set(Uuid::new_v4()),
        session_id_fk: Set(fx.session_id),
        player_id_fk: Set(fx.player_id),
        task_id: Set(task_id),
        state: Set(state.to_string()),
        next_probe_at: Set(None),
        created_at: Set(now),
        updated_at: Set(now),
    }
    .insert(db)
    .await
    .expect("insert scheduler state");
}

/// An extra task on the same project, at `ordinal`.
pub(crate) async fn extra_task(db: &DatabaseConnection, fx: &SessionFixture, ordinal: i32) -> Uuid {
    tasks::ActiveModel {
        id: Set(Uuid::new_v4()),
        project_id_fk: Set(fx.project_id),
        ordinal: Set(ordinal),
        title: Set(format!("task-{ordinal}")),
        content: Set("c".to_string()),
        test_template: Set(serde_json::json!({"kind": "shell", "command_template": "echo hi"})),
        created_at: Set(Utc::now()),
        tags: Set(String::new()),
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
    .expect("insert task")
    .id
}
