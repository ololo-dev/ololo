//! Code-health checkpoints: what the game server and the web server share
//! beyond the wire types — the string values the `health_checkpoints` rows
//! carry, the view the browser gets built from a row, and the git reads
//! the verification needs (a first-parent log to attribute commits to
//! tasks, an exact export of a commit's tree).

use std::path::Path;

use chrono::{DateTime, Utc};
use ololo_health::compose::{Composed, compose, static_dimensions};
use ololo_health::suite::{COVERAGE_ID, TESTS_ID, metrics};
use ololo_health::{Level, Thresholds, level};
use sea_orm::{ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, QueryOrder};

use crate::entities::{health_checkpoints, player_test_commands};
use crate::protocol::{
    HealthCheckStatus, HealthCheckpointKind, HealthCheckpointView, HealthFlags, HealthReportStatus,
    HealthSide, HealthTestsView, Metrics, PlayerTestCommandsView, TestAttemptView,
    TestReportPayload, TestRunStatus, TestRunView,
};
use crate::snapshot_message::LogEntry;

impl HealthCheckpointKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            HealthCheckpointKind::Probe => "probe",
            HealthCheckpointKind::TaskFinal => "task_final",
            HealthCheckpointKind::Baseline => "baseline",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "probe" => Some(Self::Probe),
            "task_final" => Some(Self::TaskFinal),
            "baseline" => Some(Self::Baseline),
            _ => None,
        }
    }
}

impl HealthCheckStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            HealthCheckStatus::Pending => "pending",
            HealthCheckStatus::Ok => "ok",
            HealthCheckStatus::Failed => "failed",
            HealthCheckStatus::Timeout => "timeout",
            HealthCheckStatus::CommitMissing => "commit_missing",
            HealthCheckStatus::Unverified => "unverified",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "pending" => Some(Self::Pending),
            "ok" => Some(Self::Ok),
            "failed" => Some(Self::Failed),
            "timeout" => Some(Self::Timeout),
            "commit_missing" => Some(Self::CommitMissing),
            "unverified" => Some(Self::Unverified),
            _ => None,
        }
    }

    /// The server has scored the commit.
    pub fn is_verified(&self) -> bool {
        matches!(self, HealthCheckStatus::Ok)
    }

    /// Nothing more will happen to this checkpoint.
    pub fn is_terminal(&self) -> bool {
        !matches!(self, HealthCheckStatus::Pending)
    }
}

impl HealthReportStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            HealthReportStatus::Ok => "ok",
            HealthReportStatus::Failed => "failed",
            HealthReportStatus::Timeout => "timeout",
            HealthReportStatus::Skipped => "skipped",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "ok" => Some(Self::Ok),
            "failed" => Some(Self::Failed),
            "timeout" => Some(Self::Timeout),
            "skipped" => Some(Self::Skipped),
            _ => None,
        }
    }
}

/// The largest serialized result a checkpoint row keeps per side.
pub const MAX_RESULT_BYTES: usize = 16 * 1024;

/// `result` as stored: the full `HealthResult` when it fits, otherwise
/// only what the view reads (score, grade, metrics) with a note.
pub fn cap_result(result: &ololo_health::HealthResult) -> serde_json::Value {
    let full = serde_json::to_value(result).unwrap_or(serde_json::Value::Null);
    if serde_json::to_vec(&full).map(|b| b.len()).unwrap_or(0) <= MAX_RESULT_BYTES {
        return full;
    }
    serde_json::json!({
        "schema": result.schema,
        "jscpd_version": result.jscpd_version,
        "score": result.score,
        "grade": result.grade,
        "level": result.level,
        "metrics": result.metrics,
        "duration_ms": result.duration_ms,
        "truncated": true,
    })
}

fn metrics_of(result: Option<&serde_json::Value>) -> Option<Metrics> {
    result
        .and_then(|v| v.get("metrics"))
        .and_then(|m| serde_json::from_value(m.clone()).ok())
}

