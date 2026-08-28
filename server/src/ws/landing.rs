//! Unauthenticated landing-page observer WebSocket handler.
//!
//! Mounted at `GET /ws/landing/observe`. No credentials required. Forwards
//! `ProjectSessionUpdate` and countdown frames for sessions of PUBLIC
//! projects only — the fan-out sites enforce the public check before
//! publishing to `AppState::landing_tx`, so this handler is a plain relay.

use crate::state::AppState;
use axum::extract::ws::{Message, WebSocket};
use axum::{
    extract::{State, WebSocketUpgrade},
    response::IntoResponse,
};
use tokio::sync::broadcast;

pub async fn ws_landing_observe_handler(
    State(state): State<AppState>,
    ws: Result<WebSocketUpgrade, axum::extract::ws::rejection::WebSocketUpgradeRejection>,
) -> impl IntoResponse {
    let ws = match ws {
        Ok(ws) => ws,
        Err(rej) => return rej.into_response(),
    };
    let rx = state.landing_tx.subscribe();
    ws.on_upgrade(move |socket| handle_landing_socket(socket, rx))
        .into_response()
}

async fn handle_landing_socket(
    mut socket: WebSocket,
    mut rx: broadcast::Receiver<arena_core::protocol::ArenaFrame>,
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
                tracing::debug!(skipped = n, "ws_landing_observe: lagged");
            }
            Err(broadcast::error::RecvError::Closed) => break,
        }
    }
}
