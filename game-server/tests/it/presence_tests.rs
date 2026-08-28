//! Agent presence writes: connect/disconnect flip `players.agent_connected`,
//! and a game-server restart clears every stale flag it owns — a crash with
//! players connected must not leave them "Live" forever (that would also
//! blind the idle sweep, which skips sessions that appear occupied).

use std::sync::Arc;

use arena_core::entities::{game_servers, players, projects, sessions, users};
use arena_core::session_status::SessionStatus;
use chrono::Utc;
use dashmap::DashMap;
use game_server::state::GameServerState;
use game_server::ws::player_agent::presence::{
    reset_agent_presence_on_startup, set_agent_presence,
};
use game_server::zmq_pub::NoopEventPublisher;
use jsonwebtoken::{DecodingKey, EncodingKey};
use migration::{Migrator, MigratorTrait};
use sea_orm::{ActiveModelTrait, DatabaseConnection, EntityTrait, Set};
use tokio::sync::Semaphore;
use uuid::Uuid;

async fn setup() -> (DatabaseConnection, GameServerState, Uuid, Uuid) {
    let db = sea_orm::Database::connect("sqlite::memory:")
        .await
        .expect("sqlite connect");
    Migrator::up(&db, None).await.expect("migrate up");

    let server_id = Uuid::new_v4();
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
    .insert(&db)
    .await
    .expect("game server");

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
    .insert(&db)
    .await
    .expect("user");

    let project = Uuid::new_v4();
    projects::ActiveModel {
        id: Set(project),
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
        idle_timeout_secs: Set(300),
        default_min_interval_secs: Set(5),
        default_interval_increment_secs: Set(5),
        default_max_interval_secs: Set(60),
        memory_schema: Set(None),
        show_tasks: Set(true),
        parent_project_id_fk: Set(None),
        part_ordinal: Set(None),
    }
    .insert(&db)
    .await
    .expect("project");

    let session = Uuid::new_v4();
    sessions::ActiveModel {
        id: Set(session),
        name: Set("s".to_string()),
        created_at: Set(Utc::now()),
        owner_id_fk: Set(None),
        status: Set(SessionStatus::Running),
        join_code: Set("PRSNC1".to_string()),
        started_at: Set(Some(Utc::now())),
        finished_at: Set(None),
        paused_at: Set(None),
        paused_duration_secs: Set(None),
        project_id_fk: Set(project),
        game_server_id: Set(Some(server_id)),
        cancel_reason: Set(None),
        cancelled_by: Set(None),
    }
    .insert(&db)
    .await
    .expect("session");

    let secret = b"test-secret-32-bytes-or-more-xxxxxxx".to_vec();
    let state = GameServerState {
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
        judge_semaphore: Arc::new(Semaphore::new(2)),
        settings_encryption: std::sync::Arc::new(
            arena_core::settings_encryption::SettingsEncryption::new(
                b"test-secret-key-for-settings-enc",
            ),
        ),
    };
    (db, state, server_id, session)
}

async fn seed_player(db: &DatabaseConnection, session_id: Uuid) -> Uuid {
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
        agent_connected: Set(false),
        agent_last_seen_at: Set(None),
    }
    .insert(db)
    .await
    .expect("player");
    id
}

#[tokio::test]
async fn connect_and_disconnect_are_recorded_with_a_last_seen_stamp() {
    let (db, state, _server, session) = setup().await;
    let player = seed_player(&db, session).await;

    set_agent_presence(&state, "PRSNC1", player, true).await;
    let row = players::Entity::find_by_id(player)
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    assert!(row.agent_connected, "connect recorded");
    let seen_at_connect = row
        .agent_last_seen_at
        .expect("last_seen stamped on connect");

    set_agent_presence(&state, "PRSNC1", player, false).await;
    let row = players::Entity::find_by_id(player)
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    assert!(!row.agent_connected, "disconnect recorded");
    assert!(
        row.agent_last_seen_at.expect("still stamped") >= seen_at_connect,
        "disconnect refreshes last_seen — the idle clock starts from it"
    );
}

#[tokio::test]
async fn startup_reset_clears_only_this_servers_stale_flags() {
    let (db, state, server_id, session) = setup().await;
    let mine = seed_player(&db, session).await;
    set_agent_presence(&state, "PRSNC1", mine, true).await;

    // A session owned by ANOTHER game server keeps its flags: that server's
    // sockets may be alive and well.
    let other_server = Uuid::new_v4();
    game_servers::ActiveModel {
        id: Set(other_server),
        url: Set("ws://elsewhere:8081".to_string()),
        zmq_url: Set(None),
        display_name: Set(None),
        capacity: Set(64),
        active_sessions: Set(0),
        status: Set("active".to_string()),
        last_heartbeat: Set(Utc::now()),
        created_at: Set(Utc::now()),
        updated_at: Set(Utc::now()),
    }
    .insert(&db)
    .await
    .expect("other server");
    let other_session = Uuid::new_v4();
    let project = sessions::Entity::find_by_id(session)
        .one(&db)
        .await
        .unwrap()
        .unwrap()
        .project_id_fk;
    sessions::ActiveModel {
        id: Set(other_session),
        name: Set("s2".to_string()),
        created_at: Set(Utc::now()),
        owner_id_fk: Set(None),
        status: Set(SessionStatus::Running),
        join_code: Set("PRSNC2".to_string()),
        started_at: Set(Some(Utc::now())),
        finished_at: Set(None),
        paused_at: Set(None),
        paused_duration_secs: Set(None),
        project_id_fk: Set(project),
        game_server_id: Set(Some(other_server)),
        cancel_reason: Set(None),
        cancelled_by: Set(None),
    }
    .insert(&db)
    .await
    .expect("other session");
    let theirs = seed_player(&db, other_session).await;
    set_agent_presence(&state, "PRSNC2", theirs, true).await;

    let reset = reset_agent_presence_on_startup(&db, server_id)
        .await
        .expect("reset");
    assert_eq!(reset, 1, "exactly the one stale flag this server owns");

    let mine_row = players::Entity::find_by_id(mine)
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    let theirs_row = players::Entity::find_by_id(theirs)
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    assert!(!mine_row.agent_connected, "own stale flag cleared");
    assert!(theirs_row.agent_connected, "other server's flag untouched");
}

/// The startup reset must also refresh `agent_last_seen_at`: the flipped
/// agents were connected until the process died, and the idle sweep measures
/// "empty since" from that column. A stale stamp makes a deploy instantly
/// exceed the idle timeout and cancel a live session before its CLI can
/// reconnect (session Y3W66Z was cancelled ~10s after a restart this way).
#[tokio::test]
async fn startup_reset_restarts_the_idle_clock() {
    let (db, state, server_id, session) = setup().await;
    let player = seed_player(&db, session).await;
    set_agent_presence(&state, "PRSNC1", player, true).await;

    // Age the stamp past any plausible idle timeout.
    let stale = Utc::now() - chrono::Duration::hours(2);
    let mut am: players::ActiveModel = players::Entity::find_by_id(player)
        .one(&db)
        .await
        .unwrap()
        .unwrap()
        .into();
    am.agent_last_seen_at = Set(Some(stale));
    am.update(&db).await.expect("age the stamp");

    let before = Utc::now();
    reset_agent_presence_on_startup(&db, server_id)
        .await
        .expect("reset");

    let row = players::Entity::find_by_id(player)
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    assert!(!row.agent_connected, "flag cleared");
    assert!(
        row.agent_last_seen_at.expect("stamp kept") >= before,
        "reset restarts the idle clock so the CLI gets its reconnect window"
    );
}
