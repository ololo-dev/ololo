//! Integration tests for judge execution: semaphore concurrency,
//! task-commit resolution, internal REST endpoint, and error-skip paths.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;
use dashmap::DashMap;
use jsonwebtoken::{DecodingKey, EncodingKey};
use migration::{Migrator, MigratorTrait};
use sea_orm::{ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, Set};
use serde_json::json;
use tempfile::TempDir;
use tokio::sync::Semaphore;
use uuid::Uuid;

use arena_core::entities::{
    judge_results, judges, players, projects, sessions, task_judges, tasks, users,
};
use arena_core::judging::{AgentResponse, JudgeError, JudgeLlm, ToolDef};
use arena_core::session_status::SessionStatus;
use game_server::judge_queue::{enqueue_judge_run, execute_judge_run, resolve_judge_run};
use game_server::state::GameServerState;
use game_server::zmq_pub::NoopEventPublisher;

// ---------------------------------------------------------------------------
// ScriptedFakeJudgeLlm — returns a canned final verdict (no delay).
// ---------------------------------------------------------------------------

struct ScriptedFakeJudgeLlm;

#[async_trait]
impl JudgeLlm for ScriptedFakeJudgeLlm {
    async fn run_agent(
        &self,
        _system: &str,
        _user: &str,
        _tools: Vec<ToolDef>,
        _prior_tool_result: Option<&str>,
    ) -> Result<AgentResponse, JudgeError> {
        Ok(AgentResponse::Final {
            text: r#"{"rating": 7.0, "feedback": "good"}"#.to_string(),
        })
    }
}

// ---------------------------------------------------------------------------
// Test DB + state helpers.
// ---------------------------------------------------------------------------

async fn setup_db() -> DatabaseConnection {
    let db = sea_orm::Database::connect("sqlite::memory:")
        .await
        .expect("sqlite connect");
    Migrator::up(&db, None).await.expect("migrate up");
    db
}

async fn insert_user(db: &DatabaseConnection) -> Uuid {
    let id = Uuid::new_v4();
    users::ActiveModel {
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
    .expect("insert user");
    id
}

async fn insert_project(db: &DatabaseConnection, owner_id: Uuid) -> Uuid {
    let id = Uuid::new_v4();
    projects::ActiveModel {
        id: Set(id),
        name: Set("proj".to_string()),
        slug: Set(None),
        description: Set(String::new()),
        category: Set(None),
        tags: Set(String::new()),
        cover_image_url: Set(None),
        owner_user_id_fk: Set(owner_id),
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
    .expect("insert project");
    id
}

async fn insert_session(db: &DatabaseConnection, project_id: Uuid) -> Uuid {
    let id = Uuid::new_v4();
    sessions::ActiveModel {
        id: Set(id),
        name: Set("s".to_string()),
        created_at: Set(Utc::now()),
        owner_id_fk: Set(None),
        status: Set(SessionStatus::Running),
        join_code: Set("JC1".to_string()),
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
    .expect("insert session");
    id
}

async fn insert_player(db: &DatabaseConnection, session_id: Uuid) -> Uuid {
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
    .expect("insert player");
    id
}

async fn insert_chain(db: &DatabaseConnection, project_id: Uuid) -> (Uuid, Uuid, Uuid) {
    let task_id = Uuid::new_v4();
    tasks::ActiveModel {
        id: Set(task_id),
        project_id_fk: Set(project_id),
        ordinal: Set(0),
        title: Set("Reverse a string".to_string()),
        content: Set("Write a function that reverses a string.".to_string()),
        test_template: Set(json!({"kind":"shell","command_template":"echo ok"})),
        created_at: Set(Utc::now()),
        tags: Set("algorithms".to_string()),
        point_value: Set(10),
        deadline_secs: Set(Some(300)),
        min_interval_secs: Set(Some(5)),
        interval_increment_secs: Set(Some(0)),
        max_interval_secs: Set(Some(300)),
        fail_points: Set(0),
        no_response_points: Set(0),
        completion_bonus_points: Set(0),
        evaluation: Set(None),
    }
    .insert(db)
    .await
    .expect("insert task");

    let judge_id = Uuid::new_v4();
    judges::ActiveModel {
        id: Set(judge_id),
        slug: Set("code-quality".to_string()),
        name: Set("Code Quality Judge".to_string()),
        description: Set(String::new()),
        prompt: Set("Evaluate code quality.".to_string()),
        rating_scale: Set(json!({"min": 0.0, "max": 10.0, "step": 0.5})),
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
        max_interactive: Set(None),
        avatar_url: Set(None),
        ignore_paths: Set(None),
        probes_config: Set(None),
    }
    .insert(db)
    .await
    .expect("insert judge");

    let task_judge_id = Uuid::new_v4();
    task_judges::ActiveModel {
        id: Set(task_judge_id),
        task_id: Set(task_id),
        judge_id: Set(judge_id),
        ordinal: Set(0),
        rating_scale_override: Set(None),
        created_at: Set(Utc::now()),
        updated_at: Set(Utc::now()),
        weight: Set(None),
    }
    .insert(db)
    .await
    .expect("insert task_judge");

    (task_id, judge_id, task_judge_id)
}

/// Model config passed to `execute_judge_run` in these tests — recording
/// only (the fake `JudgeLlm` never issues a network call).
fn test_model_cfg() -> arena_core::llm::ModelConfig {
    arena_core::llm::ModelConfig {
        provider_name: None,
        provider: "ollama".to_string(),
        model: "test-model".to_string(),
        base_url: None,
        api_key: None,
    }
}

fn make_state(db: DatabaseConnection, judge_max: usize) -> GameServerState {
    let secret = b"test-secret-32-bytes-or-more-xxxxxxx".to_vec();
    GameServerState {
        db: db.clone(),
        server_id: Uuid::new_v4(),
        advertise_url: "ws://localhost:8081".to_string(),
        jwt_encoding_key: Arc::new(EncodingKey::from_secret(&secret)),
        jwt_decoding_key: Arc::new(DecodingKey::from_secret(&secret)),
        jwt_signing_secret: Arc::new(secret),
        session_registry: Arc::new(DashMap::new()),
        player_agent_registry: Arc::new(DashMap::new()),
        lobby_timer_secs: 60,
        event_publisher: Arc::new(NoopEventPublisher),
        judge_semaphore: Arc::new(Semaphore::new(judge_max)),
        settings_encryption: std::sync::Arc::new(
            arena_core::settings_encryption::SettingsEncryption::new(
                b"test-secret-key-for-settings-enc",
            ),
        ),
    }
}

// ---------------------------------------------------------------------------
// Git repo helpers (mirror arena-core/src/judging/tests.rs).
// ---------------------------------------------------------------------------

fn git_bin() -> std::path::PathBuf {
    which::which("git").expect("git binary found")
}

fn make_repo(dir: &Path) {
    let git = git_bin();
    let run = |args: &[&str]| {
        let out = std::process::Command::new(&git)
            .arg("-C")
            .arg(dir)
            .args(args)
            .output()
            .expect("git command");
        assert!(
            out.status.success(),
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&out.stderr)
        );
    };
    run(&["init", "-q"]);
    run(&["config", "user.email", "t@t"]);
    run(&["config", "user.name", "T"]);
}

fn commit(dir: &Path, msg: &str) -> String {
    let git = git_bin();
    let out = std::process::Command::new(&git)
        .arg("-C")
        .arg(dir)
        .args(["commit", "-q", "-m", msg])
        .output()
        .expect("git commit");
    assert!(
        out.status.success(),
        "commit failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let out = std::process::Command::new(&git)
        .arg("-C")
        .arg(dir)
        .args(["rev-parse", "HEAD"])
        .output()
        .expect("rev-parse");
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn write_file(dir: &Path, name: &str, content: &str) {
    std::fs::write(dir.join(name), content).expect("write file");
    let git = git_bin();
    let out = std::process::Command::new(&git)
        .arg("-C")
        .arg(dir)
        .args(["add", name])
        .output()
        .expect("git add");
    assert!(out.status.success());
}

// ---------------------------------------------------------------------------
// Test: semaphore limits concurrency to 3.
// ---------------------------------------------------------------------------

/// The scale `run_execution_judge` now expects from its caller, resolved the
/// way production does. A generous payout leaves the judge's own scale
/// intact — these cases are about the grading, not the gate.
fn gated_scale(
    judge: &arena_core::judging::JudgeRow,
    tj: &arena_core::judging::TaskJudgeRow,
) -> arena_core::validation::judge_results::RatingScale {
    match arena_core::judging::gate_task_judge(&judge.rating_scale, &tj.rating_scale_override, 1000)
    {
        arena_core::judging::TaskJudgeGate::Run(scale) => scale,
        arena_core::judging::TaskJudgeGate::Skip { reason } => {
            panic!("unexpected gate skip: {reason}")
        }
    }
}

#[tokio::test]
async fn semaphore_limits_concurrency_to_max() {
    let db = setup_db().await;
    let owner = insert_user(&db).await;
    let project = insert_project(&db, owner).await;
    let session = insert_session(&db, project).await;
    let player = insert_player(&db, session).await;
    let (task_id, _judge_id, _) = insert_chain(&db, project).await;

    // Create 3 additional task_judge rows (4 total) so each concurrent run
    // targets a distinct (task_judge_id, player_id) — avoids UNIQUE constraint
    // races on the judge_results upsert.
    let mut judge_ids = vec![_judge_id];
    for _ in 0..3 {
        let jid = Uuid::new_v4();
        judges::ActiveModel {
            id: Set(jid),
            slug: Set(format!("j{}", judge_ids.len())),
            name: Set(format!("Judge {}", judge_ids.len())),
            description: Set(String::new()),
            prompt: Set("Evaluate code quality.".to_string()),
            rating_scale: Set(json!({"min": 0.0, "max": 10.0, "step": 0.5})),
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
            max_interactive: Set(None),
            avatar_url: Set(None),
            ignore_paths: Set(None),
            probes_config: Set(None),
        }
        .insert(&db)
        .await
        .expect("insert judge");
        task_judges::ActiveModel {
            id: Set(Uuid::new_v4()),
            task_id: Set(task_id),
            judge_id: Set(jid),
            ordinal: Set(judge_ids.len() as i32),
            rating_scale_override: Set(None),
            created_at: Set(Utc::now()),
            updated_at: Set(Utc::now()),
            weight: Set(None),
        }
        .insert(&db)
        .await
        .expect("insert task_judge");
        judge_ids.push(jid);
    }

    // Create a player repo dir under a temp OLOLO_GIT_REPOS_DIR so
    // `repos_base_dir()` resolves to our tempdir.
    let repos_root = tempfile::tempdir().expect("repos tempdir");
    // SAFETY: tests run single-threaded per-process in cargo's default test
    // harness; this env var is scoped to the test process and removed at end.
    unsafe {
        std::env::set_var("OLOLO_GIT_REPOS_DIR", repos_root.path());
    }

    let base = arena_core::git_store::repos_base_dir().expect("base dir");
    let repo_dir = arena_core::git_store::player_repo_path(&base, session, player);
    std::fs::create_dir_all(&repo_dir).expect("mkdir repo");
    make_repo(&repo_dir);
    write_file(&repo_dir, "a.txt", "x");
    commit(&repo_dir, "init");

    let state = make_state(db.clone(), 3);

    let active = Arc::new(AtomicUsize::new(0));
    let max_seen = Arc::new(AtomicUsize::new(0));
    let started = Arc::new(AtomicUsize::new(0));

    // 4 concurrent resolve+execute calls, each with a distinct judge_id.
    // resolve acquires the permit (held through execute). With semaphore=3,
    // the 4th must wait.
    let mut handles = Vec::new();
    for jid in judge_ids {
        let state = state.clone();
        let db = db.clone();
        let llm_active = active.clone();
        let llm_max = max_seen.clone();
        let llm_started = started.clone();
        handles.push(tokio::spawn(async move {
            let resolved = resolve_judge_run(&state, &db, session, player, task_id, jid)
                .await
                .expect("resolve");
            let llm = TrackingFakeJudgeLlm {
                active: llm_active,
                max_seen: llm_max,
                hold_until: 3,
                started: llm_started,
                total: 4,
            };
            execute_judge_run(
                &state,
                &db,
                resolved,
                &llm,
                &test_model_cfg(),
                session,
                player,
                task_id,
                None,
                None,
            )
            .await
            .expect("execute");
        }));
    }

    for h in handles {
        h.await.expect("task join");
    }

    unsafe {
        std::env::remove_var("OLOLO_GIT_REPOS_DIR");
    }

    let peak = max_seen.load(Ordering::SeqCst);
    assert!(peak <= 3, "concurrency exceeded semaphore: peak={peak}");
    assert_eq!(peak, 3, "expected exactly 3 concurrent, got {peak}");
}

struct TrackingFakeJudgeLlm {
    active: Arc<AtomicUsize>,
    max_seen: Arc<AtomicUsize>,
    /// Hold the permit until this many runs are in flight together, so the
    /// observed peak is a fact rather than a race the scheduler usually wins.
    hold_until: usize,
    /// How many runs have entered, and how many ever will. The run that comes
    /// after the semaphore has already been saturated has nobody left to wait
    /// for, and must not sit out the deadline alone.
    started: Arc<AtomicUsize>,
    total: usize,
}

#[async_trait]
impl JudgeLlm for TrackingFakeJudgeLlm {
    async fn run_agent(
        &self,
        _system: &str,
        _user: &str,
        _tools: Vec<ToolDef>,
        _prior_tool_result: Option<&str>,
    ) -> Result<AgentResponse, JudgeError> {
        let cur = self.active.fetch_add(1, Ordering::SeqCst) + 1;
        // Record peak concurrency.
        let mut max = self.max_seen.load(Ordering::SeqCst);
        while cur > max {
            match self
                .max_seen
                .compare_exchange(max, cur, Ordering::SeqCst, Ordering::SeqCst)
            {
                Ok(_) => break,
                Err(actual) => max = actual,
            }
        }
        // Wait for the others rather than sleeping a hopeful 50ms: on a loaded
        // runner the first run finished before the third started and the peak
        // came out 2. The deadline means a semaphore that never lets three
        // through fails the assertion instead of hanging the suite.
        self.started.fetch_add(1, Ordering::SeqCst);
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while self.active.load(Ordering::SeqCst) < self.hold_until
            && self.started.load(Ordering::SeqCst) < self.total
            && std::time::Instant::now() < deadline
        {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        self.active.fetch_sub(1, Ordering::SeqCst);
        Ok(AgentResponse::Final {
            text: r#"{"rating": 5.0, "feedback": "ok"}"#.to_string(),
        })
    }
}

// ---------------------------------------------------------------------------
// Test: resolve_task_commit finds matching commit.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn resolve_task_commit_finds_matching() {
    let dir = tempfile::tempdir().unwrap();
    make_repo(dir.path());
    let task_id = Uuid::new_v4();
    write_file(dir.path(), "a.txt", "x");
    let sha = commit(dir.path(), &format!("feat({task_id}): reverse"));

    let res = arena_core::judging::resolve_task_commit(dir.path(), task_id)
        .await
        .expect("resolve_task_commit");
    let (found_sha, subject) = res.expect("some");
    assert_eq!(found_sha, sha);
    assert!(subject.contains(&task_id.to_string()));
}

#[tokio::test]
async fn resolve_task_commit_no_match_returns_none() {
    let dir = tempfile::tempdir().unwrap();
    make_repo(dir.path());
    write_file(dir.path(), "a.txt", "x");
    commit(dir.path(), "unrelated");

    let res = arena_core::judging::resolve_task_commit(dir.path(), Uuid::new_v4())
        .await
        .expect("resolve_task_commit");
    assert!(res.is_none());
}

// ---------------------------------------------------------------------------
// Test: enqueue_judge_run errors on missing task_judge (caller can log+skip).
// ---------------------------------------------------------------------------

#[tokio::test]
async fn enqueue_judge_run_missing_task_judge_returns_error() {
    let db = setup_db().await;
    let state = make_state(db.clone(), 3);

    // No task_judges row: random UUIDs → error.
    let err = enqueue_judge_run(
        &state,
        &db,
        Uuid::new_v4(),
        Uuid::new_v4(),
        Uuid::new_v4(),
        Uuid::new_v4(),
        false,
    )
    .await
    .expect_err("should error on missing task_judge");

    assert!(
        matches!(err, JudgeError::GitReadError(ref m) if m.contains("task_judge not found")),
        "got {err:?}"
    );
}

// ---------------------------------------------------------------------------
// Test: resolve_judge_run errors on missing task_judge, judge, or task.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn resolve_judge_run_missing_judge_returns_error() {
    let db = setup_db().await;
    let owner = insert_user(&db).await;
    let project = insert_project(&db, owner).await;
    let session = insert_session(&db, project).await;
    let player = insert_player(&db, session).await;
    let (task_id, judge_id, _) = insert_chain(&db, project).await;

    let state = make_state(db.clone(), 3);

    // Delete task_judge first (FK restricts judge deletion), then the judge.
    arena_core::entities::task_judges::Entity::delete_many()
        .filter(arena_core::entities::task_judges::Column::JudgeId.eq(judge_id))
        .exec(&db)
        .await
        .expect("delete task_judge");
    judges::Entity::delete_by_id(judge_id)
        .exec(&db)
        .await
        .expect("delete judge");

    let err = resolve_judge_run(&state, &db, session, player, task_id, judge_id)
        .await
        .expect_err("should error on missing judge");

    assert!(
        matches!(err, JudgeError::GitReadError(ref m) if m.contains("judge not found")),
        "got {err:?}"
    );
}

// ---------------------------------------------------------------------------
// Test: execute_judge_run with a FakeJudgeLlm persists a judge_results row.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn execute_judge_run_persists_result() {
    let db = setup_db().await;
    let owner = insert_user(&db).await;
    let project = insert_project(&db, owner).await;
    let session = insert_session(&db, project).await;
    let player = insert_player(&db, session).await;
    let (task_id, judge_id, _) = insert_chain(&db, project).await;

    // Set up a player repo under a temp repos dir.
    let repos_root = tempfile::tempdir().expect("repos tempdir");
    unsafe {
        std::env::set_var("OLOLO_GIT_REPOS_DIR", repos_root.path());
    }

    let base = arena_core::git_store::repos_base_dir().expect("base dir");
    let repo_dir = arena_core::git_store::player_repo_path(&base, session, player);
    std::fs::create_dir_all(&repo_dir).expect("mkdir repo");
    make_repo(&repo_dir);
    write_file(&repo_dir, "a.txt", "x");
    commit(&repo_dir, "init");

    let state = make_state(db.clone(), 3);

    let resolved = resolve_judge_run(&state, &db, session, player, task_id, judge_id)
        .await
        .expect("resolve");

    let llm = ScriptedFakeJudgeLlm;
    let out = execute_judge_run(
        &state,
        &db,
        resolved,
        &llm,
        &test_model_cfg(),
        session,
        player,
        task_id,
        None,
        None,
    )
    .await
    .expect("execute");

    unsafe {
        std::env::remove_var("OLOLO_GIT_REPOS_DIR");
    }

    assert_eq!(out.rating, 7.0);
    assert_eq!(out.feedback, "good");
    assert_eq!(out.model, "test-model");

    let rows = judge_results::Entity::find().all(&db).await.expect("find");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].player_id_fk, player);
    assert_eq!(rows[0].point_delta, 7);

    // Unified LLM telemetry: the concluded run lands exactly one
    // llm_requests row with the fake run's model and full attribution.
    let telemetry = arena_core::entities::llm_requests::Entity::find()
        .all(&db)
        .await
        .expect("find llm_requests");
    assert_eq!(telemetry.len(), 1, "one row per concluded judge run");
    let row = &telemetry[0];
    assert_eq!(row.operation, "judge");
    assert_eq!(row.model, "test-model");
    assert_eq!(row.provider, "ollama");
    assert_eq!(row.status, "ok");
    assert_eq!(row.session_id, Some(session));
    assert_eq!(row.player_id, Some(player));
    assert_eq!(row.task_id, Some(task_id));
    assert_eq!(row.judge_slug.as_deref(), Some("code-quality"));
    let detail: serde_json::Value =
        serde_json::from_str(row.detail_json.as_deref().expect("detail_json")).expect("json");
    assert_eq!(detail["events"], 0, "no recorder was attached in this test");
}

