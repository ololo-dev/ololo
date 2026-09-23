//! Code-health checkpoints: what the game server and the web server share
//! beyond the wire types — the string values the `health_checkpoints` rows
//! carry, the view the browser gets built from a row, and the git reads
//! the verification needs (a first-parent log to attribute commits to
//! tasks, an exact export of a commit's tree).

use std::path::Path;

use chrono::{DateTime, Utc};
use ololo_health::{Level, Thresholds, level};

use crate::entities::health_checkpoints;
use crate::protocol::{
    HealthCheckStatus, HealthCheckpointKind, HealthCheckpointView, HealthFlags, HealthReportStatus,
    HealthSide, Metrics,
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

/// The browser's view of a checkpoint row.
pub fn checkpoint_view(
    row: &health_checkpoints::Model,
    task_title: Option<String>,
    session_started_at: Option<DateTime<Utc>>,
    thresholds: &Thresholds,
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
    // The server's number once verified, the client's until then.
    let score = if server_status.is_verified() {
        row.server_score
    } else {
        row.client_score
    };
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
}
