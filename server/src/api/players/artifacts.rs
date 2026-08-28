//! `GET /api/sessions/:code/players/:player_id/artifacts/:probe_id` — stream
//! an interactive-probe artifact out of the player's bare repo.
//!
//! The artifact never leaves git: the probe row stores a `"<sha>:<path>"`
//! blob reference the game-server wrote when the push arrived, and this
//! endpoint reads that exact blob. Owner-only (admins may view any player),
//! mirroring the snapshot/memory endpoints.

use crate::auth::jwt::AccessClaims;
use crate::state::AppState;
use arena_core::entities::{probes, sessions, tests};
use arena_core::evaluation::ProbeConfig;
use axum::extract::{Path, State};
use axum::http::header;
use axum::response::{IntoResponse, Response};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use uuid::Uuid;

use super::error::PlayerError;

/// Resolve the session by join code and the player by param, and enforce
/// the read authorization both endpoints below share: the player's own
/// user, or an admin.
async fn authorize_player_read(
    db: &sea_orm::DatabaseConnection,
    code: &str,
    player_param: &str,
    user_id: Uuid,
) -> Result<
    (
        arena_core::entities::sessions::Model,
        arena_core::entities::players::Model,
    ),
    PlayerError,
> {
    let session = sessions::Entity::find()
        .filter(sessions::Column::JoinCode.eq(code.to_uppercase()))
        .one(db)
        .await
        .map_err(|e| PlayerError::Internal(e.to_string()))?
        .ok_or(PlayerError::NotFound)?;
    let player = super::find_player_by_param(db, session.id, player_param)
        .await
        .map_err(|e| PlayerError::Internal(e.to_string()))?
        .ok_or(PlayerError::NotFound)?;
    if player.user_id_fk != Some(user_id)
        && !crate::auth::is_user_admin(db, user_id)
            .await
            .map_err(|e| PlayerError::Internal(e.to_string()))?
    {
        return Err(PlayerError::Forbidden);
    }
    Ok((session, player))
}

pub async fn get_player_artifact(
    Path((code, player_param, probe_id)): Path<(String, String, Uuid)>,
    axum::extract::Query(q): axum::extract::Query<ArtifactQuery>,
    State(state): State<AppState>,
    claims: AccessClaims,
) -> Result<Response, PlayerError> {
    let user_id = claims.user_id().map_err(|_| PlayerError::Forbidden)?;
    let (session, player) = authorize_player_read(&state.db, &code, &player_param, user_id).await?;

    let probe = probes::Entity::find_by_id(probe_id)
        .one(&state.db)
        .await
        .map_err(|e| PlayerError::Internal(e.to_string()))?
        .ok_or(PlayerError::NotFound)?;
    if probe.session_id != session.id || probe.player_id != player.id {
        return Err(PlayerError::NotFound);
    }
    let reference = artifact_reference(&probe, q.i.unwrap_or(0)).ok_or(PlayerError::NotFound)?;

    // Content type from the delivered file's extension — the request only
    // declares what the judge EXPECTED (a video/webm ask often arrives as a
    // .gif); the declared type is the fallback for unknown extensions.
    let file_path = reference
        .split_once(':')
        .map(|(_, p)| p)
        .unwrap_or(reference.as_str());
    let content_type = match arena_core::evaluation::artifact_content_type_for_path(file_path) {
        Some(ct) => ct.to_string(),
        None => tests::Entity::find_by_id(probe.test_id)
            .one(&state.db)
            .await
            .map_err(|e| PlayerError::Internal(e.to_string()))?
            .and_then(|t| t.probe_config)
            .and_then(|c| ProbeConfig::from_json(&c).ok())
            .and_then(|c| c.artifact.map(|a| a.content_type))
            .unwrap_or_else(|| "application/octet-stream".to_string()),
    };

    let repos_base = arena_core::git_store::repos_base_dir()
        .ok_or_else(|| PlayerError::Internal("git repo store is not configured".to_string()))?;
    let repo_dir = arena_core::git_store::player_repo_path(&repos_base, session.id, player.id);
    let bytes = read_artifact_blob(&repo_dir, &reference)
        .await
        .ok_or(PlayerError::NotFound)?;

    let mut resp = bytes.into_response();
    resp.headers_mut().insert(
        header::CONTENT_TYPE,
        content_type
            .parse()
            .unwrap_or(header::HeaderValue::from_static("application/octet-stream")),
    );
    // Player-supplied content: never render in the page origin as HTML.
    resp.headers_mut().insert(
        header::CONTENT_SECURITY_POLICY,
        header::HeaderValue::from_static("sandbox"),
    );
    resp.headers_mut().insert(
        header::HeaderName::from_static("x-content-type-options"),
        header::HeaderValue::from_static("nosniff"),
    );
    Ok(resp)
}