// ---------------------------------------------------------------------------
// Test: a scored run records what it was looking at, and which definition of
// the judge looked. Without this, a verdict from last week is unexplainable:
// `judges/*.md` is re-seeded over the same slug on every boot.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_run_records_the_snapshot_it_judged_from() {
    let db = setup_db().await;
    let owner = insert_user(&db).await;
    let project = insert_project(&db, owner).await;
    let session = insert_session(&db, project).await;
    let player = insert_player(&db, session).await;
    let (task_id, judge_id, _) = insert_chain(&db, project).await;

    let repos_root = tempfile::tempdir().expect("repos tempdir");
    unsafe {
        std::env::set_var("OLOLO_GIT_REPOS_DIR", repos_root.path());
    }
    let base = arena_core::git_store::repos_base_dir().expect("base dir");
    let repo_dir = arena_core::git_store::player_repo_path(&base, session, player);
    std::fs::create_dir_all(&repo_dir).expect("mkdir repo");
    make_repo(&repo_dir);
    write_file(&repo_dir, "a.txt", "x");
    commit(&repo_dir, "init");

    let state = make_state(db.clone(), 3);
    let resolved = resolve_judge_run(&state, &db, session, player, task_id, judge_id)
        .await
        .expect("resolve");

    let recorder = arena_core::judging::JudgeRunRecorder::default();
    execute_judge_run(
        &state,
        &db,
        resolved,
        &ScriptedFakeJudgeLlm,
        &test_model_cfg(),
        session,
        player,
        task_id,
        Some(&recorder),
        None,
    )
    .await
    .expect("execute");

    unsafe {
        std::env::remove_var("OLOLO_GIT_REPOS_DIR");
    }

    // The same snapshot leads the telemetry trace: the drawer is where an
    // operator asks "on what basis", and a run whose evidence is only in a
    // file on the game server's disk cannot answer there.
    let telemetry = arena_core::entities::llm_requests::Entity::find()
        .all(&db)
        .await
        .expect("find llm_requests");
    assert_eq!(telemetry.len(), 1, "one row per concluded judge run");
    let events: serde_json::Value = serde_json::from_str(
        telemetry[0]
            .events_json
            .as_deref()
            .expect("the run has a trace"),
    )
    .expect("trace is JSON");
    assert_eq!(
        events[0]["kind"], "evidence",
        "the snapshot leads the trace: {events}"
    );
    let detail: serde_json::Value =
        serde_json::from_str(telemetry[0].detail_json.as_deref().expect("detail_json"))
            .expect("detail is JSON");
    assert_eq!(detail["judge_fingerprint"].as_str().map(str::len), Some(16));

    let seen = recorder.seen().expect("the run recorded its snapshot");
    assert_eq!(
        seen.judge_fingerprint.len(),
        16,
        "got {}",
        seen.judge_fingerprint
    );
    assert_eq!(seen.evidence["task"]["id"], serde_json::json!(task_id));
    assert_eq!(seen.evidence["judge"]["slug"], "code-quality");
    // A 0..10 judge rewards; the snapshot must not describe it as a penalty.
    assert_eq!(seen.evidence["judge"]["is_penalty"], false);
    assert!(seen.evidence["tasks"].is_array());
}

