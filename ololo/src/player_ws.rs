//! Player-agent WebSocket loop for the `ololo join` command.
//!
//! Connects to `/ws/player/agent/{join_code}`, receives `PlayerAgentFrame`
//! probes, executes them under a shell, and returns `PlayerAgentClientFrame`
//! results.
//!
//! Before connecting, calls the resolve endpoint to discover the game server URL.
//!
//! When called with a `Some(sink)` argument, every `ui::*` call has a
//! parallel `sink.send(Origin::Network, TuiEvent::…)` emission. The
//! text-mode path (no sink) is byte-for-byte identical; the TUI path
//! consumes the events on the render side.

use crate::util;
use anyhow::{Context, Result};
use futures_util::{SinkExt, StreamExt};
use std::sync::Arc;
use std::time::Duration;
use tokio_tungstenite::{
    connect_async,
    tungstenite::{Message, client::IntoClientRequest, http::header::HeaderName},
};
use uuid::Uuid;

use crate::permissions;
use crate::tui::event::{
    EventSink, HeaderDelta, LogLevel, Origin, PermissionPrompt, ProbeInfo, ProbeResultInfo,
    TuiEvent,
};
use wire::ResolveError;

/// Maximum stdout bytes captured and reported back (matches server constant).
const STDOUT_TAIL_MAX_BYTES: usize = 64 * 1024;

/// Reconnect backoff parameters: start 1 s, ×2, cap 30 s.
const BACKOFF_INITIAL_MS: u64 = 1_000;
const BACKOFF_MULTIPLIER: f64 = 2.0;
const BACKOFF_CAP_MS: u64 = 30_000;

mod connect;
mod shell;
#[cfg(test)]
mod tests;
mod wire;

// ---- Public entry point ----

/// Optional sink parameter: when `Some`, every progress event is
/// also published as a `TuiEvent` for the TUI's bus consumer.
/// When `None`, the original text-mode `ui::*` calls run unchanged.
pub type SinkArg = Option<Arc<dyn EventSink>>;

fn emit(sink: SinkArg, ev: TuiEvent) {
    if let Some(s) = sink {
        let _ = s.send(Origin::Network, ev);
    }
}

/// Agent WS URL for a session. Carries our `player_id` (learned from the
/// resolve endpoint) so the game server drives OUR player row — without it,
/// multi-player sessions resolve every socket to one arbitrary player.
fn agent_ws_url(ws_base: &str, join_code: &str, player_id: Option<uuid::Uuid>) -> String {
    match player_id {
        Some(pid) => format!("{ws_base}/ws/player/agent/{join_code}?player_id={pid}"),
        None => format!("{ws_base}/ws/player/agent/{join_code}"),
    }
}

/// Run the player-agent WS loop with automatic exponential-backoff reconnects.
///
/// First calls the resolve endpoint to discover the game server URL,
/// then connects to the game server's player-agent WebSocket.
/// Returns when a `SessionComplete` frame is received or `Ctrl-C` is pressed.
pub async fn run(base_url: &str, join_code: &str, pat: &str) -> Result<()> {
    run_with_sink(base_url, join_code, pat, None, None).await
}

