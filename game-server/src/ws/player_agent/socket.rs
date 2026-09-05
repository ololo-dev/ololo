use crate::state::GameServerState;
use crate::ws::player_agent::grading::{
    SleepOutcome, await_lobby_started, drain_during_sleep, grade_test_result, resolve_test_fixtures,
};
use crate::ws::player_agent::interval::clamp_interval_bounds;
use crate::ws::player_agent::probe_action::{ProbeAction, decide_probe_action};
use crate::ws::player_agent::registry_guard::{RegistryGuard, is_current_connection};
use crate::ws::player_agent::scheduler::{
    advance_to_next_task, ensure_adapted_test, pick_next_adapted_test, resolve_current_task_id,
    test_position, update_next_probe_at,
};
use crate::ws::player_agent::scoring::{broadcast_leaderboard, publish_score_change};
use arena_core::entities::{players, probes, sessions, tasks};
use arena_core::protocol::{ArenaFrame, PlayerAgentClientFrame, PlayerAgentFrame};
use arena_core::session_status::SessionStatus;
use axum::extract::ws::{Message, WebSocket};
use chrono::Utc;
use sea_orm::Order;
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, QueryOrder, Set};
use tokio::sync::broadcast;
use uuid::Uuid;

/// Floor on the delay before re-dispatching after a passing probe. A pass
/// skips the normal inter-probe backoff to keep a fast agent's tempo, but
/// with zero delay the loop spins at network round-trip speed — and every
/// probe costs ~5 DB statements, JSONL appends, and a ZMQ fan-out. One
/// second caps the per-player dispatch rate without hurting gameplay.
const PASS_REDISPATCH_FLOOR_SECS: i32 = 1;

/// How long the loop sleeps before the next dispatch: the floor after a
/// pass (or a completion-flag nudge), the backed-off interval otherwise.
/// One rule, shared with the `ProbeGraded.next_probe_in_secs` hint the
/// agent narrates, so the countdown the player sees is the sleep the
/// scheduler takes.
pub(crate) fn next_probe_delay_secs(
    current_interval_secs: i32,
    dispatch_next_immediately: bool,
) -> i32 {
    if dispatch_next_immediately {
        PASS_REDISPATCH_FLOOR_SECS
    } else {
        current_interval_secs.max(1)
    }
}

/// Which `players` row an agent socket may drive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SocketPlayer {
    Resolved(Uuid),
    /// No such live player in this session.
    NotFound,
    /// The client sent no `player_id` and the session has more than one live
    /// player, so there is nothing to resolve without guessing.
    Ambiguous {
        live_players: usize,
    },
}

/// Resolve which `players` row this agent socket drives.
///
/// Every branch is scoped to `user_id` — the PAT-authenticated owner of the
/// connection (SEC-H1). A `player_id` from the query string is a claim, not
/// proof: without the ownership filter, anyone holding the 6-char join code
/// could name a victim's `player_id` and drive their row (read their probes,
/// submit fake results, burn their points). Rows with `user_id_fk = NULL`
/// cannot be driven at all, matching the resolve endpoint's strict equality.
///
/// New clients pass the `player_id` they learned from the resolve endpoint; it
/// must belong to this session, to this user, and not be revoked.
///
/// A client that sends nothing can only be served when the user has exactly
/// one live row in the session. This used to fall back to "the session's first
/// row", which is correct in a solo session and silently catastrophic in a
/// shared one: a second client without an id became the FIRST player. Both
/// then dispatched probes under one `player_id`, so its owner banked points
/// earned on someone else's machine (and that machine's wrong answers), while
/// the real owner of the second row recorded nothing at all and never had a
/// judge run. Observed in session VKIBCB. Refuse instead — a client that
/// cannot identify itself needs upgrading, not a guess.
pub async fn resolve_socket_player(
    db: &sea_orm::DatabaseConnection,
    session_id: Uuid,
    user_id: Uuid,
    requested: Option<Uuid>,
) -> Result<SocketPlayer, sea_orm::DbErr> {
    if let Some(pid) = requested {
        return Ok(players::Entity::find_by_id(pid)
            .filter(players::Column::SessionIdFk.eq(session_id))
            .filter(players::Column::UserIdFk.eq(user_id))
            .filter(players::Column::RevokedAt.is_null())
            .one(db)
            .await?
            .map(|p| SocketPlayer::Resolved(p.id))
            .unwrap_or(SocketPlayer::NotFound));
    }

    let live = players::Entity::find()
        .filter(players::Column::SessionIdFk.eq(session_id))
        .filter(players::Column::UserIdFk.eq(user_id))
        .filter(players::Column::RevokedAt.is_null())
        .all(db)
        .await?;
    match live.len() {
        0 => Ok(SocketPlayer::NotFound),
        1 => Ok(SocketPlayer::Resolved(live[0].id)),
        n => Ok(SocketPlayer::Ambiguous { live_players: n }),
    }
}