// ---------------------------------------------------------------------------
// Test: a run the gate stopped is still accounted for — under its own
// provider, so it shows up in telemetry without reading as model spend.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_gated_run_is_accounted_for_without_faking_model_spend() {
    let db = setup_db().await;
    let owner = insert_user(&db).await;
    let project = insert_project(&db, owner).await;
    let session = insert_session(&db, project).await;
    let player = insert_player(&db, session).await;
    let (task_id, judge_id, _) = insert_chain(&db, project).await;

    // A penalty judge on a task that paid nothing: the gate stops it before
    // either the model or the sandbox is reached.
    let judge_row = judges::Entity::find_by_id(judge_id)
        .one(&db)
        .await
        .expect("find judge")
        .expect("judge exists");
    let mut am: judges::ActiveModel = judge_row.into();
    am.rating_scale = Set(json!({"min": -50.0, "max": 0.0, "step": 1.0}));
    am.update(&db).await.expect("make it a penalty judge");

    let repos_root = tempfile::tempdir().expect("repos tempdir");
    unsafe {
        std::env::set_var("OLOLO_GIT_REPOS_DIR", repos_root.path());
    }
    let base = arena_core::git_store::repos_base_dir().expect("base dir");
    let repo_dir = arena_core::git_store::player_repo_path(&base, session, player);
    std::fs::create_dir_all(&repo_dir).expect("mkdir repo");
    make_repo(&repo_dir);
    write_file(&repo_dir, "a.txt", "x");
    commit(&repo_dir, "init");

    let state = make_state(db.clone(), 3);
    let resolved = resolve_judge_run(&state, &db, session, player, task_id, judge_id)
        .await
        .expect("resolve");

    let out = execute_judge_run(
        &state,
        &db,
        resolved,
        &NeverAskedJudgeLlm,
        &test_model_cfg(),
        session,
        player,
        task_id,
        None,
        None,
    )
    .await
    .expect("execute");

    unsafe {
        std::env::remove_var("OLOLO_GIT_REPOS_DIR");
    }
    assert_eq!(out.point_delta, 0);

    let telemetry = arena_core::entities::llm_requests::Entity::find()
        .all(&db)
        .await
        .expect("find llm_requests");
    assert_eq!(telemetry.len(), 1, "a gated run is still a run");
    let row = &telemetry[0];
    assert_eq!(row.operation, "judge");
    assert_eq!(
        row.provider, "gate",
        "the gate must not be reported as the resolved provider"
    );
    assert_eq!(row.provider_name, None);
    assert_eq!(row.status, "ok");
    assert_eq!(
        (row.tokens_input, row.tokens_output),
        (0, 0),
        "no request was made, so no spend"
    );
    assert_eq!(row.judge_slug.as_deref(), Some("code-quality"));
    assert_eq!(row.task_id, Some(task_id));
    let detail: serde_json::Value =
        serde_json::from_str(row.detail_json.as_deref().expect("detail_json")).expect("json");
    assert_eq!(detail["outcome"], "gated");
    assert_eq!(detail["task_earned"], 0);
}

// ---------------------------------------------------------------------------
// Test: a judge's own program decides before the model is reached — and when
// it skips, no model is asked at all.
// ---------------------------------------------------------------------------

struct NeverAskedJudgeLlm;

#[async_trait]
impl JudgeLlm for NeverAskedJudgeLlm {
    async fn run_agent(
        &self,
        _system: &str,
        _user: &str,
        _tools: Vec<ToolDef>,
        _prior_tool_result: Option<&str>,
    ) -> Result<AgentResponse, JudgeError> {
        panic!("the judge's program skipped this run; no model should have been asked");
    }
}

#[tokio::test]
async fn a_program_that_skips_costs_no_model_call() {
    let db = setup_db().await;
    let owner = insert_user(&db).await;
    let project = insert_project(&db, owner).await;
    let session = insert_session(&db, project).await;
    let player = insert_player(&db, session).await;
    let (task_id, judge_id, _) = insert_chain(&db, project).await;

    // Give the judge a program that decides on its own.
    let judge_row = judges::Entity::find_by_id(judge_id)
        .one(&db)
        .await
        .expect("find judge")
        .expect("judge exists");
    let mut am: judges::ActiveModel = judge_row.into();
    am.prompt = Set(
        "Evaluate code quality.\n\n```js decide\nreturn skip(\"no probe of this task passed\");\n```\n"
            .to_string(),
    );
    am.update(&db).await.expect("attach program");

    let repos_root = tempfile::tempdir().expect("repos tempdir");
    unsafe {
        std::env::set_var("OLOLO_GIT_REPOS_DIR", repos_root.path());
    }
    let base = arena_core::git_store::repos_base_dir().expect("base dir");
    let repo_dir = arena_core::git_store::player_repo_path(&base, session, player);
    std::fs::create_dir_all(&repo_dir).expect("mkdir repo");
    make_repo(&repo_dir);
    write_file(&repo_dir, "a.txt", "x");
    commit(&repo_dir, "init");

    let state = make_state(db.clone(), 3);
    let resolved = resolve_judge_run(&state, &db, session, player, task_id, judge_id)
        .await
        .expect("resolve");

    let out = execute_judge_run(
        &state,
        &db,
        resolved,
        &NeverAskedJudgeLlm,
        &test_model_cfg(),
        session,
        player,
        task_id,
        None,
        None,
    )
    .await
    .expect("execute");

    unsafe {
        std::env::remove_var("OLOLO_GIT_REPOS_DIR");
    }

    assert_eq!(out.model, "decide:skip");
    assert_eq!(out.point_delta, 0);
    assert_eq!(out.feedback, "no probe of this task passed");

    // The terminal row still has to exist: the settle poll waits on one per
    // attached judge, and a silent skip would hold the session open.
    let rows = judge_results::Entity::find().all(&db).await.expect("find");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].status, "scored");
    assert_eq!(rows[0].point_delta, 0);
}

// ---------------------------------------------------------------------------
// Test: execute_judge_run error (non-JSON verdict) propagates, caller can
// log + skip without panic.
// ---------------------------------------------------------------------------

struct GarbageJudgeLlm;

#[async_trait]
impl JudgeLlm for GarbageJudgeLlm {
    async fn run_agent(
        &self,
        _system: &str,
        _user: &str,
        _tools: Vec<ToolDef>,
        _prior_tool_result: Option<&str>,
    ) -> Result<AgentResponse, JudgeError> {
        Ok(AgentResponse::Final {
            text: "this is not json".to_string(),
        })
    }
}

#[tokio::test]
async fn execute_judge_run_parse_error_propagates() {
    let db = setup_db().await;
    let owner = insert_user(&db).await;
    let project = insert_project(&db, owner).await;
    let session = insert_session(&db, project).await;
    let player = insert_player(&db, session).await;
    let (task_id, judge_id, _) = insert_chain(&db, project).await;

    let repos_root = tempfile::tempdir().expect("repos tempdir");
    unsafe {
        std::env::set_var("OLOLO_GIT_REPOS_DIR", repos_root.path());
    }

    let base = arena_core::git_store::repos_base_dir().expect("base dir");
    let repo_dir = arena_core::git_store::player_repo_path(&base, session, player);
    std::fs::create_dir_all(&repo_dir).expect("mkdir repo");
    make_repo(&repo_dir);
    write_file(&repo_dir, "a.txt", "x");
    commit(&repo_dir, "init");

    let state = make_state(db.clone(), 3);

    let resolved = resolve_judge_run(&state, &db, session, player, task_id, judge_id)
        .await
        .expect("resolve");

    let llm = GarbageJudgeLlm;
    let err = execute_judge_run(
        &state,
        &db,
        resolved,
        &llm,
        &test_model_cfg(),
        session,
        player,
        task_id,
        None,
        None,
    )
    .await
    .expect_err("non-JSON → parse error");

    unsafe {
        std::env::remove_var("OLOLO_GIT_REPOS_DIR");
    }

    assert!(matches!(err, JudgeError::AiParseError), "got {err:?}");

    // No judge_results row written on parse error.
    let rows = judge_results::Entity::find().all(&db).await.expect("find");
    assert!(rows.is_empty(), "parse error should not persist a row");
}

// ---------------------------------------------------------------------------
// Test: internal REST handler returns 404 on missing task_judge.
// Uses the axum oneshot harness against the game-server router.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn internal_judge_run_endpoint_404_on_missing_task_judge() {
    use axum::body::Body;
    use axum::http::{Method, Request, StatusCode, header};
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    let db = setup_db().await;
    let state = make_state(db, 3);
    let app = game_server::build_router(state);

    let body = serde_json::json!({
        "session_id": Uuid::new_v4(),
        "player_id": Uuid::new_v4(),
        "task_id": Uuid::new_v4(),
        "judge_id": Uuid::new_v4(),
    });
    let req = Request::builder()
        .method(Method::POST)
        .uri("/internal/judge-run")
        .header(header::CONTENT_TYPE, "application/json")
        .header(
            arena_core::auth::INTERNAL_API_HEADER,
            arena_core::auth::internal_api_token(b"test-secret-32-bytes-or-more-xxxxxxx"),
        )
        .body(Body::from(serde_json::to_vec(&body).expect("serialize")))
        .expect("build req");

    let resp = app.oneshot(req).await.expect("resp");
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);

    let bytes = resp
        .into_body()
        .collect()
        .await
        .expect("collect body")
        .to_bytes();
    let value: serde_json::Value = serde_json::from_slice(&bytes).expect("json body");
    assert_eq!(value["error"], serde_json::json!("not_found"));
}

// ---------------------------------------------------------------------------
// Test: /internal/* rejects requests without a valid X-Internal-Auth token.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn internal_judge_run_endpoint_401_without_auth() {
    use axum::body::Body;
    use axum::http::{Method, Request, StatusCode, header};
    use tower::ServiceExt;

    let db = setup_db().await;
    let state = make_state(db, 3);

    let body = serde_json::json!({
        "session_id": Uuid::new_v4(),
        "player_id": Uuid::new_v4(),
        "task_id": Uuid::new_v4(),
        "judge_id": Uuid::new_v4(),
    });

    // No header at all → 401.
    let app = game_server::build_router(state.clone());
    let req = Request::builder()
        .method(Method::POST)
        .uri("/internal/judge-run")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(serde_json::to_vec(&body).expect("serialize")))
        .expect("build req");
    let resp = app.oneshot(req).await.expect("resp");
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED, "missing token");

    // Wrong token → 401.
    let app = game_server::build_router(state);
    let req = Request::builder()
        .method(Method::POST)
        .uri("/internal/judge-run")
        .header(header::CONTENT_TYPE, "application/json")
        .header(arena_core::auth::INTERNAL_API_HEADER, "deadbeef")
        .body(Body::from(serde_json::to_vec(&body).expect("serialize")))
        .expect("build req");
    let resp = app.oneshot(req).await.expect("resp");
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED, "wrong token");
}

