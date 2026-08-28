// ENV_LOCK guards are held across awaits on purpose: they serialize the
// tests that mutate process-global env vars, and the guarded sections span
// the awaited probe runs.
#![allow(clippy::await_holding_lock)]

//! Server-side deterministic probes: materialize the player's pushed HEAD,
//! run in the (unsandboxed, opt-in) backend, grade with the shared grader,
//! record an ordinary `probes` row. Also the `unavailable` semantics: an
//! empty repo or a refused sandbox is our problem, never the player's.

use std::path::Path;
use std::sync::Mutex;

use arena_core::entities::{players, probes, projects, sessions, tasks, tests, users};
use arena_core::evaluation::{OUTCOME_UNAVAILABLE, ProbeConfig};
use arena_core::session_status::SessionStatus;
use chrono::Utc;
use game_server::probe_exec::{record_server_probe, run_deterministic_probe};
use migration::MigratorTrait;
use sea_orm::{ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, Set};
use uuid::Uuid;

// Env vars are process-global; serialize the tests that flip them.
static ENV_LOCK: Mutex<()> = Mutex::new(());

async fn setup_db() -> DatabaseConnection {
    let db = sea_orm::Database::connect("sqlite::memory:")
        .await
        .expect("connect");
    migration::Migrator::up(&db, None).await.expect("migrate");
    db
}

fn git(dir: &Path, args: &[&str]) {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@t")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@t")
        .output()
        .expect("git spawn");
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// A bare repo whose HEAD contains `answer.sh` printing `42`.
fn fixture_repo() -> (tempfile::TempDir, std::path::PathBuf) {
    let root = tempfile::tempdir().expect("tmp");
    let work = root.path().join("work");
    std::fs::create_dir_all(&work).unwrap();
    git(&work, &["init", "-q", "-b", "main"]);
    std::fs::write(work.join("answer.sh"), "echo 42\n").unwrap();
    git(&work, &["add", "."]);
    git(&work, &["commit", "-q", "-m", "feat: answer"]);
    let bare = root.path().join("player.git");
    let out = std::process::Command::new("git")
        .arg("clone")
        .arg("-q")
        .arg("--bare")
        .arg(&work)
        .arg(&bare)
        .output()
        .expect("git clone");
    assert!(out.status.success());
    (root, bare)
}

/// user → project → session → player → task, so the FK chains hold.
async fn seed_chain(db: &DatabaseConnection) -> (Uuid, Uuid, Uuid) {
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
        join_code: Set(format!("T{}", &Uuid::new_v4().simple().to_string()[..5]).to_uppercase()),
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

    let task_id = tasks::ActiveModel {
        id: Set(Uuid::new_v4()),
        project_id_fk: Set(project_id),
        ordinal: Set(0),
        title: Set("t".to_string()),
        content: Set(String::new()),
        test_template: Set(serde_json::json!({"kind":"shell","command_template":"echo ok"})),
        created_at: Set(Utc::now()),
        tags: Set(String::new()),
        point_value: Set(10),
        deadline_secs: Set(None),
        min_interval_secs: Set(None),
        interval_increment_secs: Set(None),
        max_interval_secs: Set(None),
        fail_points: Set(0),
        no_response_points: Set(0),
        completion_bonus_points: Set(0),
        evaluation: Set(None),
    }
    .insert(db)
    .await
    .expect("insert task")
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
    .expect("insert player")
    .id;

    (session_id, player_id, task_id)
}

