//! Coverage for the `recovery::resume_on_startup` orchestration path.

use std::sync::Arc;

use arena_core::entities::{game_servers, projects, sessions, users};
use arena_core::session_status::SessionStatus;
use chrono::Utc;
use dashmap::DashMap;
use game_server::recovery::resume_on_startup;
use game_server::state::GameServerState;
use game_server::ws::session_lifecycle::running_timer;
use game_server::zmq_pub::NoopEventPublisher;
use jsonwebtoken::{DecodingKey, EncodingKey};
use migration::MigratorTrait;
use sea_orm::{ActiveModelTrait, DatabaseConnection, EntityTrait, Set};
use tokio::sync::Semaphore;
use uuid::Uuid;

async fn setup_db() -> DatabaseConnection {
    let db = sea_orm::Database::connect("sqlite::memory:")
        .await
        .expect("connect");
    migration::Migrator::up(&db, None).await.expect("migrate");
    db
}

async fn insert_project(db: &DatabaseConnection) -> Uuid {
    insert_project_with_duration(db, 3600).await
}

async fn insert_project_with_duration(db: &DatabaseConnection, duration_secs: i64) -> Uuid {
    let owner = users::ActiveModel {
        id: Set(Uuid::new_v4()),
        email: Set(format!("u{}@example.com", Uuid::new_v4())),
        password_hash: Set(None),
        display_name: Set("tester".to_string()),
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
    .expect("insert user")
    .id;
    projects::ActiveModel {
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
        created_at: Set(Utc::now()),
        updated_at: Set(Utc::now()),
        default_value_points: Set(10),
        default_fail_points: Set(-5),
        default_no_response_points: Set(-10),
        default_completion_bonus_points: Set(10),
        default_deadline_secs: Set(60),
        default_session_duration_secs: Set(duration_secs),
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
    .id
}

async fn insert_game_server(db: &DatabaseConnection, server_id: Uuid) {
    game_servers::ActiveModel {
        id: Set(server_id),
        url: Set("ws://localhost:8081".to_string()),
        zmq_url: Set(None),
        display_name: Set(None),
        capacity: Set(64),
        active_sessions: Set(0),
        status: Set("active".to_string()),
        last_heartbeat: Set(Utc::now()),
        created_at: Set(Utc::now()),
        updated_at: Set(Utc::now()),
    }
    .insert(db)
    .await
    .expect("insert game server");
}

async fn insert_running_session(
    db: &DatabaseConnection,
    server_id: Uuid,
    project_id: Uuid,
) -> Uuid {
    // Started an hour ago → remaining already elapsed → FinishNow.
    insert_running_session_started(
        db,
        server_id,
        project_id,
        Utc::now() - chrono::Duration::hours(1),
    )
    .await
}

async fn insert_running_session_started(
    db: &DatabaseConnection,
    server_id: Uuid,
    project_id: Uuid,
    started_at: chrono::DateTime<Utc>,
) -> Uuid {
    let id = Uuid::new_v4();
    sessions::ActiveModel {
        id: Set(id),
        name: Set("s".to_string()),
        created_at: Set(Utc::now()),
        owner_id_fk: Set(None),
        status: Set(SessionStatus::Running),
        join_code: Set("JC1".to_string()),
        started_at: Set(Some(started_at)),
        finished_at: Set(None),
        paused_at: Set(None),
        paused_duration_secs: Set(None),
        project_id_fk: Set(project_id),
        game_server_id: Set(Some(server_id)),
        cancel_reason: Set(None),
        cancelled_by: Set(None),
    }
    .insert(db)
    .await
    .expect("insert session");
    id
}

fn test_state(db: DatabaseConnection, server_id: Uuid) -> GameServerState {
    let secret = b"test-secret-32-bytes-or-more-xxxxxxx".to_vec();
    GameServerState {
        db: db.clone(),
        server_id,
        advertise_url: "ws://localhost:8081".to_string(),
        jwt_encoding_key: Arc::new(EncodingKey::from_secret(&secret)),
        jwt_decoding_key: Arc::new(DecodingKey::from_secret(&secret)),
        jwt_signing_secret: Arc::new(secret),
        session_registry: Arc::new(DashMap::new()),
        player_agent_registry: Arc::new(DashMap::new()),
        lobby_timer_secs: 60,
        event_publisher: Arc::new(NoopEventPublisher),
        judge_semaphore: Arc::new(Semaphore::new(3)),
        settings_encryption: std::sync::Arc::new(
            arena_core::settings_encryption::SettingsEncryption::new(
                b"test-secret-key-for-settings-enc",
            ),
        ),
    }
}

#[tokio::test]
async fn resume_finishes_expired_running_session() {
    let db = setup_db().await;
    let server_id = Uuid::new_v4();
    let project_id = insert_project(&db).await;
    insert_game_server(&db, server_id).await;
    let session_id = insert_running_session(&db, server_id, project_id).await;

    let state = test_state(db.clone(), server_id);
    resume_on_startup(state).await.expect("resume");

    let row = sessions::Entity::find_by_id(session_id)
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.status, SessionStatus::Finished);
    assert!(row.finished_at.is_some());
}

#[tokio::test]
async fn resume_uses_project_session_duration() {
    // Project configures a 120s session; the session started 600s ago, so its
    // remaining is already elapsed and recovery must finish it. If recovery
    // used a process-wide duration (e.g. the old 3600s env default) instead of
    // the project row, remaining would still be positive and the session would
    // stay Running.
    let db = setup_db().await;
    let server_id = Uuid::new_v4();
    let project_id = insert_project_with_duration(&db, 120).await;
    insert_game_server(&db, server_id).await;
    let session_id = insert_running_session_started(
        &db,
        server_id,
        project_id,
        Utc::now() - chrono::Duration::seconds(600),
    )
    .await;

    let state = test_state(db.clone(), server_id);
    resume_on_startup(state).await.expect("resume");

    let row = sessions::Entity::find_by_id(session_id)
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        row.status,
        SessionStatus::Finished,
        "recovery must source duration from projects.default_session_duration_secs"
    );
}

