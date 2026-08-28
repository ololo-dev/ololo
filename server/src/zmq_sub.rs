use arena_core::protocol::ZmqEvent;
use async_trait::async_trait;
use tokio::task::JoinHandle;
use zeromq::{Socket, SocketRecv, SubSocket};

#[async_trait]
pub trait EventSubscriber: Send + Sync {
    async fn recv(&mut self) -> Option<ZmqEvent>;
}

pub struct ZmqEventSubscriber {
    socket: SubSocket,
}

impl ZmqEventSubscriber {
    pub async fn connect(addr: &str, topic: &str) -> anyhow::Result<Self> {
        let mut socket = SubSocket::new();
        socket.connect(addr).await?;
        socket.subscribe(topic).await?;
        tracing::info!("ZMQ SUB connected to {}, topic={}", addr, topic);
        Ok(Self { socket })
    }
}

#[async_trait]
impl EventSubscriber for ZmqEventSubscriber {
    async fn recv(&mut self) -> Option<ZmqEvent> {
        loop {
            match self.socket.recv().await {
                Ok(msg) => {
                    let frame = match msg.into_vec().into_iter().next() {
                        Some(f) => f,
                        None => {
                            tracing::warn!("zmq sub: empty message, skipping");
                            continue;
                        }
                    };
                    match serde_json::from_slice::<ZmqEvent>(&frame) {
                        Ok(event) => return Some(event),
                        Err(e) => {
                            tracing::warn!("zmq sub: deserialize failed: {e}");
                            continue;
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!("zmq sub: recv failed: {e}");
                    return None;
                }
            }
        }
    }
}

pub struct InMemoryEventSubscriber {
    rx: tokio::sync::mpsc::Receiver<ZmqEvent>,
}

impl InMemoryEventSubscriber {
    pub fn new(rx: tokio::sync::mpsc::Receiver<ZmqEvent>) -> Self {
        Self { rx }
    }
}

#[async_trait]
impl EventSubscriber for InMemoryEventSubscriber {
    async fn recv(&mut self) -> Option<ZmqEvent> {
        self.rx.recv().await
    }
}

pub fn spawn_subscriber_task(
    game_server_id: uuid::Uuid,
    zmq_url: String,
    state: crate::state::AppState,
) -> JoinHandle<()> {
    let url_for_log = zmq_url.clone();
    tokio::spawn(async move {
        let mut subscriber = match ZmqEventSubscriber::connect(&zmq_url, "").await {
            Ok(s) => s,
            Err(e) => {
                tracing::error!(
                    "failed to connect ZMQ SUB for game_server {} to {}: {e}",
                    game_server_id,
                    url_for_log,
                );
                return;
            }
        };
        tracing::info!(
            "ZMQ SUB task running for game_server {}, listening on {}",
            game_server_id,
            zmq_url,
        );
        loop {
            match subscriber.recv().await {
                Some(event) => {
                    tracing::debug!(
                        "received ZmqEvent type={} join_code={}",
                        event_variant(&event),
                        event.join_code(),
                    );
                    if let Err(e) = route_event(&state, &event).await {
                        tracing::warn!("event routing failed: {e}");
                    }
                }
                None => {
                    tracing::warn!(
                        "ZMQ SUB for game_server {} returned None, reconnecting...",
                        game_server_id
                    );
                    tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
                    let addr = zmq_url.clone();
                    subscriber = match ZmqEventSubscriber::connect(&addr, "").await {
                        Ok(s) => s,
                        Err(e) => {
                            tracing::error!(
                                "reconnect failed for game_server {}: {e}",
                                game_server_id
                            );
                            return;
                        }
                    };
                }
            }
        }
    })
}

fn event_variant(event: &ZmqEvent) -> &'static str {
    match event {
        ZmqEvent::SessionTimer { .. } => "session_timer",
        ZmqEvent::SessionStatus { .. } => "session_status",
        ZmqEvent::PlayerJoin { .. } => "player_join",
        ZmqEvent::PlayerLeave { .. } => "player_leave",
        ZmqEvent::AgentPresence { .. } => "agent_presence",
        ZmqEvent::ScoreChange { .. } => "score_change",
        ZmqEvent::TaskStarted { .. } => "task_started",
        ZmqEvent::JudgeStarted { .. } => "judge_started",
        ZmqEvent::JudgeFailed { .. } => "judge_failed",
        ZmqEvent::JudgeScored { .. } => "judge_scored",
        ZmqEvent::ProbeFinished { .. } => "probe_finished",
        ZmqEvent::ProbeRegistered { .. } => "probe_registered",
        ZmqEvent::ArtifactReceived { .. } => "artifact_received",
        ZmqEvent::ArtifactAwaited { .. } => "artifact_awaited",
        ZmqEvent::SessionReportReady { .. } => "session_report_ready",
        ZmqEvent::EvaluationReady { .. } => "evaluation_ready",
        ZmqEvent::SessionAwarded { .. } => "session_awarded",
        ZmqEvent::SessionSettled { .. } => "session_settled",
    }
}

/// Resolve `join_code` → `(session_id, project_id)`, caching the answer.
///
/// Timer ticks arrive once a second per live session, so this must not hit
/// the DB on the steady-state path. `None` means the session is unknown and
/// the caller should skip its project fan-out.
async fn project_ref(
    state: &crate::state::AppState,
    join_code: &str,
) -> Option<(uuid::Uuid, uuid::Uuid)> {
    use arena_core::entities::sessions;
    use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};

