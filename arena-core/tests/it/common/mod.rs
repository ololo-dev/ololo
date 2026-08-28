//! Shared test helpers for `arena-core` integration tests.
//!
//! Each integration-test file includes this module via
//! `#[allow(dead_code)] mod common; use common::*;`. Helpers that are not
//! used by a particular test crate are simply dead (and silenced by the
//! allow on the `mod common;` declaration).

#![allow(dead_code)]
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use async_trait::async_trait;
use chrono::Utc;
use migration::{Migrator, MigratorTrait};
use sea_orm::{ActiveModelTrait, DatabaseConnection, Set};
use uuid::Uuid;

use arena_core::entities::{judges, players, projects, sessions, task_judges, tasks, users};
use arena_core::judging::{AgentResponse, JudgeError, JudgeLlm, JudgeRow, TaskJudgeRow, TaskRow};
use arena_core::session_status::SessionStatus;

// ---------------------------------------------------------------------------
// FakeJudgeLlm — scripted sequence of AgentResponse turns.
// ---------------------------------------------------------------------------

pub struct FakeJudgeLlm {
    responses: Mutex<Vec<AgentResponse>>,
}

impl FakeJudgeLlm {
    pub fn new(responses: Vec<AgentResponse>) -> Self {
        Self {
            responses: Mutex::new(responses),
        }
    }
}

#[async_trait]
impl JudgeLlm for FakeJudgeLlm {
    async fn run_agent(
        &self,
        _system: &str,
        _user: &str,
        _tools: Vec<arena_core::judging::ToolDef>,
        _prior_tool_result: Option<&str>,
    ) -> Result<AgentResponse, JudgeError> {
        let mut guard = self.responses.lock().unwrap();
        if guard.is_empty() {
            return Ok(AgentResponse::Final {
                text: r#"{"rating": 0.0, "feedback": "no more responses"}"#.to_string(),
            });
        }
        Ok(guard.remove(0))
    }
}

// ---------------------------------------------------------------------------
// Test DB helpers (identical across the judging + scoring test sets).
// ---------------------------------------------------------------------------

pub async fn setup_db() -> DatabaseConnection {
    let db = sea_orm::Database::connect("sqlite::memory:")
        .await
        .expect("sqlite connect");
    Migrator::up(&db, None).await.expect("migrate up");
    db
}

pub async fn insert_user(db: &DatabaseConnection) -> Uuid {
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

pub async fn insert_project(db: &DatabaseConnection, owner_id: Uuid) -> Uuid {
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

pub async fn insert_session(db: &DatabaseConnection, project_id: Uuid) -> Uuid {
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

pub async fn insert_player(db: &DatabaseConnection, session_id: Uuid) -> Uuid {
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

// ---------------------------------------------------------------------------
// Judging chain helpers (task + judge + task_judge + row snapshots).
// ---------------------------------------------------------------------------

pub async fn insert_chain(
    db: &DatabaseConnection,
    project_id: Uuid,
    session_id: Uuid,
    player_id: Uuid,
) -> (Uuid, Uuid, Uuid) {
    let task_id = Uuid::new_v4();
    tasks::ActiveModel {
        id: Set(task_id),
        project_id_fk: Set(project_id),
        ordinal: Set(0),
        title: Set("Reverse a string".to_string()),
        content: Set("Write a function that reverses a string.".to_string()),
        test_template: Set(serde_json::json!({"kind":"shell","command_template":"echo ok"})),
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
        probes_config: Set(None),
        max_interactive: Set(None),
        avatar_url: Set(None),
        ignore_paths: Set(None),
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
        weight: Set(None),
        created_at: Set(Utc::now()),
        updated_at: Set(Utc::now()),
    }
    .insert(db)
    .await
    .expect("insert task_judge");

    let _ = (session_id, player_id);
    (task_id, judge_id, task_judge_id)
}

pub fn row_snapshots(
    task_id: Uuid,
    judge_id: Uuid,
    task_judge_id: Uuid,
) -> (TaskJudgeRow, JudgeRow, TaskRow) {
    let task_judge_row = TaskJudgeRow {
        id: task_judge_id,
        task_id,
        judge_id,
        rating_scale_override: None,
        weight: None,
    };
    let judge_row = JudgeRow {
        slug: "code-quality".to_string(),
        name: "Code Quality Judge".to_string(),
        prompt: "Evaluate code quality.".to_string(),
        rating_scale: serde_json::json!({"min": 0.0, "max": 10.0, "step": 0.5}),
        kind: "llm".to_string(),
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
    let task_row = TaskRow {
        id: task_id,
        title: "Reverse a string".to_string(),
        description: "Write a function that reverses a string.".to_string(),
        tags: "algorithms".to_string(),
        point_value: 10,
        evaluation: None,
    };
    (task_judge_row, judge_row, task_row)
}

// ---------------------------------------------------------------------------
// Mock git repo helpers.
// ---------------------------------------------------------------------------

pub fn git_bin() -> PathBuf {
    which::which("git").expect("git binary found")
}

pub fn make_repo(dir: &Path) {
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

pub fn commit(dir: &Path, msg: &str) -> String {
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

/// Commit with no staged changes. The ololo CLI snapshots on a timer, so a
/// task whose files never changed still produces a commit — evidence the
/// dossier must distinguish from "no commit at all".
pub fn commit_allow_empty(dir: &Path, msg: &str) -> String {
    let git = git_bin();
    let out = std::process::Command::new(&git)
        .arg("-C")
        .arg(dir)
        .args(["commit", "-q", "--allow-empty", "-m", msg])
        .output()
        .expect("git commit --allow-empty");
    assert!(
        out.status.success(),
        "empty commit failed: {}",
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

pub fn write_file(dir: &Path, name: &str, content: &str) {
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
