// ENV_LOCK guards are held across awaits on purpose: they serialize the
// tests that mutate process-global env vars, and the guarded sections span
// the awaited probe runs.
#![allow(clippy::await_holding_lock)]

//! The server-probe ticker: due-scan over open-ended tasks, interval
//! semantics computed from probe history (restart-safe by construction),
//! and the participant-side due-pick used by the agent loop.

use std::sync::{Arc, Mutex};

use arena_core::entities::{players, probes, projects, sessions, tasks, tests, users};
use arena_core::session_status::SessionStatus;
use chrono::{Duration, Utc};
use dashmap::DashMap;
use game_server::state::GameServerState;
use game_server::ws::player_agent::scheduler::most_overdue_participant_section;
use game_server::zmq_pub::NoopEventPublisher;
use jsonwebtoken::{DecodingKey, EncodingKey};
use migration::MigratorTrait;
use sea_orm::{ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, Set};
use tokio::sync::Semaphore;
use uuid::Uuid;

static ENV_LOCK: Mutex<()> = Mutex::new(());

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

struct Seeded {
    session_id: Uuid,
    player_id: Uuid,
    task: tasks::Model,
}

async fn seed(db: &DatabaseConnection, game_server_id: Option<Uuid>) -> Seeded {
    if let Some(gs_id) = game_server_id {
        arena_core::entities::game_servers::ActiveModel {
            id: Set(gs_id),
            url: Set("ws://localhost:8081".to_string()),
            zmq_url: Set(None),
            display_name: Set(None),
            capacity: Set(10),
            active_sessions: Set(0),
            status: Set("active".to_string()),
            last_heartbeat: Set(Utc::now()),
            created_at: Set(Utc::now()),
            updated_at: Set(Utc::now()),
        }
        .insert(db)
        .await
        .expect("game server row");
    }
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
        join_code: Set(format!("P{}", &Uuid::new_v4().simple().to_string()[..5]).to_uppercase()),
        started_at: Set(Some(Utc::now())),
        finished_at: Set(None),
        paused_at: Set(None),
        paused_duration_secs: Set(None),
        project_id_fk: Set(project_id),
        game_server_id: Set(game_server_id),
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
        title: Set("Build".to_string()),
        content: Set(String::new()),
        test_template: Set(serde_json::json!({"kind":"shell","command_template":"## X\n"})),
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
        evaluation: Set(Some(serde_json::json!({
            "kind": "open_ended",
            "completion": { "probe": "X", "deadline_secs": 3600 },
            "criteria": [ { "key": "k", "weight": 1.0 } ]
        }))),
    }
    .insert(db)
    .await
    .expect("task");

    arena_core::entities::session_scheduler_state::ActiveModel {
        id: Set(Uuid::new_v4()),
        session_id_fk: Set(session_id),
        player_id_fk: Set(player_id),
        task_id: Set(Some(task.id)),
        state: Set("idle".to_string()),
        next_probe_at: Set(None),
        created_at: Set(Utc::now()),
        updated_at: Set(Utc::now()),
    }
    .insert(db)
    .await
    .expect("cursor");

    Seeded {
        session_id,
        player_id,
        task,
    }
}

async fn insert_probe_test(
    db: &DatabaseConnection,
    session_id: Uuid,
    task_id: Uuid,
    ordinal: i32,
    config: serde_json::Value,
) -> tests::Model {
    tests::ActiveModel {
        id: Set(Uuid::new_v4()),
        command_template: Set("echo measured".to_string()),
        answer_template: Set(String::new()),
        fixture_definitions: Set(r#"{"kind":"js","script":"({})"}"#.to_string()),
        created_at: Set(Utc::now()),
        session_id: Set(session_id),
        task_id: Set(task_id),
        ordinal: Set(ordinal),
        prompt: Set(String::new()),
        description: Set(None),
        probe_config: Set(Some(config)),
        initiator: Set("system".to_string()),
        registered_by_judge_id: Set(None),
    }
    .insert(db)
    .await
    .expect("test row")
}

/// A bare repo with one commit so server probes have a HEAD to measure.
fn fixture_repo() -> (tempfile::TempDir, std::path::PathBuf) {
    let root = tempfile::tempdir().expect("tmp");
    let work = root.path().join("work");
    std::fs::create_dir_all(&work).unwrap();
    let git = |args: &[&str]| {
        let out = std::process::Command::new("git")
            .arg("-C")
            .arg(&work)
            .args(args)
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@t")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@t")
            .output()
            .expect("git");
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
    };
    git(&["init", "-q", "-b", "main"]);
    std::fs::write(work.join("f.txt"), "x").unwrap();
    git(&["add", "."]);
    git(&["commit", "-q", "-m", "feat: seed"]);
    let bare = root.path().join("bare.git");
    let out = std::process::Command::new("git")
        .args(["clone", "-q", "--bare"])
        .arg(&work)
        .arg(&bare)
        .output()
        .unwrap();
    assert!(out.status.success());
    (root, bare)
}