/// `player_id` arrives pre-resolved: the upgrade handler authenticates the
/// PAT and runs [`resolve_socket_player`] BEFORE accepting the upgrade, so an
/// unauthenticated or non-member caller never reaches this loop.
pub async fn handle_player_agent_socket(
    mut socket: WebSocket,
    session_id: Uuid,
    join_code: String,
    player_id: Uuid,
    state: GameServerState,
) {
    // Sized to absorb a burst of judge verdicts (multiple judges per task, plus
    // recovery re-drives) without dropping frames when the agent drains slowly.
    let (agent_tx, mut agent_rx) = tokio::sync::mpsc::channel::<PlayerAgentFrame>(64);

    // ponytail: project defaults cached for session lifetime; mid-session
    // PATCH /api/projects does not invalidate running sessions. Acceptable —
    // probe cadence changes mid-run are cosmetic. resolve_intervals clamp
    // bounds the damage if defaults drift to a broken state. Upgrade path:
    // broadcast invalidate on project PATCH if this ever bites.
    let proj_defaults = match sessions::Entity::find_by_id(session_id)
        .one(&state.db)
        .await
        .ok()
        .flatten()
        .map(|s| s.project_id_fk)
    {
        Some(pid) => match arena_core::entities::projects::Entity::find_by_id(pid)
            .one(&state.db)
            .await
        {
            Ok(Some(p)) => Some(p),
            _ => None,
        },
        None => None,
    };

    // ponytail: register agent channel so judge_queue can push JudgeScored
    // frames back to this socket. Removed on scope drop via the guard below.
    state
        .player_agent_registry
        .insert(player_id, agent_tx.clone());
    let mut _registry_guard = RegistryGuard::new(
        state.player_agent_registry.clone(),
        player_id,
        agent_tx.clone(),
    );

    // Announce the connection (web Live indicator + idle sweep), and arm the
    // guard so every exit path — including error returns and supersession-safe
    // teardown — records the disconnect without each `return` remembering to.
    crate::ws::player_agent::presence::set_agent_presence(&state, &join_code, player_id, true)
        .await;
    _registry_guard.arm_presence(state.clone(), join_code.clone());

    let mut current_interval_secs: i32 = 1;

    // Subscribe to the session broadcast channel once — reused for both the lobby
    // wait and for streaming RunningCountdown frames during the active probe phase.
    let mut session_rx: Option<broadcast::Receiver<ArenaFrame>> = state
        .session_registry
        .get(&join_code)
        .map(|e| e.tx.subscribe());

    // If the session is still in lobby phase, forward countdown frames to the client
    // and wait for SessionStarted before dispatching any probes.
    let current_phase = state
        .session_registry
        .get(&join_code)
        .and_then(|e| e.cache.read().ok().map(|c| c.phase))
        .unwrap_or(SessionStatus::Lobby);

    if current_phase == SessionStatus::Lobby
        && await_lobby_started(
            &mut socket,
            &mut session_rx,
            session_id,
            state.lobby_timer_secs,
        )
        .await
    {
        return;
    }

    // Open-ended judge-probe phase marker: the task whose judges were fired
    // and when — the loop holds that task until the judges' probes settle
    // (or the phase cap passes), then advances. Local to this connection; a
    // reconnect resumes from the DB-recorded judge runs.
    let mut judge_wait: Option<(Uuid, std::time::Instant)> = None;

    loop {
        // Stop as soon as a newer connection for this player has taken over.
        // The registry is keyed by player_id, so a reconnect overwrites the
        // entry while this handler keeps running — and two handlers dispatch
        // every probe twice. The duplicates then execute concurrently on the
        // player's machine, race over the shared `.ololo/tmp` state, and return
        // each other's output, which corrupts scoring in both directions.
        if !is_current_connection(&state.player_agent_registry, player_id, &agent_tx) {
            tracing::info!(
                session_id = %session_id, player_id = %player_id,
                "player agent superseded by a newer connection; stopping dispatch"
            );
            return;
        }

        // Re-read session status each iteration. Paused skips dispatch (keeps
        // WS open, waits on cancel or short sleep); Finished/Cancelled exit;
        // Running proceeds with dispatch as today. In-flight probes complete
        // naturally — their result is recorded in the dispatch body, and the
        // next iteration's top-of-loop status check prevents ordinal advance
        // while paused (advance_to_next_task only runs inside the dispatch
        // body, which is skipped when Paused).
        let _cancel_token = state
            .session_registry
            .get(&join_code)
            .map(|e| e.cancel.clone())
            .unwrap_or_default();
        let session_status = match sessions::Entity::find_by_id(session_id)
            .one(&state.db)
            .await
        {
            Ok(Some(s)) => s.status,
            Ok(None) => {
                tracing::warn!(
                    session_id = %session_id,
                    "player_agent: session vanished from DB, exiting probe loop",
                );
                break;
            }
            Err(e) => {
                tracing::error!(
                    session_id = %session_id,
                    error = %e,
                    "player_agent: DB error reading session status, exiting probe loop",
                );
                break;
            }
        };
        match decide_probe_action(session_status) {
            ProbeAction::Dispatch => {}
            ProbeAction::Pause => {
                tracing::debug!(
                    session_id = %session_id,
                    status = %session_status,
                    "player_agent: session not running, skipping dispatch",
                );
                // Forward session broadcast frames (RunningCountdown paused,
                // SessionComplete, …) to the client while paused so the TUI
                // reflects the paused/cancelled state instead of freezing on the
                // last "Running" value.
                if drain_during_sleep(&mut socket, &mut session_rx, session_id, 1).await
                    == SleepOutcome::Disconnected
                {
                    return;
                }
            }
            ProbeAction::Exit => {
                tracing::info!(
                    session_id = %session_id,
                    status = %session_status,
                    "player_agent: session ended, exiting probe loop",
                );
                // Flush any queued session broadcast (e.g. SessionComplete with
                // reason "cancelled"/"time_expired") so the TUI shows the final
                // state before the socket closes.
                drain_during_sleep(&mut socket, &mut session_rx, session_id, 1).await;
                break;
            }
        }

        let task_id =
            match resolve_current_task_id(&state, session_id, player_id, &join_code, &mut socket)
                .await
            {
                Some(id) => id,
                None => break,
            };

        let task_row = match tasks::Entity::find_by_id(task_id).one(&state.db).await {
            Ok(Some(t)) => t,
            _ => {
                tracing::warn!(session_id = %session_id, "player_agent: task not found");
                break;
            }
        };

        let adapted_test = match pick_next_adapted_test(&state, &task_row, session_id, player_id)
            .await
        {
            Some(t) => t,
            None => {
                tracing::info!(session_id = %session_id, task_id = %task_id, "player_agent: all adapted tests passed, advancing task");
                // Open-ended: did the completion probe pass, or did the work
                // deadline force this? The bonus is earned only by actually
                // completing; a deadline cut takes what exists, unrewarded.
                let open_ended = if task_row.evaluation.is_some() {
                    crate::ws::player_agent::scheduler::open_ended_state(
                        &state, &task_row, session_id, player_id,
                    )
                    .await
                } else {
                    None
                };
                let completed_via_probe = open_ended.as_ref().is_none_or(|oe| oe.completion_passed);
                // Open-ended lifecycle: the completion contract is met (or the
                // window closed), but the task is not done yet — the judges
                // get to look first. Snapshot + judges fire once; the task is
                // then HELD while judges investigate and any probes they
                // register (screenshots, captures) resolve. Only then does it
                // advance — evaluation itself continues asynchronously.
                if open_ended.is_some() {
                    match judge_wait.filter(|(t, _)| *t == task_id) {
                        None => {
                            // First finalize pass for this task (or a
                            // reconnect): ask for the snapshot, fire judges.
                            let reason = if completed_via_probe {
                                arena_core::protocol::SNAPSHOT_REASON_TODO_COMPLETE
                            } else {
                                arena_core::protocol::SNAPSHOT_REASON_DEADLINE
                            };
                            let already_fired =
                                crate::ws::player_agent::scheduler::judge_runs_started(
                                    &state, task_id, player_id,
                                )
                                .await;
                            if !already_fired {
                                let frame = PlayerAgentFrame::SnapshotRequest {
                                    task_id,
                                    task_title: task_row.title.clone(),
                                    reason: reason.to_string(),
                                };
                                let json = serde_json::to_string(&frame).unwrap_or_default();
                                let _ = socket.send(Message::Text(json)).await;
                                spawn_task_judges(&state, session_id, player_id, task_id).await;
                            }
                            judge_wait = Some((task_id, std::time::Instant::now()));
                            // Tell every UI what this silence is: probes are
                            // done, the panel is looking.
                            crate::ws::player_agent::scheduler::mark_scheduler_judging(
                                &state, session_id, player_id,
                            )
                            .await;
                            // Keep the "next probe" clock alive: the judge
                            // phase is otherwise silent, and a stale past
                            // timestamp reads as a wedged session.
                            update_next_probe_at(
                                &state,
                                session_id,
                                player_id,
                                Some(Utc::now() + chrono::Duration::seconds(6)),
                            )
                            .await;
                            if drain_during_sleep(&mut socket, &mut session_rx, session_id, 2).await
                                == SleepOutcome::Disconnected
                            {
                                return;
                            }
                            continue;
                        }
                        Some((_, since)) => {
                            // Judge probes are dispatched by the regular loop
                            // (pick_next_adapted_test returns them first), so
                            // this hold only waits for them to pass.
                            let settled = crate::ws::player_agent::scheduler::judge_probes_settled(
                                &state, task_id, session_id, player_id,
                            )
                            .await;
                            // The cap runs from the LATER of the phase start
                            // and the participant's last delivery: captures
                            // arriving mean the requests are being worked.
                            let cap = crate::ws::player_agent::scheduler::judge_phase_cap_secs(
                                &state, task_id, session_id,
                            )
                            .await;
                            let quiet_since_delivery =
                                crate::ws::player_agent::scheduler::last_artifact_at(
                                    &state, task_id, session_id, player_id,
                                )
                                .await
                                .is_none_or(|t| (Utc::now() - t).num_seconds() >= cap as i64);
                            let capped = since.elapsed().as_secs() >= cap && quiet_since_delivery;
                            if !settled && !capped {
                                crate::ws::player_agent::scheduler::mark_scheduler_judging(
                                    &state, session_id, player_id,
                                )
                                .await;
                                update_next_probe_at(
                                    &state,
                                    session_id,
                                    player_id,
                                    Some(Utc::now() + chrono::Duration::seconds(8)),
                                )
                                .await;
                                if drain_during_sleep(&mut socket, &mut session_rx, session_id, 5)
                                    .await
                                    == SleepOutcome::Disconnected
                                {
                                    return;
                                }
                                continue;
                            }
                            if !settled {
                                tracing::warn!(
                                    session_id = %session_id, player_id = %player_id, task_id = %task_id,
                                    "judge-probe phase cap reached; expiring undelivered judge probes"
                                );
                            }
                        }
                    }
                    // Idempotent: marks only still-open probes of unpassed
                    // judge requests. Runs on every advance so a request
                    // that aged out of the queue (its own phase cap) does
                    // not linger as an eternally-pending probe row.
                    crate::ws::player_agent::scheduler::expire_unpassed_judge_probes(
                        &state, task_id, session_id, player_id,
                    )
                    .await;
                }
                judge_wait = None;
                let advanced =
                    advance_to_next_task(&state, session_id, player_id, &join_code, task_id).await;
                if task_row.completion_bonus_points != 0 && completed_via_probe {
                    let _ = arena_core::scoring::award_completion_bonus(
                        &state.db,
                        session_id,
                        player_id,
                        task_id,
                        task_row.completion_bonus_points,
                    )
                    .await;
                    publish_score_change(
                        &state,
                        session_id,
                        player_id,
                        task_row.completion_bonus_points as i64,
                        &join_code,
                    )
                    .await;
                    broadcast_leaderboard(&state, session_id, &join_code).await;
                }
                // Classic tasks fire their judges after the advance (ololo
                // commits on seeing the next TestPush, and the spawned run
                // waits for that commit). Open-ended tasks fired theirs at
                // the start of the judge-probe phase above.
                if open_ended.is_none() {
                    spawn_task_judges(&state, session_id, player_id, task_id).await;
                }
                if !advanced {
                    tracing::info!(session_id = %session_id, player_id = %player_id, "player_agent: player completed all tasks");
                    // Per-player acknowledgment first, then finish the session
                    // only when THIS player was the last eligible one still
                    // working (the broadcast SessionComplete with the real
                    // session-end reason is drained in the wait loop below).
                    let (frame, _finished) =
                        crate::state::on_player_tasks_exhausted(&state, session_id, &join_code)
                            .await;
                    let json = serde_json::to_string(&frame).unwrap_or_default();
                    let _ = socket.send(Message::Text(json)).await;
                    update_next_probe_at(&state, session_id, player_id, None).await;
                    // Idle until the session itself ends — mirrors the paused
                    // wait above: sleep in 1s slices while forwarding session
                    // broadcasts (leaderboard, final SessionComplete) so the
                    // real `all_tasks_completed`/`time_expired`/`cancelled`
                    // reason reaches the client when the session ends.
                    loop {
                        let status = match sessions::Entity::find_by_id(session_id)
                            .one(&state.db)
                            .await
                        {
                            Ok(Some(s)) => s.status,
                            _ => break,
                        };
                        if matches!(decide_probe_action(status), ProbeAction::Exit) {
                            // Flush the queued final broadcast before closing.
                            drain_during_sleep(&mut socket, &mut session_rx, session_id, 1).await;
                            break;
                        }
                        if drain_during_sleep(&mut socket, &mut session_rx, session_id, 1).await
                            == SleepOutcome::Disconnected
                        {
                            break; // client closed the socket
                        }
                    }
                    break;
                }
                continue;
            }
        };

        let adapted_test = match adapted_test {
            Some(t) => t,
            None => match ensure_adapted_test(&state, task_id, session_id).await {
                Ok(Some(t)) => t,
                Ok(None) => {
                    tracing::warn!(session_id = %session_id, "player_agent: no adapted test available");
                    break;
                }
                Err(e) => {
                    tracing::error!(session_id = %session_id, error = %e, "player_agent: failed to ensure adapted test");
                    break;
                }
            },
        };

        // Resolve interval fields: task override → project default (cached at session start).
        // ponytail: project defaults cached for session lifetime; mid-session
        // PATCH /api/projects does not invalidate running sessions. The clamp
        // below bounds the damage if defaults drift. Upgrade path: broadcast
        // invalidate on project PATCH if this ever bites.
        let (deadline_secs, min_interval_secs, interval_increment_secs, max_interval_secs) =
            match &proj_defaults {
                Some(p) => {
                    let dl = task_row
                        .deadline_secs
                        .unwrap_or(p.default_deadline_secs)
                        .max(1);
                    let mn = task_row
                        .min_interval_secs
                        .unwrap_or(p.default_min_interval_secs)
                        .max(0);
                    let inc = task_row
                        .interval_increment_secs
                        .unwrap_or(p.default_interval_increment_secs)
                        .max(1);
                    let mx = task_row
                        .max_interval_secs
                        .unwrap_or(p.default_max_interval_secs)
                        .max(0);
                    (dl, mn, inc, mx)
                }
                None => {
                    // No project row — use task values or safe baselines.
                    let dl = task_row.deadline_secs.unwrap_or(60).max(1);
                    let mn = task_row.min_interval_secs.unwrap_or(5).max(0);
                    let inc = task_row.interval_increment_secs.unwrap_or(5).max(1);
                    let mx = task_row.max_interval_secs.unwrap_or(60).max(0);
                    (dl, mn, inc, mx)
                }
            };
        // Defensive clamp: enforce min <= max, increment <= span.
        let (min_interval_secs, max_interval_secs, interval_increment_secs) = clamp_interval_bounds(
            min_interval_secs,
            max_interval_secs,
            interval_increment_secs,
        );
        if current_interval_secs < min_interval_secs {
            current_interval_secs = min_interval_secs;
        }

        // Merged per-player session memory (project defaults ⊕ LLM-extracted
        // values). Loaded per probe so extractions landing mid-task apply to
        // the next dispatch.
        let memory = crate::session_memory::load_memory_map(
            &state.db,
            session_id,
            player_id,
            proj_defaults
                .as_ref()
                .and_then(|p| p.memory_schema.as_deref()),
        )
        .await;

        let resolved = match resolve_test_fixtures(&adapted_test, session_id, &memory).await {
            Some(r) => r,
            None => break,
        };
        let js_validation_mode = resolved.js_validation_mode;
        let js_fixtures_for_eval = resolved.js_fixtures_for_eval;
        let secret_meta = resolved.secret_meta;
        let fixture_defs = resolved.fixture_defs;
        let fixture_scalars = resolved.fixture_scalars;
        let rendered_command = resolved.rendered_command;
        let fixture_values_for_eval = resolved.fixture_values_for_eval;
        let expected_answer_display = resolved.expected_answer_display;

        let probe_id = Uuid::new_v4();
        let now = Utc::now();
        let deadline_at = now + chrono::Duration::seconds(deadline_secs);

        // fixture_values JSON shipped to the client must have secret keys
        // redacted. The unredacted scalars were already used for command
        // rendering above; only the persisted/shipped JSON is stripped.
        let fixture_values_raw = serde_json::to_string(&fixture_scalars).unwrap_or_default();
        let fixture_values_json = secret_meta.redact_fixture_values(&fixture_values_raw);
        let secret_meta_json = secret_meta.to_json_string();
        let probe_am = probes::ActiveModel {
            id: Set(probe_id),
            test_id: Set(adapted_test.id),
            player_id: Set(player_id),
            session_id: Set(session_id),
            attempt: Set(1),
            rendered_command: Set(rendered_command.clone()),
            fixture_values: Set(fixture_values_json.clone()),
            expected_answer: Set(expected_answer_display.clone()),
            resolved_answer: Set(None),
            secret_meta: Set(secret_meta_json.clone()),
            outcome: Set(None),
            dispatched_at: Set(now),
            deadline_at: Set(deadline_at),
            resolved_at: Set(None),
            output: Set(None),
            exit_code: Set(None),
            duration_ms: Set(None),
            point_delta: Set(None),
            updated_at: Set(Some(now)),
            result_json: Set(None),
            artifact_path: Set(None),
        };
        if let Err(e) = probe_am.insert(&state.db).await {
            tracing::error!(session_id = %session_id, error = %e, "player_agent: probe insert failed");
            break;
        }
        crate::session_log_store::record(
            crate::session_log_store::base_dir(),
            session_id,
            Some(player_id),
            "probe_dispatched",
            serde_json::json!({
                "player_id": player_id,
                "task_id": task_id,
                "task_ordinal": task_row.ordinal,
                "probe_id": probe_id,
                "rendered_command": rendered_command,
                "expected_answer": expected_answer_display,
                "deadline_secs": deadline_secs,
            }),
        )
        .await;

        // When the task marks the expected value as secret, suppress
        // expected_answer and answer_template in the TestPush frame — the
        // server still grades with them, but the player never sees the
        // expected value. The DB row keeps the raw values for grading.
        let push_expected_answer = if secret_meta.expected {
            None
        } else {
            expected_answer_display.clone()
        };
        let push_answer_template = if secret_meta.expected {
            String::new()
        } else {
            adapted_test.answer_template.clone()
        };
        let (test_ordinal, test_total) =
            test_position(&state, task_id, session_id, adapted_test.id).await;
        let push_frame = PlayerAgentFrame::TestPush {
            probe_id,
            rendered_command: rendered_command.clone(),
            deadline_secs,
            task_id: Some(task_id),
            task_ordinal: task_row.ordinal,
            task_title: task_row.title.clone(),
            task_description: task_row.content.clone(),
            test_ordinal,
            test_total,
            test_label: arena_core::protocol::test_display_label(&adapted_test.prompt)
                .unwrap_or_default()
                .to_string(),
            test_description: adapted_test
                .description
                .as_deref()
                .map(str::trim)
                .unwrap_or_default()
                .to_string(),
            expected_answer: push_expected_answer,
            answer_template: push_answer_template,
            validation_kind: if js_validation_mode {
                arena_core::protocol::ValidationKind::Javascript
            } else {
                arena_core::protocol::ValidationKind::Minijinja
            },
        };
        let push_json = serde_json::to_string(&push_frame).unwrap_or_default();
        if socket.send(Message::Text(push_json)).await.is_err() {
            tracing::warn!(session_id = %session_id, "player_agent: socket send failed, disconnecting");
            break;
        }

        let deadline_duration = std::time::Duration::from_secs(deadline_secs.max(1) as u64);
        // Set when the agent announces a pushed completion flag while a probe
        // is in flight — the post-grading sleep is skipped so the completion
        // probe dispatches right after the current probe settles.
        let mut completion_flag_nudge = false;
        let result = tokio::time::timeout(deadline_duration, async {
            loop {
                tokio::select! {
                    msg = socket.recv() => {
                        match msg {
                            Some(Ok(Message::Text(text))) => {
                                if let Ok(frame) = serde_json::from_str::<PlayerAgentClientFrame>(&text) {
                                    // Not a probe result: the agent is telling
                                    // us it pushed new memory sources. Act on
                                    // it and keep waiting — returning it here
                                    // would hand a non-TestResult to grading
                                    // and fail the probe.
                                    if matches!(frame, PlayerAgentClientFrame::MemorySourcesPushed) {
                                        let mem_state = state.clone();
                                        tokio::spawn(async move {
                                            crate::session_memory::extract_after_commit(
                                                mem_state, session_id, player_id,
                                            )
                                            .await;
                                        });
                                        continue;
                                    }
                                    // Same rule: a completion-flag announce is
                                    // not a probe result. Note it and keep
                                    // waiting for the in-flight probe.
                                    if let PlayerAgentClientFrame::CompletionFlagPushed { path } = &frame {
                                        tracing::info!(
                                            session_id = %session_id,
                                            path = %path,
                                            "player_agent: completion flag pushed during probe wait"
                                        );
                                        completion_flag_nudge = true;
                                        continue;
                                    }
                                    return Some(frame);
                                }
                            }
                            Some(Ok(Message::Close(_))) | None => return None,
                            _ => {}
                        }
                    }
                    pushed = agent_rx.recv() => {
                        if let Some(f) = pushed {
                            if let PlayerAgentFrame::SessionComplete { .. } = &f {
                                let json = serde_json::to_string(&f).unwrap_or_default();
                                let _ = socket.send(Message::Text(json)).await;
                                return None;
                            }
                            let json = serde_json::to_string(&f).unwrap_or_default();
                            let _ = socket.send(Message::Text(json)).await;
                        }
                    }
                    bcast = async {
                        if let Some(rx) = session_rx.as_mut() {
                            rx.recv().await.ok()
                        } else {
                            std::future::pending::<Option<ArenaFrame>>().await
                        }
                    } => {
                        if let Some(frame) = bcast {
                            // Bridge dashboard frames to the agent wire. Only
                            // `SessionComplete` carries a `reason` the agent
                            // needs (e.g. "cancelled"); forward it as the
                            // agent-frame variant so ololo can show the end state.
                            let json = match frame {
                                ArenaFrame::SessionComplete { reason, .. } => {
                                    serde_json::to_string(&PlayerAgentFrame::SessionComplete {
                                        session_id,
                                        reason: Some(reason),
                                    })
                                    .unwrap_or_default()
                                }
                                other => serde_json::to_string(&other).unwrap_or_default(),
                            };
                            let _ = socket.send(Message::Text(json)).await;
                        }
                    }
                }
            }
        })
        .await;

        // ponytail: on pass, skip the inter-probe backoff and re-dispatch after
        // only PASS_REDISPATCH_FLOOR_SECS. Failures still use
        // current_interval_secs backoff.
        let mut dispatch_next_immediately = false;
        let should_break = grade_test_result(
            result,
            &state,
            &mut socket,
            session_id,
            player_id,
            &join_code,
            probe_id,
            task_id,
            &task_row,
            &adapted_test,
            &fixture_defs,
            &fixture_values_for_eval,
            &expected_answer_display,
            &secret_meta,
            js_validation_mode,
            &js_fixtures_for_eval,
            &memory,
            interval_increment_secs,
            min_interval_secs,
            max_interval_secs,
            &mut current_interval_secs,
            &mut dispatch_next_immediately,
        )
        .await;
        if should_break {
            break;
        }

        if completion_flag_nudge {
            dispatch_next_immediately = true;
        }
        // A judge's ask just went out and other judges' asks have never
        // been handed over: send those now rather than one per interval, so
        // the agent sees every open request together (4I2GFR: the second
        // judge's capture reached the participant a minute and a half after
        // the first, and the participant worked them one at a time).
        if adapted_test.registered_by_judge_id.is_some()
            && crate::ws::player_agent::scheduler::undispatched_judge_requests(
                &state, task_id, session_id, player_id,
            )
            .await
                > 0
        {
            dispatch_next_immediately = true;
        }

        let sleep_secs = next_probe_delay_secs(current_interval_secs, dispatch_next_immediately);
        let next_probe_at = Utc::now() + chrono::Duration::seconds(sleep_secs as i64);
        update_next_probe_at(&state, session_id, player_id, Some(next_probe_at)).await;
        // Drain the session broadcast during the inter-probe sleep so RunningCountdown
        // (and any other session) frames reach the client without waiting for the
        // next probe dispatch. Also handles clean client disconnect and pings.
        match drain_during_sleep(&mut socket, &mut session_rx, session_id, sleep_secs as u64).await
        {
            SleepOutcome::Disconnected => return,
            // The agent pushed a completion flag file: dispatch the next
            // probe now — for an open-ended task the scheduler picks the
            // completion probe, so the declared build reaches the judges
            // without waiting out the interval.
            SleepOutcome::CompletionFlag => {
                update_next_probe_at(&state, session_id, player_id, Some(Utc::now())).await;
            }
            SleepOutcome::Elapsed => {}
        }
    }

    tracing::info!(session_id = %session_id, join_code = %join_code, "player_agent: WS disconnected");
}