fn grade_of(grade: Option<&str>) -> Option<char> {
    grade.and_then(|g| g.chars().next())
}

/// Seconds since the session started, the chart's x.
pub fn elapsed_secs(started_at: Option<DateTime<Utc>>, at: DateTime<Utc>) -> Option<f64> {
    // Clamped at 0 like the score history: a commit stamped a moment before
    // the session's `started_at` (clock skew, the start marker racing the
    // status flip) belongs to the session's first second, not before it.
    started_at.map(|s| ((at - s).num_milliseconds() as f64 / 1000.0).max(0.0))
}

// ────────────────────────────── the test suite ──────────────────────────────

/// A completed run of the project's tests, as a checkpoint counts it.
#[derive(Debug, Clone, PartialEq)]
pub struct CountedRun {
    pub report: TestReportPayload,
    /// The run belongs to an earlier checkpoint.
    pub inherited: bool,
}

/// The test report stored on a checkpoint row, if any.
pub fn test_report_of(row: &health_checkpoints::Model) -> Option<TestReportPayload> {
    row.tests_result
        .as_ref()
        .and_then(|v| serde_json::from_value(v.clone()).ok())
}

/// A run whose numbers count: it ran to its end and measured something.
fn completed(report: &TestReportPayload) -> bool {
    report.status == TestRunStatus::Ok && report.result.is_some()
}

/// What each of one player's checkpoints (oldest first) counts: its own
/// completed run, else the last completed run before it. Runs happen after
/// a probe's analysis and only when the code changed since the last one,
/// so "the last run before" is the suite's verdict on the code as it then
/// stood.
pub fn counted_runs(rows: &[&health_checkpoints::Model]) -> Vec<Option<CountedRun>> {
    let mut last: Option<TestReportPayload> = None;
    rows.iter()
        .map(|row| match test_report_of(row).filter(completed) {
            Some(own) => {
                last = Some(own.clone());
                Some(CountedRun {
                    report: own,
                    inherited: false,
                })
            }
            None => last.clone().map(|report| CountedRun {
                report,
                inherited: true,
            }),
        })
        .collect()
}

/// The run a checkpoint made at `at` counts: the player's last completed
/// run at or before it. `own_commit` marks the checkpoint's own run as not
/// inherited.
pub async fn counted_run_at<C: ConnectionTrait>(
    db: &C,
    session_id: uuid::Uuid,
    player_id: uuid::Uuid,
    at: DateTime<Utc>,
    own_commit: &str,
) -> Option<CountedRun> {
    let row = health_checkpoints::Entity::find()
        .filter(health_checkpoints::Column::SessionIdFk.eq(session_id))
        .filter(health_checkpoints::Column::PlayerIdFk.eq(player_id))
        .filter(health_checkpoints::Column::TestsStatus.eq(TestRunStatus::Ok.as_str()))
        .filter(health_checkpoints::Column::CreatedAt.lte(at))
        .order_by_desc(health_checkpoints::Column::CreatedAt)
        .one(db)
        .await
        .ok()
        .flatten()?;
    let report = test_report_of(&row).filter(completed)?;
    Some(CountedRun {
        inherited: row.commit_sha != own_commit,
        report,
    })
}

/// The side whose numbers a checkpoint shows: the server's once verified,
/// the client's until then.
fn shown_side(
    row: &health_checkpoints::Model,
) -> (Option<f64>, Option<char>, Option<&serde_json::Value>) {
    let verified = HealthCheckStatus::parse(&row.server_status).is_some_and(|s| s.is_verified());
    if verified {
        (
            row.server_score,
            grade_of(row.server_grade.as_deref()),
            row.server_result.as_ref(),
        )
    } else {
        (
            row.client_score,
            grade_of(row.client_grade.as_deref()),
            row.client_result.as_ref(),
        )
    }
}

