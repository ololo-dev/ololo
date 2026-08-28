// ENV_LOCK guards are held across awaits on purpose: they serialize the
// tests that mutate process-global env vars.
#![allow(clippy::await_holding_lock)]

//! `register_probe`: interactive registrations respect the judge's own
//! budget and the task-wide cap, are idempotent per judge, and flip the run
//! into `waiting`; the ticker resolves their timeouts to `no_response`.

use std::sync::{Arc, Mutex};

// Serializes the tests that mutate the process-global OLOLO_GIT_REPOS_DIR.
static ENV_LOCK: Mutex<()> = Mutex::new(());

use arena_core::entities::{
    judges, players, probes, projects, sessions, task_judges, tasks, tests, users,
};
use arena_core::judging::ProbeRegistrar;
use arena_core::session_status::SessionStatus;
use chrono::{Duration, Utc};
use dashmap::DashMap;
use game_server::judge_registrar::JudgeProbeRegistrar;
use game_server::state::GameServerState;
use game_server::zmq_pub::NoopEventPublisher;
use jsonwebtoken::{DecodingKey, EncodingKey};
use migration::MigratorTrait;
use sea_orm::{ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, Set};
use tokio::sync::Semaphore;
use uuid::Uuid;

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
    task_id: Uuid,
    judge_id: Uuid,
}

