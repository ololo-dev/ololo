use crate::api::players::build_snapshot;
use crate::auth::jwt::verify_access_token;
use crate::state::{AppState, PlayerChannel};
use arena_core::entities::{players, sessions, users};
use arena_core::protocol::PlayerFrame;
use arena_core::session_status::SessionStatus;
use axum::extract::ws::{Message, WebSocket};
use axum::{
    extract::{Path, Query, State, WebSocketUpgrade},
    http::StatusCode,
    response::IntoResponse,
};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use serde::Deserialize;
use std::sync::Arc;
use uuid::Uuid;

#[derive(Debug, Deserialize)]
pub struct PlayerWsParams {
    pub session_code: String,
    pub token: String,
}

pub async fn ws_player_handler(
    Path(player_id): Path<Uuid>,
    Query(params): Query<PlayerWsParams>,
    State(state): State<AppState>,
    ws: WebSocketUpgrade,
) -> impl IntoResponse {
    // (1) Verify JWT
    let claims = match verify_access_token(&state.jwt_decoding_key, &params.token) {
        Ok(c) => c,
        Err(_) => return StatusCode::UNAUTHORIZED.into_response(),
    };

    // (2) Parse user_id from claims
    let user_id: Uuid = match claims.sub.parse() {
        Ok(u) => u,
        Err(_) => return StatusCode::UNAUTHORIZED.into_response(),
    };

    // (3) Load user from DB to determine admin status
    let user = match users::Entity::find_by_id(user_id).one(&state.db).await {
        Ok(Some(u)) => u,
        Ok(None) => return StatusCode::FORBIDDEN.into_response(),
        Err(e) => {
            tracing::error!(error = %e, "ws_player_handler: DB error looking up user");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    // (4) Load player from DB
    let player = match players::Entity::find_by_id(player_id).one(&state.db).await {
        Ok(Some(p)) => p,
        Ok(None) => return StatusCode::NOT_FOUND.into_response(),
        Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    };

    // (5) Verify the player belongs to the authenticated user (admins bypass ownership)
    if !user.is_admin && player.user_id_fk != Some(user_id) {
        return StatusCode::FORBIDDEN.into_response();
    }

    // (6) Look up session by code in DB (registry can be empty after restart).
    let session_code_upper = params.session_code.to_uppercase();
    let session = match sessions::Entity::find()
        .filter(sessions::Column::JoinCode.eq(session_code_upper.clone()))
        .one(&state.db)
        .await
    {
        Ok(Some(s)) => s,
        Ok(None) => return StatusCode::NOT_FOUND.into_response(),
        Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    };
    let session_id = session.id;

    // (7) Check session phase. A cancelled session is done. A finished session
    // is normally done too — EXCEPT while its judges are still settling: judge
    // verdicts publish AFTER the session finishes, so closing the socket here
    // is exactly why they never reached the page in realtime. Keep it open
    // until every expected judge row exists (the same signal the award settle
    // poll waits on), then return GONE like any closed session.
    if session.status == SessionStatus::Cancelled {
        return StatusCode::GONE.into_response();
    }
    if session.status == SessionStatus::Finished {
        let judges_pending =
            arena_core::session_completion::expired_session_pending_judges(&state.db, session_id)
                .await
                .unwrap_or(0);
        if judges_pending == 0 {
            return StatusCode::GONE.into_response();
        }
    }

    // (8) Verify the player belongs to this session
    if player.session_id_fk != session_id {
        return StatusCode::FORBIDDEN.into_response();
    }

    let display_name = player.display_name.clone();

    ws.on_upgrade(move |socket| {
        handle_player_socket(
            socket,
            player_id,
            session_id,
            session_code_upper,
            display_name,
            state,
        )
    })
    .into_response()
}

async fn handle_player_socket(
    mut socket: WebSocket,
    player_id: Uuid,
    session_id: Uuid,
    join_code: String,
    display_name: String,
    state: AppState,
) {
    // Newest connection wins: register our own channel, superseding any prior
    // connection for this player. Previously the handler reused a shared channel
    // and did `if !contains_key { insert }` then `.get().unwrap()` — a TOCTOU
    // panic if another handler removed in between, and its unconditional remove
    // on teardown (below) let an old handler delete a live reconnect's channel,
    // cutting that connection's JudgeScored/ScoreRankUpdated delivery. The client
    // re-syncs from the fresh snapshot below, so resetting the per-connection
    // sequence counter on reconnect is fine.
    let channel = {
        use tokio::sync::broadcast;
        let (tx, _) = broadcast::channel::<PlayerFrame>(64);
        let channel = Arc::new(PlayerChannel::new(tx));
        state.player_registry.insert(player_id, channel.clone());
        channel
    };

    // Build snapshot from DB, capturing last_seq from channel
    let last_seq = channel.seq.load(std::sync::atomic::Ordering::SeqCst);
    let snapshot_payload =
        build_snapshot(&state, player_id, session_id, &display_name, last_seq).await;

    if let Ok(json) = serde_json::to_string(&PlayerFrame::PlayerSnapshot(snapshot_payload))
        && socket.send(Message::Text(json)).await.is_err()
    {
        return;
    }

    let db_poll_handle = tokio::spawn(db_poll_task(
        state.db.clone(),
        channel.clone(),
        player_id,
        session_id,
        state.session_registry.clone(),
        state.player_registry.clone(),
        join_code.clone(),
    ));

    let mut rx = channel.subscribe();
    // Subscribe to ZMQ session-status events so the player WS learns about
    // pause/resume/cancel transitions that originate in game-server.
    let mut zmq_rx = state.zmq_events_tx.subscribe();

    loop {
        tokio::select! {
            msg = socket.recv() => {
                match msg {
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Ok(Message::Text(_))) => {}
                    _ => {}
                }
            }
            frame_result = rx.recv() => {
                match frame_result {
                    Ok(frame) => {
                        let json = serde_json::to_string(&frame).unwrap_or_default();
                        if socket.send(Message::Text(json)).await.is_err() {
                            break;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                        let _ = socket.send(Message::Close(None)).await;
                        break;
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                }
            }
            zmq_event = zmq_rx.recv() => {
                if let Ok(event) = zmq_event {
                    use arena_core::protocol::ZmqEvent;
                    use arena_core::session_status::SessionStatus;
                    // Only forward events for this session.
                    let mine = match &event {
                        ZmqEvent::SessionStatus { join_code: jc, .. } => jc == &join_code,
                        _ => false,
                    };
                    if !mine { continue; }
                    if let ZmqEvent::SessionStatus { status, cancel_reason, cancelled_by, .. } = event {
                        let new_status = match status.parse::<SessionStatus>() {
                            Ok(s) => s,
                            Err(e) => {
                                tracing::warn!("dropping SessionStatus for {join_code}: {e}");
                                continue;
                            }
                        };
                        let frame = PlayerFrame::SessionStatusChange {
                            status: new_status,
                            cancel_reason,
                            cancelled_by,
                        };
                        let json = serde_json::to_string(&frame).unwrap_or_default();
                        if socket.send(Message::Text(json)).await.is_err() {
                            break;
                        }
                    }
                }
            }
        }
    }

    db_poll_handle.abort();
    // Remove only while we are still the registered connection: a newer reconnect
    // may have superseded us, and deleting its channel would silently cut its
    // frame delivery (the game-server's RegistryGuard::remove_if does the same).
    state
        .player_registry
        .remove_if(&player_id, |_, current| Arc::ptr_eq(current, &channel));
}

mod poll;
use poll::db_poll_task;

#[cfg(test)]
mod tests;
