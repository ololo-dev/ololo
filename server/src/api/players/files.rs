//! The player repo's file tree, as the internal git server holds it:
//!
//! - `GET /api/sessions/:code/players/:player_id/files` — every file at
//!   HEAD (path + size) and the commit HEAD points at.
//! - `GET /api/sessions/:code/players/:player_id/files/content?path=&sha=` —
//!   one file's text at that commit (HEAD when `sha` is absent).
//!
//! Owner-or-admin, like the history endpoint: this is the player's source.
//! User input never reaches git as rev syntax — `sha` must be a bare hex
//! object id, and the path is looked up with `ls-tree` and compared
//! verbatim before its blob id is read.

use crate::auth::jwt::AccessClaims;
use crate::state::AppState;
use axum::Json;
use axum::extract::{Path, Query, State};
use std::path::{Path as FsPath, PathBuf};

use super::error::PlayerError;

/// A repo with more files than this is listed partially (`truncated`).
const MAX_FILES: usize = 5000;
/// Larger files are reported (`too_large`) but their text is not sent.
const MAX_CONTENT_BYTES: u64 = 512 * 1024;
/// How far into a file to look for a NUL byte when sniffing binaries —
/// the same window git's own binary heuristic uses.
const BINARY_SNIFF_BYTES: usize = 8000;

#[derive(Debug, serde::Serialize)]
pub struct PlayerFilesHead {
    pub sha: String,
    pub subject: String,
    pub author_time: String,
}

#[derive(Debug, serde::Serialize)]
pub struct PlayerFileEntry {
    pub path: String,
    pub size: u64,
}

#[derive(Debug, serde::Serialize)]
pub struct PlayerFilesResponse {
    /// `None` before the first push (or when no repo store is configured).
    pub head: Option<PlayerFilesHead>,
    pub files: Vec<PlayerFileEntry>,
    pub truncated: bool,
}

#[derive(Debug, serde::Deserialize)]
pub struct PlayerFileQuery {
    pub path: String,
    /// The commit to read from — the `head.sha` of the listing, so a push
    /// landing between the two calls cannot mismatch tree and content.
    #[serde(default)]
    pub sha: Option<String>,
}

#[derive(Debug, serde::Serialize)]
pub struct PlayerFileContent {
    pub path: String,
    pub size: u64,
    /// Not shown as text: a NUL byte or invalid UTF-8.
    pub binary: bool,
    /// Over `MAX_CONTENT_BYTES`; the text is withheld.
    pub too_large: bool,
    pub content: Option<String>,
}

pub async fn get_player_files(
    Path((code, player_param)): Path<(String, String)>,
    State(state): State<AppState>,
    claims: AccessClaims,
) -> Result<Json<PlayerFilesResponse>, PlayerError> {
    let user_id = claims.user_id().map_err(|_| PlayerError::Forbidden)?;
    let (session, player) =
        super::authorize_player_inspect(&state.db, &code, &player_param, user_id).await?;
    let empty = PlayerFilesResponse {
        head: None,
        files: vec![],
        truncated: false,
    };
    let Some(repo_dir) = repo_dir_if_present(session.id, player.id) else {
        return Ok(Json(empty));
    };
    let listing = tokio::task::spawn_blocking(move || read_head_tree(&repo_dir))
        .await
        .map_err(|e| PlayerError::Internal(e.to_string()))??;
    Ok(Json(listing.unwrap_or(empty)))
}

pub async fn get_player_file(
    Path((code, player_param)): Path<(String, String)>,
    Query(q): Query<PlayerFileQuery>,
    State(state): State<AppState>,
    claims: AccessClaims,
) -> Result<Json<PlayerFileContent>, PlayerError> {
    let user_id = claims.user_id().map_err(|_| PlayerError::Forbidden)?;
    let (session, player) =
        super::authorize_player_inspect(&state.db, &code, &player_param, user_id).await?;
    let rev = match q.sha {
        Some(sha) if is_object_id(&sha) => sha,
        Some(_) => return Err(PlayerError::NotFound),
        None => "HEAD".to_string(),
    };
    if q.path.is_empty() || q.path.contains('\0') {
        return Err(PlayerError::NotFound);
    }
    let repo_dir = repo_dir_if_present(session.id, player.id).ok_or(PlayerError::NotFound)?;
    let content = tokio::task::spawn_blocking(move || read_file(&repo_dir, &rev, &q.path))
        .await
        .map_err(|e| PlayerError::Internal(e.to_string()))??;
    content.map(Json).ok_or(PlayerError::NotFound)
}