#[tokio::test]
async fn running_timer_uses_project_duration() {
    // Project configures a 120s session; the session started an hour ago, so
    // the first tick computes remaining 0 from the project-sourced duration
    // and finishes the session (running_timer returns after completing).
    let db = setup_db().await;
    let server_id = Uuid::new_v4();
    let project_id = insert_project_with_duration(&db, 120).await;
    insert_game_server(&db, server_id).await;
    let session_id = insert_running_session(&db, server_id, project_id).await;

    let state = test_state(db.clone(), server_id);
    running_timer(state, session_id, String::new(), "JC1".to_string()).await;

    let row = sessions::Entity::find_by_id(session_id)
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.status, SessionStatus::Finished);
}

#[tokio::test]
async fn running_timer_rereads_project_duration_each_tick() {
    // The running countdown re-reads projects.default_session_duration_secs on
    // every tick: shrinking the duration mid-run must finish the session
    // within a tick or two, without restarting the timer.
    let db = setup_db().await;
    let server_id = Uuid::new_v4();
    let project_id = insert_project_with_duration(&db, 3600).await;
    insert_game_server(&db, server_id).await;
    let session_id = insert_running_session_started(&db, server_id, project_id, Utc::now()).await;

    let state = test_state(db.clone(), server_id);
    let handle = tokio::spawn(running_timer(
        state,
        session_id,
        String::new(),
        "JC1".to_string(),
    ));

    // Let the timer take its first tick with the original 3600s duration,
    // then shrink the project duration to 1s.
    tokio::time::sleep(std::time::Duration::from_millis(1300)).await;
    let project = projects::Entity::find_by_id(project_id)
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    let mut am: projects::ActiveModel = project.into();
    am.default_session_duration_secs = Set(1);
    am.update(&db).await.expect("update project duration");

    // Elapsed already exceeds the new 1s duration, so the next tick must
    // compute remaining 0 and finish the session.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        let row = sessions::Entity::find_by_id(session_id)
            .one(&db)
            .await
            .unwrap()
            .unwrap();
        if row.status == SessionStatus::Finished {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "session not finished within deadline after project duration shrank; status={}",
            row.status
        );
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    handle.await.expect("running_timer task");
}

