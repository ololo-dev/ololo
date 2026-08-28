//! GET /api/sessions/:code/players/metadata
//!
//! Returns a JSON object keyed by player_id (UUID) mapping to player
//! environment metadata. Accessible to admin users only.

use crate::{
    api::session_players::ToolEntry,
    entities::{players, sessions},
    state::AppState,
};
use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use std::collections::HashMap;
use uuid::Uuid;

/// Internal deserialization target for the `metadata_json` blob stored on
/// player rows. No `deny_unknown_fields` — must remain forward-compatible
/// with future ololo client versions.
#[derive(serde::Deserialize, Default)]
struct ParsedMetadata {
    #[serde(default)]
    ai_agents: Vec<ToolEntry>,
    #[serde(default)]
    build_tools: Vec<ToolEntry>,
    #[serde(default)]
    languages: Vec<ToolEntry>,
    #[serde(default)]
    test_tools: Vec<ToolEntry>,
    #[serde(default)]
    utility_tools: Vec<ToolEntry>,
    #[serde(default)]
    probe_duration_ms: Option<u64>,
    #[serde(default)]
    platform: Option<String>,
}

/// Wire type returned to the client for each player entry.
#[derive(serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct PlayerMetadataResponse {
    pub player_id: Uuid,
    pub display_name: String,
    pub joined_at: chrono::DateTime<chrono::Utc>,
    pub fingerprint: Option<String>,
    pub ai_agents: Vec<ToolEntry>,
    pub build_tools: Vec<ToolEntry>,
    pub languages: Vec<ToolEntry>,
    pub test_tools: Vec<ToolEntry>,
    pub utility_tools: Vec<ToolEntry>,
    pub probe_duration_ms: Option<u64>,
    pub platform: Option<String>,
}

/// `GET /api/sessions/:code/players/metadata`
///
/// Returns all players in the session with their parsed environment metadata.
/// Requires admin authentication.
pub async fn get_session_players_metadata(
    _admin: crate::api::settings::AdminUser,
    State(state): State<AppState>,
    Path(code): Path<String>,
) -> Result<Json<HashMap<Uuid, PlayerMetadataResponse>>, (StatusCode, &'static str)> {
    // Query 1: look up the session by join_code.
    let session = sessions::Entity::find()
        .filter(sessions::Column::JoinCode.eq(code.trim().to_uppercase()))
        .one(&state.db)
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "database error"))?
        .ok_or((StatusCode::NOT_FOUND, "session not found"))?;

    // Query 2: fetch all players belonging to this session.
    let player_rows = players::Entity::find()
        .filter(players::Column::SessionIdFk.eq(session.id))
        .all(&state.db)
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "database error"))?;

    let map: HashMap<Uuid, PlayerMetadataResponse> = player_rows
        .into_iter()
        .map(|p| {
            let meta: ParsedMetadata = p
                .metadata_json
                .as_deref()
                .and_then(|json| serde_json::from_str(json).ok())
                .unwrap_or_default();

            let entry = PlayerMetadataResponse {
                player_id: p.id,
                display_name: p.display_name,
                joined_at: p.joined_at,
                fingerprint: p.fingerprint,
                ai_agents: meta.ai_agents,
                build_tools: meta.build_tools,
                languages: meta.languages,
                test_tools: meta.test_tools,
                utility_tools: meta.utility_tools,
                probe_duration_ms: meta.probe_duration_ms,
                platform: meta.platform,
            };
            (p.id, entry)
        })
        .collect();

    Ok(Json(map))
}