async fn seed(db: &DatabaseConnection) -> Seeded {
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
        join_code: Set(format!("R{}", &Uuid::new_v4().simple().to_string()[..5]).to_uppercase()),
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

    let task_id = tasks::ActiveModel {
        id: Set(Uuid::new_v4()),
        project_id_fk: Set(project_id),
        ordinal: Set(0),
        title: Set("Build".to_string()),
        content: Set(String::new()),
        test_template: Set(serde_json::json!({"kind":"shell","command_template":"## X\n"})),
        created_at: Set(Utc::now()),
        tags: Set(String::new()),
        point_value: Set(200),
        deadline_secs: Set(None),
        min_interval_secs: Set(None),
        interval_increment_secs: Set(None),
        max_interval_secs: Set(None),
        fail_points: Set(0),
        no_response_points: Set(0),
        completion_bonus_points: Set(0),
        evaluation: Set(Some(serde_json::json!({
            "kind": "open_ended",
            "completion": { "probe": "X", "deadline_secs": 3600 },
            "criteria": [ { "key": "product", "weight": 1.0 } ],
            "limits": { "interactive_probes_per_task": 1 }
        }))),
    }
    .insert(db)
    .await
    .expect("task")
    .id;

    let judge_id = judges::ActiveModel {
        id: Set(Uuid::new_v4()),
        slug: Set("product-fit".to_string()),
        name: Set("Product Fit".to_string()),
        description: Set(String::new()),
        prompt: Set("judge".to_string()),
        rating_scale: Set(serde_json::json!({"min": 0.0, "max": 10.0, "step": 0.1})),
        kind: Set("llm".to_string()),
        scope: Set("task".to_string()),
        evidence_mode: Set("tools".to_string()),
        evidence_needs: Set(None),
        criteria: Set(Some(r#"["product"]"#.to_string())),
        probes_config: Set(None),
        max_interactive: Set(Some(1)),
        avatar_url: Set(None),
        ignore_paths: Set(None),
        llm_provider_id_fk: Set(None),
        llm_model: Set(None),
        llm_pool_id_fk: Set(None),
        llm_source_order: Set("pool_first".to_string()),
        created_at: Set(Utc::now()),
        updated_at: Set(Utc::now()),
    }
    .insert(db)
    .await
    .expect("judge")
    .id;

    task_judges::ActiveModel {
        id: Set(Uuid::new_v4()),
        task_id: Set(task_id),
        judge_id: Set(judge_id),
        ordinal: Set(0),
        rating_scale_override: Set(None),
        weight: Set(None),
        created_at: Set(Utc::now()),
        updated_at: Set(Utc::now()),
    }
    .insert(db)
    .await
    .expect("task judge");

    Seeded {
        session_id,
        player_id,
        task_id,
        judge_id,
    }
}

/// Registers a live agent channel for the player — interactive
/// registration refuses when nobody could answer the request.
fn connect_agent(state: &GameServerState, player_id: Uuid) {
    let (tx, rx) = tokio::sync::mpsc::channel(8);
    std::mem::forget(rx); // keep the channel open for the test's lifetime
    state.player_agent_registry.insert(player_id, tx);
}

fn registrar(
    state: &GameServerState,
    s: &Seeded,
    max_interactive: i32,
) -> Arc<JudgeProbeRegistrar> {
    connect_agent(state, s.player_id);
    JudgeProbeRegistrar::new(
        state.clone(),
        s.session_id,
        s.player_id,
        s.task_id,
        s.judge_id,
        "product-fit".to_string(),
        max_interactive,
        1, // per-task cap from the contract above
        true,
    )
}

#[tokio::test]
async fn a_request_runs_on_the_session_clock_not_the_judges_number() {
    // The judge's own window is what lost session TJQJPJ its ux-review
    // verdict: the request expired five minutes before the session did, and
    // the judge then scored a product it had never seen. The session ending
    // is the only deadline that cannot be wrong.
    let db = setup_db().await;
    let state = test_state(db.clone());
    let seeded = seed(&db).await;
    let reg = registrar(&state, &seeded, 1);

    let out = reg
        .register(&serde_json::json!({
            "mode": "interactive",
            "instruction": "Screenshot the main page",
            "content_type": "image/png",
            "deadline_secs": 300
        }))
        .await;
    let parsed: serde_json::Value = serde_json::from_str(&out).unwrap();
    let test_id: Uuid = parsed["test_id"].as_str().unwrap().parse().unwrap();
    let test = tests::Entity::find_by_id(test_id)
        .one(&db)
        .await
        .unwrap()
        .expect("tests row");
    let config =
        arena_core::evaluation::ProbeConfig::from_json(test.probe_config.as_ref().expect("config"))
            .expect("parse");
    let deadline = config.deadline_secs.expect("a deadline");
    // The fixture session runs an hour and started just now.
    assert!(
        (3500..=3600).contains(&deadline),
        "the request stands until the session ends, not {deadline}s"
    );
}

#[tokio::test]
async fn a_request_opened_in_the_last_seconds_still_gets_a_moment() {
    // A capture already being taken as the clock runs out should still land:
    // a zero-second request would score the product nobody looked at.
    let db = setup_db().await;
    let state = test_state(db.clone());
    let seeded = seed(&db).await;
    let mut session: sessions::ActiveModel = sessions::Entity::find_by_id(seeded.session_id)
        .one(&db)
        .await
        .unwrap()
        .expect("session")
        .into();
    session.started_at = Set(Some(Utc::now() - chrono::Duration::seconds(3595)));
    session.update(&db).await.expect("age the session");

    let reg = registrar(&state, &seeded, 1);
    let out = reg
        .register(&serde_json::json!({
            "mode": "interactive",
            "instruction": "Screenshot the main page",
            "content_type": "image/png"
        }))
        .await;
    let parsed: serde_json::Value = serde_json::from_str(&out).unwrap();
    let test_id: Uuid = parsed["test_id"].as_str().unwrap().parse().unwrap();
    let test = tests::Entity::find_by_id(test_id)
        .one(&db)
        .await
        .unwrap()
        .expect("tests row");
    let config =
        arena_core::evaluation::ProbeConfig::from_json(test.probe_config.as_ref().expect("config"))
            .expect("parse");
    assert_eq!(config.deadline_secs, Some(60), "the floor holds");
}

#[tokio::test]
async fn interactive_registration_is_a_regular_probe_and_idempotent() {
    let db = setup_db().await;
    let state = test_state(db.clone());
    let seeded = seed(&db).await;
    let reg = registrar(&state, &seeded, 1);

    let out = reg
        .register(&serde_json::json!({
            "mode": "interactive",
            "instruction": "Screenshot the main page",
            "content_type": "image/png",
            "deadline_secs": 300
        }))
        .await;
    let parsed: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(parsed["status"], "queued", "{out}");
    assert!(reg.interactive_pending(), "run must end waiting");

    let test_id: Uuid = parsed["test_id"].as_str().unwrap().parse().unwrap();
    let test = tests::Entity::find_by_id(test_id)
        .one(&db)
        .await
        .unwrap()
        .expect("tests row");
    // The request is a runnable availability check — a REGULAR probe the
    // ordinary dispatch loop delivers and grades.
    assert!(test.command_template.contains("ARTIFACT REQUEST"));
    assert!(test.command_template.contains("ls -A .ololo/artifacts/"));
    assert!(test.answer_template.contains("delivered"));
    assert!(
        parsed["artifact_path"]
            .as_str()
            .unwrap()
            .starts_with(".ololo/artifacts/"),
    );
    // No pre-created probe row: the dispatch loop owns probe rows.
    let probe_rows = probes::Entity::find()
        .filter(probes::Column::TestId.eq(test_id))
        .all(&db)
        .await
        .unwrap();
    assert!(probe_rows.is_empty());

    // Same judge asks again (e.g. after a retry): the same request returns.
    let again = reg
        .register(&serde_json::json!({
            "mode": "interactive",
            "instruction": "Screenshot the main page"
        }))
        .await;
    let parsed_again: serde_json::Value = serde_json::from_str(&again).unwrap();
    assert_eq!(parsed_again["test_id"], parsed["test_id"]);
    let count = tests::Entity::find()
        .filter(tests::Column::RegisteredByJudgeId.eq(seeded.judge_id))
        .all(&db)
        .await
        .unwrap()
        .len();
    assert_eq!(count, 1, "no duplicate registration");
}

#[tokio::test]
async fn limits_refuse_with_a_message_not_a_failure() {
    let db = setup_db().await;
    let state = test_state(db.clone());
    let seeded = seed(&db).await;

    // Judge with no interactive budget.
    let no_budget = registrar(&state, &seeded, 0);
    let out = no_budget
        .register(&serde_json::json!({
            "mode": "interactive", "instruction": "anything"
        }))
        .await;
    assert!(out.contains("no interactive budget"), "{out}");
    assert!(!no_budget.interactive_pending());

    // Fill the task-wide cap with another judge's registration, then hit it.
    let other_judge = judges::ActiveModel {
        id: Set(Uuid::new_v4()),
        slug: Set("other".to_string()),
        name: Set("Other".to_string()),
        description: Set(String::new()),
        prompt: Set("judge".to_string()),
        rating_scale: Set(serde_json::json!({"min": 0.0, "max": 10.0, "step": 0.1})),
        kind: Set("llm".to_string()),
        scope: Set("task".to_string()),
        evidence_mode: Set("tools".to_string()),
        evidence_needs: Set(None),
        criteria: Set(None),
        probes_config: Set(None),
        max_interactive: Set(Some(1)),
        avatar_url: Set(None),
        ignore_paths: Set(None),
        llm_provider_id_fk: Set(None),
        llm_model: Set(None),
        llm_pool_id_fk: Set(None),
        llm_source_order: Set("pool_first".to_string()),
        created_at: Set(Utc::now()),
        updated_at: Set(Utc::now()),
    }
    .insert(&db)
    .await
    .expect("other judge")
    .id;
    let mut other_seeded = Seeded {
        session_id: seeded.session_id,
        player_id: seeded.player_id,
        task_id: seeded.task_id,
        judge_id: other_judge,
    };
    let other_reg = registrar(&state, &other_seeded, 1);
    let ok = other_reg
        .register(&serde_json::json!({
            "mode": "interactive", "instruction": "export the data"
        }))
        .await;
    assert!(ok.contains("queued"), "{ok}");

    other_seeded.judge_id = seeded.judge_id;
    let capped = registrar(&state, &other_seeded, 1);
    // A different content type: the open-request attach guard must not
    // absorb this ask, so the task-wide cap is what answers.
    let out = capped
        .register(&serde_json::json!({
            "mode": "interactive", "instruction": "one more thing",
            "content_type": "video/webm"
        }))
        .await;
    assert!(out.contains("task-wide interactive limit"), "{out}");
}

#[tokio::test]
async fn ticker_times_out_interactive_probes_to_no_response() {
    let db = setup_db().await;
    let state = test_state(db.clone());
    let seeded = seed(&db).await;
    let reg = registrar(&state, &seeded, 1);
    let out = reg
        .register(&serde_json::json!({
            "mode": "interactive", "instruction": "screenshot", "deadline_secs": 60
        }))
        .await;
    let parsed: serde_json::Value = serde_json::from_str(&out).unwrap();
    let test_id: Uuid = parsed["test_id"].as_str().unwrap().parse().unwrap();
    // The dispatch loop owns probe rows now; simulate one in-flight attempt.
    let probe_id = Uuid::new_v4();
    probes::ActiveModel {
        id: Set(probe_id),
        test_id: Set(test_id),
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
    .expect("probe row");

    // Age the deadline past due, then tick.
    let _ = probes::Entity::update_many()
        .col_expr(
            probes::Column::DeadlineAt,
            sea_orm::prelude::Expr::value(Utc::now() - Duration::seconds(30)),
        )
        .filter(probes::Column::Id.eq(probe_id))
        .exec(&db)
        .await;
    game_server::probe_scheduler::tick(&state)
        .await
        .expect("tick");

    let probe = probes::Entity::find_by_id(probe_id)
        .one(&db)
        .await
        .unwrap()
        .expect("probe");
    assert_eq!(probe.outcome.as_deref(), Some("no_response"));
    assert_eq!(
        probe.point_delta,
        Some(0),
        "silence is never a penalty here"
    );
}

#[tokio::test]
async fn pushed_artifact_resolves_the_probe_with_a_blob_reference() {
    let _guard = ENV_LOCK.lock().unwrap();
    let db = setup_db().await;
    let state = test_state(db.clone());
    let seeded = seed(&db).await;
    let reg = registrar(&state, &seeded, 1);
    let out = reg
        .register(&serde_json::json!({
            "mode": "interactive", "instruction": "screenshot",
            "content_type": "image/png", "max_bytes": 1024, "deadline_secs": 600
        }))
        .await;
    let parsed: serde_json::Value = serde_json::from_str(&out).unwrap();
    let test_id: Uuid = parsed["test_id"].as_str().unwrap().parse().unwrap();
    let artifact_dir = parsed["artifact_path"].as_str().unwrap().to_string();
    // One in-flight attempt, as the dispatch loop would create.
    let probe_id = Uuid::new_v4();
    probes::ActiveModel {
        id: Set(probe_id),
        test_id: Set(test_id),
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
        deadline_at: Set(Utc::now() + Duration::seconds(600)),
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

    // Build the player's repo with the artifact committed at the expected
    // path, where git_store will look for it.
    let repos_base = tempfile::tempdir().unwrap();
    let work = repos_base.path().join("work");
    std::fs::create_dir_all(work.join(artifact_dir.trim_end_matches('/'))).unwrap();
    std::fs::write(
        work.join(artifact_dir.trim_end_matches('/'))
            .join("shot.png"),
        vec![0u8; 512],
    )
    .unwrap();
    let git = |args: &[&str]| {
        let outp = std::process::Command::new("git")
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
            outp.status.success(),
            "{}",
            String::from_utf8_lossy(&outp.stderr)
        );
    };
    git(&["init", "-q", "-b", "main"]);
    git(&["add", "."]);
    git(&["commit", "-q", "-m", "artifact: sync"]);
    let bare = repos_base
        .path()
        .join(seeded.session_id.to_string())
        .join(format!("{}.git", seeded.player_id));
    std::fs::create_dir_all(bare.parent().unwrap()).unwrap();
    let outp = std::process::Command::new("git")
        .args(["clone", "-q", "--bare"])
        .arg(&work)
        .arg(&bare)
        .output()
        .unwrap();
    assert!(outp.status.success());

    // safety: this test file's env mutations are process-wide but this test
    // is the only OLOLO_GIT_REPOS_DIR user here.
    unsafe { std::env::set_var("OLOLO_GIT_REPOS_DIR", repos_base.path()) };
    game_server::probe_scheduler::tick(&state)
        .await
        .expect("tick");
    unsafe { std::env::remove_var("OLOLO_GIT_REPOS_DIR") };

    let probe = probes::Entity::find_by_id(probe_id)
        .one(&db)
        .await
        .unwrap()
        .expect("probe");
    assert_eq!(probe.outcome.as_deref(), Some("pass"));
    let reference = probe.artifact_path.expect("blob reference");
    assert!(reference.contains(":.ololo/artifacts/"), "{reference}");
    let rj = probe.result_json.expect("measurement");
    assert_eq!(rj["artifact"]["size"], 512);
    assert_eq!(rj["artifact"]["within_cap"], true);
}

#[tokio::test]
async fn oversized_artifact_resolves_error_not_pass() {
    let _guard = ENV_LOCK.lock().unwrap();
    let db = setup_db().await;
    let state = test_state(db.clone());
    let seeded = seed(&db).await;
    let reg = registrar(&state, &seeded, 1);
    let out = reg
        .register(&serde_json::json!({
            "mode": "interactive", "instruction": "screenshot",
            "content_type": "image/png", "max_bytes": 100, "deadline_secs": 600
        }))
        .await;
    let parsed: serde_json::Value = serde_json::from_str(&out).unwrap();
    let test_id: Uuid = parsed["test_id"].as_str().unwrap().parse().unwrap();
    let artifact_dir = parsed["artifact_path"].as_str().unwrap().to_string();
    // One in-flight attempt, as the dispatch loop would create.
    let probe_id = Uuid::new_v4();
    probes::ActiveModel {
        id: Set(probe_id),
        test_id: Set(test_id),
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
        deadline_at: Set(Utc::now() + Duration::seconds(600)),
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

    let repos_base = tempfile::tempdir().unwrap();
    let work = repos_base.path().join("work");
    std::fs::create_dir_all(work.join(artifact_dir.trim_end_matches('/'))).unwrap();
    std::fs::write(
        work.join(artifact_dir.trim_end_matches('/'))
            .join("big.png"),
        vec![0u8; 4096],
    )
    .unwrap();
    let git = |args: &[&str]| {
        let outp = std::process::Command::new("git")
            .arg("-C")
            .arg(&work)
            .args(args)
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@t")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@t")
            .output()
            .expect("git");
        assert!(outp.status.success());
    };
    git(&["init", "-q", "-b", "main"]);
    git(&["add", "."]);
    git(&["commit", "-q", "-m", "artifact: too big"]);
    let bare = repos_base
        .path()
        .join(seeded.session_id.to_string())
        .join(format!("{}.git", seeded.player_id));
    std::fs::create_dir_all(bare.parent().unwrap()).unwrap();
    let outp = std::process::Command::new("git")
        .args(["clone", "-q", "--bare"])
        .arg(&work)
        .arg(&bare)
        .output()
        .unwrap();
    assert!(outp.status.success());

    unsafe { std::env::set_var("OLOLO_GIT_REPOS_DIR", repos_base.path()) };
    game_server::probe_scheduler::tick(&state)
        .await
        .expect("tick");
    unsafe { std::env::remove_var("OLOLO_GIT_REPOS_DIR") };

    let probe = probes::Entity::find_by_id(probe_id)
        .one(&db)
        .await
        .unwrap()
        .expect("probe");
    assert_eq!(
        probe.outcome.as_deref(),
        Some("error"),
        "oversized = a fact, not a pass"
    );
    assert_eq!(probe.result_json.unwrap()["artifact"]["within_cap"], false);
}

/// The judge decides what artifacts it needs from the work itself — a TODO
/// list with a landing page and a profile page means two captures. Distinct
/// instructions register distinct probes within the judge's budget; the
/// budget refuses the one-past-the-end request with a message.
#[tokio::test]
async fn distinct_instructions_register_distinct_probes_within_budget() {
    let db = setup_db().await;
    let state = test_state(db.clone());
    let seeded = seed(&db).await;
    connect_agent(&state, seeded.player_id);
    let reg = JudgeProbeRegistrar::new(
        state.clone(),
        seeded.session_id,
        seeded.player_id,
        seeded.task_id,
        seeded.judge_id,
        "ux-review".to_string(),
        2, // per-judge budget
        4, // task-wide cap
        true,
    );

    let landing = reg
        .register(&serde_json::json!({
            "mode": "interactive",
            "instruction": "Screenshot the landing page at desktop and mobile widths"
        }))
        .await;
    let profile = reg
        .register(&serde_json::json!({
            "mode": "interactive",
            "instruction": "Screenshot the user profile page at desktop and mobile widths"
        }))
        .await;
    let landing: serde_json::Value = serde_json::from_str(&landing).unwrap();
    let profile: serde_json::Value = serde_json::from_str(&profile).unwrap();
    assert_eq!(landing["status"], "queued");
    assert_eq!(profile["status"], "queued");
    assert_ne!(
        landing["test_id"], profile["test_id"],
        "two distinct probes"
    );

    // A third distinct request exceeds the budget of 2 — refused, not failed.
    let third = reg
        .register(&serde_json::json!({
            "mode": "interactive",
            "instruction": "Screenshot the settings page"
        }))
        .await;
    let third: serde_json::Value = serde_json::from_str(&third).unwrap();
    assert!(
        third["error"]
            .as_str()
            .is_some_and(|e| e.contains("budget")),
        "{third}"
    );

    // Re-asking for the landing page dedupes onto the earlier request.
    let again = reg
        .register(&serde_json::json!({
            "mode": "interactive",
            "instruction": "Screenshot the landing page at desktop and mobile widths"
        }))
        .await;
    let again: serde_json::Value = serde_json::from_str(&again).unwrap();
    assert_eq!(again["test_id"], landing["test_id"]);

    let count = tests::Entity::find()
        .filter(tests::Column::RegisteredByJudgeId.eq(seeded.judge_id))
        .all(&db)
        .await
        .unwrap()
        .len();
    assert_eq!(count, 2, "budget bounds the registrations");
}

/// A judge re-driven after the session ended (or after the agent dropped)
/// must not open a request nobody can answer — session MWCAD4 waited 15
/// minutes on exactly that zombie before timing out.
#[tokio::test]
async fn registration_refuses_when_no_agent_can_answer() {
    let db = setup_db().await;
    let state = test_state(db.clone());
    let seeded = seed(&db).await;
    // No connect_agent: the registry is empty, as after a disconnect.
    let reg = JudgeProbeRegistrar::new(
        state.clone(),
        seeded.session_id,
        seeded.player_id,
        seeded.task_id,
        seeded.judge_id,
        "ux-review".to_string(),
        1,
        4,
        true,
    );
    let out = reg
        .register(&serde_json::json!({
            "mode": "interactive",
            "instruction": "Screenshot the page"
        }))
        .await;
    let parsed: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert!(
        parsed["error"]
            .as_str()
            .is_some_and(|e| e.contains("no longer reachable")),
        "{out}"
    );
    assert!(!reg.interactive_pending(), "nothing was opened");
}

/// A judge whose evidence cannot carry images (no `images` need) must not
/// send the participant off to record a capture it would never see —
/// session 6YJGRC had the data and code-quality judges both collect
/// screencasts that their evidence pipeline cannot show them.
#[tokio::test]
async fn visual_request_from_a_text_only_judge_is_refused() {
    let db = setup_db().await;
    let state = test_state(db.clone());
    let seeded = seed(&db).await;
    connect_agent(&state, seeded.player_id);
    let reg = JudgeProbeRegistrar::new(
        state.clone(),
        seeded.session_id,
        seeded.player_id,
        seeded.task_id,
        seeded.judge_id,
        "data".to_string(),
        1,
        4,
        false, // needs: [probes] — no images
    );
    let out = reg
        .register(&serde_json::json!({
            "mode": "interactive",
            "instruction": "Record a screencast of the widget switching cities",
            "content_type": "video/webm"
        }))
        .await;
    let parsed: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert!(
        parsed["error"]
            .as_str()
            .is_some_and(|e| e.contains("visual")),
        "{out}"
    );
    // A text artifact is still fine for the same judge.
    let ok = reg
        .register(&serde_json::json!({
            "mode": "interactive",
            "instruction": "Write a short data-flow note",
            "content_type": "text/markdown"
        }))
        .await;
    let parsed: serde_json::Value = serde_json::from_str(&ok).unwrap();
    assert!(parsed["test_id"].is_string(), "{ok}");
}

/// Once the session is finished nobody is left to deliver: a re-driven
/// judge must verdict from what it has instead of opening a request that
/// dangles forever (6YJGRC registered fresh captures 15s after finish).
#[tokio::test]
async fn registration_refuses_after_the_session_finished() {
    let db = setup_db().await;
    let state = test_state(db.clone());
    let seeded = seed(&db).await;
    connect_agent(&state, seeded.player_id);
    let mut session: sessions::ActiveModel = sessions::Entity::find_by_id(seeded.session_id)
        .one(&db)
        .await
        .unwrap()
        .unwrap()
        .into();
    session.finished_at = Set(Some(Utc::now()));
    session.update(&db).await.expect("finish session");

    let reg = registrar(&state, &seeded, 1);
    let out = reg
        .register(&serde_json::json!({
            "mode": "interactive",
            "instruction": "Screenshot the page"
        }))
        .await;
    let parsed: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert!(
        parsed["error"]
            .as_str()
            .is_some_and(|e| e.contains("session is over")),
        "{out}"
    );
}

/// Artifacts one judge already collected are offered to the next asker:
/// the call returns the inventory instead of registering, and only an
/// explicit `confirm: true` opens a genuinely new request — 6YJGRC had
/// three judges collect the same forecast screencast from the player.
#[tokio::test]
async fn delivered_artifacts_are_offered_for_reuse_before_a_new_capture() {
    let db = setup_db().await;
    let state = test_state(db.clone());
    let seeded = seed(&db).await;
    connect_agent(&state, seeded.player_id);
    let reg = JudgeProbeRegistrar::new(
        state.clone(),
        seeded.session_id,
        seeded.player_id,
        seeded.task_id,
        seeded.judge_id,
        "ux-review".to_string(),
        3,
        6,
        true,
    );

    // First request registers and gets its artifact delivered.
    let first = reg
        .register(&serde_json::json!({
            "mode": "interactive",
            "instruction": "Record the forecast flow as a screencast"
        }))
        .await;
    let parsed: serde_json::Value = serde_json::from_str(&first).unwrap();
    let test_id: Uuid = parsed["test_id"].as_str().unwrap().parse().unwrap();
    probes::ActiveModel {
        id: Set(Uuid::new_v4()),
        test_id: Set(test_id),
        player_id: Set(seeded.player_id),
        session_id: Set(seeded.session_id),
        attempt: Set(1),
        rendered_command: Set(String::new()),
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
        duration_ms: Set(None),
        point_delta: Set(None),
        result_json: Set(None),
        artifact_path: Set(Some("blob:forecast-flow.webm".to_string())),
    }
    .insert(&db)
    .await
    .expect("delivered probe row");

    // The next distinct request is answered with the inventory, not a probe.
    let second = reg
        .register(&serde_json::json!({
            "mode": "interactive",
            "instruction": "Record the city switching flow as a screencast"
        }))
        .await;
    let parsed: serde_json::Value = serde_json::from_str(&second).unwrap();
    assert!(
        parsed["already_delivered"].is_array() && parsed["test_id"].is_null(),
        "{second}"
    );

    // Insisting with confirm: true opens the new request.
    let confirmed = reg
        .register(&serde_json::json!({
            "mode": "interactive",
            "instruction": "Record the city switching flow as a screencast",
            "confirm": true
        }))
        .await;
    let parsed: serde_json::Value = serde_json::from_str(&confirmed).unwrap();
    assert!(parsed["test_id"].is_string(), "{confirmed}");
}

/// A re-driven run rephrases its ask: exact-instruction dedupe missed the
/// reworded "capture desktop.png + mobile.png" in session TTGNDZ and the
/// participant recorded the same screenshots twice. An OPEN request of the
/// same content type by the same judge on the same task now answers with
/// the existing request instead of opening another; `confirm: true` still
/// allows a genuinely different extra capture.
#[tokio::test]
async fn open_same_type_request_absorbs_a_reworded_duplicate() {
    let db = setup_db().await;
    let state = test_state(db.clone());
    let seeded = seed(&db).await;
    connect_agent(&state, seeded.player_id);
    let reg = JudgeProbeRegistrar::new(
        state.clone(),
        seeded.session_id,
        seeded.player_id,
        seeded.task_id,
        seeded.judge_id,
        "ux-review".to_string(),
        3,
        6,
        true,
    );

    let first = reg
        .register(&serde_json::json!({
            "mode": "interactive",
            "instruction": "Capture desktop.png and mobile.png of the page",
            "content_type": "image/png"
        }))
        .await;
    let parsed: serde_json::Value = serde_json::from_str(&first).unwrap();
    let first_id = parsed["test_id"].as_str().unwrap().to_string();

    // A re-driven run = a fresh registrar for the same judge. Same content
    // type, different wording — absorbed into the earlier open request.
    let redriven = JudgeProbeRegistrar::new(
        state.clone(),
        seeded.session_id,
        seeded.player_id,
        seeded.task_id,
        seeded.judge_id,
        "ux-review".to_string(),
        3,
        6,
        true,
    );
    let reworded = redriven
        .register(&serde_json::json!({
            "mode": "interactive",
            "instruction": "Please capture the shipped page as desktop.png plus mobile.png",
            "content_type": "image/png"
        }))
        .await;
    let parsed: serde_json::Value = serde_json::from_str(&reworded).unwrap();
    assert_eq!(
        parsed["test_id"].as_str(),
        Some(first_id.as_str()),
        "{reworded}"
    );
    assert_eq!(parsed["status"].as_str(), Some("waiting"), "{reworded}");

    // A deliberately different capture gets through with confirm.
    let confirmed = redriven
        .register(&serde_json::json!({
            "mode": "interactive",
            "instruction": "Capture the error state as error.png",
            "content_type": "image/png",
            "confirm": true
        }))
        .await;
    let parsed: serde_json::Value = serde_json::from_str(&confirmed).unwrap();
    assert!(parsed["test_id"].is_string(), "{confirmed}");
    assert_ne!(parsed["test_id"].as_str(), Some(first_id.as_str()));
}

#[tokio::test]
async fn matching_another_judges_open_request_attaches_instead_of_duplicating() {
    use arena_core::entities::artifact_request_watchers;

    let db = setup_db().await;
    let state = test_state(db.clone());
    let seeded = seed(&db).await;
    connect_agent(&state, seeded.player_id);

    // A second judge on the same task with its own interactive budget.
    let other_judge_id = judges::ActiveModel {
        id: Set(Uuid::new_v4()),
        slug: Set("ux-review".to_string()),
        name: Set("UX Review".to_string()),
        description: Set(String::new()),
        prompt: Set("judge".to_string()),
        rating_scale: Set(serde_json::json!({"min": 0.0, "max": 10.0, "step": 0.1})),
        kind: Set("llm".to_string()),
        scope: Set("task".to_string()),
        evidence_mode: Set("tools".to_string()),
        evidence_needs: Set(None),
        criteria: Set(Some(r#"["product"]"#.to_string())),
        probes_config: Set(None),
        max_interactive: Set(Some(1)),
        avatar_url: Set(None),
        ignore_paths: Set(None),
        llm_provider_id_fk: Set(None),
        llm_model: Set(None),
        llm_pool_id_fk: Set(None),
        llm_source_order: Set("pool_first".to_string()),
        created_at: Set(Utc::now()),
        updated_at: Set(Utc::now()),
    }
    .insert(&db)
    .await
    .expect("second judge")
    .id;

    let first_judge = JudgeProbeRegistrar::new(
        state.clone(),
        seeded.session_id,
        seeded.player_id,
        seeded.task_id,
        seeded.judge_id,
        "product-fit".to_string(),
        1,
        6,
        true,
    );
    let first = first_judge
        .register(&serde_json::json!({
            "mode": "interactive",
            "instruction": "Capture desktop.png and mobile.png of the page",
            "content_type": "image/png"
        }))
        .await;
    let parsed: serde_json::Value = serde_json::from_str(&first).unwrap();
    let first_id = parsed["test_id"].as_str().unwrap().to_string();

    // The second judge asks for an equivalent capture: no new request goes
    // out — it is attached to the open one and its run ends waiting.
    let second_judge = JudgeProbeRegistrar::new(
        state.clone(),
        seeded.session_id,
        seeded.player_id,
        seeded.task_id,
        other_judge_id,
        "ux-review".to_string(),
        1,
        6,
        true,
    );
    let attached = second_judge
        .register(&serde_json::json!({
            "mode": "interactive",
            "instruction": "Show me the landing page rendered on desktop and phone",
            "content_type": "image/png"
        }))
        .await;
    let parsed: serde_json::Value = serde_json::from_str(&attached).unwrap();
    assert_eq!(
        parsed["test_id"].as_str(),
        Some(first_id.as_str()),
        "{attached}"
    );
    assert_eq!(parsed["status"].as_str(), Some("waiting"), "{attached}");
    assert!(parsed["open_instruction"].is_string(), "{attached}");
    assert!(second_judge.interactive_pending());

    let watcher = artifact_request_watchers::Entity::find()
        .filter(artifact_request_watchers::Column::JudgeId.eq(other_judge_id))
        .one(&db)
        .await
        .expect("query watchers")
        .expect("watcher row");
    assert_eq!(watcher.test_id.to_string(), first_id);

    // Attaching twice stays a single subscription, same reply.
    let again = second_judge
        .register(&serde_json::json!({
            "mode": "interactive",
            "instruction": "Different words, same ask: desktop + mobile screenshots",
            "content_type": "image/png"
        }))
        .await;
    let parsed: serde_json::Value = serde_json::from_str(&again).unwrap();
    assert_eq!(
        parsed["test_id"].as_str(),
        Some(first_id.as_str()),
        "{again}"
    );
    let watcher_count = artifact_request_watchers::Entity::find()
        .filter(artifact_request_watchers::Column::JudgeId.eq(other_judge_id))
        .all(&db)
        .await
        .expect("query watchers")
        .len();
    assert_eq!(watcher_count, 1);

    // Insisting on a genuinely different capture still works.
    let confirmed = second_judge
        .register(&serde_json::json!({
            "mode": "interactive",
            "instruction": "Capture the empty state as empty.png",
            "content_type": "image/png",
            "confirm": true
        }))
        .await;
    let parsed: serde_json::Value = serde_json::from_str(&confirmed).unwrap();
    assert!(parsed["test_id"].is_string(), "{confirmed}");
    assert_ne!(parsed["test_id"].as_str(), Some(first_id.as_str()));
}

/// A judge asking for a command to be run asks the PLAYER to run it.
///
/// It used to run the command itself, in a scratch copy of the snapshot: a
/// registered `npm test` failed there with
/// `Could not find '/tmp/.tmpXXXX/test/**/*.test.ts'` while the very same
/// command passed in the player's workspace, and the verdict was written off
/// the server's answer. Nothing executes here now — the row is queued for the
/// participant's loop, and the run is revisited when it lands.
#[tokio::test]
async fn a_registered_command_is_queued_to_the_player_not_run_here() {
    let db = setup_db().await;
    let state = test_state(db.clone());
    let seeded = seed(&db).await;
    let reg = registrar(&state, &seeded, 1);

    let out = reg
        .register(&serde_json::json!({
            "mode": "deterministic",
            "command": "npm test",
            "validation": "exit_code === 0",
            "purpose": "Run the committed suite"
        }))
        .await;
    let parsed: serde_json::Value = serde_json::from_str(&out).unwrap();

    // The answer is a queue slip, not a result: no outcome, no output.
    assert_eq!(parsed["status"], "queued", "{parsed}");
    assert!(parsed.get("outcome").is_none(), "{parsed}");
    assert!(parsed.get("output").is_none(), "{parsed}");

    let test_id: Uuid = parsed["test_id"].as_str().unwrap().parse().unwrap();
    let test = tests::Entity::find_by_id(test_id)
        .one(&db)
        .await
        .unwrap()
        .expect("tests row");
    let config =
        arena_core::evaluation::ProbeConfig::from_json(test.probe_config.as_ref().expect("config"))
            .expect("parse");
    assert_eq!(
        config.effective_executor(),
        arena_core::evaluation::ProbeExecutor::Participant,
        "a judge's command belongs on the player's machine"
    );

    // And nothing ran: no probe row exists until the participant answers.
    let ran = arena_core::entities::probes::Entity::find()
        .filter(arena_core::entities::probes::Column::TestId.eq(test_id))
        .all(&db)
        .await
        .unwrap();
    assert!(ran.is_empty(), "the server executed it anyway: {ran:?}");

    // The judge is told to verdict on what it has and be revisited.
    assert!(reg.interactive_pending(), "the run must be revisited");
}