#[tokio::test]
async fn tick_runs_due_server_probes_and_interval_gates_reruns() {
    let _guard = ENV_LOCK.lock().unwrap();
    // safety: serialized by ENV_LOCK
    unsafe {
        std::env::set_var("OLOLO_SANDBOX", "none");
        std::env::set_var("OLOLO_ALLOW_UNSANDBOXED_EXEC", "1");
    }

    let db = setup_db().await;
    let state = test_state(db.clone());
    let seeded = seed(&db, Some(state.server_id)).await;

    // The player repo lives where git_store computes it from the env base.
    let (_root, bare) = fixture_repo();
    let repos_base = tempfile::tempdir().unwrap();
    let dest = repos_base
        .path()
        .join(seeded.session_id.to_string())
        .join(format!("{}.git", seeded.player_id));
    std::fs::create_dir_all(dest.parent().unwrap()).unwrap();
    let out = std::process::Command::new("cp")
        .arg("-R")
        .arg(&bare)
        .arg(&dest)
        .output()
        .unwrap();
    assert!(out.status.success());
    // safety: serialized by ENV_LOCK
    unsafe { std::env::set_var("OLOLO_GIT_REPOS_DIR", repos_base.path()) };

    let probe_test = insert_probe_test(
        &db,
        seeded.session_id,
        seeded.task.id,
        1,
        serde_json::json!({
            // Explicitly server-side: this test is about the server path, and
            // a deterministic probe now runs on the participant's machine
            // unless a project asks otherwise.
            "mode": "deterministic",
            "executor": "server",
            "schedule": { "on": ["interval"], "interval_secs": 3600 }
        }),
    )
    .await;

    // First tick: never ran → due → recorded.
    game_server::probe_scheduler::tick(&state)
        .await
        .expect("tick");
    let count = probes::Entity::find()
        .filter(probes::Column::TestId.eq(probe_test.id))
        .all(&db)
        .await
        .unwrap()
        .len();
    assert_eq!(count, 1, "due probe ran and was recorded");

    // Second tick immediately after: interval not elapsed → no new row.
    game_server::probe_scheduler::tick(&state)
        .await
        .expect("tick");
    let count = probes::Entity::find()
        .filter(probes::Column::TestId.eq(probe_test.id))
        .all(&db)
        .await
        .unwrap()
        .len();
    assert_eq!(count, 1, "interval gates the rerun");

    // Age the recorded dispatch past the interval → due again.
    let _ = probes::Entity::update_many()
        .col_expr(
            probes::Column::DispatchedAt,
            sea_orm::prelude::Expr::value(Utc::now() - Duration::seconds(7200)),
        )
        .filter(probes::Column::TestId.eq(probe_test.id))
        .exec(&db)
        .await;
    game_server::probe_scheduler::tick(&state)
        .await
        .expect("tick");
    let rows = probes::Entity::find()
        .filter(probes::Column::TestId.eq(probe_test.id))
        .all(&db)
        .await
        .unwrap();
    assert_eq!(rows.len(), 2, "elapsed interval re-fires");
    assert!(
        rows.iter().all(|r| r.outcome.is_some()),
        "history, resolved rows"
    );

    // safety: serialized by ENV_LOCK
    unsafe {
        std::env::remove_var("OLOLO_SANDBOX");
        std::env::remove_var("OLOLO_ALLOW_UNSANDBOXED_EXEC");
        std::env::remove_var("OLOLO_GIT_REPOS_DIR");
    }
}

#[tokio::test]
async fn tick_ignores_sessions_owned_by_other_servers_and_classic_tasks() {
    let _guard = ENV_LOCK.lock().unwrap();
    // safety: serialized by ENV_LOCK
    unsafe {
        std::env::set_var("OLOLO_SANDBOX", "none");
        std::env::set_var("OLOLO_ALLOW_UNSANDBOXED_EXEC", "1");
    }

    let db = setup_db().await;
    let state = test_state(db.clone());
    // Owned by a DIFFERENT game server.
    let other = seed(&db, Some(Uuid::new_v4())).await;
    let probe_test = insert_probe_test(
        &db,
        other.session_id,
        other.task.id,
        1,
        serde_json::json!({
            "mode": "deterministic",
            "schedule": { "on": ["start"] }
        }),
    )
    .await;

    game_server::probe_scheduler::tick(&state)
        .await
        .expect("tick");
    let count = probes::Entity::find()
        .filter(probes::Column::TestId.eq(probe_test.id))
        .all(&db)
        .await
        .unwrap()
        .len();
    assert_eq!(count, 0, "another server's session is not touched");

    // safety: serialized by ENV_LOCK
    unsafe {
        std::env::remove_var("OLOLO_SANDBOX");
        std::env::remove_var("OLOLO_ALLOW_UNSANDBOXED_EXEC");
    }
}