#[tokio::test]
async fn resume_is_noop_without_matching_sessions() {
    let db = setup_db().await;
    let server_id = Uuid::new_v4();
    let state = test_state(db.clone(), server_id);
    // No sessions assigned to this server → returns Ok without error.
    resume_on_startup(state).await.expect("resume noop");
}

/// A judge run that was never recorded must be re-driven, and only once.
///
/// Judge runs live in memory only: the socket spawns a task that waits up to 90s
/// for the task's snapshot commit, and anything ending the process inside that
/// window drops it with nothing to restart it. Session VKIBCB reached exactly
/// that state — all 17 judges `pending`, the session scored and awarded — so
/// nothing ever evaluated the player's code.
#[tokio::test]
async fn missed_judge_runs_are_re_driven_exactly_once() {
    use arena_core::entities::{
        judge_results, judges, session_scheduler_state, task_judges, tasks,
    };
    use game_server::recovery::enqueue_missed_judge_runs;

    let db = setup_db().await;
    let server_id = Uuid::new_v4();
    insert_game_server(&db, server_id).await;
    let project = insert_project(&db).await;

    // A session this server owns that finished recently — the sweep's window.
    let session_id = Uuid::new_v4();
    sessions::ActiveModel {
        id: Set(session_id),
        name: Set("s".to_string()),
        created_at: Set(Utc::now()),
        owner_id_fk: Set(None),
        status: Set(SessionStatus::Finished),
        join_code: Set("JCSWEEP".to_string()),
        started_at: Set(Some(Utc::now() - chrono::Duration::hours(1))),
        finished_at: Set(Some(Utc::now() - chrono::Duration::minutes(5))),
        paused_at: Set(None),
        paused_duration_secs: Set(None),
        project_id_fk: Set(project),
        game_server_id: Set(Some(server_id)),
        cancel_reason: Set(None),
        cancelled_by: Set(None),
    }
    .insert(&db)
    .await
    .expect("insert finished session");

    let player_id = Uuid::new_v4();
    arena_core::entities::players::ActiveModel {
        id: Set(player_id),
        session_id_fk: Set(session_id),
        user_id_fk: Set(None),
        display_name: Set("p".to_string()),
        fingerprint: Set(None),
        metadata_json: Set(None),
        joined_at: Set(Utc::now()),
        reconnected_at: Set(None),
        revoked_at: Set(None),
        agent_connected: Set(false),
        agent_last_seen_at: Set(None),
    }
    .insert(&db)
    .await
    .expect("insert player");

    // Two tasks, a judge attached to each.
    let mut task_ids = Vec::new();
    for ordinal in 0..2 {
        let task_id = Uuid::new_v4();
        tasks::ActiveModel {
            id: Set(task_id),
            project_id_fk: Set(project),
            ordinal: Set(ordinal),
            title: Set(format!("Task {ordinal}")),
            content: Set("do it".to_string()),
            test_template: Set(serde_json::json!({"kind": "shell"})),
            // Before the session finished: tasks seeded later are not part
            // of the session (aec0573).
            created_at: Set(Utc::now() - chrono::Duration::hours(2)),
            tags: Set(String::new()),
            point_value: Set(10),
            deadline_secs: Set(Some(300)),
            min_interval_secs: Set(Some(5)),
            interval_increment_secs: Set(Some(0)),
            max_interval_secs: Set(Some(300)),
            fail_points: Set(0),
            no_response_points: Set(0),
            completion_bonus_points: Set(0),
            evaluation: Set(None),
        }
        .insert(&db)
        .await
        .expect("insert task");
        task_ids.push(task_id);
    }

    let judge_id = Uuid::new_v4();
    judges::ActiveModel {
        id: Set(judge_id),
        slug: Set("code-cleanliness".to_string()),
        name: Set("Cleanliness".to_string()),
        description: Set(String::new()),
        prompt: Set("judge".to_string()),
        rating_scale: Set(serde_json::json!({"min": 0.0, "max": 10.0, "step": 1.0})),
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
    .insert(&db)
    .await
    .expect("insert judge");
    for task_id in &task_ids {
        task_judges::ActiveModel {
            id: Set(Uuid::new_v4()),
            task_id: Set(*task_id),
            judge_id: Set(judge_id),
            ordinal: Set(0),
            rating_scale_override: Set(None),
            // Before the session finished: attachments created later are
            // (correctly) not expected of it.
            created_at: Set(Utc::now() - chrono::Duration::hours(1)),
            updated_at: Set(Utc::now()),
            weight: Set(None),
        }
        .insert(&db)
        .await
        .expect("attach judge");
    }

    // The player finished every task, so both pairs are expected.
    session_scheduler_state::ActiveModel {
        id: Set(Uuid::new_v4()),
        session_id_fk: Set(session_id),
        player_id_fk: Set(player_id),
        task_id: Set(None),
        state: Set(arena_core::session_completion::SCHEDULER_STATE_COMPLETED.to_string()),
        next_probe_at: Set(None),
        created_at: Set(Utc::now()),
        updated_at: Set(Utc::now()),
    }
    .insert(&db)
    .await
    .expect("insert scheduler row");

    assert_eq!(
        judge_results::Entity::find().all(&db).await.unwrap().len(),
        0,
        "no judge ever ran — the state VKIBCB was left in"
    );

    // No repo and no reachable LLM, so each recovered run fails — but failing is
    // terminal, which is what unblocks the settle poll.
    let state = test_state(db.clone(), server_id);
    enqueue_missed_judge_runs(state.clone())
        .await
        .expect("sweep");

    let after_first = judge_results::Entity::find().all(&db).await.unwrap();
    assert_eq!(
        after_first.len(),
        2,
        "both missed pairs got a row: {:?}",
        after_first.iter().map(|r| &r.status).collect::<Vec<_>>()
    );
    let updated_at: Vec<_> = after_first.iter().map(|r| r.updated_at).collect();

    // Second pass must be a no-op: terminal rows are never re-enqueued, or every
    // restart would burn LLM tokens re-judging settled sessions.
    enqueue_missed_judge_runs(state)
        .await
        .expect("second sweep");
    let after_second = judge_results::Entity::find().all(&db).await.unwrap();
    assert_eq!(after_second.len(), 2, "no duplicate rows");
    assert_eq!(
        after_second
            .iter()
            .map(|r| r.updated_at)
            .collect::<Vec<_>>(),
        updated_at,
        "already-terminal pairs must not be touched again"
    );
}

// The periodic sweep must not re-drive a judge for a session that only just
// finished — a run legitimately in flight has no terminal row yet and would be
// double-driven. The age floor gates on finished_at: too recent → skipped;
// older than the floor → a genuinely stuck judge is re-driven.
#[tokio::test]
async fn periodic_sweep_honors_the_age_floor() {
    use arena_core::entities::{
        judge_results, judges, session_scheduler_state, task_judges, tasks,
    };
    use game_server::recovery::enqueue_missed_judge_runs_older_than;

    let db = setup_db().await;
    let server_id = Uuid::new_v4();
    insert_game_server(&db, server_id).await;
    let project = insert_project(&db).await;

    let session_id = Uuid::new_v4();
    // Freshly finished — inside the floor.
    sessions::ActiveModel {
        id: Set(session_id),
        name: Set("s".to_string()),
        created_at: Set(Utc::now()),
        owner_id_fk: Set(None),
        status: Set(SessionStatus::Finished),
        join_code: Set("JCFLOOR".to_string()),
        started_at: Set(Some(Utc::now() - chrono::Duration::hours(1))),
        finished_at: Set(Some(Utc::now() - chrono::Duration::minutes(1))),
        paused_at: Set(None),
        paused_duration_secs: Set(None),
        project_id_fk: Set(project),
        game_server_id: Set(Some(server_id)),
        cancel_reason: Set(None),
        cancelled_by: Set(None),
    }
    .insert(&db)
    .await
    .expect("insert finished session");

    let player_id = Uuid::new_v4();
    arena_core::entities::players::ActiveModel {
        id: Set(player_id),
        session_id_fk: Set(session_id),
        user_id_fk: Set(None),
        display_name: Set("p".to_string()),
        fingerprint: Set(None),
        metadata_json: Set(None),
        joined_at: Set(Utc::now()),
        reconnected_at: Set(None),
        revoked_at: Set(None),
        agent_connected: Set(false),
        agent_last_seen_at: Set(None),
    }
    .insert(&db)
    .await
    .expect("insert player");

    let task_id = Uuid::new_v4();
    tasks::ActiveModel {
        id: Set(task_id),
        project_id_fk: Set(project),
        ordinal: Set(0),
        title: Set("Task 0".to_string()),
        content: Set("do it".to_string()),
        test_template: Set(serde_json::json!({"kind": "shell"})),
        // Before the session finished: tasks seeded later are not part of
        // the session (aec0573).
        created_at: Set(Utc::now() - chrono::Duration::hours(2)),
        tags: Set(String::new()),
        point_value: Set(10),
        deadline_secs: Set(Some(300)),
        min_interval_secs: Set(Some(5)),
        interval_increment_secs: Set(Some(0)),
        max_interval_secs: Set(Some(300)),
        fail_points: Set(0),
        no_response_points: Set(0),
        completion_bonus_points: Set(0),
        evaluation: Set(None),
    }
    .insert(&db)
    .await
    .expect("insert task");

    let judge_id = Uuid::new_v4();
    judges::ActiveModel {
        id: Set(judge_id),
        slug: Set("code-cleanliness".to_string()),
        name: Set("Cleanliness".to_string()),
        description: Set(String::new()),
        prompt: Set("judge".to_string()),
        rating_scale: Set(serde_json::json!({"min": 0.0, "max": 10.0, "step": 1.0})),
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
    .insert(&db)
    .await
    .expect("insert judge");
    task_judges::ActiveModel {
        id: Set(Uuid::new_v4()),
        task_id: Set(task_id),
        judge_id: Set(judge_id),
        ordinal: Set(0),
        rating_scale_override: Set(None),
        // Before the session finished: attachments created later are
        // (correctly) not expected of it.
        created_at: Set(Utc::now() - chrono::Duration::hours(1)),
        updated_at: Set(Utc::now()),
        weight: Set(None),
    }
    .insert(&db)
    .await
    .expect("attach judge");

    session_scheduler_state::ActiveModel {
        id: Set(Uuid::new_v4()),
        session_id_fk: Set(session_id),
        player_id_fk: Set(player_id),
        task_id: Set(None),
        state: Set(arena_core::session_completion::SCHEDULER_STATE_COMPLETED.to_string()),
        next_probe_at: Set(None),
        created_at: Set(Utc::now()),
        updated_at: Set(Utc::now()),
    }
    .insert(&db)
    .await
    .expect("insert scheduler row");

    let state = test_state(db.clone(), server_id);

    // Finished 1 min ago, floor is 5 min → skipped, no rows.
    enqueue_missed_judge_runs_older_than(state.clone(), std::time::Duration::from_secs(300))
        .await
        .expect("floored sweep");
    assert_eq!(
        judge_results::Entity::find().all(&db).await.unwrap().len(),
        0,
        "a session inside the age floor must not be re-driven (a run may be in flight)"
    );

    // Age it past the floor → the stuck pair is re-driven.
    let mut session: sessions::ActiveModel = sessions::Entity::find_by_id(session_id)
        .one(&db)
        .await
        .unwrap()
        .unwrap()
        .into();
    session.finished_at = Set(Some(Utc::now() - chrono::Duration::minutes(10)));
    session.update(&db).await.expect("age the session");
    enqueue_missed_judge_runs_older_than(state, std::time::Duration::from_secs(300))
        .await
        .expect("floored sweep 2");
    assert_eq!(
        judge_results::Entity::find().all(&db).await.unwrap().len(),
        1,
        "past the floor, a judge with no terminal row is re-driven"
    );
}

/// A judge run that STARTED and lost its process (deploy, crash) leaves a
/// `running` row nothing re-drives — the missed-run sweep skips cancelled
/// sessions on purpose. The orphan sweep must settle exactly that row
/// (session Y3W66Z: ux-review froze at "Evaluating…" this way), while a
/// fresh `running` row inside the age floor — a run legitimately executing —
/// stays untouched.
#[tokio::test]
async fn orphaned_running_judge_row_of_a_cancelled_session_is_requeued() {
    use arena_core::entities::{judge_results, judges, task_judges, tasks};
    use game_server::recovery::requeue_orphaned_judge_runs;
    use sea_orm::{ColumnTrait, QueryFilter};

    let db = setup_db().await;
    let server_id = Uuid::new_v4();
    insert_game_server(&db, server_id).await;
    let project = insert_project(&db).await;

    // A cancelled session this server owns — the idle sweep ended it while
    // judging was in flight.
    let session_id = Uuid::new_v4();
    sessions::ActiveModel {
        id: Set(session_id),
        name: Set("s".to_string()),
        created_at: Set(Utc::now()),
        owner_id_fk: Set(None),
        status: Set(SessionStatus::Cancelled),
        join_code: Set("JCORPH".to_string()),
        started_at: Set(Some(Utc::now() - chrono::Duration::hours(1))),
        finished_at: Set(Some(Utc::now() - chrono::Duration::minutes(30))),
        paused_at: Set(None),
        paused_duration_secs: Set(None),
        project_id_fk: Set(project),
        game_server_id: Set(Some(server_id)),
        cancel_reason: Set(Some("idle_timeout".to_string())),
        cancelled_by: Set(None),
    }
    .insert(&db)
    .await
    .expect("insert cancelled session");

    let player_id = Uuid::new_v4();
    arena_core::entities::players::ActiveModel {
        id: Set(player_id),
        session_id_fk: Set(session_id),
        user_id_fk: Set(None),
        display_name: Set("p".to_string()),
        fingerprint: Set(None),
        metadata_json: Set(None),
        joined_at: Set(Utc::now()),
        reconnected_at: Set(None),
        revoked_at: Set(None),
        agent_connected: Set(false),
        agent_last_seen_at: Set(None),
    }
    .insert(&db)
    .await
    .expect("insert player");

    let task_id = Uuid::new_v4();
    tasks::ActiveModel {
        id: Set(task_id),
        project_id_fk: Set(project),
        ordinal: Set(0),
        title: Set("Task".to_string()),
        content: Set("do it".to_string()),
        test_template: Set(serde_json::json!({"kind": "shell"})),
        // Before the session finished: tasks seeded later are not part of
        // the session (aec0573).
        created_at: Set(Utc::now() - chrono::Duration::hours(2)),
        tags: Set(String::new()),
        point_value: Set(10),
        deadline_secs: Set(Some(300)),
        min_interval_secs: Set(Some(5)),
        interval_increment_secs: Set(Some(0)),
        max_interval_secs: Set(Some(300)),
        fail_points: Set(0),
        no_response_points: Set(0),
        completion_bonus_points: Set(0),
        evaluation: Set(None),
    }
    .insert(&db)
    .await
    .expect("insert task");

    let mut tj_ids = Vec::new();
    for ordinal in 0..2 {
        let judge_id = Uuid::new_v4();
        judges::ActiveModel {
            id: Set(judge_id),
            slug: Set(format!("orph-{judge_id}")),
            name: Set("Judge".to_string()),
            description: Set(String::new()),
            prompt: Set("judge".to_string()),
            rating_scale: Set(serde_json::json!({"min": 0.0, "max": 10.0, "step": 1.0})),
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
        .insert(&db)
        .await
        .expect("insert judge");
        let tj = task_judges::ActiveModel {
            id: Set(Uuid::new_v4()),
            task_id: Set(task_id),
            judge_id: Set(judge_id),
            ordinal: Set(ordinal),
            rating_scale_override: Set(None),
            // Before the session finished: attachments created later are
            // (correctly) not expected of it.
            created_at: Set(Utc::now() - chrono::Duration::hours(1)),
            updated_at: Set(Utc::now()),
            weight: Set(None),
        }
        .insert(&db)
        .await
        .expect("attach judge");
        tj_ids.push(tj.id);
    }

    // Row 0: orphaned — `running` since well before the age floor.
    // Row 1: fresh — a run that could legitimately still be executing.
    for (idx, tj_id) in tj_ids.iter().enumerate() {
        let updated = if idx == 0 {
            Utc::now() - chrono::Duration::minutes(20)
        } else {
            Utc::now()
        };
        judge_results::ActiveModel {
            id: Set(Uuid::new_v4()),
            session_id_fk: Set(session_id),
            player_id_fk: Set(player_id),
            task_judge_id: Set(*tj_id),
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
            status: Set("running".to_string()),
            error: Set(None),
            created_at: Set(updated),
            updated_at: Set(updated),
            verdict_kind: Set(None),
        }
        .insert(&db)
        .await
        .expect("insert running row");
    }

    // No repo and no reachable LLM: the recovered run fails — terminal, which
    // is what unfreezes the player page.
    let state = test_state(db.clone(), server_id);
    requeue_orphaned_judge_runs(state, std::time::Duration::from_secs(300))
        .await
        .expect("orphan sweep");

    let stale = judge_results::Entity::find()
        .filter(judge_results::Column::TaskJudgeId.eq(tj_ids[0]))
        .one(&db)
        .await
        .expect("query")
        .expect("row");
    assert_ne!(
        stale.status, "running",
        "the orphaned run must be re-driven to a terminal status"
    );

    let fresh = judge_results::Entity::find()
        .filter(judge_results::Column::TaskJudgeId.eq(tj_ids[1]))
        .one(&db)
        .await
        .expect("query")
        .expect("row");
    assert_eq!(
        fresh.status, "running",
        "a run inside the age floor may be live — it must not be double-driven"
    );
}

/// The reaper settles runs nothing else can reach: a session finished outside
/// the recovery window, bound to a server id no live process owns. Expected
/// pairs get a zero-delta terminal `failed` row; attachments created after the
/// finish are not expected at all; a second pass inserts nothing.
#[tokio::test]
async fn abandoned_judge_runs_are_reaped_as_failed() {
    use arena_core::entities::{
        judge_results, judges, session_scheduler_state, task_judges, tasks,
    };
    use game_server::recovery::{ABANDONED_RUN_ERROR, fail_abandoned_judge_runs};

    let db = setup_db().await;
    let dead_server = Uuid::new_v4();
    insert_game_server(&db, dead_server).await;
    let project = insert_project(&db).await;

    let finished_at = Utc::now() - chrono::Duration::hours(25);
    let session_id = Uuid::new_v4();
    sessions::ActiveModel {
        id: Set(session_id),
        name: Set("s".to_string()),
        created_at: Set(Utc::now() - chrono::Duration::hours(26)),
        owner_id_fk: Set(None),
        status: Set(SessionStatus::Finished),
        join_code: Set("JCREAP".to_string()),
        started_at: Set(Some(Utc::now() - chrono::Duration::hours(26))),
        finished_at: Set(Some(finished_at)),
        paused_at: Set(None),
        paused_duration_secs: Set(None),
        project_id_fk: Set(project),
        game_server_id: Set(Some(dead_server)),
        cancel_reason: Set(None),
        cancelled_by: Set(None),
    }
    .insert(&db)
    .await
    .expect("insert finished session");

    let player_id = Uuid::new_v4();
    arena_core::entities::players::ActiveModel {
        id: Set(player_id),
        session_id_fk: Set(session_id),
        user_id_fk: Set(None),
        display_name: Set("p".to_string()),
        fingerprint: Set(None),
        metadata_json: Set(None),
        joined_at: Set(Utc::now()),
        reconnected_at: Set(None),
        revoked_at: Set(None),
        agent_connected: Set(false),
        agent_last_seen_at: Set(None),
    }
    .insert(&db)
    .await
    .expect("insert player");

    let task_id = Uuid::new_v4();
    tasks::ActiveModel {
        id: Set(task_id),
        project_id_fk: Set(project),
        ordinal: Set(0),
        title: Set("Task".to_string()),
        content: Set("do it".to_string()),
        test_template: Set(serde_json::json!({"kind": "shell"})),
        created_at: Set(Utc::now() - chrono::Duration::hours(26)),
        tags: Set(String::new()),
        point_value: Set(10),
        deadline_secs: Set(Some(300)),
        min_interval_secs: Set(Some(5)),
        interval_increment_secs: Set(Some(0)),
        max_interval_secs: Set(Some(300)),
        fail_points: Set(0),
        no_response_points: Set(0),
        completion_bonus_points: Set(0),
        evaluation: Set(None),
    }
    .insert(&db)
    .await
    .expect("insert task");

    let judge_id = Uuid::new_v4();
    judges::ActiveModel {
        id: Set(judge_id),
        slug: Set("code-cleanliness".to_string()),
        name: Set("Cleanliness".to_string()),
        description: Set(String::new()),
        prompt: Set("judge".to_string()),
        rating_scale: Set(serde_json::json!({"min": 0.0, "max": 10.0, "step": 1.0})),
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
    .insert(&db)
    .await
    .expect("insert judge");

    // One attachment the session ran with, one attached after the finish.
    let tj_before = Uuid::new_v4();
    task_judges::ActiveModel {
        id: Set(tj_before),
        task_id: Set(task_id),
        judge_id: Set(judge_id),
        ordinal: Set(0),
        rating_scale_override: Set(None),
        created_at: Set(finished_at - chrono::Duration::hours(1)),
        updated_at: Set(Utc::now()),
        weight: Set(None),
    }
    .insert(&db)
    .await
    .expect("attach judge before finish");
    // task_judges is unique on (task_id, judge_id) — the late attachment
    // needs its own judge row.
    let judge2_id = Uuid::new_v4();
    judges::ActiveModel {
        id: Set(judge2_id),
        slug: Set("late-panel".to_string()),
        name: Set("Late Panel".to_string()),
        description: Set(String::new()),
        prompt: Set("judge".to_string()),
        rating_scale: Set(serde_json::json!({"min": 0.0, "max": 10.0, "step": 1.0})),
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
    .insert(&db)
    .await
    .expect("insert late judge");
    let tj_after = Uuid::new_v4();
    task_judges::ActiveModel {
        id: Set(tj_after),
        task_id: Set(task_id),
        judge_id: Set(judge2_id),
        ordinal: Set(1),
        rating_scale_override: Set(None),
        created_at: Set(finished_at + chrono::Duration::hours(1)),
        updated_at: Set(Utc::now()),
        weight: Set(None),
    }
    .insert(&db)
    .await
    .expect("attach judge after finish");

    session_scheduler_state::ActiveModel {
        id: Set(Uuid::new_v4()),
        session_id_fk: Set(session_id),
        player_id_fk: Set(player_id),
        task_id: Set(None),
        state: Set(arena_core::session_completion::SCHEDULER_STATE_COMPLETED.to_string()),
        next_probe_at: Set(None),
        created_at: Set(Utc::now()),
        updated_at: Set(Utc::now()),
    }
    .insert(&db)
    .await
    .expect("insert scheduler row");

    // The reaping process is a DIFFERENT server — the session's own id is dead.
    let state = test_state(db.clone(), Uuid::new_v4());
    fail_abandoned_judge_runs(state.clone())
        .await
        .expect("reap");

    let rows = judge_results::Entity::find().all(&db).await.unwrap();
    assert_eq!(
        rows.len(),
        1,
        "only the attachment the session ran with is reaped: {rows:?}"
    );
    let row = &rows[0];
    assert_eq!(row.task_judge_id, tj_before);
    assert_eq!(row.status, "failed");
    assert_eq!(row.point_delta, 0);
    assert_eq!(row.error.as_deref(), Some(ABANDONED_RUN_ERROR));

    fail_abandoned_judge_runs(state).await.expect("reap again");
    assert_eq!(
        judge_results::Entity::find().all(&db).await.unwrap().len(),
        1,
        "reaping is idempotent"
    );
}
