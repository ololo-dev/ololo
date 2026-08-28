use crate::config::Config;
use crate::ui;
use futures_util::{SinkExt, StreamExt};
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tokio::time::timeout;
use tokio_tungstenite::{connect_async, tungstenite::Message};

/// Wire frames for CLI authentication — mirrors the server protocol.
/// Variants use explicit serde renames to match the server's snake_case discriminants.
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
enum CliAuthFrame {
    #[serde(rename = "cli_auth_challenge")]
    Challenge { cli_token: String },
    #[serde(rename = "cli_auth_success")]
    Success { token: String },
    #[serde(rename = "cli_auth_error")]
    Error { code: String, message: String },
}

/// Run the `ololo login` WebSocket device-flow. With `no_browser` the
/// authorization URL is printed but not opened — scripted logins parse the
/// `cli_token` from the output and confirm via `POST /auth/cli/confirm`.
pub async fn run_login(server_url: String, profile: &str, no_browser: bool) -> anyhow::Result<()> {
    // 1. Convert http(s) to ws(s)
    let ws_base = server_url
        .trim_end_matches('/')
        .replace("https://", "wss://")
        .replace("http://", "ws://");
    let ws_url = format!("{}/ws/cli-auth", ws_base);

    // 2. Connect via WebSocket
    ui::step(format!("Connecting to {server_url}..."));
    let (ws_stream, _) = connect_async(&ws_url)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to connect: {e}"))?;

    let (mut write, mut read) = ws_stream.split();

    // 3. Read first frame — must be CliAuthChallenge
    let challenge = match timeout(Duration::from_secs(30), read.next()).await {
        Err(_elapsed) => ui::fatal("Timed out waiting for server. Please try again."),
        Ok(None) => anyhow::bail!("Server closed connection unexpectedly"),
        Ok(Some(msg)) => msg.map_err(|e| anyhow::anyhow!("WebSocket error: {e}"))?,
    };

    let frame: CliAuthFrame = match challenge {
        Message::Text(s) => serde_json::from_str(&s)
            .map_err(|e| anyhow::anyhow!("Failed to parse server frame: {e}"))?,
        other => anyhow::bail!("Unexpected message type: {other:?}"),
    };

    let cli_token = match frame {
        CliAuthFrame::Challenge { cli_token } => cli_token,
        _ => anyhow::bail!("Expected challenge frame, got something else"),
    };

    // Built from the server URL we already resolved, not a server-supplied
    // value — otherwise the server would need to independently guess its own
    // public domain, which drifts across deployments (e.g. preview envs).
    let auth_url = format!(
        "{}/cli-auth?cli_token={}",
        server_url.trim_end_matches('/'),
        cli_token
    );

    // 4. Display URL and (unless suppressed) open the browser
    if no_browser {
        ui::step("Authorize by opening the URL below (browser launch suppressed):");
    } else {
        ui::step("Opening your browser to authorize...");
    }
    ui::url(&auth_url);
    ui::hint("To authorize manually, open the URL above in your browser.");
    if !no_browser {
        let _ = open::that(&auth_url); // best-effort, ignore error
    }

    // 5. Wait up to 120 s for CliAuthSuccess or CliAuthError
    ui::waiting("Waiting for you to authorize (up to 2 minutes)...");
    let result_msg = match timeout(Duration::from_secs(120), read.next()).await {
        Err(_elapsed) => ui::fatal("Timed out waiting for authorization. Please try again."),
        Ok(None) => anyhow::bail!("Server closed connection without completing authentication"),
        Ok(Some(msg)) => msg.map_err(|e| anyhow::anyhow!("WebSocket error: {e}"))?,
    };

    let result_frame: CliAuthFrame = match result_msg {
        Message::Text(s) => serde_json::from_str(&s)
            .map_err(|e| anyhow::anyhow!("Failed to parse server response: {e}"))?,
        other => anyhow::bail!("Unexpected message type: {other:?}"),
    };

    match result_frame {
        CliAuthFrame::Success { token } => {
            let cfg = Config {
                token,
                server_url: server_url.clone(),
            };
            if let Err(e) = cfg.save(profile) {
                ui::warn(format!("Failed to save credentials: {e}"));
            }
            // 6. Send WS Close frame
            let _ = write.send(Message::Close(None)).await;
            if profile == "default" {
                ui::success(format!("Logged in to {server_url}"));
            } else {
                ui::success(format!("Logged in to {server_url} (profile: {profile})"));
            }
            Ok(())
        }
        CliAuthFrame::Error { code, message } => {
            ui::fatal(format!("Authorization failed ({code}): {message}"));
        }
        CliAuthFrame::Challenge { .. } => {
            anyhow::bail!("Unexpected challenge frame received after initial challenge")
        }
    }
}

/// Validate the stored token against the server.
///
/// Returns `Ok(())` when the token is accepted (HTTP 2xx), the server is
/// unreachable, or the request otherwise fails (network errors must not block
/// the caller — the real command will surface those on its own).
///
/// Returns `Err` only when the server explicitly rejects the token (HTTP 401
/// or 403), with a message that tells the user to re-login.
pub async fn validate_token(server_url: &str, token: &str) -> anyhow::Result<()> {
    let base = server_url.trim_end_matches('/');
    let url = format!("{base}/api/users/me");
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap_or_else(|_| reqwest::Client::new());
    match client.get(&url).bearer_auth(token).send().await {
        Ok(resp)
            if resp.status() == StatusCode::UNAUTHORIZED
                || resp.status() == StatusCode::FORBIDDEN =>
        {
            anyhow::bail!("token expired or revoked — run 'ololo login' to re-authenticate")
        }
        _ => Ok(()),
    }
}