// ---------------------------------------------------------------------------
// Test: internal REST handler denies unknown fields (422).
// ---------------------------------------------------------------------------

#[tokio::test]
async fn internal_judge_run_endpoint_rejects_unknown_fields() {
    use axum::body::Body;
    use axum::http::{Method, Request, StatusCode, header};
    use tower::ServiceExt;

    let db = setup_db().await;
    let state = make_state(db, 3);
    let app = game_server::build_router(state);

    let body = serde_json::json!({
        "session_id": Uuid::new_v4(),
        "player_id": Uuid::new_v4(),
        "task_id": Uuid::new_v4(),
        "judge_id": Uuid::new_v4(),
        "extra": "bad",
    });
    let req = Request::builder()
        .method(Method::POST)
        .uri("/internal/judge-run")
        .header(header::CONTENT_TYPE, "application/json")
        .header(
            arena_core::auth::INTERNAL_API_HEADER,
            arena_core::auth::internal_api_token(b"test-secret-32-bytes-or-more-xxxxxxx"),
        )
        .body(Body::from(serde_json::to_vec(&body).expect("serialize")))
        .expect("build req");

    let resp = app.oneshot(req).await.expect("resp");
    assert!(
        resp.status() == StatusCode::UNPROCESSABLE_ENTITY
            || resp.status() == StatusCode::BAD_REQUEST,
        "deny unknown fields: {}",
        resp.status()
    );
}

// ponytail: TrackingFakeJudgeLlm + BlockingFakeJudgeLlm both exist; the
// semaphore test uses TrackingFakeJudgeLlm (records peak concurrency via
// compare_exchange). BlockingFakeJudgeLlm kept for callers that want a
// simpler fixed-delay stub.
#[allow(dead_code)]
fn _silence_unused() -> Arc<TempDir> {
    Arc::new(tempfile::tempdir().unwrap())
}

// ---------------------------------------------------------------------------
// Execution judge: re-runs the task's own probes server-side against the
// committed solution, in a sandbox. Forces the unsandboxed backend so the
// test does not depend on `bwrap` being installed.
// ---------------------------------------------------------------------------

use arena_core::entities::{probes, tests as entity_tests};
use arena_core::judging::{JudgeRow, TaskJudgeRow};
use game_server::judge_exec::run_execution_judge;

