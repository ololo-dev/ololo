use crate::state::{
    GameServerState, SessionCacheInner, SessionEntry, broadcast_frame, bump_session_version,
    compute_remaining, finish_session,
};
use arena_core::entities::{projects, sessions, tasks};
use arena_core::protocol::{ArenaFrame, SessionSnapshotPayload, ZmqEvent};
use arena_core::session_status::SessionStatus;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use futures::StreamExt;
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter, Set};
use std::sync::{Arc, RwLock};
use uuid::Uuid;

pub async fn ws_cli_session_handler(
    Path(join_code): Path<String>,
    State(state): State<GameServerState>,
    ws: Result<WebSocketUpgrade, axum::extract::ws::rejection::WebSocketUpgradeRejection>,
) -> Response {
    let join_code_upper = join_code.to_uppercase();

    tracing::info!(join_code = %join_code_upper, "cli_session: WS upgrade request received");

    let (session_id, session_phase) = match resolve_session(&state, &join_code_upper).await {
        Some(s) => s,
        None => {
            tracing::warn!(join_code = %join_code_upper, server_id = %state.server_id, "cli_session: session not found or not assigned to this server");
            return StatusCode::NOT_FOUND.into_response();
        }
    };

    if session_phase == SessionStatus::Finished || session_phase == SessionStatus::Cancelled {
        tracing::warn!(join_code = %join_code_upper, session_id = %session_id, phase = %session_phase, "cli_session: session already ended");
        return StatusCode::GONE.into_response();
    }

    let ws = match ws {
        Ok(ws) => ws,
        Err(rej) => {
            tracing::warn!(join_code = %join_code_upper, "cli_session: WS upgrade rejected: {:?}", rej);
            return rej.into_response();
        }
    };

    tracing::info!(join_code = %join_code_upper, session_id = %session_id, "cli_session: WS upgrade accepted, upgrading socket");

    let jc = join_code_upper.clone();

    ws.on_upgrade(move |socket| handle_cli_socket(socket, session_id, jc, state))
        .into_response()
}

async fn resolve_session(
    state: &GameServerState,
    join_code: &str,
) -> Option<(Uuid, SessionStatus)> {
    let session = match sessions::Entity::find()
        .filter(sessions::Column::JoinCode.eq(join_code))
        .one(&state.db)
        .await
    {
        Ok(Some(s)) => s,
        Ok(None) => {
            tracing::warn!(join_code = %join_code, "cli_session: no session row for join_code");
            return None;
        }
        Err(e) => {
            tracing::error!(join_code = %join_code, error = %e, "cli_session: DB error in lookup");
            return None;
        }
    };

    match session.game_server_id {
        Some(sid) if sid == state.server_id => {}
        Some(sid) => {
            tracing::warn!(join_code = %join_code, session_id = %session.id, expected = %state.server_id, actual = %sid, "cli_session: session assigned to different game server");
            return None;
        }
        None => {
            tracing::info!(join_code = %join_code, session_id = %session.id, "cli_session: claiming unassigned session");
            let mut am: sessions::ActiveModel = session.clone().into();
            am.game_server_id = Set(Some(state.server_id));
            if let Err(e) = am.update(&state.db).await {
                tracing::error!(session_id = %session.id, error = %e, "cli_session: failed to claim session");
                return None;
            }
        }
    }

    Some((session.id, session.status))
}

