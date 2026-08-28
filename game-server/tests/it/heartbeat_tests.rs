//! Coverage for src/heartbeat.rs: `register` (all three branches),
//! `send_heartbeat`, and `run_heartbeat`.

use arena_core::entities::{game_servers, sessions};
use arena_core::session_status::SessionStatus;
use chrono::Utc;
use game_server::heartbeat;
use migration::MigratorTrait;
use sea_orm::{ActiveModelTrait, DatabaseConnection, EntityTrait, Set};
use uuid::Uuid;

async fn setup_db() -> DatabaseConnection {
    let db = sea_orm::Database::connect("sqlite::memory:")
        .await
        .expect("connect");
    migration::Migrator::up(&db, None).await.expect("migrate");
    db
}

async fn clear_game_servers(db: &DatabaseConnection) {
    game_servers::Entity::delete_many().exec(db).await.ok();
}

async fn server_row(db: &DatabaseConnection, id: Uuid, url: &str, status: &str) {
    game_servers::ActiveModel {
        id: Set(id),
        url: Set(url.to_string()),
        zmq_url: Set(None),
        display_name: Set(None),
        capacity: Set(16),
        active_sessions: Set(0),
        status: Set(status.to_string()),
        last_heartbeat: Set(Utc::now()),
        created_at: Set(Utc::now()),
        updated_at: Set(Utc::now()),
    }
    .insert(db)
    .await
    .expect("insert game server");
}

async fn insert_user(db: &DatabaseConnection) -> Uuid {
    let id = Uuid::new_v4();
    arena_core::entities::users::ActiveModel {
        id: Set(id),
        email: Set(format!("u{id}@example.com")),
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
    .id
}

async fn insert_project(db: &DatabaseConnection) -> Uuid {
    let owner = insert_user(db).await;
    let id = Uuid::new_v4();
    arena_core::entities::projects::ActiveModel {
        id: Set(id),
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

async fn running_session(db: &DatabaseConnection, server_id: Uuid) {
    let project_id = insert_project(db).await;
    sessions::ActiveModel {
        id: Set(Uuid::new_v4()),
        name: Set("s".to_string()),
        created_at: Set(Utc::now()),
        owner_id_fk: Set(None),
        status: Set(SessionStatus::Running),
        join_code: Set(Uuid::new_v4().to_string()),
        started_at: Set(Some(Utc::now())),
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
}

async fn status_of(db: &DatabaseConnection, id: Uuid) -> String {
    game_servers::Entity::find_by_id(id)
        .one(db)
        .await
        .unwrap()
        .map(|r| r.status)
        .unwrap_or_default()
}

#[tokio::test]
async fn register_inserts_new_server_when_absent() {
    let db = setup_db().await;
    clear_game_servers(&db).await;
    let id = Uuid::new_v4();
    heartbeat::register(&db, id, "ws://new", 8, Some("tcp://x:1"))
        .await
        .expect("register");
    assert_eq!(status_of(&db, id).await, "active");
    let row = game_servers::Entity::find_by_id(id)
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.url, "ws://new");
    assert_eq!(row.capacity, 8);
    assert_eq!(row.zmq_url.as_deref(), Some("tcp://x:1"));
}

#[tokio::test]
async fn register_preserves_admin_overridden_status_by_id() {
    let db = setup_db().await;
    clear_game_servers(&db).await;
    let id = Uuid::new_v4();
    server_row(&db, id, "ws://admin", "draining").await;
    heartbeat::register(&db, id, "ws://admin2", 8, None)
        .await
        .expect("register");
    // Status preserved (admin override), URL still updated.
    assert_eq!(status_of(&db, id).await, "draining");
    let row = game_servers::Entity::find_by_id(id)
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.url, "ws://admin2");
}

#[tokio::test]
async fn register_reactivates_when_status_active_by_id() {
    let db = setup_db().await;
    clear_game_servers(&db).await;
    let id = Uuid::new_v4();
    server_row(&db, id, "ws://norm", "active").await;
    heartbeat::register(&db, id, "ws://norm2", 4, None)
        .await
        .expect("register");
    assert_eq!(status_of(&db, id).await, "active");
    let row = game_servers::Entity::find_by_id(id)
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.url, "ws://norm2");
    assert_eq!(row.capacity, 4);
}

#[tokio::test]
async fn register_replaces_existing_by_url_with_preserved_status() {
    let db = setup_db().await;
    clear_game_servers(&db).await;
    let old = Uuid::new_v4();
    let new = Uuid::new_v4();
    // Different id, same url as the to-be-registered server.
    server_row(&db, old, "ws://dup", "inactive").await;
    heartbeat::register(&db, new, "ws://dup", 8, None)
        .await
        .expect("register");
    // Old row gone, new row present, status preserved ("inactive" is admin-overridden).
    assert!(
        game_servers::Entity::find_by_id(old)
            .one(&db)
            .await
            .unwrap()
            .is_none()
    );
    let row = game_servers::Entity::find_by_id(new)
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.url, "ws://dup");
    assert_eq!(row.status, "inactive");
}

#[tokio::test]
async fn register_replaces_existing_by_url_activates_non_overridden() {
    let db = setup_db().await;
    clear_game_servers(&db).await;
    let old = Uuid::new_v4();
    let new = Uuid::new_v4();
    server_row(&db, old, "ws://dup2", "active").await;
    heartbeat::register(&db, new, "ws://dup2", 8, None)
        .await
        .expect("register");
    let row = game_servers::Entity::find_by_id(new)
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.status, "active");
}

#[tokio::test]
async fn send_heartbeat_updates_active_count() {
    let db = setup_db().await;
    clear_game_servers(&db).await;
    let id = Uuid::new_v4();
    server_row(&db, id, "ws://hb", "active").await;
    running_session(&db, id).await;
    running_session(&db, id).await;

    heartbeat::send_heartbeat(&db, id)
        .await
        .expect("send_heartbeat");

    let row = game_servers::Entity::find_by_id(id)
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.active_sessions, 2);
    // last_heartbeat refreshed (within last minute).
    assert!(
        Utc::now()
            .signed_duration_since(row.last_heartbeat)
            .num_seconds()
            < 60
    );
}

#[tokio::test]
async fn send_heartbeat_noop_when_server_absent() {
    let db = setup_db().await;
    clear_game_servers(&db).await;
    // No row → should be a no-op, not an error.
    heartbeat::send_heartbeat(&db, Uuid::new_v4())
        .await
        .expect("send_heartbeat noop");
}

#[tokio::test]
async fn run_heartbeat_ticks_and_updates() {
    let db = setup_db().await;
    clear_game_servers(&db).await;
    let id = Uuid::new_v4();
    server_row(&db, id, "ws://run", "active").await;
    running_session(&db, id).await;

    let hb_db = db.clone();
    // Drive a couple of ticks then abort.
    let handle = tokio::spawn(async move {
        heartbeat::run_heartbeat(hb_db, id).await;
    });
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    handle.abort();

    let row = game_servers::Entity::find_by_id(id)
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.active_sessions, 1);
}
