//! Unauthenticated project-observer WebSocket handler.
//!
//! Mounted at `GET /ws/project/:project_id/observe`. No credentials required.
//! Forwards `ProjectSessionUpdate` frames for the given project. The broadcast
//! channel is created lazily on first subscriber connect and lives for the
//! server lifetime (projects are bounded in number).

use crate::state::AppState;
use arena_core::protocol::ArenaFrame;
use axum::extract::ws::{Message, WebSocket};
use axum::{
    extract::{Path, State, WebSocketUpgrade},
    response::IntoResponse,
};
use tokio::sync::broadcast;
use uuid::Uuid;

pub async fn ws_project_observe_handler(
    State(state): State<AppState>,
    Path(project_id): Path<Uuid>,
    ws: Result<WebSocketUpgrade, axum::extract::ws::rejection::WebSocketUpgradeRejection>,
) -> impl IntoResponse {
    let ws = match ws {
        Ok(ws) => ws,
        Err(rej) => return rej.into_response(),
    };

    // Lazily create the broadcast channel for this project if none exists yet.
    // entry() holds a shard write-lock only for the duration of the insert.
    let tx = state
        .project_registry
        .entry(project_id)
        .or_insert_with(|| {
            let (tx, _) = broadcast::channel::<ArenaFrame>(64);
            tx
        })
        .value()
        .clone();

    let rx = tx.subscribe();

    ws.on_upgrade(move |socket| handle_project_socket(socket, project_id, rx))
        .into_response()
}

async fn handle_project_socket(
    mut socket: WebSocket,
    project_id: Uuid,
    mut rx: broadcast::Receiver<ArenaFrame>,
) {
    loop {
        match rx.recv().await {
            Ok(frame) => {
                if let Ok(json) = serde_json::to_string(&frame)
                    && socket.send(Message::Text(json)).await.is_err()
                {
                    break;
                }
            }
            Err(broadcast::error::RecvError::Lagged(n)) => {
                tracing::debug!(
                    project_id = %project_id,
                    skipped = n,
                    "ws_project_observe: lagged"
                );
            }
            Err(broadcast::error::RecvError::Closed) => {
                break;
            }
        }
    }
}
