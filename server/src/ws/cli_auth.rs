use crate::state::AppState;
use arena_core::protocol::CliAuthFrame;
use axum::extract::State;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use futures::SinkExt;
use futures::StreamExt;
use rand::RngCore;

pub async fn ws_cli_auth_handler(
    State(app): State<AppState>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Response {
    if let Some(origin) = headers.get("Origin") {
        let origin_str = origin.to_str().unwrap_or("");
        if !app.frontend_origins.iter().any(|o| o == origin_str) {
            return (
                StatusCode::FORBIDDEN,
                axum::Json(serde_json::json!({ "error": "origin_not_allowed" })),
            )
                .into_response();
        }
    }

    ws.on_upgrade(move |socket| handle_cli_auth_socket(socket, app))
}

async fn handle_cli_auth_socket(socket: WebSocket, app: AppState) {
    // Generate 32 random bytes and hex-encode to 64-char cli_token.
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    let cli_token = hex::encode(bytes);

    let (tx, mut rx) = tokio::sync::mpsc::channel::<CliAuthFrame>(1);

    app.cli_auth_sessions.insert(
        cli_token.clone(),
        crate::state::CliAuthSession {
            tx,
            created_at: std::time::Instant::now(),
        },
    );

    let (mut sink, _stream) = socket.split();

    let challenge = CliAuthFrame::CliAuthChallenge {
        cli_token: cli_token.clone(),
    };
    let json = match serde_json::to_string(&challenge) {
        Ok(s) => s,
        Err(e) => {
            tracing::error!(error = %e, "failed to serialize challenge frame");
            app.cli_auth_sessions.remove(&cli_token);
            return;
        }
    };
    if sink.send(Message::Text(json)).await.is_err() {
        app.cli_auth_sessions.remove(&cli_token);
        return;
    }

    // Spawn timeout: after 120s, remove session and send expiry error.
    let cli_token_timeout = cli_token.clone();
    let sessions_timeout = app.cli_auth_sessions.clone();
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(120)).await;
        if let Some((_, session)) = sessions_timeout.remove(&cli_token_timeout) {
            let _ = session.tx.try_send(CliAuthFrame::CliAuthError {
                code: "session_expired".to_string(),
                message: "The CLI authentication session has expired.".to_string(),
            });
        }
    });

    // Wait for the terminal frame (success or error from timeout/confirm).
    let terminal_frame = rx.recv().await;

    // Session is already removed by the confirmer or timeout task;
    // ensure cleanup in case neither ran (channel closed early).
    app.cli_auth_sessions.remove(&cli_token);

    if let Some(frame) = terminal_frame {
        let json = match serde_json::to_string(&frame) {
            Ok(s) => s,
            Err(e) => {
                tracing::error!(error = %e, "failed to serialize terminal frame");
                let _ = sink.send(Message::Close(None)).await;
                return;
            }
        };
        let _ = sink.send(Message::Text(json)).await;
    }

    let _ = sink.send(Message::Close(None)).await;
}