/// The checkpoint's tree scored together with the run it counts. Without
/// a run — or without dimensions to compose with — it is the tree's own
/// score.
pub fn composed_score(row: &health_checkpoints::Model, counted: Option<&CountedRun>) -> Composed {
    let (score, grade, result) = shown_side(row);
    let dims = result.and_then(static_dimensions);
    let extra = counted
        .and_then(|c| c.report.result.as_ref())
        .map(metrics)
        .unwrap_or_default();
    match dims {
        Some(dims) => compose(Some(&dims), &extra),
        None => Composed {
            score,
            grade,
            dimensions: Vec::new(),
        },
    }
}

/// The test suite's part of a checkpoint's view.
fn tests_view(
    row: &health_checkpoints::Model,
    counted: Option<&CountedRun>,
    composed: &Composed,
) -> Option<HealthTestsView> {
    let attempt = test_report_of(row)
        .filter(|r| !completed(r))
        .map(|r| TestAttemptView {
            status: r.status,
            command: r.command,
            error: r.error,
            duration_ms: r.duration_ms,
        });
    let counted = counted.and_then(|c| {
        Some(TestRunView {
            command: c.report.command.clone(),
            coverage_run: c.report.coverage_run,
            result: c.report.result.clone()?,
            duration_ms: c.report.duration_ms,
            log: c.report.log.clone(),
            probe_seq: c.report.probe_seq,
            inherited: c.inherited,
        })
    });
    (attempt.is_some() || counted.is_some()).then(|| HealthTestsView {
        counted,
        attempt,
        tests_score: composed.sub_score(TESTS_ID),
        coverage_score: composed.sub_score(COVERAGE_ID),
    })
}

/// A player's test commands, for the dashboards.
pub fn test_commands_view(row: &player_test_commands::Model) -> PlayerTestCommandsView {
    PlayerTestCommandsView {
        test: row.test_command.clone(),
        coverage: row.coverage_command.clone(),
        sources: row.source_list(),
        updated_at: row.updated_at,
    }
}

/// The browser's view of a checkpoint row. `counted` is the test run the
/// checkpoint counts (see [`counted_runs`] / [`counted_run_at`]).
pub fn checkpoint_view(
    row: &health_checkpoints::Model,
    task_title: Option<String>,
    session_started_at: Option<DateTime<Utc>>,
    thresholds: &Thresholds,
    counted: Option<&CountedRun>,
) -> HealthCheckpointView {
    let server_status =
        HealthCheckStatus::parse(&row.server_status).unwrap_or(HealthCheckStatus::Pending);
    let client = row.client_status.as_deref().map(|status| HealthSide {
        status: HealthReportStatus::parse(status).unwrap_or(HealthReportStatus::Failed),
        score: row.client_score,
        grade: grade_of(row.client_grade.as_deref()),
        level: level(row.client_score, thresholds),
        jscpd_version: row.client_jscpd_version.clone().unwrap_or_default(),
        duration_ms: row.client_duration_ms.unwrap_or(0).max(0) as u64,
        metrics: metrics_of(row.client_result.as_ref()),
        error: row.client_error.clone(),
    });
    let server = server_status.is_terminal().then(|| HealthSide {
        status: match server_status {
            HealthCheckStatus::Ok => HealthReportStatus::Ok,
            HealthCheckStatus::Timeout => HealthReportStatus::Timeout,
            _ => HealthReportStatus::Failed,
        },
        score: row.server_score,
        grade: grade_of(row.server_grade.as_deref()),
        level: level(row.server_score, thresholds),
        jscpd_version: row.server_jscpd_version.clone().unwrap_or_default(),
        duration_ms: row.server_duration_ms.unwrap_or(0).max(0) as u64,
        metrics: metrics_of(row.server_result.as_ref()),
        error: row.server_error.clone(),
    });
    // The server's number once verified, the client's until then — with
    // the test run it counts composed in.
    let composed = composed_score(row, counted);
    let tests = tests_view(row, counted, &composed);
    let score = composed.score;
    HealthCheckpointView {
        id: row.id,
        kind: HealthCheckpointKind::parse(&row.kind).unwrap_or(HealthCheckpointKind::Probe),
        probe_id: row.probe_id_fk,
        probe_seq: u32::try_from(row.probe_seq).unwrap_or(0),
        task_id: row.task_id_fk,
        task_title,
        commit: row.commit_sha.clone(),
        created_at: row.created_at,
        t: elapsed_secs(session_started_at, row.created_at),
        client,
        server,
        server_status,
        flags: HealthFlags {
            score_mismatch: row.score_mismatch,
            version_mismatch: row.version_mismatch,
            task_mismatch: row.task_mismatch,
            late: row.late,
            history_rewritten: row.history_rewritten,
        },
        level: if score.is_some() {
            level(score, thresholds)
        } else {
            Level::Unknown
        },
        score,
        grade: composed.grade,
        tests,
    }
}

