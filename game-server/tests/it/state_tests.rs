//! Coverage for src/state.rs: `bump_session_version`, `broadcast_frame`,
//! `finish_session`, and `SessionCacheInner::new`.

use std::sync::Arc;

use arena_core::entities::sessions;
use arena_core::protocol::ArenaFrame;
use arena_core::session_status::SessionStatus;
use chrono::Utc;
use dashmap::DashMap;
use game_server::state::{
    SessionCacheInner, SessionEntry, broadcast_frame, bump_session_version, compute_remaining,
    finish_session,
};
use game_server::zmq_pub::NoopEventPublisher;
use jsonwebtoken::{DecodingKey, EncodingKey};
use migration::MigratorTrait;
use sea_orm::{ActiveModelTrait, DatabaseConnection, EntityTrait, Set};
use std::sync::RwLock;
use tokio::sync::{Semaphore, broadcast};
use uuid::Uuid;

async fn setup_db() -> DatabaseConnection {
    let db = sea_orm::Database::connect("sqlite::memory:")
        .await
        .expect("connect");
    migration::Migrator::up(&db, None).await.expect("migrate");
    db
}

/// Records what the state machine publishes, so a test can assert that a
/// session was announced as settled. `NoopEventPublisher` swallows events,
/// which is fine everywhere the assertion is about the database instead.
#[derive(Default)]
struct RecordingPublisher {
    events: std::sync::Mutex<Vec<arena_core::protocol::ZmqEvent>>,
}

impl RecordingPublisher {
    /// Join codes of the `SessionSettled` events published so far.
    fn settled(&self) -> Vec<String> {
        self.events
            .lock()
            .unwrap()
            .iter()
            .filter_map(|e| match e {
                arena_core::protocol::ZmqEvent::SessionSettled { join_code, .. } => {
                    Some(join_code.clone())
                }
                _ => None,
            })
            .collect()
    }
}

#[async_trait::async_trait]
impl game_server::zmq_pub::EventPublisher for RecordingPublisher {
    async fn publish(&self, event: &arena_core::protocol::ZmqEvent) {
        self.events.lock().unwrap().push(event.clone());
    }
}

/// `test_state`, but with a publisher whose events the caller can read back.
fn test_state_recording(
    db: DatabaseConnection,
) -> (game_server::state::GameServerState, Arc<RecordingPublisher>) {
    let publisher = Arc::new(RecordingPublisher::default());
    let mut state = test_state(db);
    state.event_publisher = publisher.clone();
    (state, publisher)
}

fn test_state(db: DatabaseConnection) -> game_server::state::GameServerState {
    let secret = b"test-secret-32-bytes-or-more-xxxxxxx".to_vec();
    game_server::state::GameServerState {
        db: db.clone(),
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

async fn insert_project(db: &DatabaseConnection) -> Uuid {
    let owner = arena_core::entities::users::ActiveModel {
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
    arena_core::entities::projects::ActiveModel {
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
    .id
}

async fn insert_session(db: &DatabaseConnection, status: SessionStatus) -> Uuid {
    let project_id = insert_project(db).await;
    let id = Uuid::new_v4();
    sessions::ActiveModel {
        id: Set(id),
        name: Set("s".to_string()),
        created_at: Set(Utc::now()),
        owner_id_fk: Set(None),
        status: Set(status),
        join_code: Set("JC1".to_string()),
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
    id
}

fn register_entry(
    state: &game_server::state::GameServerState,
    join_code: &str,
) -> broadcast::Receiver<ArenaFrame> {
    let (tx, rx) = broadcast::channel::<ArenaFrame>(64);
    let cache = Arc::new(RwLock::new(SessionCacheInner::new(
        Uuid::new_v4(),
        SessionStatus::Running,
        Some(Utc::now()),
    )));
    state.session_registry.insert(
        join_code.to_string(),
        SessionEntry {
            tx,
            cache,
            cancel: tokio_util::sync::CancellationToken::new(),
        },
    );
    rx
}

#[test]
fn cache_new_starts_at_version_zero_lobby() {
    let cache = SessionCacheInner::new(Uuid::new_v4(), SessionStatus::Lobby, None);
    assert_eq!(cache.version, 0);
    assert_eq!(cache.phase, SessionStatus::Lobby);
    assert!(cache.participants.is_empty());
    assert!(cache.leaderboard.is_empty());
    assert!(cache.started_at.is_none());
}

#[test]
fn bump_version_increments_and_returns_value() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let db = setup_db().await;
        let state = test_state(db);
        let rx = register_entry(&state, "JC1");
        std::mem::drop(rx); // not asserting broadcast here
        assert_eq!(bump_session_version(&state, "JC1"), 1);
        assert_eq!(bump_session_version(&state, "JC1"), 2);
        // Unknown join_code → 0.
        assert_eq!(bump_session_version(&state, "NOPE"), 0);
    });
}

#[test]
fn broadcast_frame_delivers_to_subscriber() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let db = setup_db().await;
        let state = test_state(db);
        let mut rx = register_entry(&state, "JC1");
        broadcast_frame(
            &state,
            "JC1",
            ArenaFrame::LobbyCountdown {
                session_id: Uuid::new_v4(),
                seconds_remaining: 10,
                version: 1,
            },
        );
        let frame = rx.try_recv().expect("frame delivered");
        assert!(matches!(
            frame,
            ArenaFrame::LobbyCountdown {
                seconds_remaining: 10,
                ..
            }
        ));
        // No entry → silent no-op.
        broadcast_frame(
            &state,
            "NOPE",
            ArenaFrame::LobbyCountdown {
                session_id: Uuid::new_v4(),
                seconds_remaining: 1,
                version: 1,
            },
        );
    });
}