/// Same as [`run`] but with an optional `EventSink` for the TUI bus.
/// When `sink` is `Some`, every progress event is published as a
/// `TuiEvent`; the text-mode `ui::*` calls continue to run.
pub async fn run_with_sink(
    base_url: &str,
    join_code: &str,
    pat: &str,
    sink: SinkArg,
    memory: Option<MemoryChannel>,
) -> Result<()> {
    let mut memory = memory;
    // Resolve → connect, forever. A lost game server used to end the CLI —
    // five WS attempts, then ONE re-resolve whose retryable failure (503
    // while the server re-registers, a network blip) was treated as fatal.
    // The player's process exited mid-session and the idle sweep cancelled
    // the whole run minutes later. Now every exhausted connect cycle goes
    // back to resolve (which itself retries), so the loop only ends when the
    // session completes, the server says something genuinely fatal (auth,
    // session gone), or the player interrupts.
    let mut backoff_ms = BACKOFF_INITIAL_MS;
    let max_ws_attempts = 5;
    let mut cycles: u64 = 0;
    loop {
        let (game_server_url, player_id_raw) = loop {
            match connect::resolve_session(base_url, join_code, pat).await {
                Ok((url, pid)) => break (url, pid),
                Err(ResolveError::Retry(msg)) => {
                    crate::ui::hint(format!("Resolve: {}. Retrying in 3s…", msg));
                    emit(
                        sink.clone(),
                        TuiEvent::Log {
                            level: LogLevel::Hint,
                            msg: format!("Resolve: {msg}. Retrying in 3s…"),
                        },
                    );
                    tokio::select! {
                        _ = tokio::time::sleep(Duration::from_secs(3)) => {}
                        _ = tokio::signal::ctrl_c() => {
                            crate::ui::hint("Interrupted.");
                            emit(sink.clone(), TuiEvent::Log { level: LogLevel::Hint, msg: "Interrupted.".into() });
                            return Ok(());
                        }
                    }
                }
                Err(ResolveError::Fatal(msg)) => {
                    anyhow::bail!("Resolve fatal error: {}", msg);
                }
            }
        };

        let viewer_player_id = player_id_raw.as_deref().and_then(parse_player_id);
        if let Some(vid) = viewer_player_id {
            emit(sink.clone(), TuiEvent::ViewerIdentified(vid));
        }

        let full_ws_url = agent_ws_url(
            &util::ws_base_url(&game_server_url),
            join_code,
            viewer_player_id,
        );
        tracing::info!("resolved game server: {}", game_server_url);
        tracing::info!("ws url: {}", full_ws_url);

        if connect::run_connect_loop(
            &full_ws_url,
            pat,
            sink.clone(),
            viewer_player_id,
            &mut backoff_ms,
            Some(max_ws_attempts),
            &mut memory,
        )
        .await
        {
            return Ok(());
        }

        // Attempts exhausted against this URL: the game server may have come
        // back somewhere else (restart, re-register) — ask again. Resolve
        // spaces its own retries, so reset the WS backoff for the fresh URL.
        cycles += 1;
        crate::ui::hint("Game server unreachable — re-resolving…");
        emit(
            sink.clone(),
            TuiEvent::Log {
                level: LogLevel::Hint,
                msg: "Game server unreachable — re-resolving…".into(),
            },
        );
        if cycles == 1 {
            crate::ui::hint(format!(
                "Probes cannot run while disconnected; this process keeps retrying. \
                 If it dies, reconnect with: ololo join {join_code}"
            ));
        }
        backoff_ms = BACKOFF_INITIAL_MS;
    }
}

// ---- Single connection attempt ----

/// `Ok(true)` — `SessionComplete` received (exit cleanly).
/// `Ok(false)` — socket closed without terminal frame (reconnect).
/// `Err(_)` — protocol / IO error (reconnect).
/// Memory-source syncing wired into the probe loop: a handle to ask for a
/// check on each probe, and the channel the sync task reports back on.
pub struct MemoryChannel {
    pub handle: crate::memory_sync::MemorySyncHandle,
    pub frames: tokio::sync::mpsc::UnboundedReceiver<arena_core::protocol::PlayerAgentClientFrame>,
}

