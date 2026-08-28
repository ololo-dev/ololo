use arena_core::entities::activity_event;
use chrono::Utc;
use migration::{Migrator, MigratorTrait};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect, Set,
};
use server::AppState;
use server::entities::{players, projects, sessions, tasks, users};
use uuid::Uuid;

const ORIGIN: &str = "http://localhost:5173";

async fn test_state() -> AppState {
    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("test.db");
    let url = format!("sqlite://{}?mode=rwc", db_path.display());
    let db = sea_orm::Database::connect(&url)
        .await
        .expect("sqlite connect");
    Migrator::up(&db, None).await.expect("migrate up");
    std::mem::forget(dir);
    let cfg = server::AuthConfig {
        jwt_signing_key: b"integration-test-secret-32-bytes-or-more-xxxxxxx".to_vec(),
        frontend_origins: vec![ORIGIN.to_string()],
        access_ttl: std::time::Duration::from_secs(900),
        refresh_ttl: std::time::Duration::from_secs(30 * 86_400),
        max_agents_per_session: 16,
    };
    AppState::new(db, cfg)
}

#[tokio::test]
async fn activity_event_filter_by_session_id_returns_rows() {
    let state = test_state().await;
    let owner = Uuid::new_v4();
    users::ActiveModel {
        id: Set(owner),
        email: Set(format!("o-{owner}@x.com")),
        password_hash: Set(None),
        display_name: Set("O".to_string()),
        created_at: Set(Utc::now()),
        updated_at: Set(Utc::now()),
        is_admin: Set(true),
        avatar_url: Set(None),
        email_verified: Set(false),
        username: Set(None),
        plan: Set(arena_core::quota::PLAN_PREMIUM.to_string()),
        judge_run_limit: Set(None),
        judge_run_credits: Set(0),
    }
    .insert(&state.db)
    .await
    .unwrap();

    let project_id = Uuid::new_v4();
    projects::ActiveModel {
        id: Set(project_id),
        name: Set("P".to_string()),
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
    .insert(&state.db)
    .await
    .unwrap();

    let session_id = Uuid::new_v4();
    sessions::ActiveModel {
        id: Set(session_id),
        name: Set("S".to_string()),
        created_at: Set(Utc::now()),
        owner_id_fk: Set(None),
        status: Set(arena_core::session_status::SessionStatus::Running),
        join_code: Set("REPABCD".to_string()),
        game_server_id: Set(None),
        cancel_reason: Set(None),
        cancelled_by: Set(None),
        started_at: Set(Some(Utc::now())),
        finished_at: Set(None),
        paused_at: Set(None),
        paused_duration_secs: Set(None),
        project_id_fk: Set(project_id),
    }
    .insert(&state.db)
    .await
    .unwrap();

    let player_id = Uuid::new_v4();
    players::ActiveModel {
        id: Set(player_id),
        session_id_fk: Set(session_id),
        user_id_fk: Set(Some(owner)),
        display_name: Set("Alpha".to_string()),
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
    .unwrap();

    let task_id = Uuid::new_v4();
    tasks::ActiveModel {
        id: Set(task_id),
        project_id_fk: Set(project_id),
        ordinal: Set(1),
        title: Set("Task One".to_string()),
        content: Set(String::new()),
        test_template: Set(serde_json::json!({ "kind": "shell", "command_template": "echo ok" })),
        created_at: Set(Utc::now()),
        tags: Set(String::new()),
        point_value: Set(10),
        deadline_secs: Set(Some(300)),
        min_interval_secs: Set(Some(5)),
        interval_increment_secs: Set(Some(0)),
        max_interval_secs: Set(Some(300)),
        fail_points: Set(0),
        no_response_points: Set(0),
        completion_bonus_points: Set(10),
        evaluation: Set(None),
    }
    .insert(&state.db)
    .await
    .unwrap();

    // Insert activity_event via sea-orm (same path as game-server).
    activity_event::ActiveModel {
        id: Set(Uuid::new_v4()),
        session_id_fk: Set(session_id),
        player_id_fk: Set(player_id),
        task_id_fk: Set(task_id),
        event_kind: Set("task_scored".to_string()),
        task_ordinal: Set(1),
        task_title: Set("Task One".to_string()),
        player_display_name: Set("Alpha".to_string()),
        judge_name: Set(None),
        point_delta: Set(Some(10)),
        timestamp: Set(Utc::now()),
        version: Set(1i64),
        detail: Set(None),
    }
    .insert(&state.db)
    .await
    .unwrap();

    // Query via the same pattern as get_activity / get_report.
    let rows = activity_event::Entity::find()
        .filter(activity_event::Column::SessionIdFk.eq(session_id))
        .order_by_desc(activity_event::Column::Timestamp)
        .limit(500)
        .all(&state.db)
        .await
        .unwrap_or_default();

    assert_eq!(rows.len(), 1, "expected 1 activity_event row, got 0");
    assert_eq!(rows[0].event_kind, "task_scored");
}
