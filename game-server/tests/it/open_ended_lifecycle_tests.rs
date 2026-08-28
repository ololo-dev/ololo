//! Open-ended lifecycle: the completion contract replaces the all-sections
//! gate. The completion probe is dispatched (other sections do not gate),
//! its pass completes the task, and the work deadline force-completes it.

use std::sync::Arc;

use arena_core::entities::{
    activity_event, players, probes, projects, sessions, tasks, tests, users,
};
use arena_core::session_status::SessionStatus;
use chrono::{Duration, Utc};
use dashmap::DashMap;
use game_server::state::GameServerState;
use game_server::ws::player_agent::scheduler::{
    ensure_adapted_test, open_ended_state, pick_next_adapted_test,
};
use game_server::zmq_pub::NoopEventPublisher;
use jsonwebtoken::{DecodingKey, EncodingKey};
use migration::MigratorTrait;
use sea_orm::{ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, Set};
use tokio::sync::Semaphore;
use uuid::Uuid;

/// Two sections; the completion probe is deliberately the SECOND one, so the
/// title→ordinal mapping is exercised (classic gating would ask section 0
/// first and never reach completion without passing it).
const OPEN_ENDED_MD: &str = r#"## TODO progress

```js fixtures
({})
```

```sh command
cat TODO.md
```

## Definition of done

```js fixtures
({})
```

```sh command
test -f TODO.md && echo done
```

```js validation
result.trim() === "done"
```
"#;

