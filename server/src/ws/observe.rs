//! Unauthenticated public-observer WebSocket handler.
//!
//! Mounted at `GET /ws/s/:join_code/observe`. No credentials required.
//! Sends a `SessionSnapshot` on connect, then forwards a filtered subset of
//! broadcast frames for the duration of the connection. No `session_members`
//! row is created, and no `MemberJoined`/`MemberDisconnected` is emitted for
//! observer connect/disconnect.

use crate::state::AppState;
use arena_core::protocol::ArenaFrame;
use axum::extract::ws::{Message, WebSocket};
use axum::{
    extract::{Path, State, WebSocketUpgrade},
    http::StatusCode,
    response::IntoResponse,
};

pub async fn ws_observe_handler(
    State(state): State<AppState>,
    Path(join_code): Path<String>,
    ws: Result<WebSocketUpgrade, axum::extract::ws::rejection::WebSocketUpgradeRejection>,
) -> impl IntoResponse {
    let join_code_upper = join_code.to_uppercase();

    // Pre-upgrade: return 404 if session not found (no WS required).
    if !state.session_registry.contains_key(&join_code_upper) {
        return (StatusCode::NOT_FOUND, "session not found").into_response();
    }

    let ws = match ws {
        Ok(ws) => ws,
        Err(rej) => return rej.into_response(),
    };

    ws.on_upgrade(move |socket| handle_observe_socket(socket, join_code_upper, state))
        .into_response()
}

async fn handle_observe_socket(socket: WebSocket, join_code: String, state: AppState) {
    // Re-look-up the entry; it may have been evicted between upgrade and handler.
    let (snapshot, mut rx) = match state.session_registry.get(&join_code) {
        Some(entry) => {
            let snapshot =
                crate::ws::session::build_session_snapshot(&state.db, &entry.cache).await;
            let rx = entry.tx.subscribe();
            (snapshot, rx)
        }
        None => return,
    };

    let (mut sink, mut stream) = futures::StreamExt::split(socket);

    // Send the initial snapshot.
    if let Ok(json) = serde_json::to_string(&ArenaFrame::SessionSnapshot(snapshot))
        && futures::SinkExt::send(&mut sink, Message::Text(json))
            .await
            .is_err()
    {
        return;
    }

    // Forward allowed broadcast frames. The receive half is polled too so client
    // Pings are auto-Ponged (idle observers survive proxies) and Close is
    // observed — otherwise the task and its broadcast::Receiver slot leaked until
    // the next frame arrived (never, for an idle finished session).
    loop {
        tokio::select! {
            inbound = futures::StreamExt::next(&mut stream) => {
                match inbound {
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Ok(_)) => {}
                    Some(Err(_)) => break,
                }
            }
            frame = rx.recv() => {
                match frame {
                    Ok(frame) => {
                        if !is_public_frame(&frame) {
                            continue;
                        }
                        if let Ok(json) = serde_json::to_string(&frame)
                            && futures::SinkExt::send(&mut sink, Message::Text(json))
                                .await
                                .is_err()
                        {
                            break;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        tracing::debug!(join_code = %join_code, skipped = n, "ws_observe: lagged");
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                        break;
                    }
                }
            }
        }
    }
}

fn is_public_frame(frame: &ArenaFrame) -> bool {
    matches!(
        frame,
        ArenaFrame::SessionSnapshot(_)
            | ArenaFrame::LeaderboardUpdate { .. }
            | ArenaFrame::SessionStarted { .. }
            | ArenaFrame::SessionComplete { .. }
            | ArenaFrame::MemberJoined { .. }
            | ArenaFrame::PlayerDisconnected { .. }
            | ArenaFrame::LobbyCountdown { .. }
            | ArenaFrame::RunningCountdown { .. }
            | ArenaFrame::PlayerProgressUpdate { .. }
            | ArenaFrame::TaskStarted { .. }
            | ArenaFrame::TaskScored { .. }
            | ArenaFrame::Heartbeat
    )
}
