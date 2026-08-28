//! Session completion lifecycle: a session finishes only when ALL eligible
//! (non-revoked) players have exhausted their tasks — the LAST finisher ends
//! the session. The first finisher gets a per-player `SessionComplete`
//! acknowledgment (`player_tasks_completed`) while the session keeps running.

use std::sync::Arc;

use arena_core::entities::{players, projects, sessions, tasks, users};
use arena_core::protocol::{PlayerAgentFrame, SESSION_COMPLETE_REASON_PLAYER_TASKS_COMPLETED};
use arena_core::session_status::SessionStatus;
use chrono::Utc;
use dashmap::DashMap;
use game_server::state::{GameServerState, finish_session, on_player_tasks_exhausted};
use game_server::ws::player_agent::scheduler::{advance_to_next_task, bootstrap_scheduler_state};
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

fn test_state(db: DatabaseConnection) -> GameServerState {
    let secret = b"test-secret-32-bytes-or-more-xxxxxxx".to_vec();
    GameServerState {
        db,
        server_id: Uuid::new_v4(),
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

async fn insert_player(
    db: &DatabaseConnection,
    session_id: Uuid,
    user_id: Uuid,
    name: &str,
) -> Uuid {
    players::ActiveModel {
        id: Set(Uuid::new_v4()),
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
    .expect("insert player")
    .id
}

/// Insert user, project, session, two players, and two ordinal-ordered tasks.
/// Returns (session_id, player_a, player_b, task_ids).
async fn seed_two_players_two_tasks(db: &DatabaseConnection) -> (Uuid, Uuid, Uuid, Vec<Uuid>) {
    let user_id = users::ActiveModel {
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

    let project_id = projects::ActiveModel {
        id: Set(Uuid::new_v4()),
        name: Set("proj".to_string()),
        slug: Set(None),
        description: Set(String::new()),
        category: Set(None),
        tags: Set(String::new()),
        cover_image_url: Set(None),
        owner_user_id_fk: Set(user_id),
        public: Set(true),
        archived_at: Set(None),
        created_at: Set(Utc::now()),
        updated_at: Set(Utc::now()),
        default_value_points: Set(10),
        default_fail_points: Set(-5),
        default_no_response_points: Set(-10),
        default_completion_bonus_points: Set(10),
        default_deadline_secs: Set(60),
        default_min_interval_secs: Set(5),
        default_interval_increment_secs: Set(5),
        default_max_interval_secs: Set(60),
        default_session_duration_secs: Set(3600),
        idle_timeout_secs: Set(300),
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
        created_at: Set(Utc::now()),
        owner_id_fk: Set(None),
        status: Set(SessionStatus::Running),
        join_code: Set("SC1".to_string()),
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
    .expect("insert session")
    .id;

    let player_a = insert_player(db, session_id, user_id, "alice").await;
    let player_b = insert_player(db, session_id, user_id, "bob").await;

    let mut task_ids = Vec::new();
    for ordinal in 0..2 {
        let template = serde_json::json!({
            "kind": "shell",
            "command_template": "echo hi",
        });
        let task_id = tasks::ActiveModel {
            id: Set(Uuid::new_v4()),
            project_id_fk: Set(project_id),
            ordinal: Set(ordinal),
            title: Set(format!("Task {ordinal}")),
            content: Set("do the thing".to_string()),
            test_template: Set(template),
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
        .expect("insert task")
        .id;
        task_ids.push(task_id);
    }

    (session_id, player_a, player_b, task_ids)
}

/// Walk one player through both tasks until `advance_to_next_task` reports
/// no next task (their scheduler row lands in the "completed" state).
async fn exhaust_tasks(
    state: &GameServerState,
    session_id: Uuid,
    player_id: Uuid,
    task_ids: &[Uuid],
) {
    let first = bootstrap_scheduler_state(state, session_id, player_id, "SC1")
        .await
        .expect("bootstrap");
    assert_eq!(first, task_ids[0]);
    assert!(
        advance_to_next_task(state, session_id, player_id, "SC1", task_ids[0]).await,
        "first advance moves to the second task"
    );
    assert!(
        !advance_to_next_task(state, session_id, player_id, "SC1", task_ids[1]).await,
        "second advance exhausts the task list"
    );
}

async fn session_row(db: &DatabaseConnection, session_id: Uuid) -> sessions::Model {
    sessions::Entity::find_by_id(session_id)
        .one(db)
        .await
        .expect("query session")
        .expect("session exists")
}

fn assert_player_done_ack(frame: &PlayerAgentFrame, session_id: Uuid) {
    match frame {
        PlayerAgentFrame::SessionComplete {
            session_id: sid,
            reason,
        } => {
            assert_eq!(*sid, session_id);
            assert_eq!(
                reason.as_deref(),
                Some(SESSION_COMPLETE_REASON_PLAYER_TASKS_COMPLETED),
                "first finisher must be acknowledged with the per-player reason"
            );
        }
        other => panic!("expected SessionComplete ack, got {other:?}"),
    }
}

#[tokio::test]
async fn first_finisher_gets_ack_and_session_keeps_running() {
    let db = setup_db().await;
    let state = test_state(db.clone());
    let (session_id, player_a, player_b, task_ids) = seed_two_players_two_tasks(&db).await;

    bootstrap_scheduler_state(&state, session_id, player_b, "SC1")
        .await
        .expect("bootstrap player b");
    exhaust_tasks(&state, session_id, player_a, &task_ids).await;

    let (frame, finished) = on_player_tasks_exhausted(&state, session_id, "SC1").await;
    assert_player_done_ack(&frame, session_id);
    assert!(
        !finished,
        "player b is still mid-task — session must not finish"
    );

    let row = session_row(&db, session_id).await;
    assert_eq!(
        row.status,
        SessionStatus::Running,
        "session must stay Running while any eligible player has tasks left"
    );
    assert!(row.finished_at.is_none());
}

#[tokio::test]
async fn last_finisher_ends_session_with_all_tasks_completed() {
    let db = setup_db().await;
    let state = test_state(db.clone());
    let (session_id, player_a, player_b, task_ids) = seed_two_players_two_tasks(&db).await;

    exhaust_tasks(&state, session_id, player_a, &task_ids).await;
    let (_, finished) = on_player_tasks_exhausted(&state, session_id, "SC1").await;
    assert!(!finished);

    exhaust_tasks(&state, session_id, player_b, &task_ids).await;
    let (frame, finished) = on_player_tasks_exhausted(&state, session_id, "SC1").await;
    assert_player_done_ack(&frame, session_id);
    assert!(finished, "last finisher must end the session");

    let row = session_row(&db, session_id).await;
    assert_eq!(row.status, SessionStatus::Finished);
    assert!(row.finished_at.is_some());
}

#[tokio::test]
async fn revoked_player_does_not_block_completion() {
    let db = setup_db().await;
    let state = test_state(db.clone());
    let (session_id, player_a, player_b, task_ids) = seed_two_players_two_tasks(&db).await;

    exhaust_tasks(&state, session_id, player_a, &task_ids).await;

    // Player B never progressed and is revoked mid-session — they must not
    // hold the session open.
    let b_row = players::Entity::find_by_id(player_b)
        .one(&db)
        .await
        .expect("query player b")
        .expect("player b exists");
    let mut am: players::ActiveModel = b_row.into();
    am.revoked_at = Set(Some(Utc::now()));
    am.update(&db).await.expect("revoke player b");

    let (_, finished) = on_player_tasks_exhausted(&state, session_id, "SC1").await;
    assert!(finished, "a revoked player must never block completion");
    assert_eq!(
        session_row(&db, session_id).await.status,
        SessionStatus::Finished
    );
}

#[tokio::test]
async fn double_finish_applies_the_transition_exactly_once() {
    let db = setup_db().await;
    let state = test_state(db.clone());
    let (session_id, player_a, player_b, task_ids) = seed_two_players_two_tasks(&db).await;

    exhaust_tasks(&state, session_id, player_a, &task_ids).await;
    exhaust_tasks(&state, session_id, player_b, &task_ids).await;

    let (_, finished) = on_player_tasks_exhausted(&state, session_id, "SC1").await;
    assert!(finished);
    let first = session_row(&db, session_id).await;
    assert_eq!(first.status, SessionStatus::Finished);

    // A concurrent finisher (or the running timer) racing past its own check
    // must lose at the DB status guard: the row keeps the first transition's
    // finished_at.
    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    finish_session(&state, session_id, "SC1", "all_tasks_completed").await;
    let second = session_row(&db, session_id).await;
    assert_eq!(
        second.finished_at, first.finished_at,
        "second finish must not re-apply the transition"
    );
}
