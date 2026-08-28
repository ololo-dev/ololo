use arena_core::session_status::SessionStatus;
use chrono::Utc;
use migration::{Migrator, MigratorTrait};
use sea_orm::{ActiveModelTrait, Database, EntityTrait, Set};
use uuid::Uuid;

use super::*;

async fn fresh_state() -> AppState {
    let db = Database::connect("sqlite::memory:")
        .await
        .expect("db connect");
    Migrator::up(&db, None).await.expect("migrate");
    let cfg = crate::AuthConfig {
        jwt_signing_key: b"sessions-tests-secret-key-32-bytes!".to_vec(),
        frontend_origins: vec!["http://localhost:5173".to_string()],
        access_ttl: std::time::Duration::from_secs(900),
        refresh_ttl: std::time::Duration::from_secs(86400),
        max_agents_per_session: 16,
    };
    AppState::new(db, cfg)
}

/// A user, a project with a 1-second session duration, and a session that
/// started 10s ago — i.e. one already past its deadline on the first tick.
async fn expired_session(state: &AppState, join_code: &str, owner: Option<Uuid>) -> (Uuid, Uuid) {
    let db = &state.db;
    let user_id = Uuid::new_v4();
    arena_core::entities::users::ActiveModel {
        id: Set(user_id),
        email: Set(format!("{join_code}@test.local")),
        password_hash: Set(None),
        display_name: Set("Timer".to_string()),
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
    arena_core::entities::projects::ActiveModel {
        id: Set(project_id),
        name: Set("timer-proj".to_string()),
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
        default_session_duration_secs: Set(1),
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
    arena_core::entities::sessions::ActiveModel {
        id: Set(session_id),
        name: Set("timer-s".to_string()),
        created_at: Set(Utc::now()),
        owner_id_fk: Set(Some(user_id)),
        status: Set(SessionStatus::Running),
        join_code: Set(join_code.to_string()),
        started_at: Set(Some(Utc::now() - chrono::Duration::seconds(10))),
        finished_at: Set(None),
        paused_at: Set(None),
        paused_duration_secs: Set(None),
        project_id_fk: Set(project_id),
        game_server_id: Set(owner),
        cancel_reason: Set(None),
        cancelled_by: Set(None),
    }
    .insert(db)
    .await
    .expect("insert session");

    (session_id, project_id)
}

async fn insert_game_server(state: &AppState, heartbeat: chrono::DateTime<Utc>) -> Uuid {
    let id = Uuid::new_v4();
    arena_core::entities::game_servers::ActiveModel {
        id: Set(id),
        url: Set(format!("wss://gs-{id}.test")),
        zmq_url: Set(None),
        display_name: Set(None),
        capacity: Set(10),
        active_sessions: Set(0),
        status: Set("active".to_string()),
        last_heartbeat: Set(heartbeat),
        created_at: Set(Utc::now()),
        updated_at: Set(Utc::now()),
    }
    .insert(&state.db)
    .await
    .expect("insert game server");
    id
}

async fn status_of(state: &AppState, session_id: Uuid) -> SessionStatus {
    arena_core::entities::sessions::Entity::find_by_id(session_id)
        .one(&state.db)
        .await
        .expect("query session")
        .expect("session row")
        .status
}

#[tokio::test]
async fn an_owned_session_is_left_for_its_game_server_to_finish() {
    // The game server's ending is not a status write: interrupted-task
    // judges, session-scoped judges, the settle poll and the Arena Points
    // award all hang off it. Winning that race here skips every one of them.
    let state = fresh_state().await;
    let owner = insert_game_server(&state, Utc::now()).await;
    let (session_id, project_id) = expired_session(&state, "TMR002", Some(owner)).await;

    tokio::spawn(running_timer(
        state.clone(),
        session_id,
        "TMR002".to_string(),
        project_id,
    ));
    tokio::time::sleep(std::time::Duration::from_secs(3)).await;

    assert_eq!(
        status_of(&state, session_id).await,
        SessionStatus::Running,
        "the owning game server must be the one to finish its session"
    );
}

#[tokio::test]
async fn a_session_whose_owner_stopped_heartbeating_is_finished_here() {
    // The deference has to stop somewhere: a dead owner would otherwise
    // strand the session in `running` forever.
    let state = fresh_state().await;
    let stale = Utc::now() - chrono::Duration::seconds(600);
    let owner = insert_game_server(&state, stale).await;
    let (session_id, project_id) = expired_session(&state, "TMR003", Some(owner)).await;

    tokio::spawn(running_timer(
        state.clone(),
        session_id,
        "TMR003".to_string(),
        project_id,
    ));

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        if status_of(&state, session_id).await == SessionStatus::Finished {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "a session whose owner is presumed dead must still be finished"
        );
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
}

#[tokio::test]
async fn the_grace_ends_and_a_wedged_owner_does_not_strand_the_session() {
    // Owner alive, but its own timer never fired. Waiting forever would be
    // worse than an ending that skipped the judge chain — the game server's
    // recovery sweeps re-drive missed judge runs and unawarded sessions.
    let state = fresh_state().await;
    let owner = insert_game_server(&state, Utc::now()).await;
    let (session_id, _) = expired_session(&state, "TMR004", Some(owner)).await;

    assert!(
        !wait_for_owner_to_finish(
            &state,
            session_id,
            Some(owner),
            std::time::Duration::from_millis(50)
        )
        .await,
        "the grace must expire and hand the ending back to the caller"
    );
}

#[tokio::test]
async fn running_timer_finishes_session_per_project_duration() {
    let state = fresh_state().await;
    let db = &state.db;

    let user_id = Uuid::new_v4();
    arena_core::entities::users::ActiveModel {
        id: Set(user_id),
        email: Set("timer@test.local".to_string()),
        password_hash: Set(None),
        display_name: Set("Timer".to_string()),
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
    arena_core::entities::projects::ActiveModel {
        id: Set(project_id),
        name: Set("timer-proj".to_string()),
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
        // A 1-second session: with started_at 10s in the past the timer must
        // finish the session on its first tick if it honors the project value.
        default_session_duration_secs: Set(1),
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
    arena_core::entities::sessions::ActiveModel {
        id: Set(session_id),
        name: Set("timer-s".to_string()),
        created_at: Set(Utc::now()),
        owner_id_fk: Set(Some(user_id)),
        status: Set(SessionStatus::Running),
        join_code: Set("TMR001".to_string()),
        started_at: Set(Some(Utc::now() - chrono::Duration::seconds(10))),
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

    tokio::spawn(running_timer(
        state.clone(),
        session_id,
        "TMR001".to_string(),
        project_id,
    ));

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        let model = arena_core::entities::sessions::Entity::find_by_id(session_id)
            .one(db)
            .await
            .expect("query session")
            .expect("session row");
        if model.status == SessionStatus::Finished {
            assert!(model.finished_at.is_some(), "finished_at must be set");
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "running_timer did not finish the session within 5s: \
             expected it to honor the project's default_session_duration_secs (1s), \
             session still {:?}",
            model.status
        );
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
}
