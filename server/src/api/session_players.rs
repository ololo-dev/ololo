//! PATCH /api/sessions/:session_id/players/:player_id
//!
//! Allows a participant to register their environment fingerprint and metadata.

use crate::{
    auth::jwt::AccessClaims,
    entities::{players, sessions},
    state::AppState,
};
use arena_core::protocol::ArenaFrame;
use axum::{
    Json,
    extract::{Path, State},
    response::{IntoResponse, Response},
};
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, Set};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PatchPlayerMetadataReq {
    pub fingerprint: String,
    pub metadata: PlayerMetadataPayload,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct ToolEntry {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct PlayerMetadataPayload {
    pub ai_agents: Vec<ToolEntry>,
    pub build_tools: Vec<ToolEntry>,
    pub languages: Vec<ToolEntry>,
    #[serde(default)]
    pub test_tools: Vec<ToolEntry>,
    #[serde(default)]
    pub utility_tools: Vec<ToolEntry>,
    pub probe_duration_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub platform: Option<String>,
}

#[derive(Debug, Error)]
pub enum PlayerMetadataError {
    #[error("not found")]
    NotFound,
    #[error("fingerprint conflict")]
    FingerprintConflict,
    #[error("invalid request: {0}")]
    InvalidRequest(String),
    #[error("database error: {0}")]
    DbError(#[from] sea_orm::DbErr),
    #[error("json error: {0}")]
    JsonError(#[from] serde_json::Error),
}

crate::api::error::impl_api_error!(PlayerMetadataError {
    Self::NotFound => (NOT_FOUND, "not_found"),
    Self::FingerprintConflict => (CONFLICT, "fingerprint_conflict"),
    Self::InvalidRequest(msg) => (BAD_REQUEST, "invalid_request", "detail": msg),
    Self::DbError(_) | Self::JsonError(_) => (INTERNAL_SERVER_ERROR, "internal_error"),
});

fn parse_user_id(claims: &AccessClaims) -> Result<Uuid, PlayerMetadataError> {
    claims.user_id().map_err(|_| PlayerMetadataError::NotFound)
}

pub async fn patch_player_metadata(
    State(state): State<AppState>,
    claims: AccessClaims,
    Path((session_code, player_id)): Path<(String, Uuid)>,
    Json(req): Json<PatchPlayerMetadataReq>,
) -> Result<Response, PlayerMetadataError> {
    // Validate fingerprint: must be exactly 64 lowercase hex chars
    if req.fingerprint.len() != 64
        || !req
            .fingerprint
            .chars()
            .all(|c| matches!(c, '0'..='9' | 'a'..='f'))
    {
        return Err(PlayerMetadataError::InvalidRequest(
            "fingerprint must be 64 lowercase hex chars".to_string(),
        ));
    }

    // Validate array bounds
    for entry in req
        .metadata
        .ai_agents
        .iter()
        .chain(req.metadata.build_tools.iter())
        .chain(req.metadata.languages.iter())
        .chain(req.metadata.test_tools.iter())
        .chain(req.metadata.utility_tools.iter())
    {
        if entry.name.len() > 64 {
            return Err(PlayerMetadataError::InvalidRequest(
                "array item name exceeds 64 chars".to_string(),
            ));
        }
        if entry.version.as_deref().map_or(0, str::len) > 64 {
            return Err(PlayerMetadataError::InvalidRequest(
                "array item version exceeds 64 chars".to_string(),
            ));
        }
    }
    if req.metadata.ai_agents.len() > 30
        || req.metadata.build_tools.len() > 30
        || req.metadata.languages.len() > 30
        || req.metadata.test_tools.len() > 30
        || req.metadata.utility_tools.len() > 30
    {
        return Err(PlayerMetadataError::InvalidRequest(
            "array exceeds 30 items".to_string(),
        ));
    }

    if let Some(ref platform) = req.metadata.platform
        && platform.len() > 64
    {
        return Err(PlayerMetadataError::InvalidRequest(
            "platform exceeds 64 chars".to_string(),
        ));
    }

    // Early size guard: serialize the request and check total size
    let estimated_size = serde_json::to_string(&req.metadata)
        .map(|s| s.len())
        .unwrap_or(usize::MAX)
        + req.fingerprint.len()
        + 32;
    if estimated_size > 4224 {
        return Err(PlayerMetadataError::InvalidRequest(
            "request body too large".to_string(),
        ));
    }

    let user_id = parse_user_id(&claims)?;

    let session = match Uuid::parse_str(&session_code) {
        Ok(id) => sessions::Entity::find_by_id(id)
            .one(&state.db)
            .await?
            .ok_or(PlayerMetadataError::NotFound)?,
        Err(_) => sessions::Entity::find()
            .filter(sessions::Column::JoinCode.eq(session_code.to_uppercase()))
            .one(&state.db)
            .await?
            .ok_or(PlayerMetadataError::NotFound)?,
    };
    let session_id = session.id;

    // Ownership check: player must belong to this user in this session
    let player = players::Entity::find()
        .filter(players::Column::Id.eq(player_id))
        .filter(players::Column::SessionIdFk.eq(session_id))
        .filter(players::Column::UserIdFk.eq(user_id))
        .filter(players::Column::RevokedAt.is_null())
        .one(&state.db)
        .await?
        .ok_or(PlayerMetadataError::NotFound)?;

    // Fingerprint conflict check: no other player in same session may hold this fingerprint
    let conflict = players::Entity::find()
        .filter(players::Column::Fingerprint.eq(req.fingerprint.clone()))
        .filter(players::Column::SessionIdFk.eq(session_id))
        .filter(players::Column::Id.ne(player_id))
        .one(&state.db)
        .await?;
    if conflict.is_some() {
        return Err(PlayerMetadataError::FingerprintConflict);
    }

    let metadata_json = serde_json::to_string(&req.metadata)?;
    let fingerprint = req.fingerprint.clone();
    let agent_display_name = req.metadata.ai_agents.first().map(|t| t.name.clone());
    let player_display_name = player.display_name.clone();
    let player_joined_at = player.joined_at.to_rfc3339();
    let mut am: players::ActiveModel = player.into();
    am.fingerprint = Set(Some(fingerprint));
    am.metadata_json = Set(Some(metadata_json));
    am.update(&state.db).await?;

    // Refresh the in-memory session participants cache so lobby/dashboard
    // observers see the agent label without a reconnect. Re-broadcast
    // MemberJoined for this user — the frontend updates existing entries.
    if let Some(entry) = state.session_registry.get(&session.join_code) {
        let user_id_str = user_id.to_string();
        let updated_member_version = {
            let mut cache = entry.cache.write().unwrap_or_else(|e| e.into_inner());
            if let Some(p) = cache
                .participants
                .iter_mut()
                .find(|p| p.user_id == user_id_str)
            {
                p.fingerprint = Some(req.fingerprint.clone());
                p.agent_display_name = agent_display_name.clone();
                p.player_id.get_or_insert(player_id);
            }
            cache.version = cache.version.saturating_add(1);
            cache.version
        };
        let _ = entry.tx.send(ArenaFrame::MemberJoined {
            user_id: user_id_str,
            player_id: Some(player_id),
            display_name: player_display_name,
            joined_at: player_joined_at,
            avatar_url: None,
            fingerprint: Some(req.fingerprint.clone()),
            username: None,
            agent_display_name,
            version: updated_member_version,
        });
    }

    Ok(Json(serde_json::json!({"status": "ok"})).into_response())
}
