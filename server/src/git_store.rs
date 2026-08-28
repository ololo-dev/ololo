//! Server-side bare git repo store for per-player remote repos.
//!
//! Layout: `{OLOLO_GIT_REPOS_DIR}/{session_id}/{player_id}.git`
//!
//! Provisioned at join time (idempotent — re-join reuses the existing repo).
//! Served over HTTP via `git http-backend` CGI (see `ws::git_http` route in
//! lib.rs) authenticated by the player's PAT.
//!
//! `OLOLO_GIT_REPOS_DIR` defaults to `~/.local/share/ololo/repos` on Unix
//! and `%LOCALAPPDATA%\ololo\repos` on Windows. When unset and home is
//! unresolvable, all provisioning calls return `Ok(None)` (feature disabled
//! best-effort — join still succeeds, just no remote URL).

pub use arena_core::git_store::{player_repo_path, repos_base_dir};

use std::path::{Path, PathBuf};
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum GitStoreError {
    #[error("git binary not found on PATH: {0}")]
    GitNotFound(String),
    #[error("git init failed (exit {code}): {stderr}")]
    GitInitFailed { code: i32, stderr: String },
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("repos dir not configurable (OLOLO_GIT_REPOS_DIR unset, home unresolvable)")]
    NoBaseDir,
}

/// Provision a bare repo for the player if it doesn't already exist.
/// Idempotent: returns `Ok(())` if the repo already exists.
pub fn provision_player_repo(
    base: &Path,
    session_id: Uuid,
    player_id: Uuid,
) -> Result<PathBuf, GitStoreError> {
    let repo = player_repo_path(base, session_id, player_id);
    if repo.join("HEAD").exists() {
        return Ok(repo);
    }
    std::fs::create_dir_all(repo.parent().ok_or(GitStoreError::NoBaseDir)?)?;

    let git_bin = which::which("git").map_err(|e| GitStoreError::GitNotFound(e.to_string()))?;
    let out = std::process::Command::new(git_bin)
        .arg("init")
        .arg("--bare")
        .arg("--initial-branch=main")
        .arg(&repo)
        .output()?;
    if !out.status.success() {
        return Err(GitStoreError::GitInitFailed {
            code: out.status.code().unwrap_or(-1),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        });
    }
    Ok(repo)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn player_repo_path_layout() {
        let base = Path::new("/tmp/arena-repos");
        let sid = Uuid::nil();
        let pid = Uuid::nil();
        let p = player_repo_path(base, sid, pid);
        assert!(p.starts_with("/tmp/arena-repos"));
        assert!(p.to_string_lossy().ends_with(".git"));
    }

    #[test]
    fn provision_creates_bare_repo() {
        let tmp = tempfile::tempdir().expect("tmp");
        let base = tmp.path().join("repos");
        let sid = Uuid::new_v4();
        let pid = Uuid::new_v4();
        let repo = provision_player_repo(&base, sid, pid).expect("provision");
        assert!(repo.join("HEAD").exists(), "bare repo HEAD exists");
        assert!(repo.join("objects").exists(), "objects dir exists");
        assert!(repo.join("refs").exists(), "refs dir exists");
    }

    #[test]
    fn provision_is_idempotent() {
        let tmp = tempfile::tempdir().expect("tmp");
        let base = tmp.path().join("repos");
        let sid = Uuid::new_v4();
        let pid = Uuid::new_v4();
        let first = provision_player_repo(&base, sid, pid).expect("first");
        let second = provision_player_repo(&base, sid, pid).expect("second");
        assert_eq!(first, second, "idempotent returns same path");
    }
}
