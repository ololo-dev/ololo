//! Task-commit resolution via commit-message grep.
//!
//! Resolves the most recent `feat(<task_id>): <title>` snapshot commit —
//! the tree ololo commits when the task completes. Matching the bare
//! task_id is not enough: auxiliary commits (`wip(<task_id>)` checkpoints,
//! `artifact(<task_id>): sync`, `flag(<task_id>)`) also carry the id, and
//! a judge waiting for the *final* snapshot must not mistake one of those
//! mid-task trees for it.

use std::path::Path;
use uuid::Uuid;

use super::JudgeError;

/// Resolve the most recent `feat(<task_id>)` snapshot commit.
///
/// Runs `git log --all --fixed-strings --grep='feat(<task_id>)'` inside
/// `spawn_blocking`. Returns `Ok(Some((sha, subject)))` on a match, `Ok(None)`
/// when the task's snapshot commit has not landed yet.
pub async fn resolve_task_commit(
    repo_dir: &Path,
    task_id: Uuid,
) -> Result<Option<(String, String)>, JudgeError> {
    let repo_dir = repo_dir.to_path_buf();
    let task_id_str = task_id.to_string();
    tokio::task::spawn_blocking(move || {
        let git_bin = which::which("git").map_err(|e| JudgeError::GitReadError(e.to_string()))?;
        let out = std::process::Command::new(&git_bin)
            .arg("-C")
            .arg(&repo_dir)
            .arg("log")
            .arg("--all")
            .arg("--fixed-strings")
            .arg(format!("--grep=feat({task_id_str})"))
            .arg("--format=%H%n%s")
            .arg("-n")
            .arg("1")
            .output()
            .map_err(|e| JudgeError::GitReadError(format!("git log: {e}")))?;
        if !out.status.success() {
            return Ok(None);
        }
        let text = String::from_utf8_lossy(&out.stdout);
        let text = text.trim();
        if text.is_empty() {
            return Ok(None);
        }
        let mut lines = text.lines();
        let sha = lines.next().unwrap_or("").to_string();
        let subject = lines.next().unwrap_or("").to_string();
        if sha.is_empty() {
            return Ok(None);
        }
        Ok(Some((sha, subject)))
    })
    .await
    .map_err(|e| JudgeError::GitReadError(format!("spawn_blocking join: {e}")))?
}

/// Resolve the repo's current HEAD sha and its commit unix time.
///
/// Server-side probes measure the last *pushed* state; the commit time lets
/// the measurement record how stale that snapshot was (`snapshot_age_secs`
/// in `probes.result_json`). Returns `Ok(None)` for an empty repo.
pub async fn resolve_head(repo_dir: &Path) -> Result<Option<(String, i64)>, JudgeError> {
    let repo_dir = repo_dir.to_path_buf();
    tokio::task::spawn_blocking(move || {
        let git_bin = which::which("git").map_err(|e| JudgeError::GitReadError(e.to_string()))?;
        let out = std::process::Command::new(&git_bin)
            .arg("-C")
            .arg(&repo_dir)
            .arg("log")
            .arg("-1")
            .arg("--format=%H%n%ct")
            .output()
            .map_err(|e| JudgeError::GitReadError(format!("git log -1: {e}")))?;
        if !out.status.success() {
            return Ok(None);
        }
        let text = String::from_utf8_lossy(&out.stdout);
        let mut lines = text.trim().lines();
        let sha = lines.next().unwrap_or("").to_string();
        let unix_time = lines.next().and_then(|t| t.parse::<i64>().ok());
        match (sha.is_empty(), unix_time) {
            (false, Some(t)) => Ok(Some((sha, t))),
            _ => Ok(None),
        }
    })
    .await
    .map_err(|e| JudgeError::GitReadError(format!("spawn_blocking join: {e}")))?
}