#[test]
fn finish_session_updates_cache_db_and_broadcasts() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let db = setup_db().await;
        let state = test_state(db.clone());
        let sid = insert_session(&state.db, SessionStatus::Running).await;
        let mut rx = register_entry(&state, "JC1");

        finish_session(&state, sid, "JC1", "all_tasks_completed").await;

        // Cache phase flipped to Finished.
        let phase = state
            .session_registry
            .get("JC1")
            .map(|e| e.cache.read().unwrap().phase)
            .expect("entry");
        assert_eq!(phase, SessionStatus::Finished);

        // DB row updated.
        let row = sessions::Entity::find_by_id(sid)
            .one(&state.db)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.status, SessionStatus::Finished);
        assert!(row.finished_at.is_some());

        // SessionComplete broadcast + ZmqEvent handled by NoopEventPublisher.
        let frame = rx.try_recv().expect("session complete frame");
        assert!(matches!(
            frame,
            ArenaFrame::SessionComplete { reason, .. } if reason == "all_tasks_completed"
        ));
    });
}

#[test]
fn finish_session_announces_settlement_when_no_judges_are_pending() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let db = setup_db().await;
        let (state, events) = test_state_recording(db.clone());
        let sid = insert_session(&state.db, SessionStatus::Running).await;
        register_entry(&state, "JC1");

        let user_id = Uuid::new_v4();
        arena_core::entities::users::ActiveModel {
            id: Set(user_id),
            email: Set(format!("u{user_id}@example.com")),
            password_hash: Set(None),
            display_name: Set("player".to_string()),
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
        .insert(&state.db)
        .await
        .expect("insert user");
        arena_core::entities::players::ActiveModel {
            id: Set(Uuid::new_v4()),
            session_id_fk: Set(sid),
            user_id_fk: Set(Some(user_id)),
            display_name: Set("p".to_string()),
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

        finish_session(&state, sid, "JC1", "time_expired").await;

        assert_eq!(
            events.settled(),
            vec!["JC1".to_string()],
            "nothing left to wait for: the standings are announced final"
        );

        // A second finish call must not announce twice.
        finish_session(&state, sid, "JC1", "time_expired").await;
        assert_eq!(events.settled(), vec!["JC1".to_string()]);
    });
}

#[test]
fn finish_session_skips_db_update_when_already_terminal() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let db = setup_db().await;
        let state = test_state(db.clone());
        // Insert as Finished already; finish_session must not flip it back.
        let sid = insert_session(&state.db, SessionStatus::Cancelled).await;
        register_entry(&state, "JC1");
        finish_session(&state, sid, "JC1", "time_expired").await;
        let row = sessions::Entity::find_by_id(sid)
            .one(&state.db)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.status, SessionStatus::Cancelled);
    });
}

#[test]
fn compute_remaining_is_single_source_of_truth() {
    let started = Utc::now() - chrono::Duration::seconds(30);
    assert_eq!(compute_remaining(100, started, Utc::now(), Some(0)), 70);
}

