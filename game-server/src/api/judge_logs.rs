//! Internal endpoint serving on-disk judge run logs to the main server.
//!
//! The main server's admin judge-details API proxies here because the log
//! files live on the game-server's filesystem (the two processes share a
//! database, not a disk).

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use sea_orm::EntityTrait;
use uuid::Uuid;

use arena_core::entities::{players, sessions};

use crate::judge_log_store;
use crate::state::GameServerState;

/// `GET /internal/judge-logs/:session_id/:player_id/:task_id` — the full
/// task log file (JSON object keyed by judge slug), or 404 when absent.
pub async fn get_task_log(
    State(state): State<GameServerState>,
    Path((session_id, player_id, task_id)): Path<(Uuid, Uuid, Uuid)>,
) -> Response {
    let Ok(Some(session)) = sessions::Entity::find_by_id(session_id)
        .one(&state.db)
        .await
    else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let Ok(Some(player)) = players::Entity::find_by_id(player_id).one(&state.db).await else {
        return StatusCode::NOT_FOUND.into_response();
    };
    match judge_log_store::read_task_log(
        judge_log_store::base_dir(),
        &session.join_code,
        &player.display_name,
        task_id,
    )
    .await
    {
        Some(doc) => Json(doc).into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}
