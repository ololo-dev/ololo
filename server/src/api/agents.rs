use crate::auth::jwt::AccessClaims;
use crate::state::AppState;
use arena_core::entities::{players, sessions};
use arena_core::protocol::ZmqEvent;
use arena_core::session_status::SessionStatus;
use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, Utc};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, DatabaseConnection, DbErr, EntityTrait, PaginatorTrait,
    QueryFilter, QueryOrder, Set,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

const MODEL_MIN: usize = 1;
const MODEL_MAX: usize = 120;
const NAME_MIN: usize = 1;
const NAME_MAX: usize = 80;

const KIND_CLAUDE_CODE: &str = "claude-code";
const KIND_OPENCODE: &str = "opencode";
const KIND_CUSTOM: &str = "custom";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RegisterAgentReq {
    kind: String,
    model: String,
    display_name: String,
}

#[derive(Debug, Serialize)]
struct AgentSummary {
    id: Uuid,
    session_id: Uuid,
    user_id: Uuid,
    kind: String,
    model: String,
    display_name: String,
    registered_at: DateTime<Utc>,
    revoked_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Serialize)]
struct RegisterAgentResp {
    agent: AgentSummary,
}

#[derive(Debug, Deserialize, Serialize)]
struct AgentMetadata {
    kind: String,
    model: String,
}

#[derive(Debug, Serialize)]
struct AgentListResp {
    agents: Vec<AgentSummary>,
}

#[derive(Debug)]
pub enum AgentError {
    NotFound,
    Forbidden,
    InvalidKind,
    InvalidModel,
    InvalidDisplayName,
    SessionClosed,
    CapReached,
    AlreadyRevoked,
    Db(DbErr),
}

impl From<DbErr> for AgentError {
    fn from(e: DbErr) -> Self {
        AgentError::Db(e)
    }
}

crate::api::error::impl_api_error!(AgentError {
    Self::NotFound => (NOT_FOUND, "not_found"),
    Self::Forbidden => (FORBIDDEN, "forbidden"),
    Self::InvalidKind => (UNPROCESSABLE_ENTITY, "invalid_kind"),
    Self::InvalidModel => (UNPROCESSABLE_ENTITY, "invalid_model"),
    Self::InvalidDisplayName => (UNPROCESSABLE_ENTITY, "invalid_display_name"),
    Self::SessionClosed => (CONFLICT, "session_closed"),
    Self::CapReached => (CONFLICT, "agent_cap_reached"),
    Self::AlreadyRevoked => (CONFLICT, "already_revoked"),
    Self::Db(_) => (INTERNAL_SERVER_ERROR, "database_error"),
});

fn parse_user_id(claims: &AccessClaims) -> Result<Uuid, AgentError> {
    claims.user_id().map_err(|_| AgentError::Forbidden)
}

fn validate_kind(raw: &str) -> Result<&'static str, AgentError> {
    match raw {
        KIND_CLAUDE_CODE => Ok(KIND_CLAUDE_CODE),
        KIND_OPENCODE => Ok(KIND_OPENCODE),
        KIND_CUSTOM => Ok(KIND_CUSTOM),
        _ => Err(AgentError::InvalidKind),
    }
}

fn validate_bounded(raw: &str, min: usize, max: usize) -> Option<String> {
    let trimmed = raw.trim().to_string();
    let len = trimmed.chars().count();
    if (min..=max).contains(&len) {
        Some(trimmed)
    } else {
        None
    }
}

fn to_summary(m: players::Model) -> AgentSummary {
    let (kind, model) = m
        .metadata_json
        .as_deref()
        .and_then(|raw| serde_json::from_str::<AgentMetadata>(raw).ok())
        .map(|meta| (meta.kind, meta.model))
        .unwrap_or_else(|| (KIND_CUSTOM.to_string(), String::new()));

    AgentSummary {
        id: m.id,
        session_id: m.session_id_fk,
        user_id: m.user_id_fk.unwrap_or(Uuid::nil()),
        kind,
        model,
        display_name: m.display_name,
        registered_at: m.joined_at,
        revoked_at: m.revoked_at,
    }
}

async fn load_visible(
    db: &DatabaseConnection,
    session_id: Uuid,
    user_id: Uuid,
) -> Result<sessions::Model, AgentError> {
    let row = sessions::Entity::find_by_id(session_id)
        .one(db)
        .await?
        .ok_or(AgentError::NotFound)?;
    if row.owner_id_fk == Some(user_id) {
        return Ok(row);
    }
    let is_member = players::Entity::find()
        .filter(players::Column::SessionIdFk.eq(row.id))
        .filter(players::Column::UserIdFk.eq(user_id))
        .one(db)
        .await?
        .is_some();
    if is_member {
        Ok(row)
    } else {
        Err(AgentError::NotFound)
    }
}

