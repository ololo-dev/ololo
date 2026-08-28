//! Idle sweep: a running session with no connected agents past the project's
//! `idle_timeout_secs` is cancelled; anything with a live client, a recent
//! disconnect, or the timeout disabled is left alone.

use std::sync::Arc;

use arena_core::entities::{game_servers, players, projects, sessions, users};
use arena_core::session_status::SessionStatus;
use chrono::{Duration, Utc};
use dashmap::DashMap;
use game_server::idle_sweep::sweep_once;
use game_server::state::GameServerState;
use game_server::zmq_pub::NoopEventPublisher;
use jsonwebtoken::{DecodingKey, EncodingKey};
use migration::{Migrator, MigratorTrait};
use sea_orm::{ActiveModelTrait, DatabaseConnection, EntityTrait, Set};
use tokio::sync::Semaphore;
use uuid::Uuid;

async fn setup_db() -> DatabaseConnection {
    let db = sea_orm::Database::connect("sqlite::memory:")
        .await
        .expect("sqlite connect");
    Migrator::up(&db, None).await.expect("migrate up");
    db
}

fn test_state(db: DatabaseConnection, server_id: Uuid) -> GameServerState {
    let secret = b"test-secret-32-bytes-or-more-xxxxxxx".to_vec();
    GameServerState {
        db,
        server_id,
        advertise_url: "ws://localhost:8081".to_string(),
        jwt_encoding_key: Arc::new(EncodingKey::from_secret(&secret)),
        jwt_decoding_key: Arc::new(DecodingKey::from_secret(&secret)),
        jwt_signing_secret: Arc::new(secret),
        session_registry: Arc::new(DashMap::new()),
        player_agent_registry: Arc::new(DashMap::new()),
        lobby_timer_secs: 60,
        event_publisher: Arc::new(NoopEventPublisher),
        judge_semaphore: Arc::new(Semaphore::new(2)),
        settings_encryption: std::sync::Arc::new(
            arena_core::settings_encryption::SettingsEncryption::new(
                b"test-secret-key-for-settings-enc",
            ),
        ),
    }
}

async fn seed_game_server(db: &DatabaseConnection, server_id: Uuid) {
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

async fn seed_project(db: &DatabaseConnection, idle_timeout_secs: i32) -> Uuid {
    let owner = Uuid::new_v4();
    users::ActiveModel {
        id: Set(owner),
        email: Set(format!("u{owner}@example.com")),
        password_hash: Set(None),
        display_name: Set("t".to_string()),
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

    let id = Uuid::new_v4();
    projects::ActiveModel {
        id: Set(id),
        name: Set("p".to_string()),
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
        default_min_interval_secs: Set(5),
        default_interval_increment_secs: Set(5),
        default_max_interval_secs: Set(60),
        idle_timeout_secs: Set(idle_timeout_secs),
        memory_schema: Set(None),
        show_tasks: Set(true),
        parent_project_id_fk: Set(None),
        part_ordinal: Set(None),
    }
    .insert(db)
    .await
    .expect("insert project");
    id
}

async fn seed_running_session(
    db: &DatabaseConnection,
    server_id: Uuid,
    project_id: Uuid,
    join_code: &str,
    started_mins_ago: i64,
) -> Uuid {
    let id = Uuid::new_v4();
    sessions::ActiveModel {
        id: Set(id),
        name: Set("s".to_string()),
        created_at: Set(Utc::now()),
        owner_id_fk: Set(None),
        status: Set(SessionStatus::Running),
        join_code: Set(join_code.to_string()),
        started_at: Set(Some(Utc::now() - Duration::minutes(started_mins_ago))),
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

async fn seed_player(
    db: &DatabaseConnection,
    session_id: Uuid,
    connected: bool,
    last_seen_mins_ago: Option<i64>,
) -> Uuid {
    let id = Uuid::new_v4();
    players::ActiveModel {
        id: Set(id),
        session_id_fk: Set(session_id),
        user_id_fk: Set(None),
        display_name: Set("p".to_string()),
        fingerprint: Set(None),
        metadata_json: Set(None),
        joined_at: Set(Utc::now()),
        reconnected_at: Set(None),
        revoked_at: Set(None),
        agent_connected: Set(connected),
        agent_last_seen_at: Set(last_seen_mins_ago.map(|m| Utc::now() - Duration::minutes(m))),
    }
    .insert(db)
    .await
    .expect("insert player");
    id
}

async fn status_of(db: &DatabaseConnection, session_id: Uuid) -> SessionStatus {
    sessions::Entity::find_by_id(session_id)
        .one(db)
        .await
        .unwrap()
        .unwrap()
        .status
}

#[tokio::test]
async fn cancels_a_session_abandoned_past_the_threshold() {
    let db = setup_db().await;
    let server_id = Uuid::new_v4();
    seed_game_server(&db, server_id).await;
    // 60s timeout; the player disconnected five minutes ago.
    let project = seed_project(&db, 60).await;
    let session = seed_running_session(&db, server_id, project, "IDLE01", 30).await;
    seed_player(&db, session, false, Some(5)).await;

    let state = test_state(db.clone(), server_id);
    assert_eq!(sweep_once(&state).await.expect("sweep"), 1);
    assert_eq!(status_of(&db, session).await, SessionStatus::Cancelled);

    // Second pass: already cancelled, nothing left to do — the sweep must be
    // idempotent or every tick would re-broadcast the end frames.
    assert_eq!(sweep_once(&state).await.expect("sweep"), 0);
}

#[tokio::test]
async fn leaves_alone_connected_recent_disabled_and_never_started() {
    let db = setup_db().await;
    let server_id = Uuid::new_v4();
    seed_game_server(&db, server_id).await;

    // A connected agent keeps the session alive no matter how old.
    let p1 = seed_project(&db, 60).await;
    let s1 = seed_running_session(&db, server_id, p1, "IDLE02", 120).await;
    seed_player(&db, s1, true, Some(90)).await;

    // A disconnect 10 seconds ago is a reconnect window, not abandonment.
    let p2 = seed_project(&db, 60).await;
    let s2 = seed_running_session(&db, server_id, p2, "IDLE03", 120).await;
    seed_player(&db, s2, false, Some(0)).await;

    // idle_timeout_secs = 0 disables the sweep for the project entirely.
    let p3 = seed_project(&db, 0).await;
    let s3 = seed_running_session(&db, server_id, p3, "IDLE04", 600).await;
    seed_player(&db, s3, false, Some(500)).await;

    let state = test_state(db.clone(), server_id);
    assert_eq!(sweep_once(&state).await.expect("sweep"), 0);
    assert_eq!(status_of(&db, s1).await, SessionStatus::Running);
    assert_eq!(status_of(&db, s2).await, SessionStatus::Running);
    assert_eq!(status_of(&db, s3).await, SessionStatus::Running);
}

#[tokio::test]
async fn a_session_no_client_ever_joined_counts_from_its_start() {
    let db = setup_db().await;
    let server_id = Uuid::new_v4();
    seed_game_server(&db, server_id).await;
    let project = seed_project(&db, 300).await;
    // Started 20 minutes ago, one player row, agent never connected
    // (last_seen NULL): empty-since falls back to started_at.
    let session = seed_running_session(&db, server_id, project, "IDLE05", 20).await;
    seed_player(&db, session, false, None).await;

    let state = test_state(db.clone(), server_id);
    assert_eq!(sweep_once(&state).await.expect("sweep"), 1);
    assert_eq!(status_of(&db, session).await, SessionStatus::Cancelled);
}
