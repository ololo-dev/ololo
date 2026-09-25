//! Health checkpoints end to end on the game server: a client report lands,
//! the server verifies the commit from the player's repo, flags what
//! disagrees, publishes; a closed task gets its task-final checkpoint.
//!
//! The player repo is a real bare git repo under a temp `OLOLO_GIT_REPOS_DIR`
//! (one process per test under nextest), with commits shaped exactly like
//! the ololo client writes them.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use arena_core::entities::{
    app_settings, health_checkpoints, projects, session_scheduler_state, sessions, tasks, users,
};
use arena_core::protocol::{
    HealthCheckStatus, HealthReportPayload, HealthReportStatus, PushState, PushStatus, ZmqEvent,
};
use arena_core::session_status::SessionStatus;
use arena_core::snapshot_message::{self as message, Kind, Trailers};
use chrono::Utc;
use dashmap::DashMap;
use game_server::state::GameServerState;
use game_server::zmq_pub::EventPublisher;
use jsonwebtoken::{DecodingKey, EncodingKey};
use migration::MigratorTrait;
use sea_orm::{ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, Set};
use tokio::sync::Semaphore;
use uuid::Uuid;

const CODE: &str = r#"export function summarize(items, options) {
  let total = 0;
  let count = 0;
  for (const item of items) {
    if (item.price > 0 && item.quantity > 0) {
      total += item.price * item.quantity;
      count += 1;
    } else if (options.strict) {
      throw new Error("invalid item: " + item.id);
    }
  }
  const average = count > 0 ? total / count : 0;
  return { total, count, average, currency: options.currency || "USD" };
}
"#;

/// Records every published event.
struct Recorder(Mutex<Vec<ZmqEvent>>);

#[async_trait::async_trait]
impl EventPublisher for Recorder {
    async fn publish(&self, event: &ZmqEvent) {
        self.0.lock().unwrap().push(event.clone());
    }
}

struct Rig {
    state: GameServerState,
    events: Arc<Recorder>,
    session_id: Uuid,
    player_id: Uuid,
    task_a: Uuid,
    task_b: Uuid,
    join_code: String,
    /// The player's bare repo.
    repo: PathBuf,
    /// A working clone to author commits in (the client authors with gix
    /// into a bare repo; plain git in a clone produces the same history).
    work: PathBuf,
    _repos_root: tempfile::TempDir,
}

async fn setup() -> Rig {
    let db = sea_orm::Database::connect("sqlite::memory:")
        .await
        .expect("connect");
    migration::Migrator::up(&db, None).await.expect("migrate");
    app_settings::ActiveModel {
        key: Set(arena_core::health_settings::HEALTH_ENABLED_KEY.to_string()),
        value: Set("true".to_string()),
    }
    .insert(&db)
    .await
    .expect("enable health");

    let events = Arc::new(Recorder(Mutex::new(Vec::new())));
    let secret = b"test-secret-32-bytes-or-more-xxxxxxx".to_vec();
    let server_id = Uuid::new_v4();
    let state = GameServerState {
        db: db.clone(),
        server_id,
        advertise_url: "ws://localhost:8081".to_string(),
        jwt_encoding_key: Arc::new(EncodingKey::from_secret(&secret)),
        jwt_decoding_key: Arc::new(DecodingKey::from_secret(&secret)),
        jwt_signing_secret: Arc::new(secret),
        session_registry: Arc::new(DashMap::new()),
        player_agent_registry: Arc::new(DashMap::new()),
        lobby_timer_secs: 60,
        event_publisher: events.clone(),
        judge_semaphore: Arc::new(Semaphore::new(3)),
        settings_encryption: Arc::new(arena_core::settings_encryption::SettingsEncryption::new(
            b"test-secret-key-for-settings-enc",
        )),
    };

    let (session_id, player_id, task_a, task_b) = seed(&db).await;

    let repos_root = tempfile::tempdir().expect("repos tempdir");
    // SAFETY: one process per test under nextest; the variable is scoped to
    // this test process and only read by the code under test.
    unsafe {
        std::env::set_var("OLOLO_GIT_REPOS_DIR", repos_root.path());
    }
    // What the web server provisions at join: a bare, fast-forward-only repo.
    let repo = arena_core::git_store::player_repo_path(repos_root.path(), session_id, player_id);
    std::fs::create_dir_all(repo.parent().unwrap()).unwrap();
    git(
        repos_root.path(),
        &[
            "init",
            "-q",
            "--bare",
            "--initial-branch=main",
            repo.to_str().unwrap(),
        ],
    );
    git(&repo, &["config", "receive.denyNonFastForwards", "true"]);
    let work = repos_root.path().join("work");
    git(
        repos_root.path(),
        &["clone", "-q", repo.to_str().unwrap(), "work"],
    );
    git(&work, &["checkout", "-q", "-b", "main"]);

    Rig {
        state,
        events,
        session_id,
        player_id,
        task_a,
        task_b,
        join_code: "HLTH01".into(),
        repo,
        work,
        _repos_root: repos_root,
    }
}

