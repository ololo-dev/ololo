use crate::auth::jwt::AccessClaims;
use crate::state::AppState;
use arena_core::entities::sessions;
use axum::Json;
use axum::extract::{Path, State};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use std::path::Path as FsPath;

use super::error::PlayerError;

#[derive(Debug, serde::Serialize)]
pub struct PlayerHistoryCommit {
    pub sha: String,
    pub author_name: String,
    pub author_email: String,
    pub author_time: String,
    pub message: String,
    pub files: Vec<PlayerHistoryFileDiff>,
}

#[derive(Debug, serde::Serialize)]
pub struct PlayerHistoryFileDiff {
    pub path: String,
    pub status: String, // "added" | "modified" | "deleted"
    pub patch: String,
}

#[derive(Debug, serde::Serialize)]
pub struct PlayerHistoryResponse {
    pub commits: Vec<PlayerHistoryCommit>,
}

pub async fn get_player_history(
    Path((code, player_param)): Path<(String, String)>,
    State(state): State<AppState>,
    claims: AccessClaims,
) -> Result<Json<PlayerHistoryResponse>, PlayerError> {
    let user_id = claims.user_id().map_err(|_| PlayerError::Forbidden)?;

    let session = sessions::Entity::find()
        .filter(sessions::Column::JoinCode.eq(code.to_uppercase()))
        .one(&state.db)
        .await
        .map_err(|e| PlayerError::Internal(e.to_string()))?
        .ok_or(PlayerError::NotFound)?;

    let player = super::find_player_by_param(&state.db, session.id, &player_param)
        .await
        .map_err(|e| PlayerError::Internal(e.to_string()))?
        .ok_or(PlayerError::NotFound)?;
    let player_id = player.id;
    // Owner-only, except admins may view any player's history.
    if player.user_id_fk != Some(user_id)
        && !crate::auth::is_user_admin(&state.db, user_id)
            .await
            .map_err(|e| PlayerError::Internal(e.to_string()))?
    {
        return Err(PlayerError::Forbidden);
    }

    // Resolve the bare repo path.
    let Some(base) = crate::git_store::repos_base_dir() else {
        return Ok(Json(PlayerHistoryResponse { commits: vec![] }));
    };
    let repo_dir = crate::git_store::player_repo_path(&base, session.id, player_id);
    if !repo_dir.join("HEAD").exists() {
        return Ok(Json(PlayerHistoryResponse { commits: vec![] }));
    }

    let commits = tokio::task::spawn_blocking(move || read_history(&repo_dir))
        .await
        .map_err(|e| PlayerError::Internal(e.to_string()))??;
    Ok(Json(PlayerHistoryResponse { commits }))
}