/// The blob reference for the `i`-th delivered file of an artifact request.
/// `i == 0` falls back to the stored single reference; other indexes come
/// from the file list the arrival sweep recorded in `result_json.artifact`.
pub(crate) fn artifact_reference(probe: &probes::Model, i: usize) -> Option<String> {
    let listed = probe.result_json.as_ref().and_then(|r| {
        let a = r.get("artifact")?;
        let commit = a.get("commit")?.as_str()?;
        let files = a.get("files")?.as_array()?;
        let path = files.get(i)?.get("path")?.as_str()?;
        Some(format!("{commit}:{path}"))
    });
    match (listed, i) {
        (Some(r), _) => Some(r),
        (None, 0) => probe.artifact_path.clone(),
        (None, _) => None,
    }
}

/// `git show <sha>:<path>` from the bare repo. Shared with the session
/// activity-feed artifact endpoint.
pub(crate) async fn read_artifact_blob(
    repo_dir: &std::path::Path,
    reference: &str,
) -> Option<Vec<u8>> {
    let repo_dir = repo_dir.to_path_buf();
    let reference = reference.to_string();
    tokio::task::spawn_blocking(move || {
        let git_bin = which::which("git").ok()?;
        let out = std::process::Command::new(git_bin)
            .arg("-C")
            .arg(&repo_dir)
            .arg("show")
            .arg(&reference)
            .output()
            .ok()?;
        out.status.success().then_some(out.stdout)
    })
    .await
    .ok()
    .flatten()
}

/// `GET /api/sessions/:code/players/:player_id/repo-file?path=<repo path>` —
/// stream an image file from the player repo's HEAD. Same authorization as
/// the artifact endpoint (owner or admin); image extensions only, so this
/// cannot become a generic source browser.
pub async fn get_player_repo_file(
    Path((code, player_param)): Path<(String, String)>,
    axum::extract::Query(q): axum::extract::Query<RepoFileQuery>,
    State(state): State<AppState>,
    claims: AccessClaims,
) -> Result<Response, PlayerError> {
    let user_id = claims.user_id().map_err(|_| PlayerError::Forbidden)?;
    let (session, player) = authorize_player_read(&state.db, &code, &player_param, user_id).await?;

    let path_in_repo = q.path;
    let lower = path_in_repo.to_ascii_lowercase();
    let content_type = if lower.ends_with(".png") {
        "image/png"
    } else if lower.ends_with(".jpg") || lower.ends_with(".jpeg") {
        "image/jpeg"
    } else if lower.ends_with(".webp") {
        "image/webp"
    } else if lower.ends_with(".gif") {
        "image/gif"
    } else {
        return Err(PlayerError::NotFound);
    };
    // `git show HEAD:<path>` resolves only paths inside the tree — no
    // filesystem traversal is possible — but refuse option-looking and
    // rev-syntax input outright.
    if path_in_repo.starts_with('-') || path_in_repo.contains("..") || path_in_repo.contains(':') {
        return Err(PlayerError::NotFound);
    }

    let repos_base = arena_core::git_store::repos_base_dir()
        .ok_or_else(|| PlayerError::Internal("git repo store is not configured".to_string()))?;
    let repo_dir = arena_core::git_store::player_repo_path(&repos_base, session.id, player.id);
    let bytes = read_artifact_blob(&repo_dir, &format!("HEAD:{path_in_repo}"))
        .await
        .ok_or(PlayerError::NotFound)?;
    const MAX_BYTES: usize = 10 * 1024 * 1024;
    if bytes.len() > MAX_BYTES {
        return Err(PlayerError::NotFound);
    }

    let mut resp = bytes.into_response();
    resp.headers_mut().insert(
        header::CONTENT_TYPE,
        header::HeaderValue::from_static(content_type),
    );
    Ok(resp)
}

#[derive(serde::Deserialize)]
pub struct ArtifactQuery {
    /// File index of a multi-file delivery (default 0).
    #[serde(default)]
    pub i: Option<usize>,
}

#[derive(Debug, serde::Deserialize)]
pub struct RepoFileQuery {
    pub path: String,
}
