//! `GET /api/sessions/:code/players/:player_id/memory` — the player's
//! session-memory state: the project's schema defaults overlaid with the
//! LLM-extracted values, plus which keys were actually extracted.
//!
//! Owner-only (admins may view any player), mirroring the history endpoint.

use crate::auth::jwt::AccessClaims;
use crate::state::AppState;
use arena_core::entities::{player_memory, projects, sessions};
use axum::Json;
use axum::extract::{Path, State};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};

use super::error::PlayerError;

#[derive(Debug, serde::Serialize)]
pub struct PlayerMemoryEntry {
    pub key: String,
    /// Current effective value (extracted when available, else the default).
    pub value: String,
    /// The project-schema default for this key.
    pub default: String,
    /// `true` when the value came from LLM extraction of the player's docs.
    pub extracted: bool,
}

#[derive(Debug, serde::Serialize)]
pub struct PlayerMemoryResponse {
    /// `false` when the project declares no memory schema (UI hides the tab).
    pub enabled: bool,
    pub entries: Vec<PlayerMemoryEntry>,
    /// When the last extraction landed; `null` before the first one.
    pub updated_at: Option<chrono::DateTime<chrono::Utc>>,
}

pub async fn get_player_memory(
    Path((code, player_param)): Path<(String, String)>,
    State(state): State<AppState>,
    claims: AccessClaims,
) -> Result<Json<PlayerMemoryResponse>, PlayerError> {
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
    // Owner-only, except admins may view any player's memory.
    if player.user_id_fk != Some(user_id)
        && !crate::auth::is_user_admin(&state.db, user_id)
            .await
            .map_err(|e| PlayerError::Internal(e.to_string()))?
    {
        return Err(PlayerError::Forbidden);
    }

    let memory_schema = projects::Entity::find_by_id(session.project_id_fk)
        .one(&state.db)
        .await
        .map_err(|e| PlayerError::Internal(e.to_string()))?
        .and_then(|p| p.memory_schema);
    let defaults = arena_core::memory::parse_memory_schema_column(memory_schema.as_deref());
    if defaults.is_empty() {
        return Ok(Json(PlayerMemoryResponse {
            enabled: false,
            entries: vec![],
            updated_at: None,
        }));
    }

    let row = player_memory::Entity::find()
        .filter(player_memory::Column::SessionIdFk.eq(session.id))
        .filter(player_memory::Column::PlayerIdFk.eq(player.id))
        .one(&state.db)
        .await
        .map_err(|e| PlayerError::Internal(e.to_string()))?;
    let (extracted, updated_at) = match &row {
        Some(m) => (
            serde_json::from_str::<serde_json::Value>(&m.values_json)
                .map(|v| arena_core::memory::filter_extracted_values(&defaults, &v))
                .unwrap_or_default(),
            Some(m.updated_at),
        ),
        None => (Default::default(), None),
    };

    let entries = defaults
        .iter()
        .map(|(key, default)| {
            let extracted_value = extracted.get(key);
            PlayerMemoryEntry {
                key: key.clone(),
                value: extracted_value.unwrap_or(default).clone(),
                default: default.clone(),
                extracted: extracted_value.is_some(),
            }
        })
        .collect();

    Ok(Json(PlayerMemoryResponse {
        enabled: true,
        entries,
        updated_at,
    }))
}