fn evaluation_json(deadline_secs: i64) -> serde_json::Value {
    serde_json::json!({
        "kind": "open_ended",
        "completion": { "probe": "Definition of done", "deadline_secs": deadline_secs },
        "criteria": [ { "key": "product", "weight": 1.0 } ]
    })
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

async fn seed_open_ended(
    db: &DatabaseConnection,
    deadline_secs: i64,
) -> (Uuid, Uuid, tasks::Model) {
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
    .expect("user")
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
    .expect("project")
    .id;

    let session_id = sessions::ActiveModel {
        id: Set(Uuid::new_v4()),
        name: Set("s".to_string()),
        created_at: Set(Utc::now()),
        owner_id_fk: Set(None),
        status: Set(SessionStatus::Running),
        join_code: Set(format!("O{}", &Uuid::new_v4().simple().to_string()[..5]).to_uppercase()),
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
    .expect("session")
    .id;

    let player_id = players::ActiveModel {
        id: Set(Uuid::new_v4()),
        session_id_fk: Set(session_id),
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
    .insert(db)
    .await
    .expect("player")
    .id;

    let task = tasks::ActiveModel {
        id: Set(Uuid::new_v4()),
        project_id_fk: Set(project_id),
        ordinal: Set(0),
        title: Set("Build the thing".to_string()),
        content: Set("Open-ended".to_string()),
        test_template: Set(serde_json::json!({
            "kind": "shell",
            "command_template": OPEN_ENDED_MD,
        })),
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
        evaluation: Set(Some(evaluation_json(deadline_secs))),
    }
    .insert(db)
    .await
    .expect("task");

    (session_id, player_id, task)
}

async fn record_task_started(
    db: &DatabaseConnection,
    session_id: Uuid,
    player_id: Uuid,
    task: &tasks::Model,
    at: chrono::DateTime<Utc>,
) {
    activity_event::ActiveModel {
        id: Set(Uuid::new_v4()),
        session_id_fk: Set(session_id),
        player_id_fk: Set(player_id),
        task_id_fk: Set(task.id),
        event_kind: Set("task_started".to_string()),
        task_ordinal: Set(task.ordinal),
        task_title: Set(task.title.clone()),
        player_display_name: Set("player".to_string()),
        judge_name: Set(None),
        point_delta: Set(None),
        timestamp: Set(at),
        version: Set(1),
        detail: Set(None),
    }
    .insert(db)
    .await
    .expect("activity event");
}

async fn record_pass(db: &DatabaseConnection, session_id: Uuid, player_id: Uuid, test_id: Uuid) {
    probes::ActiveModel {
        id: Set(Uuid::new_v4()),
        test_id: Set(test_id),
        player_id: Set(player_id),
        session_id: Set(session_id),
        attempt: Set(1),
        rendered_command: Set("x".to_string()),
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
        point_delta: Set(Some(0)),
        result_json: Set(None),
        artifact_path: Set(None),
    }
    .insert(db)
    .await
    .expect("probe");
}

#[tokio::test]
async fn completion_probe_gates_the_task_not_the_other_sections() {
    let db = setup_db().await;
    let state = test_state(db.clone());
    let (session_id, player_id, task) = seed_open_ended(&db, 3600).await;
    record_task_started(&db, session_id, player_id, &task, Utc::now()).await;

    ensure_adapted_test(&state, task.id, session_id)
        .await
        .expect("db ok");
    let rows: Vec<tests::Model> = tests::Entity::find()
        .filter(tests::Column::TaskId.eq(task.id))
        .all(&db)
        .await
        .unwrap();
    assert_eq!(rows.len(), 2, "both sections materialize");

    // The completion probe (ordinal 1) is what gets dispatched — the
    // unpassed TODO-progress section (ordinal 0) does not gate.
    let picked = pick_next_adapted_test(&state, &task, session_id, player_id)
        .await
        .expect("db ok")
        .expect("a probe to ask");
    assert_eq!(picked.ordinal, 1, "completion section is dispatched");

    // Completion probe passes → the task completes, section 0 still unpassed.
    let completion_row = rows.iter().find(|r| r.ordinal == 1).unwrap();
    record_pass(&db, session_id, player_id, completion_row.id).await;
    assert!(
        pick_next_adapted_test(&state, &task, session_id, player_id)
            .await
            .is_none(),
        "completion probe pass completes the task"
    );

    let oe = open_ended_state(&state, &task, session_id, player_id)
        .await
        .expect("state");
    assert!(oe.completion_passed);
    assert!(!oe.deadline_expired);
    assert_eq!(oe.contract.completion.probe, "Definition of done");
}

#[tokio::test]
async fn expired_deadline_force_completes_without_a_pass() {
    let db = setup_db().await;
    let state = test_state(db.clone());
    let (session_id, player_id, task) = seed_open_ended(&db, 60).await;
    // Started 10 minutes ago with a 60s window: long gone.
    record_task_started(
        &db,
        session_id,
        player_id,
        &task,
        Utc::now() - Duration::seconds(600),
    )
    .await;

    ensure_adapted_test(&state, task.id, session_id)
        .await
        .expect("db ok");

    assert!(
        pick_next_adapted_test(&state, &task, session_id, player_id)
            .await
            .is_none(),
        "expired deadline completes the task"
    );
    let oe = open_ended_state(&state, &task, session_id, player_id)
        .await
        .expect("state");
    assert!(!oe.completion_passed, "the probe never passed");
    assert!(oe.deadline_expired, "the window is the reason");
}

#[tokio::test]
async fn unstarted_task_is_not_deadline_expired() {
    let db = setup_db().await;
    let state = test_state(db.clone());
    let (session_id, player_id, task) = seed_open_ended(&db, 60).await;
    // No task_started event at all: the window has not opened.
    ensure_adapted_test(&state, task.id, session_id)
        .await
        .expect("db ok");

    let oe = open_ended_state(&state, &task, session_id, player_id)
        .await
        .expect("state");
    assert!(!oe.deadline_expired);
    let picked = pick_next_adapted_test(&state, &task, session_id, player_id)
        .await
        .expect("db ok")
        .expect("a probe to ask");
    assert_eq!(picked.ordinal, 1);
}

#[tokio::test]
async fn classic_task_gating_is_untouched() {
    let db = setup_db().await;
    let state = test_state(db.clone());
    let (session_id, player_id, task) = seed_open_ended(&db, 3600).await;
    // Strip the contract: same sections, classic rules.
    let mut am: tasks::ActiveModel = task.clone().into();
    am.evaluation = Set(None);
    let task = arena_core::entities::tasks::Entity::update(am)
        .exec(&db)
        .await
        .expect("update");

    ensure_adapted_test(&state, task.id, session_id)
        .await
        .expect("db ok");
    let picked = pick_next_adapted_test(&state, &task, session_id, player_id)
        .await
        .expect("db ok")
        .expect("a probe to ask");
    assert_eq!(picked.ordinal, 0, "classic rule asks the first section");
}