    if let Some(hit) = state.session_project.get(join_code) {
        return Some(*hit.value());
    }
    let row = match sessions::Entity::find()
        .filter(sessions::Column::JoinCode.eq(join_code))
        .one(&state.db)
        .await
    {
        Ok(Some(row)) => row,
        Ok(None) => return None,
        Err(e) => {
            tracing::warn!("project fan-out lookup failed for {join_code}: {e}");
            return None;
        }
    };
    let pair = (row.id, row.project_id_fk);
    state.session_project.insert(join_code.to_string(), pair);
    Some(pair)
}

/// Fan a game-server-driven session change out to the project's WS observers.
///
/// The `/projects/[slug]` page keeps its session list — and the live cards
/// above the tabs — fed from these frames. The REST handlers are the only
/// other callers of `fan_project_update`, and neither a timer expiry nor a
/// player join goes through them, so without this a session that ends on its
/// own timer stays "running" there until a reload and its player count never
/// moves.
///
/// Best-effort: an unknown session or DB error is dropped rather than failing
/// event routing.
async fn fan_project_session(
    state: &crate::state::AppState,
    join_code: &str,
    status: arena_core::session_status::SessionStatus,
    cancel_reason: Option<&str>,
    cancelled_by: Option<&str>,
) {
    use arena_core::entities::sessions;
    use sea_orm::EntityTrait;

    let Some((session_id, project_id)) = project_ref(state, join_code).await else {
        return;
    };
    // Skip the row read and the count entirely when nobody is watching —
    // neither this project's observers nor the landing's live-sessions block.
    let project_watched = state.project_registry.contains_key(&project_id);
    if !project_watched && state.landing_tx.receiver_count() == 0 {
        return;
    }
    let Ok(Some(row)) = sessions::Entity::find_by_id(session_id)
        .one(&state.db)
        .await
    else {
        return;
    };
    let player_count = crate::api::sessions::common::count_players(&state.db, session_id).await;
    let frame = crate::protocol::ArenaFrame::ProjectSessionUpdate {
        session_id,
        name: row.name,
        status,
        project_id,
        join_code: Some(row.join_code),
        created_at: row.created_at,
        player_count,
        cancel_reason: cancel_reason.map(str::to_string),
        cancelled_by: cancelled_by.map(str::to_string),
    };
    if project_watched {
        crate::api::sessions::common::fan_project_update(
            &state.project_registry,
            project_id,
            frame.clone(),
        );
    }
    crate::api::sessions::common::fan_landing_update(state, project_id, frame).await;
}