/// Record a client-side passing probe for one section: the execution judge
/// verifies CLAIMS, so only sections with a live pass enter its denominator.
async fn claim_pass(db: &DatabaseConnection, session: Uuid, player: Uuid, test_id: Uuid) {
    probes::ActiveModel {
        id: Set(Uuid::new_v4()),
        test_id: Set(test_id),
        player_id: Set(player),
        session_id: Set(session),
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
    .expect("insert passing probe");
}

/// Claim a live pass on every section of the task (the completed-task shape:
/// completion requires a pass per section).
async fn claim_all_sections(db: &DatabaseConnection, session: Uuid, player: Uuid, task_id: Uuid) {
    let rows = entity_tests::Entity::find()
        .filter(entity_tests::Column::SessionId.eq(session))
        .filter(entity_tests::Column::TaskId.eq(task_id))
        .all(db)
        .await
        .expect("load tests rows");
    for row in rows {
        claim_pass(db, session, player, row.id).await;
    }
}

/// Correct single-arg hop in POSIX sh.
const HOP_SH: &str = "n=$1\nd=0; [ $((n % 3)) -eq 0 ] && d=1\nc=0; case $n in *3*) c=1;; esac\nif [ $d -eq 1 ] && [ $c -eq 1 ]; then echo hop-hop; elif [ $d -eq 1 ] || [ $c -eq 1 ]; then echo hop; else echo \"$n\"; fi\n";

async fn insert_test_row(
    db: &DatabaseConnection,
    session_id: Uuid,
    task_id: Uuid,
    ordinal: i32,
    fixtures_js: &str,
    command: &str,
    validation_js: &str,
) -> Uuid {
    let id = Uuid::new_v4();
    entity_tests::ActiveModel {
        id: Set(id),
        command_template: Set(command.to_string()),
        answer_template: Set(validation_js.to_string()),
        fixture_definitions: Set(json!({"kind": "js", "script": fixtures_js}).to_string()),
        created_at: Set(Utc::now()),
        session_id: Set(session_id),
        task_id: Set(task_id),
        ordinal: Set(ordinal),
        prompt: Set(format!("probe {ordinal}")),
        description: Set(None),
        initiator: Set("system".to_string()),
        probe_config: Set(None),
        registered_by_judge_id: Set(None),
    }
    .insert(db)
    .await
    .expect("insert tests row");
    id
}

async fn insert_exec_task_judge(
    db: &DatabaseConnection,
    project_id: Uuid,
) -> (Uuid, Uuid, TaskJudgeRow, JudgeRow) {
    let task_id = Uuid::new_v4();
    tasks::ActiveModel {
        id: Set(task_id),
        project_id_fk: Set(project_id),
        ordinal: Set(0),
        title: Set("Golf hop".to_string()),
        content: Set("hop".to_string()),
        test_template: Set(json!({"kind": "shell"})),
        created_at: Set(Utc::now()),
        tags: Set("code-golf".to_string()),
        point_value: Set(10),
        deadline_secs: Set(Some(300)),
        min_interval_secs: Set(Some(5)),
        interval_increment_secs: Set(Some(0)),
        max_interval_secs: Set(Some(300)),
        fail_points: Set(0),
        no_response_points: Set(0),
        completion_bonus_points: Set(0),
        evaluation: Set(None),
    }
    .insert(db)
    .await
    .expect("insert task");

    let judge_id = Uuid::new_v4();
    let scale = json!({"min": 0.0, "max": 100.0, "step": 1.0});
    judges::ActiveModel {
        id: Set(judge_id),
        slug: Set("hop-verify".to_string()),
        name: Set("Hop Verify".to_string()),
        description: Set(String::new()),
        prompt: Set(String::new()),
        rating_scale: Set(scale.clone()),
        kind: Set("execution".to_string()),
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
        max_interactive: Set(None),
        avatar_url: Set(None),
        ignore_paths: Set(None),
        probes_config: Set(None),
    }
    .insert(db)
    .await
    .expect("insert judge");

    let task_judge_id = Uuid::new_v4();
    task_judges::ActiveModel {
        id: Set(task_judge_id),
        task_id: Set(task_id),
        judge_id: Set(judge_id),
        ordinal: Set(0),
        rating_scale_override: Set(None),
        created_at: Set(Utc::now()),
        updated_at: Set(Utc::now()),
        weight: Set(None),
    }
    .insert(db)
    .await
    .expect("insert task_judge");

    let tj = TaskJudgeRow {
        id: task_judge_id,
        task_id,
        judge_id,
        rating_scale_override: None,
        weight: None,
    };
    let jr = JudgeRow {
        slug: "hop-verify".to_string(),
        name: "Hop Verify".to_string(),
        prompt: String::new(),
        rating_scale: scale,
        kind: "execution".to_string(),
        scope: "task".to_string(),
        evidence_mode: "tools".to_string(),
        evidence_needs: None,
        llm_provider_id: None,
        llm_pool_id: None,
        llm_source_order: arena_core::llm::resolve::SOURCE_ORDER_POOL_FIRST.to_string(),
        llm_model: None,
        criteria: None,
        max_interactive: None,
        ignore_paths: None,
    };
    (task_id, task_judge_id, tj, jr)
}

// The golf correctness probe (single number → hop) and size probe, verbatim
// in shape to the hop-hop seed.
const CORRECT_CMD: &str = "cd {baseDir}\nR=$(sed -n 's/^run: *//p' AGENTS.md README.md 2>/dev/null | head -1 | tr -d '\\r')\n$R \"{n}\"";
const SIZE_CMD: &str = "cd {baseDir}\nfind . -type f ! -path \"*/.git/*\" ! -name AGENTS.md ! -name README.md ! -name .gitignore -exec cat {} + | wc -c";

async fn seed_golf_probes(db: &DatabaseConnection, session: Uuid, task_id: Uuid) {
    // n=6 → divisible by 3, no digit 3 → "hop".
    insert_test_row(
        db,
        session,
        task_id,
        0,
        "({ baseDir: \".\", n: 6 })",
        CORRECT_CMD,
        "assertEqual(result.trim(), \"hop\")",
    )
    .await;
    insert_test_row(
        db,
        session,
        task_id,
        1,
        "({ baseDir: \".\" })",
        SIZE_CMD,
        "Number(result.trim()) <= 200",
    )
    .await;
}

#[tokio::test]
async fn execution_judge_scores_correct_solution_full_marks() {
    unsafe { std::env::set_var("OLOLO_SANDBOX", "none") };
    unsafe { std::env::set_var("OLOLO_ALLOW_UNSANDBOXED_EXEC", "1") };
    let db = setup_db().await;
    let owner = insert_user(&db).await;
    let project = insert_project(&db, owner).await;
    let session = insert_session(&db, project).await;
    let player = insert_player(&db, session).await;
    let (task_id, task_judge_id, tj, jr) = insert_exec_task_judge(&db, project).await;
    seed_golf_probes(&db, session, task_id).await;
    claim_all_sections(&db, session, player, task_id).await;

    let repo = TempDir::new().unwrap();
    make_repo(repo.path());
    write_file(repo.path(), "AGENTS.md", "run: sh answer.sh\n");
    write_file(repo.path(), "answer.sh", HOP_SH);
    let sha = commit(repo.path(), &format!("feat({task_id}): solution"));

    let out = run_execution_judge(
        &db,
        repo.path(),
        session,
        player,
        task_id,
        Some(&sha),
        &tj,
        &gated_scale(&jr, &tj),
    )
    .await
    .expect("execution judge run");
    assert_eq!(
        out.point_delta, 100,
        "both probes pass → full marks; feedback: {}",
        out.feedback
    );
    assert!(
        out.model.starts_with("execution:"),
        "model tags backend: {}",
        out.model
    );

    let row = judge_results::Entity::find()
        .filter(judge_results::Column::TaskJudgeId.eq(task_judge_id))
        .filter(judge_results::Column::PlayerIdFk.eq(player))
        .one(&db)
        .await
        .unwrap()
        .expect("persisted judge_result");
    assert_eq!(row.point_delta, 100);
    assert_eq!(row.provider, "execution");
    assert_eq!(row.status, "scored");
}

#[tokio::test]
async fn execution_judge_penalizes_wrong_solution() {
    unsafe { std::env::set_var("OLOLO_SANDBOX", "none") };
    unsafe { std::env::set_var("OLOLO_ALLOW_UNSANDBOXED_EXEC", "1") };
    let db = setup_db().await;
    let owner = insert_user(&db).await;
    let project = insert_project(&db, owner).await;
    let session = insert_session(&db, project).await;
    let player = insert_player(&db, session).await;
    let (task_id, _tj_id, tj, jr) = insert_exec_task_judge(&db, project).await;
    seed_golf_probes(&db, session, task_id).await;
    // The player CLAIMED both sections passed — the committed code says
    // otherwise. This is the cheat shape the judge exists to catch.
    claim_all_sections(&db, session, player, task_id).await;

    let repo = TempDir::new().unwrap();
    make_repo(repo.path());
    write_file(repo.path(), "AGENTS.md", "run: sh answer.sh\n");
    // Wrong: always prints "nope". Size probe still passes → 1/2 → 50.
    write_file(repo.path(), "answer.sh", "echo nope\n");
    let sha = commit(repo.path(), &format!("feat({task_id}): wrong"));

    let out = run_execution_judge(
        &db,
        repo.path(),
        session,
        player,
        task_id,
        Some(&sha),
        &tj,
        &gated_scale(&jr, &tj),
    )
    .await
    .expect("execution judge run");
    assert_eq!(
        out.point_delta, 50,
        "1 of 2 probes pass → half; feedback: {}",
        out.feedback
    );
}

// The declared `run:` command runs on the player's own machine for the live
// probes, but the judge image only ships a few interpreters. A solution the
// live probes accepted must never be penalized because *our* sandbox cannot
// invoke it — those probes are excluded from the rating instead.
#[tokio::test]
async fn execution_judge_excludes_unrunnable_probe_from_rating() {
    unsafe { std::env::set_var("OLOLO_SANDBOX", "none") };
    unsafe { std::env::set_var("OLOLO_ALLOW_UNSANDBOXED_EXEC", "1") };
    let db = setup_db().await;
    let owner = insert_user(&db).await;
    let project = insert_project(&db, owner).await;
    let session = insert_session(&db, project).await;
    let player = insert_player(&db, session).await;
    let (task_id, _tj_id, tj, mut jr) = insert_exec_task_judge(&db, project).await;
    // The real golf-verify scale: all pass → 0, all fail → -100. A penalty
    // scale is what makes a miscount cost the player points.
    jr.rating_scale = json!({"min": -100.0, "max": 0.0, "step": 1.0});
    seed_golf_probes(&db, session, task_id).await;
    claim_all_sections(&db, session, player, task_id).await;

    let repo = TempDir::new().unwrap();
    make_repo(repo.path());
    // Correct solution, but declared behind an interpreter the sandbox lacks:
    // the correctness probe exits 127 with no stdout. The size probe still runs.
    write_file(
        repo.path(),
        "AGENTS.md",
        "run: ololo-no-such-interpreter answer.hop\n",
    );
    write_file(repo.path(), "answer.hop", "hop\n");
    let sha = commit(repo.path(), &format!("feat({task_id}): exotic language"));

    let out = run_execution_judge(
        &db,
        repo.path(),
        session,
        player,
        task_id,
        Some(&sha),
        &tj,
        &gated_scale(&jr, &tj),
    )
    .await
    .expect("execution judge run");
    assert_eq!(
        out.point_delta, 0,
        "the size probe was the only gradeable one and it passed → no penalty; feedback: {}",
        out.feedback
    );
    assert!(
        out.feedback.contains("1/1"),
        "the unrunnable probe is out of the denominator: {}",
        out.feedback
    );
    assert!(
        out.raw_output.contains("inconclusive"),
        "the skipped probe is recorded with a reason: {}",
        out.raw_output
    );
}

// When nothing could be executed there is no verdict to give. Scoring 0/0
// would hand out the scale's failing end — a full penalty for a run that
// verified nothing at all.
#[tokio::test]
async fn execution_judge_errors_when_no_probe_can_run() {
    unsafe { std::env::set_var("OLOLO_SANDBOX", "none") };
    unsafe { std::env::set_var("OLOLO_ALLOW_UNSANDBOXED_EXEC", "1") };
    let db = setup_db().await;
    let owner = insert_user(&db).await;
    let project = insert_project(&db, owner).await;
    let session = insert_session(&db, project).await;
    let player = insert_player(&db, session).await;
    let (task_id, task_judge_id, tj, mut jr) = insert_exec_task_judge(&db, project).await;
    jr.rating_scale = json!({"min": -100.0, "max": 0.0, "step": 1.0});
    // Only the correctness probe — nothing gradeable once it cannot run.
    let test_id = insert_test_row(
        &db,
        session,
        task_id,
        0,
        "({ baseDir: \".\", n: 6 })",
        CORRECT_CMD,
        "assertEqual(result.trim(), \"hop\")",
    )
    .await;
    claim_pass(&db, session, player, test_id).await;

    let repo = TempDir::new().unwrap();
    make_repo(repo.path());
    write_file(
        repo.path(),
        "AGENTS.md",
        "run: ololo-no-such-interpreter answer.hop\n",
    );
    write_file(repo.path(), "answer.hop", "hop\n");
    let sha = commit(repo.path(), &format!("feat({task_id}): exotic language"));

    let err = run_execution_judge(
        &db,
        repo.path(),
        session,
        player,
        task_id,
        Some(&sha),
        &tj,
        &gated_scale(&jr, &tj),
    )
    .await
    .expect_err("must not score a run that verified nothing");
    assert!(
        matches!(err, JudgeError::ExecFailed(_)),
        "reported as an execution failure, not a rating: {err:?}"
    );
    // No scored row: the failure path records point_delta = 0.
    let scored = judge_results::Entity::find()
        .filter(judge_results::Column::TaskJudgeId.eq(task_judge_id))
        .filter(judge_results::Column::PlayerIdFk.eq(player))
        .one(&db)
        .await
        .unwrap();
    assert!(
        scored.is_none(),
        "no judge_result persisted by the judge itself"
    );
}

// Session OD5FJA regression: the session clock expired while the player was
// mid-rung — correctness sections passed live, the size section honestly
// failed (154 bytes on a ≤140 rung). The judge re-runs only the sections the
// player passed client-side; the sections they never passed are already
// charged by the live probes and must not be re-failed into a penalty when
// the re-run finds no discrepancy at all.
#[tokio::test]
async fn execution_judge_skips_sections_the_player_never_passed() {
    unsafe { std::env::set_var("OLOLO_SANDBOX", "none") };
    unsafe { std::env::set_var("OLOLO_ALLOW_UNSANDBOXED_EXEC", "1") };
    let db = setup_db().await;
    let owner = insert_user(&db).await;
    let project = insert_project(&db, owner).await;
    let session = insert_session(&db, project).await;
    let player = insert_player(&db, session).await;
    let (task_id, _tj_id, tj, mut jr) = insert_exec_task_judge(&db, project).await;
    jr.rating_scale = json!({"min": -100.0, "max": 0.0, "step": 1.0});

    let correct_id = insert_test_row(
        &db,
        session,
        task_id,
        0,
        "({ baseDir: \".\", n: 6 })",
        CORRECT_CMD,
        "assertEqual(result.trim(), \"hop\")",
    )
    .await;
    // Size rung the committed solution genuinely misses — the player was
    // interrupted before golfing under it and never claimed it.
    insert_test_row(
        &db,
        session,
        task_id,
        1,
        "({ baseDir: \".\" })",
        SIZE_CMD,
        "Number(result.trim()) <= 10",
    )
    .await;
    claim_pass(&db, session, player, correct_id).await;

    let repo = TempDir::new().unwrap();
    make_repo(repo.path());
    write_file(repo.path(), "AGENTS.md", "run: sh answer.sh\n");
    write_file(repo.path(), "answer.sh", HOP_SH);
    let sha = commit(
        repo.path(),
        &format!("feat({task_id}): interrupted mid-rung"),
    );

    let out = run_execution_judge(
        &db,
        repo.path(),
        session,
        player,
        task_id,
        Some(&sha),
        &tj,
        &gated_scale(&jr, &tj),
    )
    .await
    .expect("execution judge run");
    assert_eq!(
        out.point_delta, 0,
        "the claimed section verified clean; the unclaimed rung must not be re-failed \
         into a penalty; feedback: {}",
        out.feedback
    );
    assert!(
        out.feedback.contains("1/1"),
        "only the claimed section is in the denominator: {}",
        out.feedback
    );
    assert!(
        out.raw_output.contains("excluded from verification"),
        "the unclaimed section is recorded with a reason: {}",
        out.raw_output
    );
}

// Interrupted before the first pass: the judge must still leave a terminal
// scored row (the awards settle poll waits on it) but has nothing to verify,
// so the delta is zero on any scale.
#[tokio::test]
async fn execution_judge_scores_zero_when_nothing_was_passed() {
    unsafe { std::env::set_var("OLOLO_SANDBOX", "none") };
    unsafe { std::env::set_var("OLOLO_ALLOW_UNSANDBOXED_EXEC", "1") };
    let db = setup_db().await;
    let owner = insert_user(&db).await;
    let project = insert_project(&db, owner).await;
    let session = insert_session(&db, project).await;
    let player = insert_player(&db, session).await;
    let (task_id, task_judge_id, tj, mut jr) = insert_exec_task_judge(&db, project).await;
    jr.rating_scale = json!({"min": -100.0, "max": 0.0, "step": 1.0});
    seed_golf_probes(&db, session, task_id).await;
    // No claim_* calls: the player never passed a probe on this task.

    let repo = TempDir::new().unwrap();
    make_repo(repo.path());
    write_file(repo.path(), "AGENTS.md", "run: sh answer.sh\n");
    write_file(repo.path(), "answer.sh", HOP_SH);
    let sha = commit(repo.path(), &format!("feat({task_id}): nothing claimed"));

    let out = run_execution_judge(
        &db,
        repo.path(),
        session,
        player,
        task_id,
        Some(&sha),
        &tj,
        &gated_scale(&jr, &tj),
    )
    .await
    .expect("execution judge run");
    assert_eq!(
        out.point_delta, 0,
        "nothing to verify → no penalty: {}",
        out.feedback
    );
    assert!(
        out.feedback.contains("nothing to verify"),
        "feedback explains the skip: {}",
        out.feedback
    );

    let row = judge_results::Entity::find()
        .filter(judge_results::Column::TaskJudgeId.eq(task_judge_id))
        .filter(judge_results::Column::PlayerIdFk.eq(player))
        .one(&db)
        .await
        .unwrap()
        .expect("persisted judge_result");
    assert_eq!(
        row.status, "scored",
        "terminal row so the awards settle poll proceeds"
    );
    assert_eq!(row.point_delta, 0);
}

// Session-scoped judges are attached per task (that is how the settle poll
// knows to expect their rows) but must not fire on each task's commit — they
// run once per player at session end. Getting this wrong reintroduces exactly
// the per-task fan-out the session scope exists to remove.
#[tokio::test]
async fn per_task_trigger_skips_session_scoped_judges() {
    use game_server::judge_queue::retain_task_scoped;

    let db = setup_db().await;
    let owner = insert_user(&db).await;
    let project = insert_project(&db, owner).await;

    let task_id = Uuid::new_v4();
    tasks::ActiveModel {
        id: Set(task_id),
        project_id_fk: Set(project),
        ordinal: Set(0),
        title: Set("A task".to_string()),
        content: Set("do a thing".to_string()),
        test_template: Set(json!({"kind": "shell"})),
        created_at: Set(Utc::now()),
        tags: Set(String::new()),
        point_value: Set(10),
        deadline_secs: Set(Some(300)),
        min_interval_secs: Set(Some(5)),
        interval_increment_secs: Set(Some(0)),
        max_interval_secs: Set(Some(300)),
        fail_points: Set(0),
        no_response_points: Set(0),
        completion_bonus_points: Set(0),
        evaluation: Set(None),
    }
    .insert(&db)
    .await
    .expect("insert task");

    let mut attached = Vec::new();
    for (ordinal, (slug, scope)) in [("per-task", "task"), ("whole-session", "session")]
        .into_iter()
        .enumerate()
    {
        let judge_id = Uuid::new_v4();
        judges::ActiveModel {
            id: Set(judge_id),
            slug: Set(slug.to_string()),
            name: Set(slug.to_string()),
            description: Set(String::new()),
            prompt: Set("judge it".to_string()),
            rating_scale: Set(json!({"min": -100.0, "max": 0.0, "step": 1.0})),
            kind: Set("llm".to_string()),
            scope: Set(scope.to_string()),
            evidence_mode: Set("tools".to_string()),
            evidence_needs: Set(None),
            created_at: Set(Utc::now()),
            updated_at: Set(Utc::now()),
            llm_provider_id_fk: Set(None),
            llm_model: Set(None),
            llm_pool_id_fk: Set(None),
            llm_source_order: Set(arena_core::llm::resolve::SOURCE_ORDER_POOL_FIRST.to_string()),
            criteria: Set(None),
            max_interactive: Set(None),
            avatar_url: Set(None),
            ignore_paths: Set(None),
            probes_config: Set(None),
        }
        .insert(&db)
        .await
        .expect("insert judge");

        let tj = task_judges::ActiveModel {
            id: Set(Uuid::new_v4()),
            task_id: Set(task_id),
            judge_id: Set(judge_id),
            ordinal: Set(ordinal as i32),
            rating_scale_override: Set(None),
            created_at: Set(Utc::now()),
            updated_at: Set(Utc::now()),
            weight: Set(None),
        }
        .insert(&db)
        .await
        .expect("attach judge");
        attached.push(tj);
    }
    assert_eq!(attached.len(), 2, "both judges attached to the task");

    let kept = retain_task_scoped(&db, attached).await;
    assert_eq!(kept.len(), 1, "only the task-scoped judge fires per commit");

    let kept_judge = judges::Entity::find_by_id(kept[0].judge_id)
        .one(&db)
        .await
        .unwrap()
        .expect("judge row");
    assert_eq!(kept_judge.slug, "per-task");
}

// The trigger's load-bearing guarantee: Arena Points are gated on
// `expired_session_pending_judges` reaching zero, and nothing but this trigger
// writes rows for a session-scoped judge. So even when the run cannot possibly
// succeed — no repo pushed, no reachable LLM — it must still leave a terminal
// row per reached task. Otherwise the award flow hangs to its deadline, which
// looks like "judging is slow" rather than a bug.
#[tokio::test]
async fn session_judge_trigger_always_leaves_terminal_rows() {
    let db = setup_db().await;
    let owner = insert_user(&db).await;
    let project = insert_project(&db, owner).await;
    let session = insert_session(&db, project).await;
    let player = insert_player(&db, session).await;

    let mut task_ids = Vec::new();
    for ordinal in 0..3 {
        let task_id = Uuid::new_v4();
        tasks::ActiveModel {
            id: Set(task_id),
            project_id_fk: Set(project),
            ordinal: Set(ordinal),
            title: Set(format!("Task {ordinal}")),
            content: Set("do it".to_string()),
            test_template: Set(json!({"kind": "shell"})),
            created_at: Set(Utc::now()),
            tags: Set(String::new()),
            point_value: Set(10),
            deadline_secs: Set(Some(300)),
            min_interval_secs: Set(Some(5)),
            interval_increment_secs: Set(Some(0)),
            max_interval_secs: Set(Some(300)),
            fail_points: Set(0),
            no_response_points: Set(0),
            completion_bonus_points: Set(0),
            evaluation: Set(None),
        }
        .insert(&db)
        .await
        .expect("insert task");
        task_ids.push(task_id);
    }

    // A session-scoped judge attached to every task, as the seed would do.
    let judge_id = Uuid::new_v4();
    judges::ActiveModel {
        id: Set(judge_id),
        slug: Set("anti-cheater".to_string()),
        name: Set("Anti Cheater".to_string()),
        description: Set(String::new()),
        prompt: Set("detect pre-implemented solutions".to_string()),
        rating_scale: Set(json!({"min": -500.0, "max": 0.0, "step": 1.0})),
        kind: Set("llm".to_string()),
        scope: Set("session".to_string()),
        evidence_mode: Set("tools".to_string()),
        evidence_needs: Set(None),
        created_at: Set(Utc::now()),
        updated_at: Set(Utc::now()),
        llm_provider_id_fk: Set(None),
        llm_model: Set(None),
        llm_pool_id_fk: Set(None),
        llm_source_order: Set(arena_core::llm::resolve::SOURCE_ORDER_POOL_FIRST.to_string()),
        criteria: Set(None),
        max_interactive: Set(None),
        avatar_url: Set(None),
        ignore_paths: Set(None),
        probes_config: Set(None),
    }
    .insert(&db)
    .await
    .expect("insert judge");
    for task_id in &task_ids {
        task_judges::ActiveModel {
            id: Set(Uuid::new_v4()),
            task_id: Set(*task_id),
            judge_id: Set(judge_id),
            ordinal: Set(0),
            rating_scale_override: Set(None),
            created_at: Set(Utc::now()),
            updated_at: Set(Utc::now()),
            weight: Set(None),
        }
        .insert(&db)
        .await
        .expect("attach judge");
    }

    // Player finished every task, so all three pairs are expected.
    arena_core::entities::session_scheduler_state::ActiveModel {
        id: Set(Uuid::new_v4()),
        session_id_fk: Set(session),
        player_id_fk: Set(player),
        task_id: Set(None),
        state: Set(arena_core::session_completion::SCHEDULER_STATE_COMPLETED.to_string()),
        next_probe_at: Set(None),
        created_at: Set(Utc::now()),
        updated_at: Set(Utc::now()),
    }
    .insert(&db)
    .await
    .expect("insert scheduler row");

    assert_eq!(
        arena_core::session_completion::expired_session_pending_judges(&db, session)
            .await
            .expect("pending"),
        3,
        "three pairs pending before the run"
    );

    // An empty repo store: the player pushed nothing, so the run bails early.
    // The LLM base URL points nowhere either — both failure paths are live.
    let repos = TempDir::new().unwrap();
    unsafe { std::env::set_var("OLOLO_GIT_REPOS_DIR", repos.path()) };
    let state = make_state(db.clone(), 2);

    game_server::judge_queue::run_session_judges(&state, session).await;

    let rows = judge_results::Entity::find()
        .filter(judge_results::Column::SessionIdFk.eq(session))
        .all(&db)
        .await
        .expect("rows");
    assert_eq!(
        rows.len(),
        3,
        "a row per reached task even when nothing could run"
    );
    assert!(
        rows.iter().all(|r| r.status == "failed"),
        "unrunnable means failed, not scored: {:?}",
        rows.iter().map(|r| &r.status).collect::<Vec<_>>()
    );
    assert!(
        rows.iter().all(|r| r.point_delta == 0),
        "a judge that could not run must not move the player's score"
    );
    assert!(
        rows.iter().all(|r| r.error.is_some()),
        "each row records why"
    );

    assert_eq!(
        arena_core::session_completion::expired_session_pending_judges(&db, session)
            .await
            .expect("pending"),
        0,
        "awards proceed instead of waiting for the settle deadline"
    );
}

// The reporter is the one session judge that does not need the repo: it writes
// about the checks, the points and what was recorded of the agent, all of which
// live in the database. A player whose snapshot never arrived used to get the
// blanket "player repository is absent" failure and no debrief — the very
// player most owed an account of what the session saw.
#[tokio::test]
async fn the_reporter_runs_for_a_player_who_pushed_nothing() {
    let db = setup_db().await;
    let owner = insert_user(&db).await;
    let project = insert_project(&db, owner).await;
    let session = insert_session(&db, project).await;
    let player = insert_player(&db, session).await;

    let task_id = Uuid::new_v4();
    tasks::ActiveModel {
        id: Set(task_id),
        project_id_fk: Set(project),
        ordinal: Set(0),
        title: Set("Task 0".to_string()),
        content: Set("do it".to_string()),
        test_template: Set(json!({"kind": "shell"})),
        created_at: Set(Utc::now()),
        tags: Set(String::new()),
        point_value: Set(10),
        deadline_secs: Set(Some(300)),
        min_interval_secs: Set(Some(5)),
        interval_increment_secs: Set(Some(0)),
        max_interval_secs: Set(Some(300)),
        fail_points: Set(0),
        no_response_points: Set(0),
        completion_bonus_points: Set(0),
        evaluation: Set(None),
    }
    .insert(&db)
    .await
    .expect("insert task");

    let judge_id = Uuid::new_v4();
    judges::ActiveModel {
        id: Set(judge_id),
        slug: Set("general".to_string()),
        name: Set("The Debrief".to_string()),
        description: Set(String::new()),
        prompt: Set("write the report".to_string()),
        rating_scale: Set(json!({"min": 0.0, "max": 1.0, "step": 1.0})),
        kind: Set(arena_core::judging::JUDGE_KIND_REPORT.to_string()),
        scope: Set("session".to_string()),
        evidence_mode: Set("dossier".to_string()),
        evidence_needs: Set(None),
        created_at: Set(Utc::now()),
        updated_at: Set(Utc::now()),
        llm_provider_id_fk: Set(None),
        llm_model: Set(None),
        llm_pool_id_fk: Set(None),
        llm_source_order: Set(arena_core::llm::resolve::SOURCE_ORDER_POOL_FIRST.to_string()),
        criteria: Set(None),
        max_interactive: Set(None),
        avatar_url: Set(None),
        ignore_paths: Set(None),
        probes_config: Set(None),
    }
    .insert(&db)
    .await
    .expect("insert judge");
    task_judges::ActiveModel {
        id: Set(Uuid::new_v4()),
        task_id: Set(task_id),
        judge_id: Set(judge_id),
        ordinal: Set(0),
        rating_scale_override: Set(None),
        created_at: Set(Utc::now()),
        updated_at: Set(Utc::now()),
        weight: Set(None),
    }
    .insert(&db)
    .await
    .expect("attach judge");

    // Parked on the first task and never finished it — the case the anchor
    // change is for — and nothing was ever pushed.
    arena_core::entities::session_scheduler_state::ActiveModel {
        id: Set(Uuid::new_v4()),
        session_id_fk: Set(session),
        player_id_fk: Set(player),
        task_id: Set(Some(task_id)),
        state: Set("waiting".to_string()),
        next_probe_at: Set(None),
        created_at: Set(Utc::now()),
        updated_at: Set(Utc::now()),
    }
    .insert(&db)
    .await
    .expect("insert scheduler row");

    let repos = TempDir::new().unwrap();
    unsafe { std::env::set_var("OLOLO_GIT_REPOS_DIR", repos.path()) };
    let state = make_state(db.clone(), 2);

    game_server::judge_queue::run_session_judges(&state, session).await;

    let rows = judge_results::Entity::find()
        .filter(judge_results::Column::SessionIdFk.eq(session))
        .all(&db)
        .await
        .expect("rows");
    assert_eq!(rows.len(), 1, "one pair, one terminal row");
    // There is no reachable LLM here, so the run still fails — but it must
    // fail at the model, having got past the repo gate, not before it.
    let error = rows[0].error.clone().unwrap_or_default();
    assert!(
        !error.contains("player repository is absent"),
        "the reporter must not be turned away for having no repo: {error}"
    );
}

// `finish_session` used to award immediately on the all-players-finished
// path unless a session-scoped judge existed — but the final task's
// task-scoped judges are enqueued at task completion and settle minutes
// later, so the award snapshot froze a stale total into
// session_awards.game_points (prod 36N4MS: report 398, Top Players 316).
// The gate is now `expired_session_pending_judges` on EVERY finish path:
// any expected-but-unsettled judge run defers the awards, and a terminal
// row releases them.
#[tokio::test]
async fn pending_task_judges_gate_awards_on_the_all_done_path() {
    let db = setup_db().await;
    let owner = insert_user(&db).await;
    let project = insert_project(&db, owner).await;
    let session = insert_session(&db, project).await;
    let player = insert_player(&db, session).await;

    let task_id = Uuid::new_v4();
    tasks::ActiveModel {
        id: Set(task_id),
        project_id_fk: Set(project),
        ordinal: Set(0),
        title: Set("A task".to_string()),
        content: Set("do it".to_string()),
        test_template: Set(json!({"kind": "shell"})),
        created_at: Set(Utc::now()),
        tags: Set(String::new()),
        point_value: Set(10),
        deadline_secs: Set(Some(300)),
        min_interval_secs: Set(Some(5)),
        interval_increment_secs: Set(Some(0)),
        max_interval_secs: Set(Some(300)),
        fail_points: Set(0),
        no_response_points: Set(0),
        completion_bonus_points: Set(0),
        evaluation: Set(None),
    }
    .insert(&db)
    .await
    .expect("insert task");

    // An ordinary TASK-scoped judge, as every panel judge is.
    let judge_id = Uuid::new_v4();
    judges::ActiveModel {
        id: Set(judge_id),
        slug: Set("per-task".to_string()),
        name: Set("Per Task".to_string()),
        description: Set(String::new()),
        prompt: Set("judge".to_string()),
        rating_scale: Set(json!({"min": -100.0, "max": 0.0, "step": 1.0})),
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
        max_interactive: Set(None),
        avatar_url: Set(None),
        ignore_paths: Set(None),
        probes_config: Set(None),
    }
    .insert(&db)
    .await
    .expect("insert judge");
    let task_judge_id = Uuid::new_v4();
    task_judges::ActiveModel {
        id: Set(task_judge_id),
        task_id: Set(task_id),
        judge_id: Set(judge_id),
        ordinal: Set(0),
        rating_scale_override: Set(None),
        created_at: Set(Utc::now()),
        updated_at: Set(Utc::now()),
        weight: Set(None),
    }
    .insert(&db)
    .await
    .expect("attach judge");

    // The player finished everything — the all-done path.
    arena_core::entities::session_scheduler_state::ActiveModel {
        id: Set(Uuid::new_v4()),
        session_id_fk: Set(session),
        player_id_fk: Set(player),
        task_id: Set(None),
        state: Set(arena_core::session_completion::SCHEDULER_STATE_COMPLETED.to_string()),
        next_probe_at: Set(None),
        created_at: Set(Utc::now()),
        updated_at: Set(Utc::now()),
    }
    .insert(&db)
    .await
    .expect("insert scheduler row");

    assert_eq!(
        arena_core::session_completion::expired_session_pending_judges(&db, session)
            .await
            .expect("pending"),
        1,
        "the final task's judge has not settled — awards must wait even though \
         every player finished"
    );

    // A terminal row releases the gate.
    judge_results::ActiveModel {
        id: Set(Uuid::new_v4()),
        session_id_fk: Set(session),
        player_id_fk: Set(player),
        task_judge_id: Set(task_judge_id),
        rating: Set(json!(0.0)),
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
        status: Set("scored".to_string()),
        error: Set(None),
        created_at: Set(Utc::now()),
        updated_at: Set(Utc::now()),
        verdict_kind: Set(None),
    }
    .insert(&db)
    .await
    .expect("terminal row");

    assert_eq!(
        arena_core::session_completion::expired_session_pending_judges(&db, session)
            .await
            .expect("pending"),
        0,
        "a settled judge run releases the awards"
    );
}

// ---------------------------------------------------------------------------
// Test: the account-plan gate at the judge pipeline's door. The metering
// logic itself is covered in arena-core; what matters here is the
// ENFORCEMENT — a denied run must be recorded as failed (never left
// dangling as "running"), and the switch must actually gate.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn quota_denial_records_a_failed_run_instead_of_dangling() {
    use arena_core::entities::app_settings;
    let db = setup_db().await;
    let owner = insert_user(&db).await;
    let project = insert_project(&db, owner).await;
    let session = insert_session(&db, project).await;
    // Quota is charged to the ACCOUNT behind the player, so the seat must
    // be linked — an anonymous player is unmetered by design.
    let player = insert_player(&db, session).await;
    players::Entity::update(players::ActiveModel {
        id: Set(player),
        user_id_fk: Set(Some(owner)),
        ..Default::default()
    })
    .exec(&db)
    .await
    .expect("link player to user");
    let (task_id, judge_id, _) = insert_chain(&db, project).await;
    let state = make_state(db.clone(), 3);

    // Plans on, and this player's account allowed zero runs this month.
    for (key, value) in [
        (arena_core::quota::PLANS_ENABLED_KEY, "true"),
        (arena_core::quota::PREMIUM_JUDGE_RUN_LIMIT_KEY, "0"),
    ] {
        app_settings::ActiveModel {
            key: Set(key.to_string()),
            value: Set(value.to_string()),
        }
        .insert(&db)
        .await
        .expect("seed setting");
    }

    let err = enqueue_judge_run(&state, &db, session, player, task_id, judge_id, false)
        .await
        .expect_err("an exhausted account must not run a judge");
    assert!(
        matches!(err, JudgeError::QuotaExceeded(ref m) if m.contains("limit reached")),
        "got {err:?}"
    );

    // The denial is visible and terminal — the settle poll must never wait
    // on a run that was refused before it started.
    let rows = judge_results::Entity::find()
        .all(&db)
        .await
        .expect("find judge results");
    assert_eq!(rows.len(), 1, "the denied run is recorded once");
    assert_eq!(rows[0].status, "failed");
    assert_eq!(rows[0].point_delta, 0, "a refused run never awards points");
    assert!(
        rows[0]
            .error
            .as_deref()
            .is_some_and(|e| e.contains("limit reached")),
        "the row says why: {:?}",
        rows[0].error
    );
}