async fn seed(db: &DatabaseConnection) -> (Uuid, Uuid, Uuid, Uuid) {
    let now = Utc::now();
    let user_id = users::ActiveModel {
        id: Set(Uuid::new_v4()),
        email: Set(format!("u{}@example.com", Uuid::new_v4())),
        password_hash: Set(None),
        display_name: Set("tester".to_string()),
        created_at: Set(now),
        updated_at: Set(now),
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
        created_at: Set(now),
        updated_at: Set(now),
        default_value_points: Set(10),
        default_fail_points: Set(-5),
        default_no_response_points: Set(-10),
        default_health_points: Set(0),
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
        created_at: Set(now),
        owner_id_fk: Set(None),
        status: Set(SessionStatus::Running),
        join_code: Set("HLTH01".to_string()),
        started_at: Set(Some(now - chrono::Duration::seconds(30))),
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
    let player_id = arena_core::entities::players::ActiveModel {
        id: Set(Uuid::new_v4()),
        session_id_fk: Set(session_id),
        user_id_fk: Set(Some(user_id)),
        display_name: Set("tester".to_string()),
        fingerprint: Set(None),
        metadata_json: Set(None),
        joined_at: Set(now),
        reconnected_at: Set(None),
        revoked_at: Set(None),
        agent_connected: Set(false),
        agent_last_seen_at: Set(None),
    }
    .insert(db)
    .await
    .expect("player")
    .id;
    let mut ids = Vec::new();
    for (ordinal, title) in [(0, "Task A"), (1, "Task B")] {
        let id = tasks::ActiveModel {
            id: Set(Uuid::new_v4()),
            project_id_fk: Set(project_id),
            ordinal: Set(ordinal),
            title: Set(title.to_string()),
            content: Set("do it".to_string()),
            test_template: Set(serde_json::json!({"kind": "shell", "command_template": "echo"})),
            created_at: Set(now),
            tags: Set("[]".to_string()),
            point_value: Set(10),
            deadline_secs: Set(None),
            min_interval_secs: Set(None),
            interval_increment_secs: Set(None),
            max_interval_secs: Set(None),
            fail_points: Set(-5),
            no_response_points: Set(-10),
            health_points: Set(20),
            completion_bonus_points: Set(10),
            evaluation: Set(None),
        }
        .insert(db)
        .await
        .expect("task")
        .id;
        ids.push(id);
    }
    session_scheduler_state::ActiveModel {
        id: Set(Uuid::new_v4()),
        session_id_fk: Set(session_id),
        player_id_fk: Set(player_id),
        task_id: Set(Some(ids[0])),
        state: Set("active".to_string()),
        next_probe_at: Set(None),
        created_at: Set(now),
        updated_at: Set(now),
    }
    .insert(db)
    .await
    .expect("scheduler state");
    (session_id, player_id, ids[0], ids[1])
}

fn git(dir: &Path, args: &[&str]) -> String {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_AUTHOR_NAME", "ololo-snapshot")
        .env("GIT_AUTHOR_EMAIL", "ololo@local")
        .env("GIT_COMMITTER_NAME", "ololo-snapshot")
        .env("GIT_COMMITTER_EMAIL", "ololo@local")
        .output()
        .expect("git");
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

impl Rig {
    /// Commit the working tree with a client-shaped message and push.
    fn commit(
        &self,
        kind: &Kind,
        task: Option<Uuid>,
        subject: &str,
        probe: Option<(Uuid, u32)>,
    ) -> String {
        let trailers = Trailers {
            format: Some(message::FORMAT_VERSION),
            session: Some(self.session_id),
            participant: Some(self.player_id),
            task,
            task_title: task.map(|t| {
                if t == self.task_a {
                    "Task A".into()
                } else {
                    "Task B".into()
                }
            }),
            probe: probe.map(|p| p.0),
            probe_seq: probe.map(|p| p.1),
            outcome: (kind == &Kind::Feat).then(|| "completed".to_string()),
            timestamp: Some(Utc::now()),
        };
        let msg = message::format(&message::subject(kind, task, subject), &trailers);
        git(&self.work, &["add", "-A"]);
        git(&self.work, &["commit", "-q", "--allow-empty", "-m", &msg]);
        git(&self.work, &["push", "-q", "origin", "main:main"]);
        git(&self.work, &["rev-parse", "HEAD"])
    }

    fn write(&self, rel: &str, content: &str) {
        let path = self.work.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, content).unwrap();
    }

    fn report(
        &self,
        probe_id: Uuid,
        seq: u32,
        task: Uuid,
        commit: &str,
        score: Option<f64>,
    ) -> HealthReportPayload {
        HealthReportPayload {
            probe_id,
            probe_seq: seq,
            task_id: Some(task),
            session_id: Some(self.session_id),
            player_id: Some(self.player_id),
            commit: Some(commit.to_string()),
            status: HealthReportStatus::Ok,
            result: score.map(ololo_health_result),
            error: None,
            duration_ms: 120,
            jscpd_version: ololo_health::JSCPD_CORE_VERSION.to_string(),
            health_schema: ololo_health::HEALTH_SCHEMA,
            push: PushStatus {
                state: PushState::Pushed,
                error: None,
                pushed_commit: Some(commit.to_string()),
            },
        }
    }

    async fn checkpoint(&self, commit: &str) -> health_checkpoints::Model {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
        loop {
            let row = health_checkpoints::Entity::find()
                .filter(health_checkpoints::Column::CommitSha.eq(commit))
                .one(&self.state.db)
                .await
                .expect("query")
                .filter(|r| r.server_status != HealthCheckStatus::Pending.as_str());
            if let Some(row) = row {
                return row;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "checkpoint {commit} never settled"
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    fn health_events(&self) -> Vec<ZmqEvent> {
        self.events
            .0
            .lock()
            .unwrap()
            .iter()
            .filter(|e| matches!(e, ZmqEvent::HealthUpdated { .. }))
            .cloned()
            .collect()
    }

    /// The row settles before its event is published; wait for the events.
    async fn health_events_at_least(&self, n: usize) -> Vec<ZmqEvent> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
        loop {
            let events = self.health_events();
            if events.len() >= n {
                return events;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "only {} health events, wanted {n}",
                events.len()
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }
}

/// The server's own scan of a tree with these files, so a "matching" client
/// score is exactly what the server will compute.
fn score_of(files: &[(&str, &str)]) -> ololo_health::HealthResult {
    let dir = tempfile::tempdir().unwrap();
    for (rel, content) in files {
        let p = dir.path().join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, content).unwrap();
    }
    ololo_health::analyze(dir.path(), &ololo_health::HealthConfig::default()).unwrap()
}

fn ololo_health_result(score: f64) -> ololo_health::HealthResult {
    let mut r = score_of(&[("src/a.js", CODE)]);
    r.score = Some(score);
    r
}

#[tokio::test]
async fn a_report_is_stored_verified_flagged_and_published() {
    let rig = setup().await;
    rig.write("src/a.js", CODE);
    rig.commit(&Kind::Session, None, "session start @ x", None);
    rig.commit(&Kind::Start, Some(rig.task_a), "Task A", None);
    rig.write("src/b.js", CODE);
    let probe = Uuid::new_v4();
    let sha = rig.commit(
        &Kind::Probe,
        Some(rig.task_a),
        "#1 Task A",
        Some((probe, 1)),
    );

    // The client scored the same tree: no mismatch.
    let expected = score_of(&[("src/a.js", CODE), ("src/b.js", CODE)]);
    let report = rig.report(probe, 1, rig.task_a, &sha, expected.score);
    game_server::health::on_report(
        rig.state.clone(),
        rig.session_id,
        rig.player_id,
        rig.join_code.clone(),
        report,
    )
    .await;

    let row = rig.checkpoint(&sha).await;
    assert_eq!(row.server_status, "ok", "{row:?}");
    assert_eq!(row.server_score, expected.score);
    assert_eq!(row.client_score, expected.score);
    assert_eq!(row.kind, "probe");
    assert_eq!(row.probe_seq, 1);
    assert_eq!(
        row.probe_id_fk, None,
        "no probe row was dispatched in this rig"
    );
    assert_eq!(row.task_id_fk, Some(rig.task_a));
    assert_eq!(
        row.derived_task_id,
        Some(rig.task_a),
        "the log attributes the commit to A"
    );
    assert!(!row.score_mismatch && !row.version_mismatch && !row.task_mismatch);
    assert!(!row.late && !row.history_rewritten);
    assert_eq!(
        row.server_jscpd_version.as_deref(),
        Some(ololo_health::JSCPD_CORE_VERSION)
    );
    assert!(row.server_result.is_some());
    assert!(row.server_verified_at.is_some());

    // Two events: the pending checkpoint, then the verified one.
    let events = rig.health_events_at_least(2).await;
    assert_eq!(events.len(), 2, "{events:?}");
    let ZmqEvent::HealthUpdated {
        checkpoint,
        player_id,
        join_code,
        ..
    } = &events[1]
    else {
        unreachable!()
    };
    assert_eq!(*player_id, rig.player_id);
    assert_eq!(join_code, "HLTH01");
    assert_eq!(checkpoint.server_status, HealthCheckStatus::Ok);
    assert_eq!(checkpoint.score, expected.score);
    assert_eq!(checkpoint.task_title.as_deref(), Some("Task A"));
    assert!(checkpoint.t.is_some(), "the session has started");
    assert!(checkpoint.client.is_some() && checkpoint.server.is_some());
}

#[tokio::test]
async fn disagreements_are_flagged_not_argued() {
    let rig = setup().await;
    rig.write("src/a.js", CODE);
    rig.commit(&Kind::Session, None, "session start @ x", None);
    rig.commit(&Kind::Start, Some(rig.task_a), "Task A", None);
    let probe = Uuid::new_v4();
    let sha = rig.commit(
        &Kind::Probe,
        Some(rig.task_a),
        "#1 Task A",
        Some((probe, 1)),
    );

    // The client claims a different score with another jscpd, for task B
    // while the scheduler is on task A, and the history says A.
    let mut report = rig.report(probe, 1, rig.task_b, &sha, Some(12.5));
    report.jscpd_version = "0.0.1".into();
    game_server::health::on_report(
        rig.state.clone(),
        rig.session_id,
        rig.player_id,
        rig.join_code.clone(),
        report,
    )
    .await;
    let row = rig.checkpoint(&sha).await;
    assert_eq!(row.server_status, "ok");
    assert!(
        row.score_mismatch,
        "12.5 is not what the server computed: {row:?}"
    );
    assert!(row.version_mismatch);
    assert!(row.task_mismatch, "the log says A, the probe said B");
    assert_eq!(row.derived_task_id, Some(rig.task_a));
    assert!(row.late, "task B is not the scheduler's current task");
    // The server's number wins in the view.
    let ZmqEvent::HealthUpdated { checkpoint, .. } =
        rig.health_events_at_least(2).await.last().cloned().unwrap()
    else {
        unreachable!()
    };
    assert_eq!(checkpoint.score, row.server_score);
    assert!(checkpoint.flags.score_mismatch && checkpoint.flags.version_mismatch);
}

#[tokio::test]
async fn a_commit_that_never_lands_is_commit_missing_and_a_rejected_push_is_unverified() {
    let rig = setup().await;
    rig.write("src/a.js", CODE);
    rig.commit(&Kind::Session, None, "session start @ x", None);

    let probe = Uuid::new_v4();
    let ghost = "1111111111111111111111111111111111111111";
    let mut report = rig.report(probe, 1, rig.task_a, ghost, Some(50.0));
    report.push.state = PushState::Pending;
    report.push.pushed_commit = None;
    game_server::health::on_report(
        rig.state.clone(),
        rig.session_id,
        rig.player_id,
        rig.join_code.clone(),
        report,
    )
    .await;
    // The verifier spawned by on_report waits minutes; settle the row from
    // here with no patience, as the sweep would.
    let pending = health_checkpoints::Entity::find()
        .filter(health_checkpoints::Column::CommitSha.eq(ghost))
        .one(&rig.state.db)
        .await
        .unwrap()
        .expect("stored while pending");
    assert_eq!(pending.server_status, "pending");
    game_server::health::verify(
        rig.state.clone(),
        pending.id,
        rig.join_code.clone(),
        Duration::ZERO,
    )
    .await;
    let row = rig.checkpoint(ghost).await;
    assert_eq!(row.server_status, "commit_missing");
    assert!(
        row.server_error
            .as_deref()
            .unwrap()
            .contains("never reached")
    );

    // A push the server refused: nothing to verify, said so up front.
    let probe2 = Uuid::new_v4();
    let stray = "2222222222222222222222222222222222222222";
    let mut report = rig.report(probe2, 2, rig.task_a, stray, Some(50.0));
    report.push.state = PushState::RejectedNonFastForward;
    game_server::health::on_report(
        rig.state.clone(),
        rig.session_id,
        rig.player_id,
        rig.join_code.clone(),
        report,
    )
    .await;
    let row = rig.checkpoint(stray).await;
    assert_eq!(row.server_status, "unverified");
    assert!(row.history_rewritten);

    // A report without a commit is stored as failed, never dropped.
    let probe3 = Uuid::new_v4();
    let mut report = rig.report(probe3, 3, rig.task_a, "x", None);
    report.commit = None;
    report.status = HealthReportStatus::Failed;
    report.error = Some("disk full".into());
    game_server::health::on_report(
        rig.state.clone(),
        rig.session_id,
        rig.player_id,
        rig.join_code.clone(),
        report,
    )
    .await;
    let row = rig.checkpoint(&format!("none:{probe3}")).await;
    assert_eq!(row.server_status, "failed");
    assert_eq!(row.client_status.as_deref(), Some("failed"));
    assert_eq!(row.client_error.as_deref(), Some("disk full"));
}

#[tokio::test]
async fn a_closed_task_gets_a_verified_task_final_checkpoint() {
    let rig = setup().await;
    rig.write("src/a.js", CODE);
    rig.commit(&Kind::Session, None, "session start @ x", None);
    rig.commit(&Kind::Start, Some(rig.task_a), "Task A", None);
    rig.write("src/b.js", CODE);
    let feat = rig.commit(&Kind::Feat, Some(rig.task_a), "Task A", None);

    let task = tasks::Entity::find_by_id(rig.task_a)
        .one(&rig.state.db)
        .await
        .unwrap()
        .unwrap();
    game_server::health::on_task_closed(
        rig.state.clone(),
        rig.session_id,
        rig.player_id,
        rig.join_code.clone(),
        task,
    )
    .await;
    let row = rig.checkpoint(&feat).await;
    assert_eq!(row.kind, "task_final");
    assert_eq!(row.server_status, "ok");
    assert_eq!(row.probe_id_fk, None);
    assert_eq!(
        row.client_status, None,
        "no client side for a task-final checkpoint"
    );
    assert_eq!(row.task_id_fk, Some(rig.task_a));
    assert_eq!(row.derived_task_id, Some(rig.task_a));
    let expected = score_of(&[("src/a.js", CODE), ("src/b.js", CODE)]);
    assert_eq!(row.server_score, expected.score);
    assert!(!row.score_mismatch, "no client number to disagree with");

    // The dashboards heard about it as a task-final point.
    let ZmqEvent::HealthUpdated { checkpoint, .. } =
        rig.health_events_at_least(2).await.last().cloned().unwrap()
    else {
        unreachable!()
    };
    assert_eq!(
        checkpoint.kind,
        arena_core::protocol::HealthCheckpointKind::TaskFinal
    );
    assert_eq!(checkpoint.commit, feat);
}

#[tokio::test]
async fn nothing_happens_when_health_is_off() {
    let rig = setup().await;
    app_settings::Entity::delete_by_id(arena_core::health_settings::HEALTH_ENABLED_KEY.to_string())
        .exec(&rig.state.db)
        .await
        .unwrap();
    let (seq, config) =
        game_server::health::probe_dispatch_info(&rig.state, rig.session_id, rig.player_id).await;
    assert_eq!((seq, config), (0, None));

    rig.write("src/a.js", CODE);
    let sha = rig.commit(&Kind::Session, None, "session start @ x", None);
    let report = rig.report(Uuid::new_v4(), 1, rig.task_a, &sha, Some(70.0));
    game_server::health::on_report(
        rig.state.clone(),
        rig.session_id,
        rig.player_id,
        rig.join_code.clone(),
        report,
    )
    .await;
    let rows = health_checkpoints::Entity::find()
        .all(&rig.state.db)
        .await
        .unwrap();
    assert!(rows.is_empty());
    assert!(rig.health_events().is_empty());
    assert!(rig.repo.join("HEAD").exists(), "the repo exists regardless");
}

#[tokio::test]
async fn a_closed_task_pays_its_health_bonus_once_from_the_verified_tree() {
    use arena_core::entities::task_results;
    let rig = setup().await;
    rig.write("src/a.js", CODE);
    rig.commit(&Kind::Session, None, "session start @ x", None);
    rig.commit(&Kind::Start, Some(rig.task_a), "Task A", None);
    // A clean tree: one file, no clones — green.
    let feat = rig.commit(&Kind::Feat, Some(rig.task_a), "Task A", None);
    let task = tasks::Entity::find_by_id(rig.task_a)
        .one(&rig.state.db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(task.health_points, 20);

    game_server::health::on_task_closed(
        rig.state.clone(),
        rig.session_id,
        rig.player_id,
        rig.join_code.clone(),
        task.clone(),
    )
    .await;
    let row = rig.checkpoint(&feat).await;
    assert_eq!(row.server_status, "ok");
    let expected = score_of(&[("src/a.js", CODE)]);
    let expected_bonus =
        ololo_health::bonus(expected.score, &ololo_health::Thresholds::default(), 20);
    assert!(
        expected_bonus.points > 0,
        "a clean tree earns: {expected:?}"
    );

    // The award follows the verification inside the same spawned task.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    let bonus_row = loop {
        let found = task_results::Entity::find()
            .filter(task_results::Column::SessionIdFk.eq(rig.session_id))
            .filter(task_results::Column::Kind.eq(task_results::KIND_HEALTH_BONUS))
            .one(&rig.state.db)
            .await
            .unwrap();
        if let Some(r) = found {
            break r;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "no health bonus row"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    assert_eq!(bonus_row.task_id, Some(rig.task_a));
    assert!(bonus_row.is_bonus);
    assert_eq!(bonus_row.point_delta, expected_bonus.points);
    assert!(
        bonus_row
            .answer
            .starts_with("health-bonus: final tree scored"),
        "{}",
        bonus_row.answer
    );
    // The row lands before the score event is published (a session-log
    // write sits between them): wait for the event, then count it.
    let score_events = || {
        rig.events
            .0
            .lock()
            .unwrap()
            .iter()
            .filter(|e| matches!(e, ZmqEvent::ScoreChange { .. }))
            .count()
    };
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    while score_events() == 0 {
        assert!(
            tokio::time::Instant::now() < deadline,
            "no score change for the bonus"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(score_events(), 1, "one score change for the bonus");

    // Closing the task again (a reconnect replays the close) pays nothing more.
    game_server::health::award_health_bonus(
        &rig.state,
        rig.session_id,
        rig.player_id,
        &task,
        &rig.join_code,
        &arena_core::health_settings::HealthSettings::load(&rig.state.db)
            .await
            .unwrap(),
    )
    .await;
    let rows = task_results::Entity::find()
        .filter(task_results::Column::Kind.eq(task_results::KIND_HEALTH_BONUS))
        .all(&rig.state.db)
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);

    // A task that pays no health points gets no row, whatever its tree.
    let mut free = task.clone();
    free.id = rig.task_b;
    free.health_points = 0;
    game_server::health::award_health_bonus(
        &rig.state,
        rig.session_id,
        rig.player_id,
        &free,
        &rig.join_code,
        &arena_core::health_settings::HealthSettings::load(&rig.state.db)
            .await
            .unwrap(),
    )
    .await;
    let rows = task_results::Entity::find()
        .filter(task_results::Column::Kind.eq(task_results::KIND_HEALTH_BONUS))
        .all(&rig.state.db)
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
}

#[tokio::test]
async fn without_a_verified_checkpoint_the_bonus_is_zero_and_says_why() {
    use arena_core::entities::task_results;
    let rig = setup().await;
    let task = tasks::Entity::find_by_id(rig.task_a)
        .one(&rig.state.db)
        .await
        .unwrap()
        .unwrap();
    game_server::health::award_health_bonus(
        &rig.state,
        rig.session_id,
        rig.player_id,
        &task,
        &rig.join_code,
        &arena_core::health_settings::HealthSettings::load(&rig.state.db)
            .await
            .unwrap(),
    )
    .await;
    let row = task_results::Entity::find()
        .filter(task_results::Column::Kind.eq(task_results::KIND_HEALTH_BONUS))
        .one(&rig.state.db)
        .await
        .unwrap()
        .expect("a zero row with the reason");
    assert_eq!(row.point_delta, 0);
    assert_eq!(row.answer, "health-bonus: no verified checkpoint → 0/20");
    assert!(
        rig.events
            .0
            .lock()
            .unwrap()
            .iter()
            .all(|e| !matches!(e, ZmqEvent::ScoreChange { .. }))
    );
}

/// Wait for the health-bonus row of `task`.
async fn bonus_row_of(rig: &Rig, task: Uuid) -> arena_core::entities::task_results::Model {
    use arena_core::entities::task_results;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    loop {
        let found = task_results::Entity::find()
            .filter(task_results::Column::SessionIdFk.eq(rig.session_id))
            .filter(task_results::Column::TaskId.eq(task))
            .filter(task_results::Column::Kind.eq(task_results::KIND_HEALTH_BONUS))
            .one(&rig.state.db)
            .await
            .unwrap();
        if let Some(r) = found {
            return r;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "no health bonus row for {task}"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[tokio::test]
async fn a_personal_task_is_paid_for_not_making_the_code_worse() {
    let rig = setup().await;
    let session = sessions::Entity::find_by_id(rig.session_id)
        .one(&rig.state.db)
        .await
        .unwrap()
        .unwrap();
    arena_core::entities::personal_projects::ActiveModel {
        project_id_fk: Set(session.project_id_fk),
        spec: Set(serde_json::json!({})),
        created_at: Set(Utc::now()),
        updated_at: Set(Utc::now()),
    }
    .insert(&rig.state.db)
    .await
    .expect("mark personal");
    let thresholds = ololo_health::Thresholds::default();

    // The codebase the session starts from.
    rig.write("src/a.js", CODE);
    let root = rig.commit(&Kind::Session, None, "session start @ x", None);
    let start = score_of(&[("src/a.js", CODE)]).score;

    // The first report of the session scores the start as the baseline.
    rig.commit(&Kind::Start, Some(rig.task_a), "Task A", None);
    let probe_commit = rig.commit(
        &Kind::Probe,
        Some(rig.task_a),
        "#1",
        Some((Uuid::new_v4(), 1)),
    );
    let report = rig.report(Uuid::new_v4(), 1, rig.task_a, &probe_commit, start);
    game_server::health::on_report(
        rig.state.clone(),
        rig.session_id,
        rig.player_id,
        rig.join_code.clone(),
        report,
    )
    .await;
    let baseline = rig.checkpoint(&root).await;
    assert_eq!(baseline.kind, "baseline");
    assert_eq!(baseline.server_status, "ok");
    assert_eq!(baseline.server_score, start);
    assert_eq!(baseline.task_id_fk, None);

    // Task A pastes the code twice more: worse than it found it.
    rig.write("src/b.js", CODE);
    rig.write("src/c.js", CODE);
    let feat_a = rig.commit(&Kind::Feat, Some(rig.task_a), "Task A", None);
    let worse = score_of(&[("src/a.js", CODE), ("src/b.js", CODE), ("src/c.js", CODE)]).score;
    assert!(
        worse < start,
        "duplication must cost: {worse:?} vs {start:?}"
    );
    let task_a = tasks::Entity::find_by_id(rig.task_a)
        .one(&rig.state.db)
        .await
        .unwrap()
        .unwrap();
    game_server::health::on_task_closed(
        rig.state.clone(),
        rig.session_id,
        rig.player_id,
        rig.join_code.clone(),
        task_a,
    )
    .await;
    assert_eq!(rig.checkpoint(&feat_a).await.server_score, worse);
    let paid_a = bonus_row_of(&rig, rig.task_a).await;
    let expected_a = ololo_health::delta_bonus(start, worse, &thresholds, 20);
    assert!(expected_a.points < 20, "{expected_a:?}");
    assert_eq!(paid_a.point_delta, expected_a.points);
    assert!(
        paid_a.answer.contains("at the session start"),
        "{}",
        paid_a.answer
    );

    // Task B leaves the tree as task A left it: held the line, paid in
    // full — measured from the end of task A, not from the session start.
    let feat_b = rig.commit(&Kind::Feat, Some(rig.task_b), "Task B", None);
    let task_b = tasks::Entity::find_by_id(rig.task_b)
        .one(&rig.state.db)
        .await
        .unwrap()
        .unwrap();
    game_server::health::on_task_closed(
        rig.state.clone(),
        rig.session_id,
        rig.player_id,
        rig.join_code.clone(),
        task_b,
    )
    .await;
    assert_eq!(rig.checkpoint(&feat_b).await.server_score, worse);
    let paid_b = bonus_row_of(&rig, rig.task_b).await;
    assert_eq!(paid_b.point_delta, 20, "{}", paid_b.answer);
    assert!(
        paid_b.answer.contains("at the end of task 1"),
        "{}",
        paid_b.answer
    );
}

// ── the project's own tests ──────────────────────────────────────────

async fn enable_tests(rig: &Rig) {
    app_settings::ActiveModel {
        key: Set(arena_core::health_settings::HEALTH_TESTS_ENABLED_KEY.to_string()),
        value: Set("true".to_string()),
    }
    .insert(&rig.state.db)
    .await
    .expect("enable the suite");
}

/// The frames the server queues for the player's CLI.
fn agent_inbox(rig: &Rig) -> tokio::sync::mpsc::Receiver<arena_core::protocol::PlayerAgentFrame> {
    let (tx, rx) = tokio::sync::mpsc::channel(16);
    rig.state.player_agent_registry.insert(rig.player_id, tx);
    rx
}

fn tests_report(
    probe_id: Uuid,
    seq: u32,
    commit: &str,
    passed: u64,
    failed: u64,
    coverage: Option<f64>,
) -> arena_core::protocol::TestReportPayload {
    use arena_core::protocol::{SuiteResult, TestCounts, TestRunStatus};
    arena_core::protocol::TestReportPayload {
        probe_id,
        probe_seq: seq,
        task_id: None,
        commit: commit.to_string(),
        status: TestRunStatus::Ok,
        command: "npm run coverage".into(),
        coverage_run: true,
        result: Some(SuiteResult {
            exit_code: Some(i32::from(failed > 0)),
            counts: Some(TestCounts {
                passed,
                failed,
                skipped: 0,
            }),
            coverage_pct: coverage,
            coverage_source: coverage.map(|_| "coverage/lcov.info".to_string()),
            summary: vec![],
        }),
        error: None,
        duration_ms: 3_000,
        log: Some(format!(".ololo/probes/{seq:04}-tests.log")),
    }
}

#[tokio::test]
async fn the_docs_name_the_test_commands_and_the_cli_is_told() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let rig = setup().await;
    let mut inbox = agent_inbox(&rig);
    rig.write(
        "AGENTS.md",
        "# Working here\n\nRun the tests with `npm test`; `npm run coverage` measures coverage.\n",
    );
    rig.commit(&Kind::Session, None, "session start @ x", None);
    let calls = Arc::new(AtomicUsize::new(0));
    let model = |calls: Arc<AtomicUsize>, answer: &'static str| {
        move |system: String, user: String| {
            let calls = calls.clone();
            async move {
                calls.fetch_add(1, Ordering::SeqCst);
                assert!(system.contains("untrusted content"));
                assert!(user.contains("=== AGENTS.md ==="), "{user}");
                Ok::<_, String>(answer.to_string())
            }
        }
    };

    // Off unless the operator turns the suite on.
    game_server::health_tests::refresh_with(
        &rig.state,
        rig.session_id,
        rig.player_id,
        model(calls.clone(), "{}"),
    )
    .await;
    assert_eq!(calls.load(Ordering::SeqCst), 0);

    enable_tests(&rig).await;
    game_server::health_tests::refresh_with(
        &rig.state,
        rig.session_id,
        rig.player_id,
        model(
            calls.clone(),
            r#"{"test": "npm test", "coverage": "npm run coverage"}"#,
        ),
    )
    .await;
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let row = arena_core::entities::player_test_commands::Entity::find()
        .one(&rig.state.db)
        .await
        .unwrap()
        .expect("stored");
    assert_eq!(row.test_command.as_deref(), Some("npm test"));
    assert_eq!(row.coverage_command.as_deref(), Some("npm run coverage"));
    assert_eq!(row.source_list(), vec!["AGENTS.md".to_string()]);
    match inbox.try_recv() {
        Ok(arena_core::protocol::PlayerAgentFrame::HealthTests(cfg)) => {
            assert_eq!(cfg.command(), Some(("npm run coverage", true)));
            assert_eq!(cfg.timeout_secs, 300);
        }
        other => panic!("expected the commands, got {other:?}"),
    }

    // The same docs are not read twice.
    game_server::health_tests::refresh_with(
        &rig.state,
        rig.session_id,
        rig.player_id,
        model(calls.clone(), "{}"),
    )
    .await;
    assert_eq!(calls.load(Ordering::SeqCst), 1);

    // Changed docs are read again (a minute after the last read) and the
    // CLI hears the change.
    rig.write("AGENTS.md", "Tests: `cargo test`.\n");
    rig.commit(&Kind::Memory, None, "sources @ x", None);
    let mut am: arena_core::entities::player_test_commands::ActiveModel = row.into();
    am.updated_at = Set(Utc::now() - chrono::Duration::minutes(2));
    am.update(&rig.state.db).await.unwrap();
    game_server::health_tests::refresh_with(
        &rig.state,
        rig.session_id,
        rig.player_id,
        model(calls.clone(), r#"{"test": "cargo test", "coverage": null}"#),
    )
    .await;
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    match inbox.try_recv() {
        Ok(arena_core::protocol::PlayerAgentFrame::HealthTests(cfg)) => {
            assert_eq!(cfg.command(), Some(("cargo test", false)));
        }
        other => panic!("expected the new commands, got {other:?}"),
    }

    // A reconnecting CLI is told again what was read.
    game_server::health_tests::on_connect(rig.state.clone(), rig.session_id, rig.player_id).await;
    match inbox.try_recv() {
        Ok(arena_core::protocol::PlayerAgentFrame::HealthTests(cfg)) => {
            assert_eq!(cfg.test.as_deref(), Some("cargo test"));
        }
        other => panic!("expected the commands on connect, got {other:?}"),
    }
}

#[tokio::test]
async fn a_test_report_lands_on_its_checkpoint_and_rescores_it() {
    let rig = setup().await;
    enable_tests(&rig).await;
    rig.write("src/a.js", CODE);
    rig.commit(&Kind::Session, None, "session start @ x", None);
    rig.commit(&Kind::Start, Some(rig.task_a), "Task A", None);
    let probe = Uuid::new_v4();
    let sha = rig.commit(
        &Kind::Probe,
        Some(rig.task_a),
        "#1 Task A",
        Some((probe, 1)),
    );
    let tree = score_of(&[("src/a.js", CODE)]);
    game_server::health::on_report(
        rig.state.clone(),
        rig.session_id,
        rig.player_id,
        rig.join_code.clone(),
        rig.report(probe, 1, rig.task_a, &sha, tree.score),
    )
    .await;
    let verified = rig.checkpoint(&sha).await;
    assert_eq!(verified.server_score, tree.score);
    let before = rig.health_events_at_least(2).await.len();

    game_server::health_tests::on_test_report(
        rig.state.clone(),
        rig.session_id,
        rig.player_id,
        rig.join_code.clone(),
        tests_report(probe, 1, &sha, 9, 1, Some(81.0)),
    )
    .await;
    let row = health_checkpoints::Entity::find_by_id(verified.id)
        .one(&rig.state.db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.tests_status.as_deref(), Some("ok"));
    assert_eq!(row.server_score, tree.score, "the tree's own score stays");

    let events = rig.health_events_at_least(before + 1).await;
    let ZmqEvent::HealthUpdated { checkpoint, .. } = events.last().unwrap() else {
        unreachable!()
    };
    let tests = checkpoint.tests.as_ref().expect("the suite's part");
    assert_eq!(tests.tests_score, Some(50.0), "one test in ten failing");
    assert_eq!(tests.coverage_score, Some(71.9), "81% covered");
    assert!(
        checkpoint.score.unwrap() < tree.score.unwrap(),
        "a failing test costs: {:?} vs {:?}",
        checkpoint.score,
        tree.score
    );
    let counted = tests.counted.as_ref().unwrap();
    assert_eq!(counted.log.as_deref(), Some(".ololo/probes/0001-tests.log"));
}

#[tokio::test]
async fn the_bonus_is_paid_on_the_tree_with_its_tests() {
    let rig = setup().await;
    enable_tests(&rig).await;
    rig.write("src/a.js", CODE);
    rig.commit(&Kind::Session, None, "session start @ x", None);
    rig.commit(&Kind::Start, Some(rig.task_a), "Task A", None);
    let probe = Uuid::new_v4();
    let sha = rig.commit(
        &Kind::Probe,
        Some(rig.task_a),
        "#1 Task A",
        Some((probe, 1)),
    );
    let tree = score_of(&[("src/a.js", CODE)]);
    game_server::health::on_report(
        rig.state.clone(),
        rig.session_id,
        rig.player_id,
        rig.join_code.clone(),
        rig.report(probe, 1, rig.task_a, &sha, tree.score),
    )
    .await;
    rig.checkpoint(&sha).await;
    // Every test failing, nothing covered: the suite is red.
    game_server::health_tests::on_test_report(
        rig.state.clone(),
        rig.session_id,
        rig.player_id,
        rig.join_code.clone(),
        tests_report(probe, 1, &sha, 0, 6, Some(0.0)),
    )
    .await;
    let feat = rig.commit(&Kind::Feat, Some(rig.task_a), "Task A", None);
    let task = tasks::Entity::find_by_id(rig.task_a)
        .one(&rig.state.db)
        .await
        .unwrap()
        .unwrap();
    game_server::health::on_task_closed(
        rig.state.clone(),
        rig.session_id,
        rig.player_id,
        rig.join_code.clone(),
        task,
    )
    .await;
    assert_eq!(rig.checkpoint(&feat).await.server_score, tree.score);
    let paid = bonus_row_of(&rig, rig.task_a).await;
    let tree_alone =
        ololo_health::bonus(tree.score, &ololo_health::Thresholds::default(), 20).points;
    assert!(tree_alone > 0, "the tree alone would earn: {tree:?}");
    assert_eq!(
        paid.point_delta, 0,
        "a red suite sinks the grade: {}",
        paid.answer
    );
    assert!(
        paid.answer.contains("(with tests 0, coverage 18)"),
        "{}",
        paid.answer
    );
}

/// A repository with no README or AGENTS.md still names its test script:
/// the manifest is read (its scripts, not its versions).
#[tokio::test]
async fn a_repository_without_docs_is_read_through_its_manifest() {
    let rig = setup().await;
    enable_tests(&rig).await;
    let mut inbox = agent_inbox(&rig);
    rig.write(
        "package.json",
        r#"{"name":"money","version":"0.1.0","scripts":{"test":"node --test test/","start":"node scripts/serve.js"},"devDependencies":{"jsdom":"^25.0.0"}}"#,
    );
    rig.write("test/a.test.js", "import test from 'node:test';\n");
    rig.commit(&Kind::Session, None, "session start @ x", None);
    game_server::health_tests::refresh_with(
        &rig.state,
        rig.session_id,
        rig.player_id,
        |_system: String, user: String| async move {
            assert!(user.contains("=== package.json ==="), "{user}");
            assert!(
                user.contains("node --test test/"),
                "the scripts travel: {user}"
            );
            assert!(!user.contains("^25.0.0"), "versions do not: {user}");
            Ok::<_, String>(r#"{"test": "npm test", "coverage": null}"#.to_string())
        },
    )
    .await;
    let row = arena_core::entities::player_test_commands::Entity::find()
        .one(&rig.state.db)
        .await
        .unwrap()
        .expect("stored");
    assert_eq!(row.test_command.as_deref(), Some("npm test"));
    assert_eq!(row.source_list(), vec!["package.json".to_string()]);
    match inbox.try_recv() {
        Ok(arena_core::protocol::PlayerAgentFrame::HealthTests(cfg)) => {
            assert_eq!(cfg.command(), Some(("npm test", false)));
            assert_eq!(cfg.sources, vec!["package.json".to_string()]);
        }
        other => panic!("expected the commands, got {other:?}"),
    }
}