async fn connect_once(
    ws_url: &str,
    pat: &str,
    sink: SinkArg,
    viewer_player_id: Option<Uuid>,
    memory: Option<&mut MemoryChannel>,
) -> Result<bool> {
    let mut request = ws_url
        .into_client_request()
        .context("building player-agent WS request")?;
    request.headers_mut().insert(
        HeaderName::from_static("x-api-key"),
        pat.parse().context("invalid X-API-Key value")?,
    );

    let (ws_stream, _) = connect_async(request).await.context("WebSocket connect")?;

    crate::ui::step("Connected to probe loop. Waiting for probes…");
    emit(
        sink.clone(),
        TuiEvent::Log {
            level: LogLevel::Step,
            msg: "Connected to probe loop. Waiting for probes…".into(),
        },
    );

    let (mut write, mut read) = ws_stream.split();

    tracing::info!("ws connected to {}", ws_url);

    let mut memory = memory;

    loop {
        let msg = tokio::select! {
            m = read.next() => m,
            // The sync task finished a push and wants the server told. It
            // cannot reach the socket itself, so it hands the frame here.
            Some(frame) = async {
                match memory.as_mut() {
                    Some(m) => m.frames.recv().await,
                    None => std::future::pending().await,
                }
            } => {
                let json = serde_json::to_string(&frame).unwrap_or_default();
                if let Err(e) = write.send(Message::Text(json)).await {
                    tracing::warn!("memory sync: notifying the server failed: {e}");
                }
                continue;
            }
            _ = tokio::signal::ctrl_c() => {
                crate::ui::hint("Interrupted.");
                return Ok(true);
            }
        };

        match msg {
            None | Some(Ok(Message::Close(_))) => return Ok(false),
            Some(Err(e)) => return Err(e.into()),
            Some(Ok(Message::Ping(data))) => {
                let _ = write.send(Message::Pong(data)).await;
            }
            Some(Ok(Message::Text(text))) => {
                let frame: wire::PlayerAgentFrame = match serde_json::from_str(&text) {
                    Ok(f) => f,
                    Err(e) => {
                        tracing::warn!("unparseable player-agent frame: {e}: {text}");
                        continue;
                    }
                };

                match frame {
                    wire::PlayerAgentFrame::SessionPaused {
                        seconds_remaining, ..
                    } => {
                        crate::ui::countdown_line("Paused", seconds_remaining);
                        emit(
                            sink.clone(),
                            TuiEvent::Header(HeaderDelta::Paused {
                                seconds: seconds_remaining,
                            }),
                        );
                    }
                    wire::PlayerAgentFrame::SessionCancelled { ref session_id } => {
                        crate::ui::countdown_done();
                        crate::ui::pause_countdown(|| {
                            crate::ui::warn("Session cancelled");
                            crate::ui::field("session", session_id);
                        });
                        emit(
                            sink.clone(),
                            TuiEvent::Header(HeaderDelta::Cancelled {
                                session_id: session_id.clone(),
                            }),
                        );
                        emit(
                            sink.clone(),
                            TuiEvent::Log {
                                level: LogLevel::Warn,
                                msg: "Session cancelled".into(),
                            },
                        );
                        return Ok(true);
                    }
                    wire::PlayerAgentFrame::SessionComplete {
                        ref session_id,
                        ref reason,
                    } => {
                        if !session_complete_is_terminal(reason.as_deref()) {
                            // Per-player ack: this player exhausted their
                            // tasks, the session keeps running for the
                            // others. Stay connected — leaderboard/score
                            // frames and the real session-end frame follow.
                            // Why the session is still open matters here: the
                            // judges are reading, and one of them may ask for
                            // a screenshot or a recording of the running
                            // product. Told only that others are still
                            // competing, a solo player packs up — and the
                            // request expires unanswered.
                            const STILL_OPEN: &str = "Waiting for the session to finish — judges are reviewing, and one may ask you for a capture. Keep the project runnable.";
                            crate::ui::pause_countdown(|| {
                                crate::ui::success("All your tasks are done");
                                crate::ui::waiting(STILL_OPEN);
                                crate::ui::field("session", session_id);
                            });
                            emit(sink.clone(), TuiEvent::Header(HeaderDelta::PlayerTasksDone));
                            emit(
                                sink.clone(),
                                TuiEvent::Log {
                                    level: LogLevel::Waiting,
                                    msg: format!("All your tasks are done — {STILL_OPEN}"),
                                },
                            );
                            continue;
                        }
                        let cancelled =
                            reason.as_deref().map(|r| r == "cancelled").unwrap_or(false);
                        crate::ui::countdown_done();
                        crate::ui::pause_countdown(|| {
                            if cancelled {
                                crate::ui::warn("Session cancelled");
                            } else {
                                crate::ui::success("Session complete");
                            }
                            crate::ui::field("session", session_id);
                        });
                        emit(
                            sink.clone(),
                            TuiEvent::Header(if cancelled {
                                HeaderDelta::Cancelled {
                                    session_id: session_id.clone(),
                                }
                            } else {
                                HeaderDelta::Complete {
                                    session_id: session_id.clone(),
                                }
                            }),
                        );
                        emit(
                            sink.clone(),
                            TuiEvent::Log {
                                level: if cancelled {
                                    LogLevel::Warn
                                } else {
                                    LogLevel::Success
                                },
                                msg: if cancelled {
                                    "Session cancelled".into()
                                } else {
                                    "Session complete".into()
                                },
                            },
                        );
                        return Ok(true);
                    }
                    wire::PlayerAgentFrame::LobbyCountdown {
                        seconds_remaining, ..
                    } => {
                        crate::ui::countdown_line("Lobby", seconds_remaining);
                        emit(
                            sink.clone(),
                            TuiEvent::Header(HeaderDelta::Lobby {
                                seconds: seconds_remaining,
                            }),
                        );
                    }
                    wire::PlayerAgentFrame::SessionStarted { total_tasks, .. } => {
                        crate::ui::countdown_done();
                        crate::ui::pause_countdown(|| {
                            crate::ui::success("Session started — probes incoming");
                        });
                        emit(sink.clone(), TuiEvent::Header(HeaderDelta::Started));
                        if let Some(n) = total_tasks {
                            emit(sink.clone(), TuiEvent::TotalTasks(n));
                        }
                    }
                    wire::PlayerAgentFrame::RunningCountdown {
                        seconds_remaining,
                        paused,
                        ..
                    } => {
                        if paused {
                            crate::ui::countdown_line("Paused", seconds_remaining);
                            emit(
                                sink.clone(),
                                TuiEvent::Header(HeaderDelta::Paused {
                                    seconds: seconds_remaining,
                                }),
                            );
                        } else {
                            crate::ui::countdown_line("Running", seconds_remaining);
                            emit(
                                sink.clone(),
                                TuiEvent::Header(HeaderDelta::Running {
                                    seconds: seconds_remaining,
                                }),
                            );
                        }
                    }
                    wire::PlayerAgentFrame::TestPush {
                        probe_id,
                        rendered_command,
                        deadline_secs,
                        task_id,
                        task_ordinal,
                        task_title,
                        task_description,
                        test_ordinal,
                        test_total,
                        test_label,
                        test_description,
                        expected_answer,
                        answer_template,
                        validation_kind,
                    } => {
                        // A probe is the one regular heartbeat we get, so it
                        // doubles as the moment to notice edited memory
                        // sources. The check itself happens off-thread.
                        if let Some(m) = memory.as_ref() {
                            m.handle.request();
                        }
                        crate::ui::pause_countdown(|| {
                            crate::ui::step("Test pushed");
                            crate::ui::field("probe_id", probe_id);
                            crate::ui::field("command", &rendered_command);
                            crate::ui::field("deadline", format!("{}s", deadline_secs));
                        });
                        emit(
                            sink.clone(),
                            TuiEvent::ProbeArrived(ProbeInfo {
                                probe_id,
                                rendered_command: rendered_command.clone(),
                                deadline_secs,
                                task_id,
                                task_ordinal,
                                task_title: task_title.clone(),
                                task_description: task_description.clone(),
                                test_ordinal,
                                test_total,
                                test_label: test_label.clone(),
                                test_description: test_description.clone(),
                                expected_answer: expected_answer.clone(),
                                answer_template: answer_template.clone(),
                                validation_kind,
                            }),
                        );
                        let deadline = Duration::from_secs(deadline_secs.max(1) as u64);
                        // Permission gate: a probe is a platform-authored
                        // shell command about to run on the player's
                        // machine — it does not run until a settings rule
                        // or the player says so.
                        let decision = match permissions::check(&rendered_command) {
                            permissions::Verdict::Allowed => permissions::Decision::Allow,
                            permissions::Verdict::Denied => {
                                crate::ui::warn("probe command denied by .ololo/settings.json");
                                permissions::Decision::Decline
                            }
                            permissions::Verdict::Ask => {
                                let rule = permissions::always_rule(&rendered_command);
                                let d = if sink.is_some() {
                                    let (tx, rx) = tokio::sync::oneshot::channel();
                                    emit(
                                        sink.clone(),
                                        TuiEvent::PermissionRequest(PermissionPrompt {
                                            probe_id,
                                            command: rendered_command.clone(),
                                            always_rule: rule.clone(),
                                            deadline_secs,
                                            responder: Arc::new(std::sync::Mutex::new(Some(tx))),
                                        }),
                                    );
                                    let d = match tokio::time::timeout(deadline, rx).await {
                                        Ok(Ok(d)) => d,
                                        // Answerer gone or deadline passed.
                                        _ => permissions::Decision::Decline,
                                    };
                                    emit(sink.clone(), TuiEvent::PermissionResolved { probe_id });
                                    d
                                } else {
                                    // Plain text mode: ask on the terminal.
                                    permissions::prompt_via_stdin(
                                        &rendered_command,
                                        &rule,
                                        deadline,
                                    )
                                    .await
                                };
                                // The single application point for the two
                                // sticky answers, whichever surface answered —
                                // the prompts themselves never touch state.
                                match d {
                                    permissions::Decision::AlwaysAllow => {
                                        if let Err(e) = permissions::record_allow(&rule) {
                                            tracing::warn!("failed to persist allow rule: {e:#}");
                                        }
                                    }
                                    permissions::Decision::AllowAllSession => {
                                        permissions::allow_all_for_session();
                                        crate::ui::step(
                                            "probe commands approved for the rest of the session",
                                        );
                                    }
                                    _ => {}
                                }
                                d
                            }
                        };
                        if decision == permissions::Decision::Decline {
                            crate::ui::step("DECLINED");
                            crate::ui::pause_countdown(|| {
                                crate::ui::field("command", &rendered_command);
                            });
                            emit(
                                sink.clone(),
                                TuiEvent::ProbeResult(ProbeResultInfo {
                                    probe_id,
                                    command: rendered_command.clone(),
                                    stdout: String::new(),
                                    exit_code: Some(-1),
                                    duration_ms: 0,
                                    error: Some("declined by player".into()),
                                    task_id,
                                    task_ordinal,
                                    task_title: task_title.clone(),
                                    task_description: task_description.clone(),
                                    test_ordinal,
                                    test_total,
                                    test_label: test_label.clone(),
                                    test_description: test_description.clone(),
                                    deadline_secs: Some(deadline_secs),
                                    expected_answer: expected_answer.clone(),
                                    answer_template: answer_template.clone(),
                                    validation_kind,
                                    outcome: None,
                                    point_delta: None,
                                    graded_expected: None,
                                }),
                            );
                            let reply = wire::PlayerAgentClientFrame::TestResult {
                                probe_id,
                                stdout: arena_core::protocol::PROBE_DECLINED_STDOUT.into(),
                                exit_code: Some(-1),
                                duration_ms: 0,
                                error: Some("declined by player".into()),
                            };
                            let json =
                                serde_json::to_string(&reply).context("serialising TestResult")?;
                            write
                                .send(Message::Text(json))
                                .await
                                .context("sending TestResult")?;
                            continue;
                        }
                        let reply = match shell::run_probe(&rendered_command, deadline).await {
                            Ok((stdout, ec, dur)) => {
                                let trimmed = stdout.trim_end();
                                let probe_info = ProbeResultInfo {
                                    probe_id,
                                    command: rendered_command.clone(),
                                    stdout: stdout.clone(),
                                    exit_code: Some(ec),
                                    duration_ms: dur,
                                    error: None,
                                    task_id,
                                    task_ordinal,
                                    task_title: task_title.clone(),
                                    task_description: task_description.clone(),
                                    test_ordinal,
                                    test_total,
                                    test_label: test_label.clone(),
                                    test_description: test_description.clone(),
                                    deadline_secs: Some(deadline_secs),
                                    expected_answer: expected_answer.clone(),
                                    answer_template: answer_template.clone(),
                                    validation_kind,
                                    outcome: None,
                                    point_delta: None,
                                    graded_expected: None,
                                };
                                // ponytail: server is sole grader now; show
                                // neutral SENT until ProbeGraded arrives.
                                crate::ui::step("SENT");
                                crate::ui::pause_countdown(|| {
                                    crate::ui::field("stdout", trimmed);
                                    crate::ui::field("exit_code", ec);
                                    crate::ui::field("duration", format!("{}ms", dur));
                                });
                                emit(sink.clone(), TuiEvent::ProbeResult(probe_info));
                                wire::PlayerAgentClientFrame::TestResult {
                                    probe_id,
                                    stdout,
                                    exit_code: Some(ec),
                                    duration_ms: dur,
                                    error: None,
                                }
                            }
                            Err(e) => {
                                crate::ui::step("ERROR");
                                crate::ui::pause_countdown(|| {
                                    crate::ui::field("error", format!("{e}"));
                                });
                                emit(
                                    sink.clone(),
                                    TuiEvent::ProbeResult(ProbeResultInfo {
                                        probe_id,
                                        command: rendered_command.clone(),
                                        stdout: String::new(),
                                        exit_code: None,
                                        duration_ms: 0,
                                        error: Some(format!("{e}")),
                                        task_id,
                                        task_ordinal,
                                        task_title: task_title.clone(),
                                        task_description: task_description.clone(),
                                        test_ordinal,
                                        test_total,
                                        test_label: test_label.clone(),
                                        test_description: test_description.clone(),
                                        deadline_secs: Some(deadline_secs),
                                        expected_answer: expected_answer.clone(),
                                        answer_template: answer_template.clone(),
                                        validation_kind,
                                        outcome: None,
                                        point_delta: None,
                                        graded_expected: None,
                                    }),
                                );
                                wire::PlayerAgentClientFrame::TestResult {
                                    probe_id,
                                    stdout: String::new(),
                                    exit_code: None,
                                    duration_ms: 0,
                                    error: Some(format!("{e}")),
                                }
                            }
                        };

                        let json =
                            serde_json::to_string(&reply).context("serialising TestResult")?;
                        write
                            .send(Message::Text(json))
                            .await
                            .context("sending TestResult")?;
                    }
                    wire::PlayerAgentFrame::LeaderboardUpdate { entries, .. } => {
                        emit(sink.clone(), TuiEvent::LeaderboardUpdate { entries });
                    }
                    wire::PlayerAgentFrame::PlayerProgressUpdate {
                        player_id,
                        attempt,
                        status,
                        ..
                    } => {
                        if viewer_player_id == Some(player_id) {
                            emit(sink.clone(), TuiEvent::PlayerProgress { attempt, status });
                        }
                    }
                    wire::PlayerAgentFrame::ProbeGraded {
                        probe_id,
                        outcome,
                        point_delta,
                        expected,
                        actual,
                        next_probe_in_secs,
                    } => {
                        match outcome {
                            arena_core::protocol::ProbeOutcome::Pass => {
                                crate::ui::success("PASS");
                            }
                            arena_core::protocol::ProbeOutcome::Error => {
                                crate::ui::step("FAIL");
                            }
                            arena_core::protocol::ProbeOutcome::NoResponse => {
                                crate::ui::warn("NO RESPONSE");
                            }
                        }
                        crate::ui::pause_countdown(|| {
                            crate::ui::field("probe", probe_id);
                            crate::ui::field("outcome", format!("{:?}", outcome));
                            crate::ui::field("points", point_delta);
                            if let Some(exp) = &expected {
                                crate::ui::field("expected", exp);
                            }
                            if let Some(act) = &actual {
                                crate::ui::field("actual", act);
                            }
                            // What happens next, in text mode too: the
                            // scheduler's own sleep, not a guess.
                            if let Some(secs) = next_probe_in_secs {
                                crate::ui::field_dim("next check", format!("in {secs}s"));
                            }
                        });
                        emit(
                            sink.clone(),
                            TuiEvent::ProbeGraded {
                                probe_id,
                                outcome,
                                point_delta,
                                expected,
                                actual,
                                next_probe_in_secs,
                            },
                        );
                    }
                    wire::PlayerAgentFrame::SnapshotRequest {
                        task_id,
                        task_title,
                        reason,
                    } => {
                        crate::ui::hint(format!("snapshot requested ({reason})"));
                        emit(
                            sink.clone(),
                            TuiEvent::SnapshotRequested {
                                task_id,
                                task_title,
                                reason,
                            },
                        );
                    }
                    wire::PlayerAgentFrame::TaskStarted {
                        player_id,
                        task_ordinal,
                        task_title,
                    } => {
                        if viewer_player_id == Some(player_id) {
                            crate::ui::hint(format!("task #{task_ordinal} started: {task_title}"));
                            emit(
                                sink.clone(),
                                TuiEvent::Log {
                                    level: LogLevel::Hint,
                                    msg: format!("Task #{task_ordinal} started: {task_title}"),
                                },
                            );
                        }
                    }
                    wire::PlayerAgentFrame::JudgeScored {
                        task_id,
                        judge_name,
                        point_delta,
                        feedback,
                    } => {
                        let sign = if point_delta >= 0 { "+" } else { "" };
                        crate::ui::success(format!("JUDGE {judge_name}: {sign}{point_delta} pts"));
                        // The full feedback runs to paragraphs — one line of it
                        // orients the player; the web page carries the rest.
                        let brief: String = feedback.chars().take(200).collect();
                        if !brief.is_empty() {
                            crate::ui::hint(&brief);
                        }
                        emit(
                            sink.clone(),
                            TuiEvent::Log {
                                level: LogLevel::Success,
                                msg: format!(
                                    "Judge {judge_name}: {sign}{point_delta} pts — {brief}"
                                ),
                            },
                        );
                        emit(
                            sink.clone(),
                            TuiEvent::JudgeScored {
                                task_id,
                                judge_name,
                                point_delta,
                                feedback,
                            },
                        );
                    }
                    wire::PlayerAgentFrame::JudgeStarted {
                        task_id,
                        judge_name,
                    } => {
                        crate::ui::hint(format!("JUDGE {judge_name} is reviewing your code…"));
                        emit(
                            sink.clone(),
                            TuiEvent::JudgeStarted {
                                task_id,
                                judge_name,
                            },
                        );
                    }
                    wire::PlayerAgentFrame::JudgeFailed {
                        task_id,
                        judge_name,
                        error,
                    } => {
                        let why = error
                            .filter(|e| !e.is_empty())
                            .map(|e| format!(" — {e}"))
                            .unwrap_or_default();
                        crate::ui::warn(format!(
                            "JUDGE {judge_name} could not score this task{why}"
                        ));
                        emit(
                            sink.clone(),
                            TuiEvent::JudgeFailed {
                                task_id,
                                judge_name,
                            },
                        );
                    }
                }
            }
            Some(Ok(_)) => {} // binary / other — ignore
        }
    }
}

/// Whether a `SessionComplete` frame ends the frame loop.
///
/// The per-player acknowledgment (reason `player_tasks_completed`, see
/// `SESSION_COMPLETE_REASON_PLAYER_TASKS_COMPLETED` in arena-core) keeps
/// the connection open: this player is done but the session continues for
/// other players, and the real session-end frame (`all_tasks_completed`,
/// `time_expired`, `cancelled`, …) arrives later. Every other reason — or
/// no reason at all (old game-servers) — is terminal.
fn session_complete_is_terminal(reason: Option<&str>) -> bool {
    reason != Some(arena_core::protocol::SESSION_COMPLETE_REASON_PLAYER_TASKS_COMPLETED)
}

/// Parse the viewer's `player_id` (String from resolve response) into a `Uuid`
/// for filtering leaderboard/progress broadcasts. Returns `None` on a malformed
/// string; callers degrade score/rank/progress to `—`.
fn parse_player_id(s: &str) -> Option<Uuid> {
    Uuid::parse_str(s.trim()).ok()
}