// ---------------------------------------------------------------------------
// Test: a judge sees its OWN earlier verdicts in the same session. A project's
// tasks build on one another, so a verdict written blind re-litigates settled
// ground — praising a structure it faulted a task ago, or charging twice for
// one flaw.
// ---------------------------------------------------------------------------

/// Attach `judge_id` to a second task at `ordinal`, returning
/// `(task_id, task_judge_id)`.
async fn extra_task_for_judge(
    db: &DatabaseConnection,
    project_id: Uuid,
    judge_id: Uuid,
    ordinal: i32,
) -> (Uuid, Uuid) {
    let task_id = Uuid::new_v4();
    tasks::ActiveModel {
        id: Set(task_id),
        project_id_fk: Set(project_id),
        ordinal: Set(ordinal),
        title: Set(format!("Task {ordinal}")),
        content: Set("content".to_string()),
        test_template: Set(json!({"kind":"shell","command_template":"echo ok"})),
        created_at: Set(Utc::now()),
        tags: Set(String::new()),
        point_value: Set(10),
        deadline_secs: Set(Some(300)),
        min_interval_secs: Set(Some(5)),
        interval_increment_secs: Set(Some(0)),
        max_interval_secs: Set(Some(300)),
        fail_points: Set(0),
        no_response_points: Set(0),
        completion_bonus_points: Set(0),
        evaluation: Set(None),
    }
    .insert(db)
    .await
    .expect("insert extra task");

    let task_judge_id = Uuid::new_v4();
    task_judges::ActiveModel {
        id: Set(task_judge_id),
        task_id: Set(task_id),
        judge_id: Set(judge_id),
        ordinal: Set(0),
        rating_scale_override: Set(None),
        created_at: Set(Utc::now()),
        updated_at: Set(Utc::now()),
        weight: Set(None),
    }
    .insert(db)
    .await
    .expect("attach judge to extra task");
    (task_id, task_judge_id)
}

