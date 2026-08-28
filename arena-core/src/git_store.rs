//! Pure path helpers for the per-player bare git repo store.
//!
//! Layout: `{OLOLO_GIT_REPOS_DIR}/{session_id}/{player_id}.git`
//!
//! `OLOLO_GIT_REPOS_DIR` defaults to `~/.local/share/ololo/repos` on Unix
//! and `%LOCALAPPDATA%\ololo\repos` on Windows. When unset and home is
//! unresolvable, `repos_base_dir` returns `None`.

use std::path::{Path, PathBuf};
use uuid::Uuid;

/// Resolve the base repos directory from env or fall back to a sensible default.
/// Returns `None` only when neither env nor home is available.
pub fn repos_base_dir() -> Option<PathBuf> {
    if let Ok(d) = std::env::var("OLOLO_GIT_REPOS_DIR")
        && !d.is_empty()
    {
        return Some(PathBuf::from(d));
    }
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .ok()?;
    Some(PathBuf::from(home).join(".local/share/ololo/repos"))
}

/// Per-player bare repo path: `{base}/{session_id}/{player_id}.git`.
pub fn player_repo_path(base: &Path, session_id: Uuid, player_id: Uuid) -> PathBuf {
    base.join(session_id.to_string())
        .join(format!("{}.git", player_id))
}