#[test]
fn finish_session_defers_settlement_until_expired_judges_settle() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let db = setup_db().await;
        let (state, events) = test_state_recording(db.clone());
        let sid = insert_session(&state.db, SessionStatus::Running).await;
        register_entry(&state, "JC1");
        let project_id = sessions::Entity::find_by_id(sid)
            .one(&state.db)
            .await
            .unwrap()
            .unwrap()
            .project_id_fk;

        let user_id = Uuid::new_v4();
        arena_core::entities::users::ActiveModel {
            id: Set(user_id),
            email: Set(format!("u{user_id}@example.com")),
            password_hash: Set(None),
            display_name: Set("player".to_string()),
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
        .insert(&state.db)
        .await
        .expect("insert user");
        let player_id = Uuid::new_v4();
        arena_core::entities::players::ActiveModel {
            id: Set(player_id),
            session_id_fk: Set(sid),
            user_id_fk: Set(Some(user_id)),
            display_name: Set("p".to_string()),
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

        // One task with an attached judge; the player completed all tasks
        // but the judge run has no result row yet -> the expiry awards must
        // wait for it.
        let task_id = Uuid::new_v4();
        arena_core::entities::tasks::ActiveModel {
            id: Set(task_id),
            project_id_fk: Set(project_id),
            ordinal: Set(0),
            title: Set("Task 0".to_string()),
            content: Set("do".to_string()),
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
        .insert(&state.db)
        .await
        .expect("insert task");
        let judge_id = Uuid::new_v4();
        arena_core::entities::judges::ActiveModel {
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
        .insert(&state.db)
        .await
        .expect("insert judge");
        let task_judge_id = Uuid::new_v4();
        arena_core::entities::task_judges::ActiveModel {
            id: Set(task_judge_id),
            task_id: Set(task_id),
            judge_id: Set(judge_id),
            ordinal: Set(0),
            rating_scale_override: Set(None),
            created_at: Set(Utc::now()),
            updated_at: Set(Utc::now()),
            weight: Set(None),
        }
        .insert(&state.db)
        .await
        .expect("insert task_judge");
        arena_core::entities::session_scheduler_state::ActiveModel {
            id: Set(Uuid::new_v4()),
            session_id_fk: Set(sid),
            player_id_fk: Set(player_id),
            task_id: Set(None),
            state: Set("completed".to_string()),
            next_probe_at: Set(None),
            created_at: Set(Utc::now()),
            updated_at: Set(Utc::now()),
        }
        .insert(&state.db)
        .await
        .expect("insert scheduler row");

        finish_session(&state, sid, "JC1", "time_expired").await;

        // Session is Finished, but the pending judge run defers settlement.
        let row = sessions::Entity::find_by_id(sid)
            .one(&state.db)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.status, SessionStatus::Finished);
        assert!(
            events.settled().is_empty(),
            "settlement must wait for the pending judge run"
        );

        // The judge run lands (scored) -> the deferred poller settles.
        arena_core::entities::judge_results::ActiveModel {
            id: Set(Uuid::new_v4()),
            session_id_fk: Set(sid),
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
            status: Set("scored".to_string()),
            error: Set(None),
            created_at: Set(Utc::now()),
            updated_at: Set(Utc::now()),
            verdict_kind: Set(None),
        }
        .insert(&state.db)
        .await
        .expect("insert judge result");

        // The spawned poller checks every 3s; give it up to 15s.
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(15);
        loop {
            if !events.settled().is_empty() {
                break;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "settlement never announced after the judge run finished"
            );
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        }
    });
}

/// A finished session with a linked player and no award rows: build the
/// exact state a deploy-during-judge-settle leaves behind.
async fn insert_unawarded_finished_session(
    state: &game_server::state::GameServerState,
    join_code: &str,
    finished_at: chrono::DateTime<chrono::Utc>,
) -> Uuid {
    // Sessions reference their owning game server; register this one.
    let _ = arena_core::entities::game_servers::ActiveModel {
        id: Set(state.server_id),
        url: Set(state.advertise_url.clone()),
        zmq_url: Set(None),
        display_name: Set(None),
        capacity: Set(64),
        active_sessions: Set(0),
        status: Set("active".to_string()),
        last_heartbeat: Set(Utc::now()),
        created_at: Set(Utc::now()),
        updated_at: Set(Utc::now()),
    }
    .insert(&state.db)
    .await;

    let project_id = insert_project(&state.db).await;
    let sid = Uuid::new_v4();
    sessions::ActiveModel {
        id: Set(sid),
        name: Set("s".to_string()),
        created_at: Set(Utc::now()),
        owner_id_fk: Set(None),
        status: Set(SessionStatus::Finished),
        join_code: Set(join_code.to_string()),
        started_at: Set(Some(finished_at - chrono::Duration::minutes(15))),
        finished_at: Set(Some(finished_at)),
        paused_at: Set(None),
        paused_duration_secs: Set(None),
        project_id_fk: Set(project_id),
        game_server_id: Set(Some(state.server_id)),
        cancel_reason: Set(None),
        cancelled_by: Set(None),
    }
    .insert(&state.db)
    .await
    .expect("insert session");

    let user_id = Uuid::new_v4();
    arena_core::entities::users::ActiveModel {
        id: Set(user_id),
        email: Set(format!("u{user_id}@example.com")),
        password_hash: Set(None),
        display_name: Set("player".to_string()),
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
    .insert(&state.db)
    .await
    .expect("insert user");
    arena_core::entities::players::ActiveModel {
        id: Set(Uuid::new_v4()),
        session_id_fk: Set(sid),
        user_id_fk: Set(Some(user_id)),
        display_name: Set("player".to_string()),
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
    sid
}

#[test]
fn settle_recovery_sweep_announces_recent_finished_sessions() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let db = setup_db().await;
        let (state, events) = test_state_recording(db.clone());

        // Recent finished session whose in-memory settle task was lost.
        let _recent = insert_unawarded_finished_session(
            &state,
            "RCV1",
            Utc::now() - chrono::Duration::hours(2),
        )
        .await;
        // Ancient session outside the sweep window — must stay untouched.
        let _ancient = insert_unawarded_finished_session(
            &state,
            "RCV2",
            Utc::now() - chrono::Duration::hours(48),
        )
        .await;

        game_server::recovery::settle_unsettled_finished_sessions(state.clone())
            .await
            .expect("sweep");

        assert_eq!(
            events.settled(),
            vec!["RCV1".to_string()],
            "the recent session is announced; the ancient one is left alone"
        );
    });
}