#[allow(clippy::too_many_arguments)]
async fn insert_verdict(
    db: &DatabaseConnection,
    session: Uuid,
    player: Uuid,
    task_judge_id: Uuid,
    rating: f64,
    point_delta: i32,
    feedback: &str,
    status: &str,
) {
    judge_results::ActiveModel {
        id: Set(Uuid::new_v4()),
        session_id_fk: Set(session),
        player_id_fk: Set(player),
        task_judge_id: Set(task_judge_id),
        rating: Set(json!(rating)),
        point_delta: Set(point_delta),
        feedback: Set(feedback.to_string()),
        model: Set("m".to_string()),
        provider: Set("p".to_string()),
        raw_output: Set(String::new()),
        duration_ms: Set(None),
        run_log: Set(None),
        tokens_input: Set(None),
        tokens_output: Set(None),
        tokens_cache_read: Set(None),
        tokens_cache_write: Set(None),
        status: Set(status.to_string()),
        error: Set(None),
        created_at: Set(Utc::now()),
        updated_at: Set(Utc::now()),
        verdict_kind: Set(None),
    }
    .insert(db)
    .await
    .expect("insert verdict");
}

#[tokio::test]
async fn a_judge_carries_its_earlier_session_verdicts_into_the_next_task() {
    let db = setup_db().await;
    let owner = insert_user(&db).await;
    let project = insert_project(&db, owner).await;
    let session = insert_session(&db, project).await;
    let player = insert_player(&db, session).await;
    let other_player = insert_player(&db, session).await;
    // Task 0 carries the judge; tasks 1 and 2 get the same judge attached.
    let (task0, judge_id, tj0) = insert_chain(&db, project).await;
    let (task1, tj1) = extra_task_for_judge(&db, project, judge_id, 1).await;
    let (task2, tj2) = extra_task_for_judge(&db, project, judge_id, 2).await;

    let repos_root = tempfile::tempdir().expect("repos tempdir");
    unsafe {
        std::env::set_var("OLOLO_GIT_REPOS_DIR", repos_root.path());
    }
    let base = arena_core::git_store::repos_base_dir().expect("base dir");
    let repo_dir = arena_core::git_store::player_repo_path(&base, session, player);
    std::fs::create_dir_all(&repo_dir).expect("mkdir repo");
    make_repo(&repo_dir);
    write_file(&repo_dir, "a.txt", "x");
    commit(&repo_dir, "init");
    let state = make_state(db.clone(), 3);

    // What this judge already said on task 0 — and noise that must NOT leak:
    // a later task's verdict, another player's, and a failed run.
    insert_verdict(
        &db,
        session,
        player,
        tj0,
        8.0,
        17,
        "Weak error handling.",
        "scored",
    )
    .await;
    insert_verdict(
        &db,
        session,
        player,
        tj2,
        9.0,
        20,
        "From the future.",
        "scored",
    )
    .await;
    insert_verdict(
        &db,
        session,
        other_player,
        tj0,
        3.0,
        5,
        "Someone else's work.",
        "scored",
    )
    .await;
    insert_verdict(
        &db,
        session,
        player,
        tj1,
        0.0,
        0,
        "Crashed mid-run.",
        "failed",
    )
    .await;

    let resolved = resolve_judge_run(&state, &db, session, player, task1, judge_id)
        .await
        .expect("resolve");

    let carried = &resolved.prior_session_verdicts;
    assert_eq!(
        carried.len(),
        1,
        "only this judge's own settled verdict on an EARLIER task travels: {carried:?}"
    );
    let v = &carried[0];
    assert_eq!(v.task_ordinal, 0);
    assert_eq!(v.rating, 8.0);
    assert_eq!(v.point_delta, 17);
    assert_eq!(v.feedback, "Weak error handling.");

    // Judging the first task carries nothing: there is no history yet.
    let first = resolve_judge_run(&state, &db, session, player, task0, judge_id)
        .await
        .expect("resolve task 0");
    assert!(
        first.prior_session_verdicts.is_empty(),
        "the opening task has no earlier verdicts: {:?}",
        first.prior_session_verdicts
    );

    // The failed run on task 1 settles on a re-run — one row per
    // (task_judge, player), so the retry UPDATES it rather than adding one.
    judge_results::Entity::update_many()
        .col_expr(
            judge_results::Column::Status,
            sea_orm::sea_query::Expr::value("scored"),
        )
        .col_expr(
            judge_results::Column::Feedback,
            sea_orm::sea_query::Expr::value("Better."),
        )
        .col_expr(
            judge_results::Column::PointDelta,
            sea_orm::sea_query::Expr::value(9),
        )
        .filter(judge_results::Column::TaskJudgeId.eq(tj1))
        .filter(judge_results::Column::PlayerIdFk.eq(player))
        .exec(&db)
        .await
        .expect("settle the retry");
    let third = resolve_judge_run(&state, &db, session, player, task2, judge_id)
        .await
        .expect("resolve task 2");
    let ordinals: Vec<i32> = third
        .prior_session_verdicts
        .iter()
        .map(|v| v.task_ordinal)
        .collect();
    assert_eq!(ordinals, vec![0, 1], "history reads oldest → newest");
}

