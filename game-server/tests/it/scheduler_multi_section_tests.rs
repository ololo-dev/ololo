//! Multi-section task templates: every `##` section must become its own
//! tests row (probe type), and the scheduler must ask them one by one in
//! ordinal order. Regression for the "1/1 types passed" bug where the
//! game-server's lazy fallback only created the first section.

use std::sync::Arc;

use arena_core::entities::{probes, projects, sessions, tasks, tests, users};
use arena_core::session_status::SessionStatus;
use chrono::Utc;
use dashmap::DashMap;
use game_server::state::GameServerState;
use game_server::ws::player_agent::scheduler::{ensure_adapted_test, pick_next_adapted_test};
use game_server::zmq_pub::NoopEventPublisher;
use jsonwebtoken::{DecodingKey, EncodingKey};
use migration::MigratorTrait;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, QueryOrder, Set,
};
use tokio::sync::Semaphore;
use uuid::Uuid;

const MULTI_SECTION_MD: &str = r#"## Start the server and answer a city question

```js fixtures
({ q: "city" })
```

```sh command
echo city
```

```js validation
result.trim() === "city"
```

## Answer a colour question

```js fixtures
({ q: "colour" })
```

```sh command
echo colour
```

```js validation
result.trim() === "colour"
```

## Answer a famous-person question

```js fixtures
({ q: "person" })
```

```sh command
echo person
```

```js validation
result.trim() === "person"
```
"#;

async fn task_row(db: &DatabaseConnection, task_id: Uuid) -> arena_core::entities::tasks::Model {
    arena_core::entities::tasks::Entity::find_by_id(task_id)
        .one(db)
        .await
        .expect("db ok")
        .expect("task row")
}

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

/// Insert user, project, session, player, and one task whose template body
/// is the multi-section markdown. Returns (session_id, player_id, task_id).
async fn seed_multi_section_task(db: &DatabaseConnection) -> (Uuid, Uuid, Uuid) {
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
        created_at: Set(Utc::now()),
        owner_id_fk: Set(None),
        status: Set(SessionStatus::Running),
        join_code: Set("MS1".to_string()),
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

    let player_id = arena_core::entities::players::ActiveModel {
        id: Set(Uuid::new_v4()),
        session_id_fk: Set(session_id),
        user_id_fk: Set(Some(user_id)),
        display_name: Set("tester".to_string()),
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
    .id;

    let template = serde_json::json!({
        "kind": "shell",
        "command_template": MULTI_SECTION_MD,
    });
    let task_id = tasks::ActiveModel {
        id: Set(Uuid::new_v4()),
        project_id_fk: Set(project_id),
        ordinal: Set(0),
        title: Set("General knowledge".to_string()),
        content: Set("answer trivia".to_string()),
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

    (session_id, player_id, task_id)
}

/// Record a passing probe for a tests row so the scheduler treats the
/// section as done.
async fn record_pass(db: &DatabaseConnection, session_id: Uuid, player_id: Uuid, test_id: Uuid) {
    probes::ActiveModel {
        id: Set(Uuid::new_v4()),
        test_id: Set(test_id),
        player_id: Set(player_id),
        session_id: Set(session_id),
        attempt: Set(1),
        rendered_command: Set("echo".to_string()),
        fixture_values: Set("{}".to_string()),
        expected_answer: Set(None),
        resolved_answer: Set(None),
        secret_meta: Set(None),
        outcome: Set(Some("pass".to_string())),
        dispatched_at: Set(Utc::now()),
        deadline_at: Set(Utc::now()),
        resolved_at: Set(Some(Utc::now())),
        updated_at: Set(Some(Utc::now())),
        output: Set(None),
        exit_code: Set(Some(0)),
        duration_ms: Set(Some(1)),
        point_delta: Set(Some(10)),
        artifact_path: Set(None),
        result_json: Set(None),
    }
    .insert(db)
    .await
    .expect("insert probe");
}

#[tokio::test]
async fn ensure_adapted_test_creates_one_row_per_section() {
    let db = setup_db().await;
    let state = test_state(db.clone());
    let (session_id, _player_id, task_id) = seed_multi_section_task(&db).await;

    let first = ensure_adapted_test(&state, task_id, session_id)
        .await
        .expect("ensure ok")
        .expect("first test returned");
    assert_eq!(first.ordinal, 0, "returns the first section");
    assert!(first.command_template.contains("echo city"));

    let rows = tests::Entity::find()
        .filter(tests::Column::TaskId.eq(task_id))
        .filter(tests::Column::SessionId.eq(session_id))
        .order_by_asc(tests::Column::Ordinal)
        .all(&db)
        .await
        .expect("query tests");
    assert_eq!(rows.len(), 3, "one tests row per ## section");
    assert!(rows[0].command_template.contains("echo city"));
    assert!(rows[1].command_template.contains("echo colour"));
    assert!(rows[2].command_template.contains("echo person"));
    assert_eq!(
        rows.iter().map(|r| r.ordinal).collect::<Vec<_>>(),
        vec![0, 1, 2]
    );

    // Idempotent: a second call returns the existing first row, no dupes.
    let again = ensure_adapted_test(&state, task_id, session_id)
        .await
        .expect("ensure ok")
        .expect("existing row");
    assert_eq!(again.id, rows[0].id);
    let count = tests::Entity::find()
        .filter(tests::Column::TaskId.eq(task_id))
        .filter(tests::Column::SessionId.eq(session_id))
        .all(&db)
        .await
        .expect("query tests")
        .len();
    assert_eq!(count, 3);
}

#[tokio::test]
async fn sections_are_asked_one_by_one_in_order() {
    let db = setup_db().await;
    let state = test_state(db.clone());
    let (session_id, player_id, task_id) = seed_multi_section_task(&db).await;

    ensure_adapted_test(&state, task_id, session_id)
        .await
        .expect("ensure ok");
    let rows = tests::Entity::find()
        .filter(tests::Column::TaskId.eq(task_id))
        .filter(tests::Column::SessionId.eq(session_id))
        .order_by_asc(tests::Column::Ordinal)
        .all(&db)
        .await
        .expect("query tests");
    assert_eq!(rows.len(), 3);

    // Nothing passed yet → section 0 is asked (repeatedly, until it passes).
    for _ in 0..3 {
        let picked =
            pick_next_adapted_test(&state, &task_row(&db, task_id).await, session_id, player_id)
                .await
                .expect("db ok")
                .expect("a section to ask");
        assert_eq!(picked.id, rows[0].id, "first un-passed section is asked");
    }

    // Section 0 passes → section 1 is asked next.
    record_pass(&db, session_id, player_id, rows[0].id).await;
    let picked =
        pick_next_adapted_test(&state, &task_row(&db, task_id).await, session_id, player_id)
            .await
            .expect("db ok")
            .expect("a section to ask");
    assert_eq!(picked.id, rows[1].id);

    // Sections 1 and 2 pass → the task is done (None = advance).
    record_pass(&db, session_id, player_id, rows[1].id).await;
    record_pass(&db, session_id, player_id, rows[2].id).await;
    assert!(
        pick_next_adapted_test(&state, &task_row(&db, task_id).await, session_id, player_id)
            .await
            .is_none(),
        "all sections passed → task complete"
    );
}
