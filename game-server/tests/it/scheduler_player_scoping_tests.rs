//! Player scoping of `session_scheduler_state`: the table holds one row per
//! (session, player) pair, and each joined player progresses through tasks
//! independently. Regression for the multi-player defect where the scheduler
//! queries filtered by session only, so all players shared/clobbered a single
//! row and one player's advancement moved everyone's task position.

use std::sync::Arc;

use arena_core::entities::{probes, projects, session_scheduler_state, sessions, tasks, users};
use arena_core::session_status::SessionStatus;
use chrono::Utc;
use dashmap::DashMap;
use game_server::state::GameServerState;
use game_server::ws::player_agent::scheduler::{
    advance_to_next_task, bootstrap_scheduler_state, ensure_adapted_test, pick_next_adapted_test,
    update_next_probe_at,
};
use game_server::zmq_pub::NoopEventPublisher;
use jsonwebtoken::{DecodingKey, EncodingKey};
use migration::MigratorTrait;
use sea_orm::{ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, Set};
use tokio::sync::Semaphore;
use uuid::Uuid;

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

async fn insert_player(
    db: &DatabaseConnection,
    session_id: Uuid,
    user_id: Uuid,
    name: &str,
) -> Uuid {
    arena_core::entities::players::ActiveModel {
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

async fn insert_user(db: &DatabaseConnection) -> Uuid {
    users::ActiveModel {
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
    .id
}

/// Insert user, project, session, two players, and two ordinal-ordered tasks.
/// Returns (session_id, player_a, player_b, task_ids).
async fn seed_two_players_two_tasks(db: &DatabaseConnection) -> (Uuid, Uuid, Uuid, Vec<Uuid>) {
    let user_id = insert_user(db).await;

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
        join_code: Set("PS1".to_string()),
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

/// Record a passing probe for a tests row on behalf of one player.
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

async fn scheduler_row(
    db: &DatabaseConnection,
    session_id: Uuid,
    player_id: Uuid,
) -> session_scheduler_state::Model {
    session_scheduler_state::Entity::find()
        .filter(session_scheduler_state::Column::SessionIdFk.eq(session_id))
        .filter(session_scheduler_state::Column::PlayerIdFk.eq(player_id))
        .one(db)
        .await
        .expect("query scheduler row")
        .expect("scheduler row exists for player")
}

#[tokio::test]
async fn each_player_gets_own_scheduler_row_and_advances_independently() {
    let db = setup_db().await;
    let state = test_state(db.clone());
    let (session_id, player_a, player_b, task_ids) = seed_two_players_two_tasks(&db).await;

    // Both players bootstrap on their first dispatch.
    let a_first = bootstrap_scheduler_state(&state, session_id, player_a, "PS1")
        .await
        .expect("bootstrap player a");
    let b_first = bootstrap_scheduler_state(&state, session_id, player_b, "PS1")
        .await
        .expect("bootstrap player b");
    assert_eq!(a_first, task_ids[0]);
    assert_eq!(b_first, task_ids[0]);

    // One row per (session, player) — not one shared row per session.
    let rows = session_scheduler_state::Entity::find()
        .filter(session_scheduler_state::Column::SessionIdFk.eq(session_id))
        .all(&db)
        .await
        .expect("query scheduler rows");
    assert_eq!(
        rows.len(),
        2,
        "two joined players must have two scheduler rows, got {}",
        rows.len()
    );
    assert_eq!(
        scheduler_row(&db, session_id, player_a).await.task_id,
        Some(task_ids[0])
    );
    assert_eq!(
        scheduler_row(&db, session_id, player_b).await.task_id,
        Some(task_ids[0])
    );

    // Player A completes task 0 and advances; player B must not move.
    let advanced = advance_to_next_task(&state, session_id, player_a, "PS1", task_ids[0]).await;
    assert!(advanced, "player a advances to the second task");

    let a_row = scheduler_row(&db, session_id, player_a).await;
    assert_eq!(a_row.task_id, Some(task_ids[1]), "player a is on task 1");
    let b_row = scheduler_row(&db, session_id, player_b).await;
    assert_eq!(
        b_row.task_id,
        Some(task_ids[0]),
        "player b's position must be untouched by player a's advancement"
    );
}

#[tokio::test]
async fn next_probe_at_is_scoped_to_the_player() {
    let db = setup_db().await;
    let state = test_state(db.clone());
    let (session_id, player_a, player_b, _task_ids) = seed_two_players_two_tasks(&db).await;

    bootstrap_scheduler_state(&state, session_id, player_a, "PS1")
        .await
        .expect("bootstrap player a");
    bootstrap_scheduler_state(&state, session_id, player_b, "PS1")
        .await
        .expect("bootstrap player b");

    let at = Utc::now();
    update_next_probe_at(&state, session_id, player_a, Some(at)).await;

    let a_row = scheduler_row(&db, session_id, player_a).await;
    assert_eq!(a_row.next_probe_at, Some(at), "player a's row is updated");
    let b_row = scheduler_row(&db, session_id, player_b).await;
    assert_eq!(
        b_row.next_probe_at, None,
        "player b's next_probe_at must not be touched by player a's schedule"
    );
}

#[tokio::test]
async fn probe_pass_by_one_player_does_not_complete_section_for_another() {
    let db = setup_db().await;
    let state = test_state(db.clone());
    let (session_id, player_a, player_b, task_ids) = seed_two_players_two_tasks(&db).await;

    let adapted = ensure_adapted_test(&state, task_ids[0], session_id)
        .await
        .expect("ensure ok")
        .expect("adapted test created");

    // Player A submits a passing probe for the task's only section.
    record_pass(&db, session_id, player_a, adapted.id).await;

    // Player A's own view: every section passed → task complete (None).
    assert!(
        pick_next_adapted_test(
            &state,
            &task_row(&db, task_ids[0]).await,
            session_id,
            player_a
        )
        .await
        .is_none(),
        "player a's own pass completes the task for player a"
    );

    // Player B never submitted anything — the section must still be pending
    // for B, not silently marked passed by A's probe.
    let picked_for_b = pick_next_adapted_test(
        &state,
        &task_row(&db, task_ids[0]).await,
        session_id,
        player_b,
    )
    .await
    .expect("db ok")
    .expect("section must still be pending for player b");
    assert_eq!(
        picked_for_b.id, adapted.id,
        "player b is still asked the section that only player a passed"
    );
}

/// The agent socket must drive the player the client identifies as — never
/// an arbitrary first row (which silently starves every other player in a
/// multi-player session).
#[tokio::test]
async fn socket_player_resolution_honors_requested_player() {
    let db = setup_db().await;
    let (session_id, player_a, player_b, _task_ids) = seed_two_players_two_tasks(&db).await;
    use game_server::ws::player_agent::socket::{SocketPlayer, resolve_socket_player};

    // Both seeded players belong to one user; resolve as that user.
    let owner = arena_core::entities::players::Entity::find_by_id(player_a)
        .one(&db)
        .await
        .unwrap()
        .unwrap()
        .user_id_fk
        .expect("seeded player has an owner");

    // A joiner identifying itself gets its own row — both of them.
    assert_eq!(
        resolve_socket_player(&db, session_id, owner, Some(player_b))
            .await
            .expect("resolve"),
        SocketPlayer::Resolved(player_b)
    );
    assert_eq!(
        resolve_socket_player(&db, session_id, owner, Some(player_a))
            .await
            .expect("resolve"),
        SocketPlayer::Resolved(player_a)
    );

    // A player id from outside this session must not fall back to another row.
    assert_eq!(
        resolve_socket_player(&db, session_id, owner, Some(Uuid::new_v4()))
            .await
            .expect("resolve"),
        SocketPlayer::NotFound
    );

    // A client with no id while the user has SEVERAL live rows must be
    // refused, not guessed at. Guessing here bound a second client to the
    // first player's row: both dispatched probes as one player, so that
    // player banked points earned on another machine while the second row
    // recorded nothing (session VKIBCB).
    assert_eq!(
        resolve_socket_player(&db, session_id, owner, None)
            .await
            .expect("resolve"),
        SocketPlayer::Ambiguous { live_players: 2 }
    );

    // A revoked player cannot be driven, even when requested explicitly.
    let row = arena_core::entities::players::Entity::find_by_id(player_b)
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    let mut am: arena_core::entities::players::ActiveModel = row.into();
    am.revoked_at = Set(Some(Utc::now()));
    am.update(&db).await.expect("revoke");
    assert_eq!(
        resolve_socket_player(&db, session_id, owner, Some(player_b))
            .await
            .expect("resolve"),
        SocketPlayer::NotFound
    );

    // …and with only one live player left, an id-less client is unambiguous
    // again, so the solo compatibility path keeps working.
    assert_eq!(
        resolve_socket_player(&db, session_id, owner, None)
            .await
            .expect("resolve"),
        SocketPlayer::Resolved(player_a)
    );
}

/// SEC-H1 regression: knowing the join code and a victim's `player_id` must
/// not be enough to drive their row — the row has to belong to the PAT's
/// user. Before this check, any caller could open the agent WS with
/// `?player_id=<victim>` and read the victim's probes / submit fake results.
#[tokio::test]
async fn socket_player_resolution_rejects_other_users_rows() {
    let db = setup_db().await;
    let (session_id, player_a, _player_b, _task_ids) = seed_two_players_two_tasks(&db).await;
    use game_server::ws::player_agent::socket::{SocketPlayer, resolve_socket_player};

    let stranger = insert_user(&db).await;

    // Naming the victim's player_id resolves nothing for a different user.
    assert_eq!(
        resolve_socket_player(&db, session_id, stranger, Some(player_a))
            .await
            .expect("resolve"),
        SocketPlayer::NotFound
    );

    // …and the id-less fallback never hands out someone else's row either.
    assert_eq!(
        resolve_socket_player(&db, session_id, stranger, None)
            .await
            .expect("resolve"),
        SocketPlayer::NotFound
    );

    // A member with their own row is unaffected.
    let member = insert_user(&db).await;
    let member_player = insert_player(&db, session_id, member, "carol").await;
    assert_eq!(
        resolve_socket_player(&db, session_id, member, Some(member_player))
            .await
            .expect("resolve"),
        SocketPlayer::Resolved(member_player)
    );
}