#[tokio::test]
async fn another_judges_verdicts_never_leak_into_this_judges_history() {
    // Panel members must not read each other's reasoning: a second judge's
    // verdict is not this judge's record, and echoing it would collapse an
    // eight-judge panel into one opinion.
    let db = setup_db().await;
    let owner = insert_user(&db).await;
    let project = insert_project(&db, owner).await;
    let session = insert_session(&db, project).await;
    let player = insert_player(&db, session).await;
    let (_task0, judge_id, tj0) = insert_chain(&db, project).await;
    let (task1, _tj1) = extra_task_for_judge(&db, project, judge_id, 1).await;

    // A different judge, also attached to task 0.
    let other_judge = Uuid::new_v4();
    judges::ActiveModel {
        id: Set(other_judge),
        slug: Set("other-judge".to_string()),
        name: Set("Other Judge".to_string()),
        description: Set(String::new()),
        prompt: Set("Evaluate something else.".to_string()),
        rating_scale: Set(json!({"min": 0.0, "max": 10.0, "step": 0.5})),
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
        max_interactive: Set(None),
        avatar_url: Set(None),
        ignore_paths: Set(None),
        probes_config: Set(None),
    }
    .insert(&db)
    .await
    .expect("insert other judge");
    let other_tj = Uuid::new_v4();
    task_judges::ActiveModel {
        id: Set(other_tj),
        task_id: Set(_task0),
        judge_id: Set(other_judge),
        ordinal: Set(1),
        rating_scale_override: Set(None),
        created_at: Set(Utc::now()),
        updated_at: Set(Utc::now()),
        weight: Set(None),
    }
    .insert(&db)
    .await
    .expect("attach other judge");

    let repos_root = tempfile::tempdir().expect("repos tempdir");
    unsafe {
        std::env::set_var("OLOLO_GIT_REPOS_DIR", repos_root.path());
    }
    let base = arena_core::git_store::repos_base_dir().expect("base dir");
    let repo_dir = arena_core::git_store::player_repo_path(&base, session, player);
    std::fs::create_dir_all(&repo_dir).expect("mkdir repo");
    make_repo(&repo_dir);
    write_file(&repo_dir, "a.txt", "x");
    commit(&repo_dir, "init");
    let state = make_state(db.clone(), 3);

    insert_verdict(&db, session, player, tj0, 8.0, 17, "Mine.", "scored").await;
    insert_verdict(
        &db,
        session,
        player,
        other_tj,
        2.0,
        3,
        "Not mine.",
        "scored",
    )
    .await;

    let resolved = resolve_judge_run(&state, &db, session, player, task1, judge_id)
        .await
        .expect("resolve");
    let carried = &resolved.prior_session_verdicts;
    assert_eq!(carried.len(), 1);
    assert_eq!(carried[0].feedback, "Mine.");
}

#[tokio::test]
async fn the_reporter_waits_until_the_panel_is_terminal() {
    // The report quotes the panel: in FBTQYR the ux-review re-verdict landed
    // eleven seconds after the report that should have carried it, because a
    // judge re-driven off an artifact runs outside the session pass the
    // reporter is sorted last in. The reporter's gate is the terminality of
    // every other expected run.
    let db = setup_db().await;
    let owner = insert_user(&db).await;
    let project = insert_project(&db, owner).await;
    let session = insert_session(&db, project).await;
    let player = insert_player(&db, session).await;

    let task_id = Uuid::new_v4();
    tasks::ActiveModel {
        id: Set(task_id),
        project_id_fk: Set(project),
        ordinal: Set(0),
        title: Set("Task 0".to_string()),
        content: Set("do it".to_string()),
        test_template: Set(json!({"kind": "shell"})),
        created_at: Set(Utc::now()),
        tags: Set(String::new()),
        point_value: Set(10),
        deadline_secs: Set(Some(300)),
        min_interval_secs: Set(Some(5)),
        interval_increment_secs: Set(Some(0)),
        max_interval_secs: Set(Some(300)),
        fail_points: Set(0),
        no_response_points: Set(0),
        completion_bonus_points: Set(0),
        evaluation: Set(None),
    }
    .insert(&db)
    .await
    .expect("insert task");

    // The sibling gate counts pairs over the tasks this player REACHED, so
    // the player needs a scheduler row saying they are on task 0.
    arena_core::entities::session_scheduler_state::ActiveModel {
        id: Set(Uuid::new_v4()),
        session_id_fk: Set(session),
        player_id_fk: Set(player),
        task_id: Set(Some(task_id)),
        state: Set(arena_core::session_completion::SCHEDULER_STATE_JUDGING.to_string()),
        next_probe_at: Set(None),
        created_at: Set(Utc::now()),
        updated_at: Set(Utc::now()),
    }
    .insert(&db)
    .await
    .expect("insert scheduler row");

    // An ordinary panel judge, attached and expected.
    let panelist = Uuid::new_v4();
    judges::ActiveModel {
        id: Set(panelist),
        slug: Set("ux-review".to_string()),
        name: Set("UX Review".to_string()),
        description: Set(String::new()),
        prompt: Set("judge".to_string()),
        rating_scale: Set(json!({"min": 0.0, "max": 10.0, "step": 0.1})),
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
        max_interactive: Set(None),
        avatar_url: Set(None),
        ignore_paths: Set(None),
        probes_config: Set(None),
    }
    .insert(&db)
    .await
    .expect("insert panelist");
    let tj_panel = Uuid::new_v4();
    task_judges::ActiveModel {
        id: Set(tj_panel),
        task_id: Set(task_id),
        judge_id: Set(panelist),
        ordinal: Set(0),
        rating_scale_override: Set(None),
        created_at: Set(Utc::now()),
        updated_at: Set(Utc::now()),
        weight: Set(None),
    }
    .insert(&db)
    .await
    .expect("attach panelist");

    assert!(
        !game_server::judge_queue::sibling_runs_terminal(&db, session, player)
            .await
            .expect("check"),
        "an expected run with no verdict holds the report"
    );

    // A run suspended on an artifact is not terminal either.
    let result_id = Uuid::new_v4();
    judge_results::ActiveModel {
        id: Set(result_id),
        session_id_fk: Set(session),
        player_id_fk: Set(player),
        task_judge_id: Set(tj_panel),
        rating: Set(json!(0.0)),
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
        error: Set(None),
        created_at: Set(Utc::now()),
        updated_at: Set(Utc::now()),
        verdict_kind: Set(None),
    }
    .insert(&db)
    .await
    .expect("insert waiting result");
    assert!(
        !game_server::judge_queue::sibling_runs_terminal(&db, session, player)
            .await
            .expect("check"),
        "a waiting run holds the report"
    );

    // Scored — and a failed run would count the same: failure is terminal.
    let mut am: judge_results::ActiveModel = judge_results::Entity::find_by_id(result_id)
        .one(&db)
        .await
        .expect("query")
        .expect("row")
        .into();
    am.status = Set("scored".to_string());
    am.update(&db).await.expect("score the run");
    assert!(
        game_server::judge_queue::sibling_runs_terminal(&db, session, player)
            .await
            .expect("check"),
        "a scored panel releases the report"
    );

    // The reporter's own attachment must not hold itself hostage.
    let reporter = Uuid::new_v4();
    judges::ActiveModel {
        id: Set(reporter),
        slug: Set("general".to_string()),
        name: Set("The Debrief".to_string()),
        description: Set(String::new()),
        prompt: Set("write the report".to_string()),
        rating_scale: Set(json!({"min": 0.0, "max": 1.0, "step": 1.0})),
        kind: Set(arena_core::judging::JUDGE_KIND_REPORT.to_string()),
        scope: Set("session".to_string()),
        evidence_mode: Set("dossier".to_string()),
        evidence_needs: Set(None),
        created_at: Set(Utc::now()),
        updated_at: Set(Utc::now()),
        llm_provider_id_fk: Set(None),
        llm_model: Set(None),
        llm_pool_id_fk: Set(None),
        llm_source_order: Set(arena_core::llm::resolve::SOURCE_ORDER_POOL_FIRST.to_string()),
        criteria: Set(None),
        max_interactive: Set(None),
        avatar_url: Set(None),
        ignore_paths: Set(None),
        probes_config: Set(None),
    }
    .insert(&db)
    .await
    .expect("insert reporter");
    task_judges::ActiveModel {
        id: Set(Uuid::new_v4()),
        task_id: Set(task_id),
        judge_id: Set(reporter),
        ordinal: Set(1),
        rating_scale_override: Set(None),
        created_at: Set(Utc::now()),
        updated_at: Set(Utc::now()),
        weight: Set(None),
    }
    .insert(&db)
    .await
    .expect("attach reporter");
    assert!(
        game_server::judge_queue::sibling_runs_terminal(&db, session, player)
            .await
            .expect("check"),
        "the reporter's own pending run does not block it"
    );

    // A judged task further down the ladder that this player never reached:
    // the panel writes no rows for it, so counting its pairs would hold the
    // report for the full wait timeout on every session with an unreached
    // tail (6O3C3Y sat out the whole 600s on a two-of-three ladder).
    let unreached = Uuid::new_v4();
    tasks::ActiveModel {
        id: Set(unreached),
        project_id_fk: Set(project),
        ordinal: Set(1),
        title: Set("Task 1, never reached".to_string()),
        content: Set("do it".to_string()),
        test_template: Set(json!({"kind": "shell"})),
        created_at: Set(Utc::now()),
        tags: Set(String::new()),
        point_value: Set(10),
        deadline_secs: Set(Some(300)),
        min_interval_secs: Set(Some(5)),
        interval_increment_secs: Set(Some(0)),
        max_interval_secs: Set(Some(300)),
        fail_points: Set(0),
        no_response_points: Set(0),
        completion_bonus_points: Set(0),
        evaluation: Set(None),
    }
    .insert(&db)
    .await
    .expect("insert unreached task");
    task_judges::ActiveModel {
        id: Set(Uuid::new_v4()),
        task_id: Set(unreached),
        judge_id: Set(panelist),
        ordinal: Set(0),
        rating_scale_override: Set(None),
        created_at: Set(Utc::now()),
        updated_at: Set(Utc::now()),
        weight: Set(None),
    }
    .insert(&db)
    .await
    .expect("attach panelist to the unreached task");
    assert!(
        game_server::judge_queue::sibling_runs_terminal(&db, session, player)
            .await
            .expect("check"),
        "a pair on a task the player never reached does not hold the report"
    );
}