pub async fn post_register(
    State(state): State<AppState>,
    claims: AccessClaims,
    Path(session_id): Path<Uuid>,
    Json(req): Json<RegisterAgentReq>,
) -> Result<Response, AgentError> {
    let user_id = parse_user_id(&claims)?;

    let kind = validate_kind(&req.kind)?;
    let model =
        validate_bounded(&req.model, MODEL_MIN, MODEL_MAX).ok_or(AgentError::InvalidModel)?;
    let display_name = validate_bounded(&req.display_name, NAME_MIN, NAME_MAX)
        .ok_or(AgentError::InvalidDisplayName)?;
    let metadata_json = serde_json::json!({ "kind": kind, "model": model }).to_string();

    let session = load_visible(&state.db, session_id, user_id).await?;
    if session.status == SessionStatus::Finished || session.status == SessionStatus::Cancelled {
        return Err(AgentError::SessionClosed);
    }

    let active = players::Entity::find()
        .filter(players::Column::SessionIdFk.eq(session.id))
        .filter(players::Column::RevokedAt.is_null())
        .count(&state.db)
        .await?;
    if active >= state.max_players_per_session as u64 {
        return Err(AgentError::CapReached);
    }

    let player_id = Uuid::new_v4();

    let model_row = players::ActiveModel {
        id: Set(player_id),
        session_id_fk: Set(session.id),
        user_id_fk: Set(Some(user_id)),
        display_name: Set(display_name),
        fingerprint: Set(None),
        metadata_json: Set(Some(metadata_json)),
        joined_at: Set(Utc::now()),
        reconnected_at: Set(None),
        revoked_at: Set(None),
        agent_connected: Set(false),
        agent_last_seen_at: Set(None),
    }
    .insert(&state.db)
    .await?;

    let resp = RegisterAgentResp {
        agent: to_summary(model_row.clone()),
    };

    let _ = state.zmq_events_tx.send(ZmqEvent::PlayerJoin {
        join_code: session.join_code.clone(),
        player_id: model_row.id,
        display_name: model_row.display_name.clone(),
        user_id: model_row.user_id_fk.map(|id| id.to_string()),
        joined_at: model_row.joined_at.to_rfc3339(),
        avatar_url: None,
        fingerprint: None,
        username: None,
        version: 0,
    });

    Ok((StatusCode::CREATED, Json(resp)).into_response())
}

pub async fn get_list(
    State(state): State<AppState>,
    claims: AccessClaims,
    Path(session_id): Path<Uuid>,
) -> Result<Response, AgentError> {
    let user_id = parse_user_id(&claims)?;
    let _session = load_visible(&state.db, session_id, user_id).await?;

    let rows = players::Entity::find()
        .filter(players::Column::SessionIdFk.eq(session_id))
        .filter(players::Column::MetadataJson.is_not_null())
        .order_by_asc(players::Column::JoinedAt)
        .all(&state.db)
        .await?;
    let summaries: Vec<AgentSummary> = rows.into_iter().map(to_summary).collect();
    Ok(Json(AgentListResp { agents: summaries }).into_response())
}

pub async fn get_one(
    State(state): State<AppState>,
    claims: AccessClaims,
    Path((session_id, agent_id)): Path<(Uuid, Uuid)>,
) -> Result<Response, AgentError> {
    let user_id = parse_user_id(&claims)?;
    let _session = load_visible(&state.db, session_id, user_id).await?;

    let row = players::Entity::find_by_id(agent_id)
        .filter(players::Column::SessionIdFk.eq(session_id))
        .filter(players::Column::MetadataJson.is_not_null())
        .one(&state.db)
        .await?
        .ok_or(AgentError::NotFound)?;
    Ok(Json(to_summary(row)).into_response())
}

pub async fn delete_one(
    State(state): State<AppState>,
    claims: AccessClaims,
    Path((session_id, agent_id)): Path<(Uuid, Uuid)>,
) -> Result<Response, AgentError> {
    let user_id = parse_user_id(&claims)?;
    let session = load_visible(&state.db, session_id, user_id).await?;

    let row = players::Entity::find_by_id(agent_id)
        .filter(players::Column::SessionIdFk.eq(session.id))
        .filter(players::Column::MetadataJson.is_not_null())
        .one(&state.db)
        .await?
        .ok_or(AgentError::NotFound)?;

    let is_session_owner = session.owner_id_fk == Some(user_id);
    let is_registering_user = row.user_id_fk == Some(user_id);
    if !is_session_owner && !is_registering_user {
        return Err(AgentError::Forbidden);
    }

    if row.revoked_at.is_some() {
        return Err(AgentError::AlreadyRevoked);
    }

    let mut am: players::ActiveModel = row.into();
    am.revoked_at = Set(Some(Utc::now()));
    am.update(&state.db).await?;

    let _ = state.zmq_events_tx.send(ZmqEvent::PlayerLeave {
        join_code: session.join_code.clone(),
        player_id: agent_id,
        version: 0,
    });

    Ok(StatusCode::NO_CONTENT.into_response())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_kind_accepts_contract_set() {
        assert_eq!(validate_kind("claude-code").unwrap(), "claude-code");
        assert_eq!(validate_kind("opencode").unwrap(), "opencode");
        assert_eq!(validate_kind("custom").unwrap(), "custom");
        assert!(matches!(
            validate_kind("gemini").unwrap_err(),
            AgentError::InvalidKind
        ));
        assert!(matches!(
            validate_kind("").unwrap_err(),
            AgentError::InvalidKind
        ));
    }

    #[test]
    fn validate_bounded_trims_and_bounds() {
        assert_eq!(
            validate_bounded("  hello  ", 1, 80).as_deref(),
            Some("hello")
        );
        assert_eq!(validate_bounded("", 1, 80), None);
        assert_eq!(validate_bounded("   ", 1, 80), None);
        let too_long = "x".repeat(81);
        assert_eq!(validate_bounded(&too_long, 1, 80), None);
    }
}