fn repo_dir_if_present(session_id: uuid::Uuid, player_id: uuid::Uuid) -> Option<PathBuf> {
    let base = crate::git_store::repos_base_dir()?;
    let repo_dir = crate::git_store::player_repo_path(&base, session_id, player_id);
    repo_dir.join("HEAD").exists().then_some(repo_dir)
}

/// A full SHA-1 or SHA-256 object id — nothing git would parse as rev syntax.
fn is_object_id(s: &str) -> bool {
    matches!(s.len(), 40 | 64) && s.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Run git against the bare repo; `Ok(None)` when git itself refuses
/// (unborn HEAD, unknown object), `Err` only when it cannot be run.
fn git(repo_dir: &FsPath, args: &[&str]) -> Result<Option<Vec<u8>>, PlayerError> {
    let git_bin = which::which("git").map_err(|e| PlayerError::Internal(e.to_string()))?;
    let out = std::process::Command::new(git_bin)
        .arg("-C")
        .arg(repo_dir)
        .args(args)
        .output()
        .map_err(|e| PlayerError::Internal(format!("git {}: {e}", args[0])))?;
    Ok(out.status.success().then_some(out.stdout))
}

/// HEAD's commit and every blob in its tree; `None` before the first push.
fn read_head_tree(repo_dir: &FsPath) -> Result<Option<PlayerFilesResponse>, PlayerError> {
    let Some(log) = git(
        repo_dir,
        &[
            "log",
            "-1",
            "--no-color",
            "--format=%H%x1f%aI%x1f%s",
            "HEAD",
        ],
    )?
    else {
        return Ok(None);
    };
    let log = String::from_utf8_lossy(&log);
    let mut fields = log.trim_end().splitn(3, '\x1f');
    let head = PlayerFilesHead {
        sha: fields.next().unwrap_or_default().to_string(),
        author_time: fields.next().unwrap_or_default().to_string(),
        subject: fields.next().unwrap_or_default().to_string(),
    };
    let tree = git(repo_dir, &["ls-tree", "-r", "-l", "-z", &head.sha])?.unwrap_or_default();
    let mut files: Vec<PlayerFileEntry> = parse_ls_tree(&tree)
        .filter(|e| e.kind == "blob")
        .map(|e| PlayerFileEntry {
            path: e.path,
            size: e.size,
        })
        .collect();
    let truncated = files.len() > MAX_FILES;
    files.truncate(MAX_FILES);
    Ok(Some(PlayerFilesResponse {
        head: Some(head),
        files,
        truncated,
    }))
}

/// One file's text at `rev`; `None` when the path is not a file there.
fn read_file(
    repo_dir: &FsPath,
    rev: &str,
    path: &str,
) -> Result<Option<PlayerFileContent>, PlayerError> {
    // `ls-tree <rev> -- <path>` matches the literal path (and, for a
    // directory, the directory entry itself); requiring an exact match
    // and a blob means a crafted path can name nothing but a file.
    let Some(listing) = git(repo_dir, &["ls-tree", "-l", "-z", rev, "--", path])? else {
        return Ok(None);
    };
    let Some(entry) = parse_ls_tree(&listing).find(|e| e.path == path && e.kind == "blob") else {
        return Ok(None);
    };
    let mut file = PlayerFileContent {
        path: entry.path,
        size: entry.size,
        binary: false,
        too_large: entry.size > MAX_CONTENT_BYTES,
        content: None,
    };
    if file.too_large {
        return Ok(Some(file));
    }
    let Some(bytes) = git(repo_dir, &["cat-file", "blob", &entry.oid])? else {
        return Ok(None);
    };
    let has_nul = bytes[..bytes.len().min(BINARY_SNIFF_BYTES)].contains(&0);
    match String::from_utf8(bytes) {
        Ok(text) if !has_nul => file.content = Some(text),
        _ => file.binary = true,
    }
    Ok(Some(file))
}

struct TreeEntry {
    kind: String,
    oid: String,
    size: u64,
    path: String,
}

/// Parse `ls-tree -l -z` records: `<mode> <type> <oid> <size>\t<path>\0`.
/// Submodule entries (`commit`, size `-`) parse with size 0.
fn parse_ls_tree(out: &[u8]) -> impl Iterator<Item = TreeEntry> + '_ {
    out.split(|&b| b == 0).filter_map(|record| {
        let record = std::str::from_utf8(record).ok()?;
        let (meta, path) = record.split_once('\t')?;
        let mut meta = meta.split_whitespace();
        let _mode = meta.next()?;
        let kind = meta.next()?.to_string();
        let oid = meta.next()?.to_string();
        let size = meta.next()?.parse().unwrap_or(0);
        Some(TreeEntry {
            kind,
            oid,
            size,
            path: path.to_string(),
        })
    })
}

#[cfg(test)]
#[path = "files_tests.rs"]
mod files_tests;
