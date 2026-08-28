//! POST/GET /api/sessions/:code/players/:player_id/task-stats
//!
//! POST: the ololo CLI reports agent statistics for a completed task
//! (token usage, message counts, tool usage, skill loads). Client-reported,
//! trusted per the honest-trust model; never used for scoring.
//! GET: the player page reads all stored per-task statistics.

use crate::{
    auth::jwt::AccessClaims,
    entities::{players, sessions, task_agent_stats},
    state::AppState,
};
use arena_core::protocol::{AgentSessionStats, TaskStatsEntry, TaskStatsReport, TaskStatsResponse};
use axum::{
    Json,
    extract::{Path, State},
    response::{IntoResponse, Response},
};
use chrono::{DateTime, Utc};
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, QueryOrder, Set};
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum TaskStatsError {
    #[error("not found")]
    NotFound,
    #[error("forbidden")]
    Forbidden,
    #[error("invalid request: {0}")]
    InvalidRequest(String),
    #[error("database error: {0}")]
    DbError(#[from] sea_orm::DbErr),
    #[error("json error: {0}")]
    JsonError(#[from] serde_json::Error),
}

crate::api::error::impl_api_error!(TaskStatsError {
    Self::NotFound => (NOT_FOUND, "not_found"),
    Self::Forbidden => (FORBIDDEN, "forbidden"),
    Self::InvalidRequest(msg) => (BAD_REQUEST, "invalid_request", "detail": msg),
    Self::DbError(_) | Self::JsonError(_) => (INTERNAL_SERVER_ERROR, "internal_error"),
});

/// Resolve the session by UUID or join code (mirrors patch_player_metadata).
async fn resolve_session(
    state: &AppState,
    session_code: &str,
) -> Result<sessions::Model, TaskStatsError> {
    match Uuid::parse_str(session_code) {
        Ok(id) => sessions::Entity::find_by_id(id)
            .one(&state.db)
            .await?
            .ok_or(TaskStatsError::NotFound),
        Err(_) => sessions::Entity::find()
            .filter(sessions::Column::JoinCode.eq(session_code.to_uppercase()))
            .one(&state.db)
            .await?
            .ok_or(TaskStatsError::NotFound),
    }
}

/// Resolve the player (UUID or display name) and verify it belongs to this
/// session and this user. With `allow_admin`, admins pass the ownership
/// check (read paths only — stat reporting stays owner-only).
async fn authorize_player(
    state: &AppState,
    claims: &AccessClaims,
    session_id: Uuid,
    player_param: &str,
    allow_admin: bool,
) -> Result<players::Model, TaskStatsError> {
    let user_id = claims.user_id().map_err(|_| TaskStatsError::Forbidden)?;
    let player = crate::api::players::find_player_by_param(&state.db, session_id, player_param)
        .await?
        .ok_or(TaskStatsError::NotFound)?;
    if player.user_id_fk != Some(user_id)
        && !(allow_admin && crate::auth::is_user_admin(&state.db, user_id).await?)
    {
        return Err(TaskStatsError::NotFound);
    }
    Ok(player)
}

fn ms_to_utc(ms: Option<i64>) -> Option<DateTime<Utc>> {
    ms.and_then(DateTime::<Utc>::from_timestamp_millis)
}

pub async fn post_task_stats(
    State(state): State<AppState>,
    claims: AccessClaims,
    Path((session_code, player_param)): Path<(String, String)>,
    Json(report): Json<TaskStatsReport>,
) -> Result<Response, TaskStatsError> {
    report
        .validate()
        .map_err(|e| TaskStatsError::InvalidRequest(e.to_string()))?;
    if report.task_ordinal < 0 {
        return Err(TaskStatsError::InvalidRequest(
            "task_ordinal must be >= 0".to_string(),
        ));
    }

    let session = resolve_session(&state, &session_code).await?;
    let player = authorize_player(&state, &claims, session.id, &player_param, false).await?;
    if player.revoked_at.is_some() {
        return Err(TaskStatsError::Forbidden);
    }
    let player_id = player.id;

    // Aggregate totals across agent sessions for queryable columns.
    let mut totals = (0u64, 0u64, 0u64, 0u64, 0u64); // in, out, cr, cw, reasoning
    let mut msgs = (0u64, 0u64, 0u64); // user, assistant, tool_calls
    let mut cost_sum = 0.0f64;
    let mut has_cost = false;
    for a in &report.agents {
        totals.0 += a.input_tokens;
        totals.1 += a.output_tokens;
        totals.2 += a.cache_read_tokens;
        totals.3 += a.cache_write_tokens;
        totals.4 += a.reasoning_tokens;
        msgs.0 += a.user_messages;
        msgs.1 += a.assistant_messages;
        msgs.2 += a.tool_calls;
        if let Some(c) = a.cost {
            cost_sum += c;
            has_cost = true;
        }
    }
    let agents_json = serde_json::to_string(&report.agents)?;
    let now = Utc::now();

    // Upsert on (session, player, task_ordinal): reports may be resent.
    let existing = task_agent_stats::Entity::find()
        .filter(task_agent_stats::Column::SessionIdFk.eq(session.id))
        .filter(task_agent_stats::Column::PlayerIdFk.eq(player_id))
        .filter(task_agent_stats::Column::TaskOrdinal.eq(report.task_ordinal))
        .one(&state.db)
        .await?;

    // sea-orm `save()` would UPDATE whenever the PK is set — for a fresh
    // UUID PK that hits zero rows (RecordNotUpdated). Branch explicitly.
    let is_new = existing.is_none();
    let mut am = match existing {
        Some(model) => {
            let mut am: task_agent_stats::ActiveModel = model.into();
            am.updated_at = Set(now);
            am
        }
        None => task_agent_stats::ActiveModel {
            id: Set(Uuid::new_v4()),
            session_id_fk: Set(session.id),
            player_id_fk: Set(player_id),
            task_ordinal: Set(report.task_ordinal),
            created_at: Set(now),
            updated_at: Set(now),
            ..Default::default()
        },
    };
    am.task_id_fk = Set(report.task_id);
    am.window_started_at = Set(ms_to_utc(report.window_started_at_ms));
    am.window_ended_at = Set(ms_to_utc(report.window_ended_at_ms));
    am.input_tokens = Set(totals.0.min(i64::MAX as u64) as i64);
    am.output_tokens = Set(totals.1.min(i64::MAX as u64) as i64);
    am.cache_read_tokens = Set(totals.2.min(i64::MAX as u64) as i64);
    am.cache_write_tokens = Set(totals.3.min(i64::MAX as u64) as i64);
    am.reasoning_tokens = Set(totals.4.min(i64::MAX as u64) as i64);
    am.cost = Set(has_cost.then_some(cost_sum));
    am.user_messages = Set(msgs.0.min(i64::MAX as u64) as i64);
    am.assistant_messages = Set(msgs.1.min(i64::MAX as u64) as i64);
    am.tool_calls = Set(msgs.2.min(i64::MAX as u64) as i64);
    am.agents_json = Set(agents_json);
    if is_new {
        am.insert(&state.db).await?;
    } else {
        am.update(&state.db).await?;
    }

    Ok(Json(serde_json::json!({"status": "ok"})).into_response())
}

pub async fn get_task_stats(
    State(state): State<AppState>,
    claims: AccessClaims,
    Path((session_code, player_param)): Path<(String, String)>,
) -> Result<Json<TaskStatsResponse>, TaskStatsError> {
    let session = resolve_session(&state, &session_code).await?;
    let player = authorize_player(&state, &claims, session.id, &player_param, true).await?;
    let player_id = player.id;

    let rows = task_agent_stats::Entity::find()
        .filter(task_agent_stats::Column::SessionIdFk.eq(session.id))
        .filter(task_agent_stats::Column::PlayerIdFk.eq(player_id))
        .order_by_asc(task_agent_stats::Column::TaskOrdinal)
        .all(&state.db)
        .await?;

    let entries = rows
        .into_iter()
        .map(|row| {
            let agents: Vec<AgentSessionStats> =
                serde_json::from_str(&row.agents_json).unwrap_or_default();
            TaskStatsEntry {
                task_id: row.task_id_fk,
                task_ordinal: row.task_ordinal,
                window_started_at: row.window_started_at.map(|t| t.to_rfc3339()),
                window_ended_at: row.window_ended_at.map(|t| t.to_rfc3339()),
                input_tokens: row.input_tokens.max(0) as u64,
                output_tokens: row.output_tokens.max(0) as u64,
                cache_read_tokens: row.cache_read_tokens.max(0) as u64,
                cache_write_tokens: row.cache_write_tokens.max(0) as u64,
                reasoning_tokens: row.reasoning_tokens.max(0) as u64,
                cost: row.cost,
                user_messages: row.user_messages.max(0) as u64,
                assistant_messages: row.assistant_messages.max(0) as u64,
                tool_calls: row.tool_calls.max(0) as u64,
                agents,
                created_at: row.created_at.to_rfc3339(),
                updated_at: row.updated_at.to_rfc3339(),
            }
        })
        .collect();

    Ok(Json(TaskStatsResponse { entries }))
}
