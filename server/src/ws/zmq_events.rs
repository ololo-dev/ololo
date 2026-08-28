use crate::auth::jwt::verify_access_token;
use crate::state::AppState;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use futures::StreamExt;
use sea_orm::EntityTrait;
use serde::Deserialize;

#[derive(Deserialize)]
pub struct ZmqEventStreamParams {
    pub token: Option<String>,
}

pub async fn ws_zmq_events_handler(
    State(state): State<AppState>,
    Query(params): Query<ZmqEventStreamParams>,
    ws: Result<WebSocketUpgrade, axum::extract::ws::rejection::WebSocketUpgradeRejection>,
) -> Response {
    let token = match params.token {
        Some(ref t) if !t.is_empty() => t.clone(),
        _ => return StatusCode::FORBIDDEN.into_response(),
    };

    let claims = match verify_access_token(&state.jwt_decoding_key, &token) {
        Ok(c) => c,
        Err(_) => return StatusCode::FORBIDDEN.into_response(),
    };

    let user_id = match claims.user_id() {
        Ok(id) => id,
        Err(_) => return StatusCode::FORBIDDEN.into_response(),
    };

    let user = match arena_core::entities::users::Entity::find_by_id(user_id)
        .one(&state.db)
        .await
    {
        Ok(Some(u)) => u,
        Ok(None) => return StatusCode::FORBIDDEN.into_response(),
        Err(e) => {
            tracing::error!(error = %e, "ws_zmq_events: DB error");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    if !user.is_admin {
        return StatusCode::FORBIDDEN.into_response();
    }

    let ws = match ws {
        Ok(ws) => ws,
        Err(rej) => return rej.into_response(),
    };

    ws.on_upgrade(move |socket| handle_zmq_event_stream(socket, state))
        .into_response()
}

async fn handle_zmq_event_stream(socket: WebSocket, state: AppState) {
    let (mut sink, mut stream) = futures::StreamExt::split(socket);
    let mut rx = state.zmq_events_tx.subscribe();
    tracing::info!("admin ZMQ event stream client connected");

    loop {
        tokio::select! {
            result = rx.recv() => {
                match result {
                    Ok(event) => {
                        let json = match serde_json::to_string(&event) {
                            Ok(j) => j,
                            Err(e) => {
                                tracing::warn!("zmq-event-stream: serialize failed: {e}");
                                continue;
                            }
                        };
                        if futures::SinkExt::send(&mut sink, Message::Text(json)).await.is_err() {
                            break;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        tracing::debug!("zmq-event-stream: lagged {n} frames");
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
            msg = stream.next() => {
                match msg {
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Ok(Message::Ping(data))) => {
                        let _ = futures::SinkExt::send(&mut sink, Message::Pong(data)).await;
                    }
                    Some(Ok(Message::Text(t))) => {
                        tracing::debug!("zmq-event-stream: client sent: {t:?}");
                    }
                    Some(Ok(_)) => {}
                    Some(Err(_)) => break,
                }
            }
        }
    }

    let _ = futures::SinkExt::close(&mut sink).await;
}