/// Bridge a countdown tick to the project observers, so the live cards show
/// the same "Starts in" / "Ends in" clock the session dashboard does.
/// `WsProjectClient` already consumes these frames — they were simply never
/// addressed to the project channel.
async fn fan_project_countdown(
    state: &crate::state::AppState,
    join_code: &str,
    phase: arena_core::session_status::SessionStatus,
    seconds_remaining: u32,
) {
    use arena_core::protocol::ArenaFrame;
    use arena_core::session_status::SessionStatus;

    let Some((session_id, project_id)) = project_ref(state, join_code).await else {
        return;
    };
    let frame = match phase {
        SessionStatus::Lobby => ArenaFrame::LobbyCountdown {
            session_id,
            seconds_remaining,
            version: 0,
        },
        SessionStatus::Running => ArenaFrame::RunningCountdown {
            session_id,
            seconds_remaining,
            version: 0,
            paused: false,
        },
        _ => return,
    };
    if let Some(tx) = state.project_registry.get(&project_id) {
        let _ = tx.send(frame.clone());
    }
    crate::api::sessions::common::fan_landing_update(state, project_id, frame).await;
}

pub async fn route_event(state: &crate::state::AppState, event: &ZmqEvent) -> anyhow::Result<()> {
    let join_code = event.join_code();
    let _ = state.zmq_events_tx.send(event.clone());
    // Kept outside the `session_registry` lookup below: that only fires when
    // someone has the session dashboard open, while project observers are a
    // separate audience watching from the project page.
    match event {
        ZmqEvent::SessionStatus {
            status,
            cancel_reason,
            cancelled_by,
            ..
        } => match status.parse::<arena_core::session_status::SessionStatus>() {
            Ok(parsed) => {
                // A terminal status is the last word on this session — drop
                // the cache entry so a recycled join code cannot resolve to it.
                let terminal = matches!(
                    parsed,
                    arena_core::session_status::SessionStatus::Finished
                        | arena_core::session_status::SessionStatus::Cancelled
                );
                fan_project_session(
                    state,
                    join_code,
                    parsed,
                    cancel_reason.as_deref(),
                    cancelled_by.as_deref(),
                )
                .await;
                if terminal {
                    state.session_project.remove(join_code);
                }
            }
            Err(e) => tracing::warn!("dropping project fan-out for {join_code}: {e}"),
        },
        // Player counts on the live cards move as people join and leave.
        ZmqEvent::PlayerJoin { .. } | ZmqEvent::PlayerLeave { .. } => {
            if let Some((session_id, _)) = project_ref(state, join_code).await
                && let Ok(Some(row)) =
                    <arena_core::entities::sessions::Entity as sea_orm::EntityTrait>::find_by_id(
                        session_id,
                    )
                    .one(&state.db)
                    .await
            {
                fan_project_session(state, join_code, row.status, None, None).await;
            }
        }
        ZmqEvent::SessionTimer {
            phase,
            seconds_remaining,
            ..
        } => {
            if let Ok(parsed) = phase.parse::<arena_core::session_status::SessionStatus>() {
                fan_project_countdown(state, join_code, parsed, *seconds_remaining).await;
            }
        }
        _ => {}
    }
    // Judge lifecycle frames target only the player's own page, so route them
    // on the player registry directly — they must arrive even when nobody has
    // the session dashboard open (no session_registry entry).
    match event {
        // Presence targets only the player's own page; route on the player
        // registry directly so it arrives even when no dashboard is open.
        ZmqEvent::AgentPresence {
            player_id,
            connected,
            timestamp,
            ..
        } => {
            use arena_core::protocol::PlayerFrame;
            if let Some(ch) = state.player_registry.get(player_id) {
                let _ = ch.sender.send(PlayerFrame::AgentPresence {
                    connected: *connected,
                    timestamp: *timestamp,
                });
            }
            return Ok(());
        }
        ZmqEvent::JudgeStarted {
            player_id,
            task_id,
            judge_slug,
            judge_name,
            timestamp,
            ..
        } => {
            use arena_core::protocol::{PlayerFrame, PlayerJudgeStatusPayload};
            if let Some(ch) = state.player_registry.get(player_id) {
                let _ = ch
                    .sender
                    .send(PlayerFrame::JudgeStarted(PlayerJudgeStatusPayload {
                        task_id: *task_id,
                        judge_slug: judge_slug.clone(),
                        judge_name: judge_name.clone(),
                        status: "running".to_string(),
                        error: None,
                        updated_at: Some(*timestamp),
                        judge_result_id: None,
                    }));
            }
            return Ok(());
        }
        ZmqEvent::JudgeFailed {
            player_id,
            task_id,
            judge_slug,
            judge_name,
            error,
            timestamp,
            ..
        } => {
            use arena_core::protocol::{PlayerFrame, PlayerJudgeStatusPayload};
            if let Some(ch) = state.player_registry.get(player_id) {
                let _ = ch
                    .sender
                    .send(PlayerFrame::JudgeFailed(PlayerJudgeStatusPayload {
                        task_id: *task_id,
                        judge_slug: judge_slug.clone(),
                        judge_name: judge_name.clone(),
                        status: "failed".to_string(),
                        // Generic message: the game-server publishes the public
                        // string; full detail stays in judge_results (admin API).
                        error: Some(error.clone()),
                        updated_at: Some(*timestamp),
                        judge_result_id: None,
                    }));
            }
            return Ok(());
        }
        ZmqEvent::ArtifactAwaited {
            player_id,
            task_id,
            probe_id,
            instruction,
            deadline_at,
            ..
        } => {
            use arena_core::protocol::PlayerFrame;
            if let Some(ch) = state.player_registry.get(player_id) {
                let _ = ch.sender.send(PlayerFrame::ArtifactAwaited {
                    task_id: *task_id,
                    probe_id: *probe_id,
                    instruction: instruction.clone(),
                    deadline_at: *deadline_at,
                });
            }
            return Ok(());
        }
        ZmqEvent::EvaluationReady {
            player_id, task_id, ..
        } => {
            use arena_core::protocol::PlayerFrame;
            if let Some(ch) = state.player_registry.get(player_id) {
                let _ = ch
                    .sender
                    .send(PlayerFrame::EvaluationReady { task_id: *task_id });
            }
            return Ok(());
        }
        ZmqEvent::SessionReportReady { player_id, .. } => {
            use arena_core::protocol::PlayerFrame;
            if let Some(ch) = state.player_registry.get(player_id) {
                let _ = ch.sender.send(PlayerFrame::SessionReportReady);
            }
            return Ok(());
        }
        // Probe telemetry: the player page learns about probe rows via its
        // db-poll; these events exist for observers/ops, no direct routing.
        ZmqEvent::ProbeFinished { .. } | ZmqEvent::ProbeRegistered { .. } => return Ok(()),
        _ => {}
    }
    // The player's own judge verdict must reach their page even when no
    // session dashboard is open — a finished session keeps no session_registry
    // entry, and none survives a restart. Route it on the player registry
    // directly, exactly like JudgeStarted/JudgeFailed above. (The dashboard
    // TaskScored broadcast + leaderboard-cache refresh below stay gated on a
    // session_registry entry, which only the dashboard needs.) Probe-pass
    // scoring reuses JudgeScored with an empty slug — not a verdict, skip it.
    if let ZmqEvent::JudgeScored {
        player_id,
        task_id,
        judge_slug,
        judge_name,
        rating,
        feedback,
        point_delta,
        timestamp,
        duration_ms,
        ..
    } = event
        && !judge_slug.is_empty()
        && let Some(ch) = state.player_registry.get(player_id)
    {
        use arena_core::protocol::{PlayerFrame, PlayerJudgeScoredPayload};
        let _ = ch
            .sender
            .send(PlayerFrame::JudgeScored(PlayerJudgeScoredPayload {
                task_id: *task_id,
                judge_slug: judge_slug.clone(),
                judge_name: judge_name.clone(),
                rating: *rating,
                feedback: feedback.clone(),
                point_delta: *point_delta,
                created_at: *timestamp,
                duration_ms: *duration_ms,
            }));
    }
    // Score-bearing events must refresh the server's cached leaderboard —
    // the probe-driven poll task only recomputes when a probe resolves, so
    // judge verdicts (and completion bonuses) landing after the last probe
    // would otherwise never reach the cache the player page's score reads.
    let score_changed = matches!(
        event,
        ZmqEvent::JudgeScored { .. } | ZmqEvent::ScoreChange { .. }
    );
    match state.session_registry.get(join_code) {
        Some(entry) => {
            // Translate ZMQ telemetry into ArenaFrame broadcasts for dashboard WS clients.
            // game-server owns the timers now (server timers deleted); its countdown
            // ticks arrive as ZmqEvent::SessionTimer and must be bridged to the
            // per-session broadcast channel that observer WS clients consume.
            match event {
                ZmqEvent::SessionTimer {
                    phase,
                    seconds_remaining,
                    ..
                } => {
                    use arena_core::protocol::ArenaFrame;
                    use arena_core::session_status::SessionStatus;
                    let new_phase = match phase.parse::<SessionStatus>() {
                        Ok(p) => p,
                        Err(e) => {
                            tracing::warn!("dropping SessionTimer for {join_code}: {e}");
                            return Ok(());
                        }
                    };
                    // Re-stamp with the SERVER's per-session counter: browser
                    // clients gate on monotonic versions, and the game-server's
                    // counter runs independently of ours — passing it through
                    // interleaves two sequences on one channel and freezes the
                    // dashboard (whichever counter is behind gets dropped).
                    let (session_id, version) = match entry.cache.write() {
                        Ok(mut cache) => {
                            cache.version = cache.version.saturating_add(1);
                            if cache.phase != new_phase {
                                cache.phase = new_phase;
                            }
                            (cache.session_id, cache.version)
                        }
                        Err(_) => return Ok(()),
                    };
                    let frame = match new_phase {
                        SessionStatus::Lobby => ArenaFrame::LobbyCountdown {
                            session_id,
                            seconds_remaining: *seconds_remaining,
                            version,
                        },
                        SessionStatus::Running => ArenaFrame::RunningCountdown {
                            session_id,
                            seconds_remaining: *seconds_remaining,
                            version,
                            paused: false,
                        },
                        SessionStatus::Paused => ArenaFrame::RunningCountdown {
                            session_id,
                            seconds_remaining: *seconds_remaining,
                            version,
                            paused: true,
                        },
                        SessionStatus::Finished | SessionStatus::Cancelled => {
                            tracing::debug!("unhandled SessionTimer phase={phase} for {join_code}");
                            return Ok(());
                        }
                    };
                    let _ = entry.tx.send(frame);
                }
                ZmqEvent::SessionStatus { status, .. } => {
                    use arena_core::protocol::ArenaFrame;
                    use arena_core::session_status::SessionStatus;
                    let new_phase = match status.parse::<SessionStatus>() {
                        Ok(s) => s,
                        Err(e) => {
                            tracing::warn!("dropping SessionStatus for {join_code}: {e}");
                            return Ok(());
                        }
                    };
                    let session_id = entry.cache.read().map(|c| c.session_id).unwrap_or_default();
                    if let Ok(mut cache) = entry.cache.write()
                        && cache.phase != new_phase
                    {
                        // Broadcast the lifecycle frame WS dashboard clients expect
                        // so they transition phase (e.g. lobby→active triggers redirect).
                        let frame = match new_phase {
                            SessionStatus::Running => ArenaFrame::SessionStarted {
                                session_id,
                                version: cache.version.saturating_add(1),
                                total_tasks: None,
                            },
                            SessionStatus::Finished | SessionStatus::Cancelled => {
                                ArenaFrame::SessionComplete {
                                    reason: status.clone(),
                                    version: cache.version.saturating_add(1),
                                }
                            }
                            _ => ArenaFrame::ProjectSessionUpdate {
                                session_id,
                                name: String::new(),
                                player_count: 0,
                                status: new_phase,
                                project_id: uuid::Uuid::nil(),
                                join_code: Some(join_code.to_string()),
                                created_at: chrono::Utc::now(),
                                cancel_reason: None,
                                cancelled_by: None,
                            },
                        };
                        let _ = entry.tx.send(frame);
                        cache.phase = new_phase;
                        cache.version = cache.version.saturating_add(1);
                    }
                }
                ZmqEvent::JudgeScored {
                    player_id,
                    player_display_name,
                    task_id,
                    task_ordinal,
                    task_title,
                    point_delta,
                    judge_name,
                    detail,
                    timestamp,
                    ..
                } => {
                    use arena_core::protocol::ArenaFrame;
                    // Re-stamped with the server's counter — see SessionTimer.
                    // The player-page verdict forward already happened above,
                    // unconditionally; here we only bridge the dashboard's
                    // TaskScored ticker, which needs this session_registry entry.
                    let version = match entry.cache.write() {
                        Ok(mut cache) => {
                            cache.version = cache.version.saturating_add(1);
                            cache.version
                        }
                        Err(_) => return Ok(()),
                    };
                    let frame = ArenaFrame::TaskScored {
                        player_id: *player_id,
                        player_display_name: player_display_name.clone(),
                        task_id: *task_id,
                        task_ordinal: *task_ordinal,
                        task_title: task_title.clone(),
                        point_delta: *point_delta,
                        judge_name: judge_name.clone(),
                        detail: detail.clone(),
                        timestamp: *timestamp,
                        version,
                    };
                    let _ = entry.tx.send(frame);
                }
                ZmqEvent::SessionSettled { session_id, .. } => {
                    use arena_core::protocol::ArenaFrame;
                    // Re-stamped with the server's counter — see SessionTimer.
                    let version = match entry.cache.write() {
                        Ok(mut cache) => {
                            cache.version = cache.version.saturating_add(1);
                            cache.version
                        }
                        Err(_) => return Ok(()),
                    };
                    let _ = entry.tx.send(ArenaFrame::SessionSettled {
                        session_id: *session_id,
                        version,
                    });
                }
                ZmqEvent::ArtifactReceived {
                    player_id,
                    player_display_name,
                    task_id,
                    task_ordinal,
                    task_title,
                    probe_id,
                    path,
                    size,
                    content_type,
                    within_cap,
                    files,
                    timestamp,
                    ..
                } => {
                    use arena_core::protocol::ArenaFrame;
                    // Re-stamped with the server's counter — see SessionTimer.
                    let version = match entry.cache.write() {
                        Ok(mut cache) => {
                            cache.version = cache.version.saturating_add(1);
                            cache.version
                        }
                        Err(_) => return Ok(()),
                    };
                    let frame = ArenaFrame::ArtifactReceived {
                        player_id: *player_id,
                        player_display_name: player_display_name.clone(),
                        task_id: *task_id,
                        task_ordinal: *task_ordinal,
                        task_title: task_title.clone(),
                        probe_id: *probe_id,
                        path: path.clone(),
                        size: *size,
                        content_type: content_type.clone(),
                        within_cap: *within_cap,
                        files: files.clone(),
                        timestamp: *timestamp,
                        version,
                    };
                    let _ = entry.tx.send(frame);
                }
                ZmqEvent::TaskStarted {
                    player_id,
                    player_display_name,
                    task_id,
                    task_ordinal,
                    task_title,
                    timestamp,
                    ..
                } => {
                    use arena_core::protocol::ArenaFrame;
                    // Re-stamped with the server's counter — see SessionTimer.
                    let version = match entry.cache.write() {
                        Ok(mut cache) => {
                            cache.version = cache.version.saturating_add(1);
                            cache.version
                        }
                        Err(_) => return Ok(()),
                    };
                    let frame = ArenaFrame::TaskStarted {
                        player_id: *player_id,
                        player_display_name: player_display_name.clone(),
                        task_id: *task_id,
                        task_ordinal: *task_ordinal,
                        task_title: task_title.clone(),
                        timestamp: *timestamp,
                        version,
                    };
                    let _ = entry.tx.send(frame);
                }
                _ => {}
            }
            tracing::debug!("routed ZmqEvent for session {join_code}");
        }
        None => {
            tracing::debug!("ignoring ZmqEvent for unknown session {join_code}");
        }
    }
    if score_changed {
        // Recompute from the DB (task_results + judge_results) and update
        // the session cache + fan out LeaderboardUpdate/ScoreRankUpdated.
        // Runs after the registry guard above is dropped.
        let session_id = state
            .session_registry
            .get(join_code)
            .and_then(|e| e.cache.read().ok().map(|c| c.session_id));
        if let Some(session_id) = session_id {
            crate::scoring::broadcast_leaderboard(
                &state.db,
                &state.session_registry,
                &state.player_registry,
                join_code,
                session_id,
            )
            .await;
        }
    }
    Ok(())
}