async fn insert_test_row(
    db: &DatabaseConnection,
    session_id: Uuid,
    task_id: Uuid,
    command: &str,
    validation: &str,
) -> tests::Model {
    tests::ActiveModel {
        id: Set(Uuid::new_v4()),
        command_template: Set(command.to_string()),
        answer_template: Set(validation.to_string()),
        fixture_definitions: Set(r#"{"kind":"js","script":"({})"}"#.to_string()),
        created_at: Set(Utc::now()),
        session_id: Set(session_id),
        task_id: Set(task_id),
        ordinal: Set(0),
        prompt: Set(String::new()),
        description: Set(None),
        probe_config: Set(None),
        initiator: Set("system".to_string()),
        registered_by_judge_id: Set(None),
    }
    .insert(db)
    .await
    .expect("insert test")
}

fn det_config(points: Option<(i32, i32)>) -> ProbeConfig {
    let mut json = serde_json::json!({ "mode": "deterministic" });
    if let Some((pass, fail)) = points {
        json["points"] = serde_json::json!({ "pass": pass, "fail": fail });
    }
    ProbeConfig::from_json(&json).expect("config")
}

#[tokio::test]
async fn passing_probe_records_snapshot_and_measurement() {
    let _guard = ENV_LOCK.lock().unwrap();
    // safety: serialized by ENV_LOCK
    unsafe {
        std::env::set_var("OLOLO_SANDBOX", "none");
        std::env::set_var("OLOLO_ALLOW_UNSANDBOXED_EXEC", "1");
    }

    let db = setup_db().await;
    let (_root, bare) = fixture_repo();
    let (session_id, player_id, task_id) = seed_chain(&db).await;
    let test = insert_test_row(
        &db,
        session_id,
        task_id,
        "sh answer.sh",
        r#"result.trim() === "42""#,
    )
    .await;

    let memory = std::collections::BTreeMap::new();
    let run = run_deterministic_probe(
        &db,
        &bare,
        session_id,
        player_id,
        &test,
        &det_config(Some((3, -1))),
        &memory,
    )
    .await;

    assert_eq!(run.outcome, "pass", "result_json: {}", run.result_json);
    assert_eq!(run.point_delta, 3, "opt-in pass points apply");
    assert_eq!(run.output.trim(), "42");
    let sha = run.result_json["snapshot_commit"].as_str().expect("sha");
    assert_eq!(sha.len(), 40);
    assert!(run.result_json["snapshot_age_secs"].as_i64().unwrap() >= 0);

    // Recorded as an ordinary probes row, resolved in one step.
    record_server_probe(&db, session_id, player_id, test.id, &run)
        .await
        .expect("record");
    let row = probes::Entity::find()
        .filter(probes::Column::TestId.eq(test.id))
        .one(&db)
        .await
        .unwrap()
        .expect("probe row");
    assert_eq!(row.outcome.as_deref(), Some("pass"));
    assert_eq!(row.point_delta, Some(3));
    assert!(row.resolved_at.is_some());
    assert_eq!(
        row.result_json.unwrap()["snapshot_commit"]
            .as_str()
            .unwrap(),
        sha
    );

    // safety: serialized by ENV_LOCK
    unsafe {
        std::env::remove_var("OLOLO_SANDBOX");
        std::env::remove_var("OLOLO_ALLOW_UNSANDBOXED_EXEC");
    }
}

#[tokio::test]
async fn failing_validation_is_error_with_default_zero_points() {
    let _guard = ENV_LOCK.lock().unwrap();
    // safety: serialized by ENV_LOCK
    unsafe {
        std::env::set_var("OLOLO_SANDBOX", "none");
        std::env::set_var("OLOLO_ALLOW_UNSANDBOXED_EXEC", "1");
    }

    let db = setup_db().await;
    let (_root, bare) = fixture_repo();
    let (session_id, player_id, task_id) = seed_chain(&db).await;
    let test = insert_test_row(
        &db,
        session_id,
        task_id,
        "sh answer.sh",
        r#"result.trim() === "not-42""#,
    )
    .await;

    let memory = std::collections::BTreeMap::new();
    let run = run_deterministic_probe(
        &db,
        &bare,
        session_id,
        player_id,
        &test,
        &det_config(None),
        &memory,
    )
    .await;

    assert_eq!(run.outcome, "error", "result_json: {}", run.result_json);
    assert_eq!(run.point_delta, 0, "measurements default to 0 points");

    // safety: serialized by ENV_LOCK
    unsafe {
        std::env::remove_var("OLOLO_SANDBOX");
        std::env::remove_var("OLOLO_ALLOW_UNSANDBOXED_EXEC");
    }
}

#[tokio::test]
async fn empty_repo_is_unavailable_never_a_penalty() {
    let _guard = ENV_LOCK.lock().unwrap();
    // safety: serialized by ENV_LOCK
    unsafe {
        std::env::set_var("OLOLO_SANDBOX", "none");
        std::env::set_var("OLOLO_ALLOW_UNSANDBOXED_EXEC", "1");
    }

    let db = setup_db().await;
    let root = tempfile::tempdir().unwrap();
    let bare = root.path().join("empty.git");
    let out = std::process::Command::new("git")
        .args(["init", "-q", "--bare"])
        .arg(&bare)
        .output()
        .unwrap();
    assert!(out.status.success());

    let (session_id, player_id, task_id) = seed_chain(&db).await;
    let test = insert_test_row(&db, session_id, task_id, "echo hi", "").await;

    let memory = std::collections::BTreeMap::new();
    let run = run_deterministic_probe(
        &db,
        &bare,
        session_id,
        player_id,
        &test,
        &det_config(Some((5, -5))),
        &memory,
    )
    .await;

    assert_eq!(run.outcome, OUTCOME_UNAVAILABLE);
    assert_eq!(run.point_delta, 0, "unavailable never charges points");
    assert!(
        run.result_json["unavailable_reason"]
            .as_str()
            .unwrap()
            .contains("no commits"),
    );

    // safety: serialized by ENV_LOCK
    unsafe {
        std::env::remove_var("OLOLO_SANDBOX");
        std::env::remove_var("OLOLO_ALLOW_UNSANDBOXED_EXEC");
    }
}

#[tokio::test]
async fn refused_sandbox_is_unavailable() {
    let _guard = ENV_LOCK.lock().unwrap();
    // Force the unsandboxed backend WITHOUT the opt-in: must refuse.
    // safety: serialized by ENV_LOCK
    unsafe {
        std::env::set_var("OLOLO_SANDBOX", "none");
        std::env::remove_var("OLOLO_ALLOW_UNSANDBOXED_EXEC");
    }

    let db = setup_db().await;
    let (_root, bare) = fixture_repo();
    let (session_id, player_id, task_id) = seed_chain(&db).await;
    let test = insert_test_row(&db, session_id, task_id, "echo hi", "").await;

    let memory = std::collections::BTreeMap::new();
    let run = run_deterministic_probe(
        &db,
        &bare,
        session_id,
        player_id,
        &test,
        &det_config(None),
        &memory,
    )
    .await;

    assert_eq!(run.outcome, OUTCOME_UNAVAILABLE);
    assert!(
        run.result_json["unavailable_reason"]
            .as_str()
            .unwrap()
            .contains("sandbox"),
    );

    // safety: serialized by ENV_LOCK
    unsafe { std::env::remove_var("OLOLO_SANDBOX") };
}

#[tokio::test]
async fn analysis_probe_with_unknown_tool_is_unavailable() {
    let db = setup_db().await;
    let (_root, bare) = fixture_repo();
    let (session_id, player_id, task_id) = seed_chain(&db).await;
    let test = insert_test_row(&db, session_id, task_id, "", "").await;

    let config = ProbeConfig::from_json(&serde_json::json!({
        "mode": "analysis", "tool": "teleport"
    }))
    .unwrap();
    let memory = std::collections::BTreeMap::new();
    let run = game_server::probe_exec::run_analysis_probe(
        &bare, session_id, player_id, &test, &config, &memory,
    )
    .await;
    assert_eq!(run.outcome, OUTCOME_UNAVAILABLE);
    assert!(
        run.result_json["unavailable_reason"]
            .as_str()
            .unwrap()
            .contains("unknown analysis tool"),
    );
    assert_eq!(run.point_delta, 0);
}