async fn handle_cli_socket(
    socket: WebSocket,
    session_id: Uuid,
    join_code: String,
    state: GameServerState,
) {
    let (mut sink, mut stream) = futures::StreamExt::split(socket);

    tracing::debug!(session_id = %session_id, join_code = %join_code, "cli_session: socket opened, sending snapshot");

    let session_entry = ensure_session_entry(&state, &join_code, session_id).await;
    let broadcast_rx = session_entry.tx.subscribe();

    let cache = {
        let cache = session_entry
            .cache
            .read()
            .unwrap_or_else(|e| e.into_inner());
        SessionSnapshotPayload {
            session_id: cache.session_id,
            phase: cache.phase,
            version: cache.version,
            participants: cache.participants.clone(),
            leaderboard: cache.leaderboard.clone(),
            started_at: cache.started_at,
            timeline: None,
            activity: None,
            score_history: None,
        }
    };
    if let Ok(json) = serde_json::to_string(&ArenaFrame::SessionSnapshot(cache)) {
        let _ = futures::SinkExt::send(&mut sink, Message::Text(json)).await;
    }

    let mut broadcast_rx = broadcast_rx;
    loop {
        tokio::select! {
            result = broadcast_rx.recv() => {
                match result {
                    Ok(frame) => {
                        let json = match serde_json::to_string(&frame) {
                            Ok(s) => s,
                            Err(e) => {
                                tracing::warn!(error = %e, "cli_session: serialize failed");
                                continue;
                            }
                        };
                        if futures::SinkExt::send(&mut sink, Message::Text(json)).await.is_err() {
                            break;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        tracing::debug!(session_id = %session_id, skipped = n, "cli_session: lagged");
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
                    Some(Ok(Message::Text(_))) => {}
                    Some(Ok(_)) => {}
                    Some(Err(_)) => break,
                }
            }
        }
    }

    tracing::info!(session_id = %session_id, join_code = %join_code, "cli_session: WS disconnected");
    let _ = futures::SinkExt::close(&mut sink).await;
}

async fn ensure_session_entry(
    state: &GameServerState,
    join_code: &str,
    session_id: Uuid,
) -> Arc<SessionEntry> {
    if let Some(existing) = state.session_registry.get(join_code) {
        return Arc::new(SessionEntry {
            tx: existing.tx.clone(),
            cache: Arc::clone(&existing.cache),
            cancel: existing.cancel.clone(),
        });
    }

    let session = sessions::Entity::find_by_id(session_id)
        .one(&state.db)
        .await
        .ok()
        .flatten();

    let (tx, _) = tokio::sync::broadcast::channel::<ArenaFrame>(256);
    let cancel = tokio_util::sync::CancellationToken::new();
    let phase = session
        .as_ref()
        .map(|s| s.status)
        .unwrap_or(SessionStatus::Lobby);
    let cache = Arc::new(RwLock::new(SessionCacheInner::new(
        session_id,
        phase,
        session.and_then(|s| s.started_at),
    )));

    let entry = SessionEntry {
        tx: tx.clone(),
        cache: cache.clone(),
        cancel: cancel.clone(),
    };

    state.session_registry.insert(join_code.to_string(), entry);

    Arc::new(SessionEntry { tx, cache, cancel })
}

pub async fn lobby_timer(
    state: GameServerState,
    session_id: Uuid,
    _session_name: String,
    project_id: Uuid,
    join_code: String,
    timer_secs: u64,
) {
    let cancel = state
        .session_registry
        .get(&join_code)
        .map(|e| e.cancel.clone())
        .unwrap_or_default();

    // Anchor the countdown to the session's created_at rather than this task's
    // spawn time: the game-server lobby_timer starts when the first player
    // connects, which can be well after session creation, while the main
    // server's lobby_timer starts at creation. Without anchoring, the two
    // countdowns (TUI vs browser) diverge by the join delay.
    let lobby_remaining = match sessions::Entity::find_by_id(session_id)
        .one(&state.db)
        .await
    {
        Ok(Some(model)) => {
            let elapsed = chrono::Utc::now()
                .signed_duration_since(model.created_at)
                .num_seconds()
                .max(0) as u64;
            timer_secs.saturating_sub(elapsed)
        }
        _ => timer_secs,
    };

    for remaining in (1..=lobby_remaining).rev() {
        let version = bump_session_version(&state, &join_code);
        let frame = ArenaFrame::LobbyCountdown {
            session_id,
            seconds_remaining: remaining as u32,
            version,
        };
        broadcast_frame(&state, &join_code, frame.clone());
        state
            .event_publisher
            .publish(&ZmqEvent::SessionTimer {
                join_code: join_code.clone(),
                phase: "lobby".to_string(),
                seconds_remaining: remaining as u32,
                version,
            })
            .await;
        tokio::select! {
            _ = cancel.cancelled() => {
                tracing::info!(session_id = %session_id, join_code = %join_code, "lobby_timer: cancelled, exiting");
                return;
            }
            _ = tokio::time::sleep(std::time::Duration::from_secs(1)) => {}
        }
    }

    // Update DB to "running" BEFORE broadcasting SessionStarted.
    // This must happen first to prevent a race where ensure_session_running (called from
    // the probe loop after it receives SessionStarted) sets status="running" before
    // our own DB check, causing spawn_running to remain false and running_timer to never run.
    let now = chrono::Utc::now();
    let mut spawn_running = false;
    match sessions::Entity::find_by_id(session_id)
        .one(&state.db)
        .await
    {
        Ok(Some(model)) if model.status == SessionStatus::Lobby => {
            let mut am: sessions::ActiveModel = model.into();
            am.status = Set(SessionStatus::Running);
            am.started_at = Set(Some(now));
            match am.update(&state.db).await {
                Ok(_) => spawn_running = true,
                Err(e) => {
                    tracing::error!(session_id = %session_id, error = %e, "lobby_timer: failed to set session to running");
                }
            }
        }
        Ok(Some(model)) if model.status == SessionStatus::Running => {
            // Server's lobby_timer already won the race and transitioned the
            // session to running before our own DB write. running_timer
            // recomputes remaining from started_at every tick, so adopting
            // here is safe — no pre-reduced remaining is ever handed over.
            spawn_running = true;
            tracing::info!(
                session_id = %session_id,
                "lobby_timer: session already running (server won race), adopting",
            );
        }
        Ok(Some(_)) => {}
        Ok(None) => {
            tracing::warn!(session_id = %session_id, "lobby_timer: session not found");
        }
        Err(e) => {
            tracing::error!(session_id = %session_id, error = %e, "lobby_timer: DB error");
        }
    }

    let started_version = bump_session_version(&state, &join_code);
    // Count tasks for the project so the player agent can display
    // passed/total progress in its sidebar header.
    let total_tasks = tasks::Entity::find()
        .filter(tasks::Column::ProjectIdFk.eq(project_id))
        .count(&state.db)
        .await
        .ok();
    broadcast_frame(
        &state,
        &join_code,
        ArenaFrame::SessionStarted {
            session_id,
            version: started_version,
            total_tasks: total_tasks.map(|c| c as u32),
        },
    );
    state
        .event_publisher
        .publish(&ZmqEvent::SessionStatus {
            join_code: join_code.clone(),
            status: SessionStatus::Running.to_string(),
            version: started_version,
            cancel_reason: None,
            cancelled_by: None,
        })
        .await;

    if spawn_running {
        if let Some(entry) = state.session_registry.get(&join_code)
            && let Ok(mut cache) = entry.cache.write()
        {
            cache.phase = SessionStatus::Running;
        }
        running_timer(state, session_id, _session_name, join_code).await;
    }
}

/// Per-second running countdown. The session duration is sourced from the
/// session's project row (`projects.default_session_duration_secs`) and
/// re-read on every tick alongside the session row, so a project edit takes
/// effect within one tick on running sessions. Remaining is recomputed from
/// `started_at` on every tick (`compute_remaining`) — never carried as a
/// pre-reduced value, which would subtract elapsed time twice and skew the
/// countdown for all game-server clients.
pub async fn running_timer(
    state: GameServerState,
    session_id: Uuid,
    _session_name: String,
    join_code: String,
) {
    let cancel = state
        .session_registry
        .get(&join_code)
        .map(|e| e.cancel.clone())
        .unwrap_or_default();

    // Last computed remaining; only consulted when DB timestamps are missing
    // (defensive — Running/Paused rows always carry started_at in practice).
    let mut remaining: Option<i64> = None;

    loop {
        // Re-read DB status (and the project's configured duration) each tick
        // BEFORE broadcast+sleep.
        let (model, project) = match sessions::Entity::find_by_id(session_id)
            .find_also_related(projects::Entity)
            .one(&state.db)
            .await
        {
            Ok(Some(pair)) => pair,
            Ok(None) => {
                tracing::warn!(session_id = %session_id, "running_timer: session not found, exiting");
                return;
            }
            Err(e) => {
                tracing::error!(session_id = %session_id, error = %e, "running_timer: DB error, exiting");
                return;
            }
        };
        let duration_secs: u64 = match project {
            Some(p) => p.default_session_duration_secs.max(0) as u64,
            None => {
                // FK is NOT NULL, so this only happens if the project row was
                // deleted out from under the session.
                tracing::error!(session_id = %session_id, "running_timer: project row missing, exiting");
                return;
            }
        };

        let now = chrono::Utc::now();
        match model.status {
            SessionStatus::Finished | SessionStatus::Cancelled => {
                let cancelled = model.status == SessionStatus::Cancelled;
                tracing::info!(
                    session_id = %session_id,
                    status = %model.status,
                    "running_timer: session already ended, exiting",
                );
                let version = bump_session_version(&state, &join_code);
                // Unambiguous cancel signal for agent/TUI clients.
                if cancelled {
                    broadcast_frame(
                        &state,
                        &join_code,
                        ArenaFrame::SessionCancelled {
                            session_id,
                            version,
                            cancel_reason: None,
                            cancelled_by: None,
                        },
                    );
                }
                // Back-compat reason frame for older clients.
                broadcast_frame(
                    &state,
                    &join_code,
                    ArenaFrame::SessionComplete {
                        reason: if cancelled { "cancelled" } else { "finished" }.to_string(),
                        version,
                    },
                );
                return;
            }
            SessionStatus::Paused => {
                // Hold remaining frozen; broadcast frozen seconds; sleep 1s; re-check.
                // Compute the frozen value from DB timestamps (elapsed measured up to
                // paused_at, prior pause windows added back) rather than trusting the
                // in-memory seed, so a timer spawned mid-pause still shows the right value.
                if let (Some(started_at), Some(paused_at)) = (model.started_at, model.paused_at) {
                    remaining = Some(compute_remaining(
                        duration_secs,
                        started_at,
                        paused_at,
                        model.paused_duration_secs,
                    ));
                }
                let version = bump_session_version(&state, &join_code);
                let frozen = remaining.unwrap_or(duration_secs as i64).max(0) as u32;
                broadcast_frame(
                    &state,
                    &join_code,
                    ArenaFrame::RunningCountdown {
                        session_id,
                        seconds_remaining: frozen,
                        version,
                        paused: true,
                    },
                );
                broadcast_frame(
                    &state,
                    &join_code,
                    ArenaFrame::SessionPaused {
                        session_id,
                        seconds_remaining: frozen,
                        version,
                    },
                );
                state
                    .event_publisher
                    .publish(&ZmqEvent::SessionTimer {
                        join_code: join_code.clone(),
                        phase: "paused".to_string(),
                        seconds_remaining: frozen,
                        version,
                    })
                    .await;
                tokio::select! {
                    _ = cancel.cancelled() => {
                        tracing::info!(session_id = %session_id, "running_timer: cancelled while paused, exiting");
                        return;
                    }
                    _ = tokio::time::sleep(std::time::Duration::from_secs(1)) => {}
                }
                continue;
            }
            SessionStatus::Running => {
                // Resume arithmetic: recompute remaining from DB timestamps so a
                // pause window is subtracted correctly. i64 clamp max(0, ...).
                if let Some(started_at) = model.started_at {
                    remaining = Some(compute_remaining(
                        duration_secs,
                        started_at,
                        now,
                        model.paused_duration_secs,
                    ));
                }
                let secs = remaining.unwrap_or(duration_secs as i64);
                if secs <= 0 {
                    // Transition to Finished via the normal completion path below.
                    break;
                }
                let version = bump_session_version(&state, &join_code);
                let frame = ArenaFrame::RunningCountdown {
                    session_id,
                    seconds_remaining: secs as u32,
                    version,
                    paused: false,
                };
                broadcast_frame(&state, &join_code, frame.clone());
                state
                    .event_publisher
                    .publish(&ZmqEvent::SessionTimer {
                        join_code: join_code.clone(),
                        phase: "running".to_string(),
                        seconds_remaining: secs as u32,
                        version,
                    })
                    .await;
                tokio::select! {
                    _ = cancel.cancelled() => {
                        tracing::info!(session_id = %session_id, "running_timer: cancelled, exiting");
                        let version = bump_session_version(&state, &join_code);
                        broadcast_frame(
                            &state,
                            &join_code,
                            ArenaFrame::SessionComplete {
                                reason: "cancelled".to_string(),
                                version,
                            },
                        );
                        return;
                    }
                    _ = tokio::time::sleep(std::time::Duration::from_secs(1)) => {}
                }
                remaining = Some(secs - 1);
                continue;
            }
            SessionStatus::Lobby => {
                // Unexpected in running_timer, but guard: treat as exit.
                tracing::warn!(
                    session_id = %session_id,
                    "running_timer: session in Lobby, exiting",
                );
                return;
            }
        }
    }

    finish_session(&state, session_id, &join_code, "time_expired").await;

    state.session_registry.remove(&join_code);
}