#[tokio::test]
async fn todo_report_stores_measurement_and_detects_progress() {
    let db = setup_db().await;
    let state = test_state(db.clone());
    let seeded = seed(&db, None).await;
    let report_test = insert_probe_test(
        &db,
        seeded.session_id,
        seeded.task.id,
        1,
        serde_json::json!({
            "mode": "deterministic", "executor": "participant", "report": "todo",
            "schedule": { "on": ["interval"], "interval_secs": 120 }
        }),
    )
    .await;
    let report_test = tests::Entity::find_by_id(report_test.id)
        .one(&db)
        .await
        .unwrap()
        .unwrap();

    let insert_probe = |db: DatabaseConnection, test_id: Uuid, player: Uuid, session: Uuid| async move {
        probes::ActiveModel {
            id: Set(Uuid::new_v4()),
            test_id: Set(test_id),
            player_id: Set(player),
            session_id: Set(session),
            attempt: Set(1),
            rendered_command: Set("cat TODO.md".to_string()),
            fixture_values: Set("{}".to_string()),
            expected_answer: Set(None),
            resolved_answer: Set(None),
            secret_meta: Set(None),
            outcome: Set(None),
            dispatched_at: Set(Utc::now()),
            deadline_at: Set(Utc::now()),
            resolved_at: Set(None),
            updated_at: Set(Some(Utc::now())),
            output: Set(None),
            exit_code: Set(None),
            duration_ms: Set(None),
            point_delta: Set(None),
            result_json: Set(None),
            artifact_path: Set(None),
        }
        .insert(&db)
        .await
        .expect("probe")
        .id
    };

    // First report: one item checked → measurement stored + progress frame.
    let p1 = insert_probe(
        db.clone(),
        report_test.id,
        seeded.player_id,
        seeded.session_id,
    )
    .await;
    let frame = game_server::ws::player_agent::grading::handle_todo_report(
        &state,
        &report_test,
        &seeded.task,
        seeded.player_id,
        p1,
        "- [x] routing\n- [ ] ui\n",
    )
    .await;
    assert!(frame.is_some(), "first checked item is progress");
    let stored = probes::Entity::find_by_id(p1)
        .one(&db)
        .await
        .unwrap()
        .unwrap()
        .result_json
        .expect("measurement stored");
    assert_eq!(stored["todo"]["checked"], 1);
    assert_eq!(stored["todo"]["total"], 2);
    assert_eq!(stored["todo"]["items"][0]["text"], "routing");

    // Second report, same counts → no progress, no frame.
    let p2 = insert_probe(
        db.clone(),
        report_test.id,
        seeded.player_id,
        seeded.session_id,
    )
    .await;
    let frame = game_server::ws::player_agent::grading::handle_todo_report(
        &state,
        &report_test,
        &seeded.task,
        seeded.player_id,
        p2,
        "- [x] routing\n- [ ] ui\n",
    )
    .await;
    assert!(frame.is_none(), "unchanged plan is not progress");

    // Third report, more checked → progress frame with the right reason.
    let p3 = insert_probe(
        db.clone(),
        report_test.id,
        seeded.player_id,
        seeded.session_id,
    )
    .await;
    let frame = game_server::ws::player_agent::grading::handle_todo_report(
        &state,
        &report_test,
        &seeded.task,
        seeded.player_id,
        p3,
        "- [x] routing\n- [x] ui\n",
    )
    .await
    .expect("progress frame");
    match frame {
        arena_core::protocol::PlayerAgentFrame::SnapshotRequest {
            reason, task_id, ..
        } => {
            assert_eq!(reason, "todo_progress");
            assert_eq!(task_id, seeded.task.id);
        }
        other => panic!("unexpected frame: {other:?}"),
    }
}

