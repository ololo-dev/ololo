//! Resolve + reconnect-loop helpers for the player-agent WebSocket.

use anyhow::Result;
use std::time::Duration;

use super::wire::{ResolveError, ResolveResponse};
use super::{BACKOFF_CAP_MS, BACKOFF_MULTIPLIER, SinkArg, connect_once, emit};
use crate::tui::event::{LogLevel, TuiEvent};
use crate::util;
use uuid::Uuid;

/// Call GET /api/sessions/resolve?join_code=<code> to discover the game server URL.
///
/// Returns:
/// - `Ok((url, Some(player_id)))` on success — the game server WS base URL and viewer player_id
/// - `Ok((url, None))` on 404 fallback (direct connection, no resolve payload)
/// - `Err(Retry)` on 503 (no game server assigned yet) — caller should retry resolve
/// - `Err(Fatal)` on 401/403/410 or parse error — caller should abort
pub async fn resolve_session(
    base_url: &str,
    join_code: &str,
    pat: &str,
) -> Result<(String, Option<String>), ResolveError> {
    let http_base = util::http_base_url(base_url);

    let url = format!("{http_base}/api/sessions/resolve?join_code={join_code}");

    let client = reqwest::Client::new();
    let resp = client
        .get(&url)
        .header("X-API-Key", pat)
        .send()
        .await
        .map_err(|e| ResolveError::Retry(format!("resolve request failed: {e}")))?;

    let status = resp.status();
    if status.is_success() {
        let body: ResolveResponse = resp
            .json()
            .await
            .map_err(|e| ResolveError::Fatal(format!("parsing resolve response: {e}")))?;
        Ok((body.game_server_url, Some(body.player_id)))
    } else if status == reqwest::StatusCode::SERVICE_UNAVAILABLE {
        Err(ResolveError::Retry(
            "no game server assigned yet".to_string(),
        ))
    } else if status == reqwest::StatusCode::NOT_FOUND {
        tracing::warn!("resolve endpoint returned 404, falling back to direct connection");
        let ws_base = util::ws_base_url(base_url);
        Ok((ws_base, None))
    } else if status == reqwest::StatusCode::UNAUTHORIZED
        || status == reqwest::StatusCode::FORBIDDEN
    {
        Err(ResolveError::Fatal(format!("auth error: {}", status)))
    } else if status == reqwest::StatusCode::GONE {
        Err(ResolveError::Fatal("session has ended".to_string()))
    } else {
        Err(ResolveError::Retry(format!(
            "unexpected status: {}",
            status
        )))
    }
}

/// Connect to `ws_url`, retrying with exponential backoff until either
/// `SessionComplete` is received (returns `true`), the user presses Ctrl-C
/// (returns `false`), or the attempt budget (`max_attempts`) is exhausted.
///
/// `backoff_ms` is both read (to size the first wait) and updated in place
/// (so a caller that re-enters after a re-resolve continues the schedule).
pub async fn run_connect_loop(
    ws_url: &str,
    pat: &str,
    sink: SinkArg,
    viewer_player_id: Option<Uuid>,
    backoff_ms: &mut u64,
    max_attempts: Option<u64>,
    memory: &mut Option<super::MemoryChannel>,
) -> bool {
    let mut attempts: u64 = 0;
    loop {
        match connect_once(ws_url, pat, sink.clone(), viewer_player_id, memory.as_mut()).await {
            Ok(true) => return true,
            Ok(false) => {
                // The socket had been established and then dropped — a
                // restart, not a dead endpoint. Start the backoff schedule
                // over so the reconnect is quick instead of inheriting the
                // 30s cap from some earlier outage.
                *backoff_ms = super::BACKOFF_INITIAL_MS;
                crate::ui::hint(format!("Disconnected. Reconnecting in {backoff_ms} ms…"));
                emit(
                    sink.clone(),
                    TuiEvent::Log {
                        level: LogLevel::Hint,
                        msg: format!("Disconnected. Reconnecting in {backoff_ms} ms…"),
                    },
                );
            }
            Err(e) => {
                crate::ui::hint(format!(
                    "Connection error: {e}. Reconnecting in {backoff_ms} ms…"
                ));
                emit(
                    sink.clone(),
                    TuiEvent::Log {
                        level: LogLevel::Warn,
                        msg: format!("Connection error: {e}. Reconnecting in {backoff_ms} ms…"),
                    },
                );
            }
        }

        if let Some(max) = max_attempts
            && attempts >= max
        {
            return false;
        }

        tokio::select! {
            _ = tokio::time::sleep(Duration::from_millis(*backoff_ms)) => {}
            _ = tokio::signal::ctrl_c() => {
                crate::ui::hint("Interrupted.");
                return false;
            }
        }

        *backoff_ms = ((*backoff_ms as f64 * BACKOFF_MULTIPLIER) as u64).min(BACKOFF_CAP_MS);
        attempts += 1;
    }
}