// ────────────────────────────── git reads ──────────────────────────────

fn git() -> Result<std::path::PathBuf, String> {
    which::which("git").map_err(|e| format!("git not found: {e}"))
}

/// Whether `sha` names a commit the repo has.
pub async fn commit_exists(repo_dir: &Path, sha: &str) -> Result<bool, String> {
    let repo_dir = repo_dir.to_path_buf();
    let spec = format!("{sha}^{{commit}}");
    tokio::task::spawn_blocking(move || {
        let out = std::process::Command::new(git()?)
            .arg("-C")
            .arg(&repo_dir)
            .args(["cat-file", "-e", &spec])
            .output()
            .map_err(|e| e.to_string())?;
        Ok(out.status.success())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Whether `sha` is on the first-parent line of `main`.
pub async fn on_main_line(repo_dir: &Path, sha: &str) -> Result<bool, String> {
    let repo_dir = repo_dir.to_path_buf();
    let sha = sha.to_string();
    tokio::task::spawn_blocking(move || {
        let out = std::process::Command::new(git()?)
            .arg("-C")
            .arg(&repo_dir)
            .args(["merge-base", "--is-ancestor", &sha, "refs/heads/main"])
            .output()
            .map_err(|e| e.to_string())?;
        Ok(out.status.success())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// The first-parent log of `main`, oldest first, capped at `limit` commits
/// from the head — what `snapshot_message::task_ranges` reads.
pub async fn first_parent_log(repo_dir: &Path, limit: usize) -> Result<Vec<LogEntry>, String> {
    let repo_dir = repo_dir.to_path_buf();
    tokio::task::spawn_blocking(move || {
        let out = std::process::Command::new(git()?)
            .arg("-C")
            .arg(&repo_dir)
            .args([
                "log",
                "--first-parent",
                &format!("-n{limit}"),
                "--format=%H%x1f%ct%x1f%B%x1e",
                "refs/heads/main",
            ])
            .output()
            .map_err(|e| e.to_string())?;
        if !out.status.success() {
            let stderr = String::from_utf8_lossy(&out.stderr);
            // No commits yet reads as an empty log, not an error.
            if stderr.contains("unknown revision") || stderr.contains("bad revision") {
                return Ok(Vec::new());
            }
            return Err(format!("git log: {}", stderr.trim()));
        }
        let text = String::from_utf8_lossy(&out.stdout);
        let mut entries: Vec<LogEntry> = text
            .split('\x1e')
            .filter_map(|record| {
                let record = record.trim_start_matches('\n');
                let mut parts = record.splitn(3, '\x1f');
                let sha = parts.next()?.trim();
                if sha.is_empty() {
                    return None;
                }
                let secs = parts.next()?.trim().parse::<i64>().ok();
                let message = parts.next().unwrap_or("").trim_end().to_string();
                Some(LogEntry {
                    sha: sha.to_string(),
                    message,
                    committed_at: secs.and_then(|s| DateTime::from_timestamp(s, 0)),
                })
            })
            .collect();
        entries.reverse();
        Ok(entries)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Write the tree of `sha` under `dest` exactly as committed: `read-tree`
/// into a throwaway index and `checkout-index`, which — unlike `git
/// archive` — ignores a `.gitattributes export-ignore` the player may have
/// committed to hide files from the scan. Touches no ref of the repo.
pub async fn export_commit(repo_dir: &Path, sha: &str, dest: &Path) -> Result<(), String> {
    let repo_dir = repo_dir.to_path_buf();
    let dest = dest.to_path_buf();
    let sha = sha.to_string();
    tokio::task::spawn_blocking(move || {
        let git = git()?;
        let index = dest.join(".ololo-export-index");
        let run = |args: &[&str]| -> Result<(), String> {
            let out = std::process::Command::new(&git)
                .arg("--git-dir")
                .arg(&repo_dir)
                .arg("--work-tree")
                .arg(&dest)
                .env("GIT_INDEX_FILE", &index)
                .args(args)
                .output()
                .map_err(|e| e.to_string())?;
            if out.status.success() {
                Ok(())
            } else {
                Err(format!(
                    "git {}: {}",
                    args.first().copied().unwrap_or(""),
                    String::from_utf8_lossy(&out.stderr).trim()
                ))
            }
        };
        run(&["read-tree", &sha])?;
        run(&["checkout-index", "-a", "-f"])?;
        let _ = std::fs::remove_file(&index);
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;

    fn git_in(dir: &Path, args: &[&str]) -> String {
        let out = std::process::Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@t")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@t")
            .output()
            .expect("git");
        assert!(
            out.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    #[tokio::test]
    async fn export_commit_ignores_export_ignore_and_reads_the_log() {
        let tmp = tempfile::tempdir().unwrap();
        let work = tmp.path().join("work");
        std::fs::create_dir_all(&work).unwrap();
        git_in(&work, &["init", "-q", "-b", "main"]);
        std::fs::write(work.join("keep.js"), "x\n").unwrap();
        std::fs::write(work.join("hidden.js"), "y\n").unwrap();
        std::fs::write(work.join(".gitattributes"), "hidden.js export-ignore\n").unwrap();
        git_in(&work, &["add", "-A"]);
        git_in(
            &work,
            &["commit", "-q", "-m", "ololo snapshot: session start @ x"],
        );
        let task = uuid::Uuid::new_v4();
        git_in(
            &work,
            &[
                "commit",
                "-q",
                "--allow-empty",
                "-m",
                &format!("start({task}): T"),
            ],
        );
        let sha = git_in(&work, &["rev-parse", "HEAD"]);
        let repo_dir = work.join(".git");

        assert!(commit_exists(&repo_dir, &sha).await.unwrap());
        assert!(
            !commit_exists(&repo_dir, "0000000000000000000000000000000000000000")
                .await
                .unwrap()
        );
        assert!(on_main_line(&repo_dir, &sha).await.unwrap());

        let dest = tmp.path().join("out");
        std::fs::create_dir_all(&dest).unwrap();
        export_commit(&repo_dir, &sha, &dest).await.unwrap();
        assert!(dest.join("keep.js").is_file());
        assert!(
            dest.join("hidden.js").is_file(),
            "export-ignore must not hide files"
        );
        assert!(!dest.join(".ololo-export-index").exists());

        let log = first_parent_log(&repo_dir, 100).await.unwrap();
        assert_eq!(log.len(), 2);
        assert!(log[0].message.starts_with("ololo snapshot: session start"));
        assert_eq!(log[1].sha, sha);
        assert!(log[1].committed_at.is_some());
        let ranges = crate::snapshot_message::task_ranges(&log);
        assert_eq!(ranges.task_of(&sha), Some(task));
    }

    #[test]
    fn status_strings_round_trip() {
        for s in [
            HealthCheckStatus::Pending,
            HealthCheckStatus::Ok,
            HealthCheckStatus::Failed,
            HealthCheckStatus::Timeout,
            HealthCheckStatus::CommitMissing,
            HealthCheckStatus::Unverified,
        ] {
            assert_eq!(HealthCheckStatus::parse(s.as_str()), Some(s));
        }
        for s in [
            HealthCheckpointKind::Probe,
            HealthCheckpointKind::TaskFinal,
            HealthCheckpointKind::Baseline,
        ] {
            assert_eq!(HealthCheckpointKind::parse(s.as_str()), Some(s));
        }
        for s in [
            HealthReportStatus::Ok,
            HealthReportStatus::Failed,
            HealthReportStatus::Timeout,
            HealthReportStatus::Skipped,
        ] {
            assert_eq!(HealthReportStatus::parse(s.as_str()), Some(s));
        }
    }

    // ── the test suite in a checkpoint ───────────────────────────────

    fn tree_result(score: f64) -> serde_json::Value {
        serde_json::json!({
            "schema": 2, "jscpd_version": "0.1.17", "score": score, "grade": "B",
            "level": "green",
            "health": {"score": score, "grade": "B", "dimensions": [
                {"id": "duplication", "source": "jscpd", "weight": 1.0, "score": score},
                {"id": "complexity", "source": "jscpd", "weight": 1.0, "score": score}
            ]},
            "metrics": {"files": 2, "code_lines": 40, "clones": 0,
                        "ignore_markers": 0, "jscpd_config_present": false},
            "duration_ms": 5
        })
    }

    fn row(seq: i32, commit: &str, tests: Option<TestReportPayload>) -> health_checkpoints::Model {
        let now = Utc::now();
        health_checkpoints::Model {
            id: uuid::Uuid::new_v4(),
            session_id_fk: uuid::Uuid::nil(),
            player_id_fk: uuid::Uuid::nil(),
            task_id_fk: None,
            probe_id_fk: None,
            kind: "probe".into(),
            probe_seq: seq,
            commit_sha: commit.into(),
            derived_task_id: None,
            score_mismatch: false,
            version_mismatch: false,
            task_mismatch: false,
            late: false,
            history_rewritten: false,
            client_status: Some("ok".into()),
            client_score: Some(80.0),
            client_grade: Some("B".into()),
            client_result: Some(tree_result(80.0)),
            client_duration_ms: Some(5),
            client_jscpd_version: Some("0.1.17".into()),
            client_error: None,
            client_reported_at: Some(now),
            server_status: "ok".into(),
            server_score: Some(80.0),
            server_grade: Some("B".into()),
            server_result: Some(tree_result(80.0)),
            server_duration_ms: Some(5),
            server_jscpd_version: Some("0.1.17".into()),
            server_error: None,
            server_verified_at: Some(now),
            tests_status: tests.as_ref().map(|t| t.status.as_str().to_string()),
            tests_result: tests.map(|t| serde_json::to_value(t).unwrap()),
            tests_reported_at: None,
            created_at: now,
            updated_at: now,
        }
    }

    fn run(seq: u32, status: TestRunStatus, passed: u64, failed: u64) -> TestReportPayload {
        TestReportPayload {
            probe_id: uuid::Uuid::new_v4(),
            probe_seq: seq,
            task_id: None,
            commit: format!("c{seq}"),
            status,
            command: "npm test".into(),
            coverage_run: false,
            result: (status == TestRunStatus::Ok).then(|| crate::protocol::SuiteResult {
                exit_code: Some(i32::from(failed > 0)),
                counts: Some(crate::protocol::TestCounts {
                    passed,
                    failed,
                    skipped: 0,
                }),
                ..Default::default()
            }),
            error: (status != TestRunStatus::Ok).then(|| "timeout after 300s".to_string()),
            duration_ms: 1_000,
            log: Some(format!(".ololo/probes/{seq:04}-tests.log")),
        }
    }

    #[test]
    fn a_checkpoint_counts_its_own_run_else_the_last_one_before_it() {
        let rows = [
            row(1, "c1", None),
            row(2, "c2", Some(run(2, TestRunStatus::Ok, 9, 1))),
            row(3, "c3", None),
            row(4, "c4", Some(run(4, TestRunStatus::Timeout, 0, 0))),
            row(5, "c5", Some(run(5, TestRunStatus::Ok, 10, 0))),
        ];
        let refs: Vec<&health_checkpoints::Model> = rows.iter().collect();
        let counted = counted_runs(&refs);
        assert!(counted[0].is_none(), "nothing ran yet");
        let own = counted[1].as_ref().unwrap();
        assert_eq!((own.report.probe_seq, own.inherited), (2, false));
        let carried = counted[2].as_ref().unwrap();
        assert_eq!((carried.report.probe_seq, carried.inherited), (2, true));
        // A run that timed out counts the last completed one.
        let after_timeout = counted[3].as_ref().unwrap();
        assert_eq!(
            (after_timeout.report.probe_seq, after_timeout.inherited),
            (2, true)
        );
        assert_eq!(counted[4].as_ref().unwrap().report.probe_seq, 5);
    }

    #[test]
    fn the_view_composes_the_counted_run_into_the_score() {
        let t = Thresholds::default();
        let plain = row(1, "c1", None);
        let view = checkpoint_view(&plain, None, None, &t, None);
        assert_eq!(view.score, Some(80.0));
        assert!(view.tests.is_none(), "no suite, no tests part");

        let tested = row(2, "c2", Some(run(2, TestRunStatus::Ok, 9, 1)));
        let counted = CountedRun {
            report: test_report_of(&tested).unwrap(),
            inherited: false,
        };
        let view = checkpoint_view(&tested, None, None, &t, Some(&counted));
        let tests = view.tests.as_ref().unwrap();
        assert_eq!(tests.tests_score, Some(50.0), "one in ten failing");
        assert_eq!(tests.coverage_score, None, "coverage was not measured");
        // geometric mean of 80, 80 and 50
        assert_eq!(view.score, Some(68.4));
        assert_eq!(view.grade, Some('C'));
        assert_eq!(view.level, Level::Amber);
        assert_eq!(
            view.server.as_ref().unwrap().score,
            Some(80.0),
            "the tree's own score stays"
        );

        // A check whose own run timed out shows the attempt and counts the earlier run.
        let timed_out = row(3, "c3", Some(run(3, TestRunStatus::Timeout, 0, 0)));
        let inherited = CountedRun {
            inherited: true,
            ..counted
        };
        let view = checkpoint_view(&timed_out, None, None, &t, Some(&inherited));
        let tests = view.tests.unwrap();
        assert_eq!(
            tests.attempt.as_ref().unwrap().status,
            TestRunStatus::Timeout
        );
        assert!(tests.counted.as_ref().unwrap().inherited);
        assert_eq!(view.score, Some(68.4));
    }

    #[test]
    fn a_tree_with_nothing_to_score_stays_unscored_whatever_the_tests_say() {
        let mut empty = row(1, "c1", Some(run(1, TestRunStatus::Ok, 5, 0)));
        empty.server_score = None;
        empty.server_grade = None;
        empty.server_result = Some(serde_json::json!({"score": null, "metrics": {}}));
        let counted = CountedRun {
            report: test_report_of(&empty).unwrap(),
            inherited: false,
        };
        let view = checkpoint_view(&empty, None, None, &Thresholds::default(), Some(&counted));
        assert_eq!(view.score, None);
        assert_eq!(view.level, Level::Unknown);
    }
}