#[tokio::test]
async fn participant_due_pick_prefers_overdue_report_probe() {
    let db = setup_db().await;
    let state = test_state(db.clone());
    let seeded = seed(&db, Some(state.server_id)).await;

    let completion = insert_probe_test(
        &db,
        seeded.session_id,
        seeded.task.id,
        0,
        serde_json::json!({
            "mode": "deterministic", "executor": "participant",
            "schedule": { "on": ["interval"], "interval_secs": 60 }
        }),
    )
    .await;
    let report = insert_probe_test(
        &db,
        seeded.session_id,
        seeded.task.id,
        1,
        serde_json::json!({
            "mode": "deterministic", "executor": "participant", "report": "todo",
            "schedule": { "on": ["interval"], "interval_secs": 120 }
        }),
    )
    .await;

    // Never ran → the report probe is due immediately.
    let picked = most_overdue_participant_section(
        &state,
        seeded.task.id,
        seeded.session_id,
        seeded.player_id,
        completion.id,
    )
    .await
    .expect("a due section");
    assert_eq!(picked.id, report.id);

    // Freshly dispatched → nothing due; the caller falls back to the
    // completion probe.
    probes::ActiveModel {
        id: Set(Uuid::new_v4()),
        test_id: Set(report.id),
        player_id: Set(seeded.player_id),
        session_id: Set(seeded.session_id),
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
    .insert(&db)
    .await
    .expect("probe");

    let picked = most_overdue_participant_section(
        &state,
        seeded.task.id,
        seeded.session_id,
        seeded.player_id,
        completion.id,
    )
    .await;
    assert!(
        picked.is_none(),
        "not due → caller polls completion instead"
    );
}

/// `mode: llm` rubric probes were removed — judges are the only LLM
/// evaluation. A stored definition from before the removal still parses
/// (legacy sessions must not 500) but resolves `unavailable` with no
/// penalty and never reaches a model.
#[tokio::test]
async fn legacy_llm_probe_resolves_unavailable_without_a_model_call() {
    let db = setup_db().await;
    let state = test_state(db.clone());
    let seeded = seed(&db, None).await;
    let test = insert_probe_test(
        &db,
        seeded.session_id,
        seeded.task.id,
        1,
        serde_json::json!({
            "mode": "llm",
            "rubric": { "inputs": ["BRIEF.md"],
                        "questions": [ { "key": "coverage", "ask": "done?" } ] }
        }),
    )
    .await;
    let config =
        arena_core::evaluation::ProbeConfig::from_json(test.probe_config.as_ref().unwrap())
            .unwrap();

    let root = tempfile::tempdir().unwrap();
    let run = game_server::probe_exec::run_server_probe(
        &state,
        root.path(),
        seeded.session_id,
        seeded.player_id,
        &test,
        &config,
        &std::collections::BTreeMap::new(),
    )
    .await;
    assert_eq!(run.outcome, "unavailable");
    assert!(
        run.result_json["unavailable_reason"]
            .as_str()
            .unwrap()
            .contains("removed"),
    );
    assert_eq!(run.point_delta, 0);

    // And new definitions are rejected outright at validation.
    let err = arena_core::evaluation::validate_probe_config(&config, false).unwrap_err();
    assert!(err.contains("removed"), "{err}");
}

/// Two judges suspended `waiting`; only one's artifact has arrived. The
/// redrive must touch exactly that judge — a judge whose own probes settled
/// starts evaluating immediately, it does not wait for the other panel
/// members' probes.
#[tokio::test]
async fn a_judge_whose_probes_settled_is_redriven_without_the_others() {
    let db = setup_db().await;
    let gs_id = Uuid::new_v4();
    let seeded = seed(&db, Some(gs_id)).await;
    let mut state = test_state(db.clone());
    state.server_id = gs_id;

    let mk_judge = |slug: String| arena_core::entities::judges::ActiveModel {
        id: Set(Uuid::new_v4()),
        slug: Set(slug),
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
        max_interactive: Set(Some(1)),
        avatar_url: Set(None),
        probes_config: Set(None),
        ignore_paths: Set(None),
    };
    let judge_a = mk_judge(format!("ja-{}", Uuid::new_v4()))
        .insert(&db)
        .await
        .expect("judge a");
    let judge_b = mk_judge(format!("jb-{}", Uuid::new_v4()))
        .insert(&db)
        .await
        .expect("judge b");

    let mut tj_ids = Vec::new();
    for (ord, j) in [(0, &judge_a), (1, &judge_b)] {
        let tj = arena_core::entities::task_judges::ActiveModel {
            id: Set(Uuid::new_v4()),
            task_id: Set(seeded.task.id),
            judge_id: Set(j.id),
            ordinal: Set(ord),
            created_at: Set(Utc::now()),
            updated_at: Set(Utc::now()),
            rating_scale_override: Set(None),
            weight: Set(None),
        }
        .insert(&db)
        .await
        .expect("task judge");
        tj_ids.push(tj.id);
    }

    for tj_id in &tj_ids {
        arena_core::entities::judge_results::ActiveModel {
            id: Set(Uuid::new_v4()),
            session_id_fk: Set(seeded.session_id),
            player_id_fk: Set(seeded.player_id),
            task_judge_id: Set(*tj_id),
            rating: Set(serde_json::json!(0.0)),
            point_delta: Set(0),
            feedback: Set(String::new()),
            model: Set("m".to_string()),
            provider: Set("p".to_string()),
            raw_output: Set(String::new()),
            duration_ms: Set(None),
            run_log: Set(None),
            tokens_input: Set(None),
            tokens_output: Set(None),
            tokens_cache_read: Set(None),
            tokens_cache_write: Set(None),
            status: Set("waiting".to_string()),
            error: Set(Some("waiting for a participant artifact".to_string())),
            created_at: Set(Utc::now()),
            updated_at: Set(Utc::now()),
            verdict_kind: Set(None),
        }
        .insert(&db)
        .await
        .expect("waiting row");
    }

    // Each judge registered one interactive probe. Judge A's artifact
    // arrived (probe resolved); judge B's is still awaited, deadline far
    // in the future so the timeout sweep cannot resolve it.
    let interactive = serde_json::json!({
        "mode": "interactive",
        "instruction": "capture",
        "artifact": { "content_type": "image/png", "max_bytes": 1000000 }
    });
    for (j, resolved) in [(&judge_a, true), (&judge_b, false)] {
        let test = tests::ActiveModel {
            id: Set(Uuid::new_v4()),
            command_template: Set(String::new()),
            answer_template: Set(String::new()),
            fixture_definitions: Set(r#"{"kind":"js","script":"({})"}"#.to_string()),
            created_at: Set(Utc::now()),
            session_id: Set(seeded.session_id),
            task_id: Set(seeded.task.id),
            ordinal: Set(if resolved { 10 } else { 11 }),
            prompt: Set(String::new()),
            description: Set(None),
            probe_config: Set(Some(interactive.clone())),
            initiator: Set("judge".to_string()),
            registered_by_judge_id: Set(Some(j.id)),
        }
        .insert(&db)
        .await
        .expect("interactive test");
        probes::ActiveModel {
            id: Set(Uuid::new_v4()),
            test_id: Set(test.id),
            player_id: Set(seeded.player_id),
            session_id: Set(seeded.session_id),
            attempt: Set(1),
            rendered_command: Set(String::new()),
            fixture_values: Set("{}".to_string()),
            expected_answer: Set(None),
            resolved_answer: Set(None),
            secret_meta: Set(None),
            outcome: Set(if resolved {
                Some("pass".to_string())
            } else {
                None
            }),
            dispatched_at: Set(Utc::now()),
            deadline_at: Set(Utc::now() + chrono::Duration::hours(1)),
            resolved_at: Set(if resolved { Some(Utc::now()) } else { None }),
            updated_at: Set(Some(Utc::now())),
            output: Set(None),
            exit_code: Set(None),
            duration_ms: Set(None),
            point_delta: Set(None),
            result_json: Set(None),
            artifact_path: Set(if resolved {
                Some("abc123:.ololo/artifacts/x/shot.png".to_string())
            } else {
                None
            }),
        }
        .insert(&db)
        .await
        .expect("probe row");
    }

    game_server::probe_scheduler::tick(&state)
        .await
        .expect("tick");

    // The redrive is spawned; wait for judge A's row to leave `waiting`
    // (the re-run fails fast here — no LLM is configured — which is still
    // proof the redrive picked it up).
    let a_left_waiting = async {
        loop {
            let row = arena_core::entities::judge_results::Entity::find()
                .filter(arena_core::entities::judge_results::Column::TaskJudgeId.eq(tj_ids[0]))
                .one(&db)
                .await
                .expect("query")
                .expect("row");
            if row.status != "waiting" {
                return row.status;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    };
    let status_a = tokio::time::timeout(std::time::Duration::from_secs(15), a_left_waiting)
        .await
        .expect("judge A must be re-driven once its own probes settled");
    assert_ne!(status_a, "waiting");

    // Judge B's probe is still awaited: its run must not have been touched.
    let row_b = arena_core::entities::judge_results::Entity::find()
        .filter(arena_core::entities::judge_results::Column::TaskJudgeId.eq(tj_ids[1]))
        .one(&db)
        .await
        .expect("query")
        .expect("row");
    assert_eq!(
        row_b.status, "waiting",
        "a judge with an unresolved probe of its own must keep waiting"
    );
}

/// A judge asked the player to run something; a red suite is the answer.
///
/// The waiting rule was written for artifact requests, where only a PASS
/// means "the file arrived" and a failed check just means "not yet". A
/// deterministic ask is the other way round: the command ran and came back
/// failing, which is exactly the fact the judge registered it to learn. On
/// the old rule that judge sat `waiting` until the session ended and then
/// verdicted `partial` — blind to the very run it asked for.
#[tokio::test]
async fn a_judge_is_redriven_when_its_command_comes_back_failing() {
    let db = setup_db().await;
    let gs_id = Uuid::new_v4();
    let seeded = seed(&db, Some(gs_id)).await;
    let mut state = test_state(db.clone());
    state.server_id = gs_id;

    let judge = arena_core::entities::judges::ActiveModel {
        id: Set(Uuid::new_v4()),
        slug: Set(format!("jc-{}", Uuid::new_v4())),
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
        max_interactive: Set(Some(1)),
        avatar_url: Set(None),
        probes_config: Set(None),
        ignore_paths: Set(None),
    }
    .insert(&db)
    .await
    .expect("judge");

    let tj = arena_core::entities::task_judges::ActiveModel {
        id: Set(Uuid::new_v4()),
        task_id: Set(seeded.task.id),
        judge_id: Set(judge.id),
        ordinal: Set(0),
        created_at: Set(Utc::now()),
        updated_at: Set(Utc::now()),
        rating_scale_override: Set(None),
        weight: Set(None),
    }
    .insert(&db)
    .await
    .expect("task judge");

    arena_core::entities::judge_results::ActiveModel {
        id: Set(Uuid::new_v4()),
        session_id_fk: Set(seeded.session_id),
        player_id_fk: Set(seeded.player_id),
        task_judge_id: Set(tj.id),
        rating: Set(serde_json::json!(0.0)),
        point_delta: Set(0),
        feedback: Set(String::new()),
        model: Set("m".to_string()),
        provider: Set("p".to_string()),
        raw_output: Set(String::new()),
        duration_ms: Set(None),
        run_log: Set(None),
        tokens_input: Set(None),
        tokens_output: Set(None),
        tokens_cache_read: Set(None),
        tokens_cache_write: Set(None),
        status: Set("waiting".to_string()),
        error: Set(Some("waiting for the participant's run".to_string())),
        created_at: Set(Utc::now()),
        updated_at: Set(Utc::now()),
        verdict_kind: Set(None),
    }
    .insert(&db)
    .await
    .expect("waiting row");

    // The registered command came back from the player's machine: failing.
    let test = tests::ActiveModel {
        id: Set(Uuid::new_v4()),
        command_template: Set("npm test".to_string()),
        answer_template: Set("exit_code === 0".to_string()),
        fixture_definitions: Set(r#"{"kind":"js","script":"({})"}"#.to_string()),
        created_at: Set(Utc::now()),
        session_id: Set(seeded.session_id),
        task_id: Set(seeded.task.id),
        ordinal: Set(2000),
        prompt: Set("registered: judge".to_string()),
        description: Set(None),
        probe_config: Set(Some(serde_json::json!({ "mode": "deterministic" }))),
        initiator: Set("judge".to_string()),
        registered_by_judge_id: Set(Some(judge.id)),
    }
    .insert(&db)
    .await
    .expect("registered test");
    probes::ActiveModel {
        id: Set(Uuid::new_v4()),
        test_id: Set(test.id),
        player_id: Set(seeded.player_id),
        session_id: Set(seeded.session_id),
        attempt: Set(1),
        rendered_command: Set("npm test".to_string()),
        fixture_values: Set("{}".to_string()),
        expected_answer: Set(None),
        resolved_answer: Set(None),
        secret_meta: Set(None),
        outcome: Set(Some("fail".to_string())),
        dispatched_at: Set(Utc::now()),
        deadline_at: Set(Utc::now() + chrono::Duration::hours(1)),
        resolved_at: Set(Some(Utc::now())),
        updated_at: Set(Some(Utc::now())),
        output: Set(Some("2 failing".to_string())),
        exit_code: Set(Some(1)),
        duration_ms: Set(Some(1200)),
        point_delta: Set(None),
        result_json: Set(None),
        artifact_path: Set(None),
    }
    .insert(&db)
    .await
    .expect("probe row");

    game_server::probe_scheduler::tick(&state)
        .await
        .expect("tick");

    let left_waiting = async {
        loop {
            let row = arena_core::entities::judge_results::Entity::find()
                .filter(arena_core::entities::judge_results::Column::TaskJudgeId.eq(tj.id))
                .one(&db)
                .await
                .expect("query")
                .expect("row");
            if row.status != "waiting" {
                return row.status;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    };
    let status = tokio::time::timeout(std::time::Duration::from_secs(15), left_waiting)
        .await
        .expect("a failing run is an answer — the judge must be re-driven");
    assert_ne!(status, "waiting");
}

/// A fresh, all-unchecked TODO.md is already worth a snapshot: the judges
/// read the committed plan to decide which artifacts to request, and a
/// plan that never reaches the repo until the first checkmark starves
/// them. Restructuring the list (item count change) also counts.
#[tokio::test]
async fn todo_creation_and_restructure_trigger_the_wip_snapshot() {
    let db = setup_db().await;
    let state = test_state(db.clone());
    let seeded = seed(&db, None).await;
    let report_test = insert_probe_test(
        &db,
        seeded.session_id,
        seeded.task.id,
        1,
        serde_json::json!({
            "mode": "deterministic", "executor": "participant", "report": "todo",
            "schedule": { "on": ["interval"], "interval_secs": 120 }
        }),
    )
    .await;

    let mk_probe = |db: DatabaseConnection| {
        let (test_id, player, session) = (report_test.id, seeded.player_id, seeded.session_id);
        async move {
            probes::ActiveModel {
                id: Set(Uuid::new_v4()),
                test_id: Set(test_id),
                player_id: Set(player),
                session_id: Set(session),
                attempt: Set(1),
                rendered_command: Set("cat TODO.md".to_string()),
                fixture_values: Set("{}".to_string()),
                expected_answer: Set(None),
                resolved_answer: Set(None),
                secret_meta: Set(None),
                outcome: Set(None),
                dispatched_at: Set(Utc::now()),
                deadline_at: Set(Utc::now()),
                resolved_at: Set(None),
                updated_at: Set(Some(Utc::now())),
                output: Set(None),
                exit_code: Set(None),
                duration_ms: Set(None),
                point_delta: Set(None),
                result_json: Set(None),
                artifact_path: Set(None),
            }
            .insert(&db)
            .await
            .expect("probe")
            .id
        }
    };

    // Creation: all unchecked, but the plan exists → snapshot.
    let p1 = mk_probe(db.clone()).await;
    let frame = game_server::ws::player_agent::grading::handle_todo_report(
        &state,
        &report_test,
        &seeded.task,
        seeded.player_id,
        p1,
        "- [ ] routing\n- [ ] ui\n- [ ] units\n",
    )
    .await;
    assert!(frame.is_some(), "a freshly created plan must be committed");

    // Restructure: same checked count, different item list → snapshot.
    let p2 = mk_probe(db.clone()).await;
    let frame = game_server::ws::player_agent::grading::handle_todo_report(
        &state,
        &report_test,
        &seeded.task,
        seeded.player_id,
        p2,
        "- [ ] routing\n- [ ] ui\n- [ ] units\n- [ ] forecast\n",
    )
    .await;
    assert!(frame.is_some(), "a restructured plan must be committed");

    // No plan at all → nothing to snapshot.
    let p3 = mk_probe(db.clone()).await;
    // A separate test row keeps the empty report free of prior history.
    let empty_test = insert_probe_test(
        &db,
        seeded.session_id,
        seeded.task.id,
        2,
        serde_json::json!({
            "mode": "deterministic", "executor": "participant", "report": "todo",
            "schedule": { "on": ["interval"], "interval_secs": 120 }
        }),
    )
    .await;
    let _ = p3;
    let p4 = {
        let (test_id, player, session) = (empty_test.id, seeded.player_id, seeded.session_id);
        probes::ActiveModel {
            id: Set(Uuid::new_v4()),
            test_id: Set(test_id),
            player_id: Set(player),
            session_id: Set(session),
            attempt: Set(1),
            rendered_command: Set("cat TODO.md".to_string()),
            fixture_values: Set("{}".to_string()),
            expected_answer: Set(None),
            resolved_answer: Set(None),
            secret_meta: Set(None),
            outcome: Set(None),
            dispatched_at: Set(Utc::now()),
            deadline_at: Set(Utc::now()),
            resolved_at: Set(None),
            updated_at: Set(Some(Utc::now())),
            output: Set(None),
            exit_code: Set(None),
            duration_ms: Set(None),
            point_delta: Set(None),
            result_json: Set(None),
            artifact_path: Set(None),
        }
        .insert(&db)
        .await
        .expect("probe")
        .id
    };
    let frame = game_server::ws::player_agent::grading::handle_todo_report(
        &state,
        &empty_test,
        &seeded.task,
        seeded.player_id,
        p4,
        "(no TODO.md yet)",
    )
    .await;
    assert!(frame.is_none(), "no items, nothing to commit");
}

/// A `waiting` judge of a session that already ENDED must not wait forever on
/// a probe nobody will ever answer: the live loop is gone, so an unresolved
/// interactive probe counts as expired and the judge is re-driven to a
/// (partial) verdict. Session Y3W66Z froze at "Evaluating…" without this.
#[tokio::test]
async fn a_waiting_judge_of_an_ended_session_is_redriven() {
    let db = setup_db().await;
    let gs_id = Uuid::new_v4();
    let seeded = seed(&db, Some(gs_id)).await;
    let mut state = test_state(db.clone());
    state.server_id = gs_id;

    let judge = arena_core::entities::judges::ActiveModel {
        id: Set(Uuid::new_v4()),
        slug: Set(format!("ended-{}", Uuid::new_v4())),
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
        max_interactive: Set(Some(1)),
        avatar_url: Set(None),
        probes_config: Set(None),
        ignore_paths: Set(None),
    }
    .insert(&db)
    .await
    .expect("judge");
    let tj = arena_core::entities::task_judges::ActiveModel {
        id: Set(Uuid::new_v4()),
        task_id: Set(seeded.task.id),
        judge_id: Set(judge.id),
        ordinal: Set(0),
        created_at: Set(Utc::now()),
        updated_at: Set(Utc::now()),
        rating_scale_override: Set(None),
        weight: Set(None),
    }
    .insert(&db)
    .await
    .expect("task judge");

    arena_core::entities::judge_results::ActiveModel {
        id: Set(Uuid::new_v4()),
        session_id_fk: Set(seeded.session_id),
        player_id_fk: Set(seeded.player_id),
        task_judge_id: Set(tj.id),
        rating: Set(serde_json::json!(0.0)),
        point_delta: Set(0),
        feedback: Set(String::new()),
        model: Set("m".to_string()),
        provider: Set("p".to_string()),
        raw_output: Set(String::new()),
        duration_ms: Set(None),
        run_log: Set(None),
        tokens_input: Set(None),
        tokens_output: Set(None),
        tokens_cache_read: Set(None),
        tokens_cache_write: Set(None),
        status: Set("waiting".to_string()),
        error: Set(Some("waiting for a participant artifact".to_string())),
        created_at: Set(Utc::now()),
        updated_at: Set(Utc::now()),
        verdict_kind: Set(None),
    }
    .insert(&db)
    .await
    .expect("waiting row");

    // The judge's interactive probe was dispatched but never answered, and
    // its deadline is far away — on a running session this means "keep
    // waiting" (the sibling test proves that); here the session is cancelled.
    let interactive = serde_json::json!({
        "mode": "interactive",
        "instruction": "capture",
        "artifact": { "content_type": "image/png", "max_bytes": 1000000 }
    });
    let test = tests::ActiveModel {
        id: Set(Uuid::new_v4()),
        command_template: Set(String::new()),
        answer_template: Set(String::new()),
        fixture_definitions: Set(r#"{"kind":"js","script":"({})"}"#.to_string()),
        created_at: Set(Utc::now()),
        session_id: Set(seeded.session_id),
        task_id: Set(seeded.task.id),
        ordinal: Set(10),
        prompt: Set(String::new()),
        description: Set(None),
        probe_config: Set(Some(interactive)),
        initiator: Set("judge".to_string()),
        registered_by_judge_id: Set(Some(judge.id)),
    }
    .insert(&db)
    .await
    .expect("interactive test");
    probes::ActiveModel {
        id: Set(Uuid::new_v4()),
        test_id: Set(test.id),
        player_id: Set(seeded.player_id),
        session_id: Set(seeded.session_id),
        attempt: Set(1),
        rendered_command: Set(String::new()),
        fixture_values: Set("{}".to_string()),
        expected_answer: Set(None),
        resolved_answer: Set(None),
        secret_meta: Set(None),
        outcome: Set(None),
        dispatched_at: Set(Utc::now()),
        deadline_at: Set(Utc::now() + chrono::Duration::hours(1)),
        resolved_at: Set(None),
        updated_at: Set(Some(Utc::now())),
        output: Set(None),
        exit_code: Set(None),
        duration_ms: Set(None),
        point_delta: Set(None),
        result_json: Set(None),
        artifact_path: Set(None),
    }
    .insert(&db)
    .await
    .expect("probe row");

    sessions::Entity::update_many()
        .col_expr(
            sessions::Column::Status,
            sea_orm::prelude::Expr::value(arena_core::session_status::SessionStatus::Cancelled),
        )
        .filter(sessions::Column::Id.eq(seeded.session_id))
        .exec(&db)
        .await
        .expect("cancel session");

    game_server::probe_scheduler::tick(&state)
        .await
        .expect("tick");

    // The re-run fails fast (no LLM configured) — leaving `waiting` at all is
    // the proof the ended-session redrive picked it up.
    let left_waiting = async {
        loop {
            let row = arena_core::entities::judge_results::Entity::find()
                .filter(arena_core::entities::judge_results::Column::TaskJudgeId.eq(tj.id))
                .one(&db)
                .await
                .expect("query")
                .expect("row");
            if row.status != "waiting" {
                return row.status;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    };
    let status = tokio::time::timeout(std::time::Duration::from_secs(15), left_waiting)
        .await
        .expect("a waiting judge of an ended session must be re-driven");
    assert_ne!(status, "waiting");
}

/// A deterministic probe that names no side belongs to the player: their
/// machine has the toolchain, the dependencies and the process the command
/// talks to. The scheduler's server sweep must leave it alone — before this,
/// an unstated `npm test` ran in a scratch copy of the snapshot and answered
/// a different question than the one the player was asked.
#[tokio::test]
async fn tick_leaves_unstated_deterministic_probes_to_the_participant() {
    let _guard = ENV_LOCK.lock().unwrap();
    // safety: serialized by ENV_LOCK
    unsafe {
        std::env::set_var("OLOLO_SANDBOX", "none");
        std::env::set_var("OLOLO_ALLOW_UNSANDBOXED_EXEC", "1");
    }

    let db = setup_db().await;
    let state = test_state(db.clone());
    let seeded = seed(&db, Some(state.server_id)).await;

    let (_root, bare) = fixture_repo();
    let repos_base = tempfile::tempdir().unwrap();
    let dest = repos_base
        .path()
        .join(seeded.session_id.to_string())
        .join(format!("{}.git", seeded.player_id));
    std::fs::create_dir_all(dest.parent().unwrap()).unwrap();
    assert!(
        std::process::Command::new("cp")
            .arg("-R")
            .arg(&bare)
            .arg(&dest)
            .output()
            .unwrap()
            .status
            .success()
    );
    // safety: serialized by ENV_LOCK
    unsafe { std::env::set_var("OLOLO_GIT_REPOS_DIR", repos_base.path()) };

    let probe_test = insert_probe_test(
        &db,
        seeded.session_id,
        seeded.task.id,
        1,
        serde_json::json!({
            "mode": "deterministic",
            "schedule": { "on": ["interval"], "interval_secs": 3600 }
        }),
    )
    .await;

    game_server::probe_scheduler::tick(&state)
        .await
        .expect("tick");

    let rows = probes::Entity::find()
        .filter(probes::Column::TestId.eq(probe_test.id))
        .all(&db)
        .await
        .expect("probes");
    assert!(
        rows.is_empty(),
        "the server ran a probe that belongs to the player: {rows:?}"
    );

    // safety: serialized by ENV_LOCK
    unsafe {
        std::env::remove_var("OLOLO_GIT_REPOS_DIR");
        std::env::remove_var("OLOLO_SANDBOX");
        std::env::remove_var("OLOLO_ALLOW_UNSANDBOXED_EXEC");
    }
}