/// Run `git log` + per-commit `git diff` on a bare repo and parse the output
/// into `PlayerHistoryCommit` entries. Uses `--format` with a NUL-separated
/// schema so messages can contain newlines safely.
fn read_history(repo_dir: &FsPath) -> Result<Vec<PlayerHistoryCommit>, PlayerError> {
    let git_bin = which::which("git").map_err(|e| PlayerError::Internal(e.to_string()))?;

    // Format: sha<NUL>author_name<NUL>author_email<NUL>iso8601<NUL>message<NUL>
    // Record separator: ASCII 0x1e (RS) between commits.
    // Field separator: ASCII 0x1f (US) — NUL (\x00) is not allowed in Command args.
    const LOG_SEP: &str = "\x1e";
    const FIELD_SEP: &str = "\x1f";
    let log_format = format!("%H{F}%an{F}%ae{F}%aI{F}%B{S}", F = FIELD_SEP, S = LOG_SEP);
    let out = std::process::Command::new(&git_bin)
        .arg("-C")
        .arg(repo_dir)
        .arg("log")
        .arg(format!("--format={log_format}"))
        .arg("--no-color")
        .output()
        .map_err(|e| PlayerError::Internal(format!("git log: {e}")))?;
    if !out.status.success() {
        return Ok(vec![]); // empty repo
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let mut commits = Vec::new();
    for record in text.split(LOG_SEP) {
        let record = record.trim_matches(|c: char| c == '\n' || c == '\r');
        if record.is_empty() {
            continue;
        }
        let fields: Vec<&str> = record.splitn(5, FIELD_SEP).collect();
        if fields.len() < 5 {
            continue;
        }
        let sha = fields[0].to_string();
        let author_name = fields[1].to_string();
        let author_email = fields[2].to_string();
        let author_time = fields[3].to_string();
        let message = fields[4].to_string();
        let files = read_commit_diff(&git_bin, repo_dir, &sha)?;
        commits.push(PlayerHistoryCommit {
            sha,
            author_name,
            author_email,
            author_time,
            message,
            files,
        });
    }
    Ok(commits)
}

/// Per-commit file diff via `git show --raw --format= --name-status -- <sha>`.
/// Returns one `PlayerHistoryFileDiff` per changed path.
fn read_commit_diff(
    git_bin: &FsPath,
    repo_dir: &FsPath,
    sha: &str,
) -> Result<Vec<PlayerHistoryFileDiff>, PlayerError> {
    // name-status: lines like "A\tpath", "M\tpath", "D\tpath".
    let status_out = std::process::Command::new(git_bin)
        .arg("-C")
        .arg(repo_dir)
        .arg("show")
        .arg("--no-color")
        .arg("--name-status")
        .arg("--format=")
        .arg(sha)
        .output()
        .map_err(|e| PlayerError::Internal(format!("git show name-status: {e}")))?;
    if !status_out.status.success() {
        return Ok(vec![]);
    }
    let status_text = String::from_utf8_lossy(&status_out.stdout);
    let mut entries: Vec<(String, String)> = Vec::new();
    for line in status_text.lines() {
        let line = line.trim_end_matches('\r');
        if line.is_empty() {
            continue;
        }
        if let Some((code, path)) = line.split_once('\t') {
            entries.push((
                code.chars().next().unwrap_or('M').to_string(),
                path.to_string(),
            ));
        }
    }
    if entries.is_empty() {
        return Ok(vec![]);
    }
    // Full patch for the commit — one blob, parsed by `diff --git` headers.
    let patch_out = std::process::Command::new(git_bin)
        .arg("-C")
        .arg(repo_dir)
        .arg("show")
        .arg("--no-color")
        .arg("--format=")
        .arg(sha)
        .output()
        .map_err(|e| PlayerError::Internal(format!("git show patch: {e}")))?;
    let patch_text = if patch_out.status.success() {
        String::from_utf8_lossy(&patch_out.stdout).to_string()
    } else {
        String::new()
    };
    // Map path → patch chunk. Split the patch on "diff --git a/... b/...".
    let mut by_path: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    let mut cur_path: Option<String> = None;
    let mut cur_buf = String::new();
    for line in patch_text.lines() {
        if line.starts_with("diff --git ") {
            // flush previous
            if let Some(p) = cur_path.take() {
                by_path.insert(p, std::mem::take(&mut cur_buf));
            }
            // parse "diff --git a/<path> b/<path>"
            if let Some(rest) = line.strip_prefix("diff --git ")
                && let Some(idx) = rest.find(" b/")
            {
                cur_path = Some(rest["a/".len()..idx].to_string());
            }
        } else if cur_path.is_some() {
            cur_buf.push_str(line);
            cur_buf.push('\n');
        }
    }
    if let Some(p) = cur_path.take() {
        by_path.insert(p, std::mem::take(&mut cur_buf));
    }

    let mut files = Vec::with_capacity(entries.len());
    for (code, path) in entries {
        let status = match code.as_str() {
            "A" => "added".to_string(),
            "D" => "deleted".to_string(),
            _ => "modified".to_string(),
        };
        let patch = by_path.get(&path).cloned().unwrap_or_default();
        files.push(PlayerHistoryFileDiff {
            path,
            status,
            patch,
        });
    }
    Ok(files)
}

#[cfg(test)]
#[path = "history_tests.rs"]
mod history_tests;