/// Fetch a task's attached judges (ordinal order) and spawn the bounded
/// judge-dispatch task for them. The runs are spawned, not awaited: the
/// spawned task first waits (bounded) for the `feat(<task_id>)` commit,
/// then runs the judges. When the admission semaphore is saturated the
/// dispatch is shed — the recovery sweep re-runs any judge that was never
/// recorded.
async fn spawn_task_judges(
    state: &GameServerState,
    session_id: Uuid,
    player_id: Uuid,
    task_id: Uuid,
) {
    let task_judges = arena_core::entities::task_judges::Entity::find()
        .filter(arena_core::entities::task_judges::Column::TaskId.eq(task_id))
        .order_by(
            arena_core::entities::task_judges::Column::Ordinal,
            Order::Asc,
        )
        .all(&state.db)
        .await
        .unwrap_or_default();
    if task_judges.is_empty() {
        return;
    }
    match crate::judge_queue::judge_admission().try_acquire() {
        Ok(permit) => {
            let judge_state = state.clone();
            tokio::spawn(async move {
                let _permit = permit;
                crate::judge_queue::run_task_judges_after_commit(
                    &judge_state,
                    session_id,
                    player_id,
                    task_id,
                    task_judges,
                )
                .await;
            });
        }
        Err(_) => {
            tracing::warn!(
                session_id = %session_id,
                player_id = %player_id,
                task_id = %task_id,
                "judge dispatch saturated; deferring to recovery sweep"
            );
        }
    }
}
