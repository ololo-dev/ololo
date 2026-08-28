//! Internal endpoint serving the on-disk player event log to the main
//! server.
//!
//! The main server's admin event-log API proxies here because the JSONL
//! files live on the game-server's filesystem (the two processes share a
//! database, not a disk).

use axum::extract::Path;
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use uuid::Uuid;

use crate::session_log_store;

/// `GET /internal/session-logs/:session_id/:player_id` — the player's full
/// event timeline (their own events merged with the session-level ones) as
/// raw JSONL, or 404 when no events were recorded for the player.
pub async fn get_player_log(Path((session_id, player_id)): Path<(Uuid, Uuid)>) -> Response {
    match session_log_store::read_player_log(session_log_store::base_dir(), session_id, player_id)
        .await
    {
        Some(raw) => ([(header::CONTENT_TYPE, "application/x-ndjson")], raw).into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}
