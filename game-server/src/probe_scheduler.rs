//! Interval ticker for server-executed system probes of open-ended tasks.
//!
//! The live agent loop dispatches participant-side probes; this ticker is
//! its server-side twin. Every tick it scans the sessions this game server
//! owns, finds each player's current open-ended task, and runs whichever
//! server-executor probes are due — `schedule.on: [start]` once, `[interval]`
//! every `interval_secs`. `done` probes are not fired here: they run at
//! completion, against the final commit (see `judge_queue`).
//!
//! Stateless on purpose: "due" is computed from the probe history itself
//! (last `dispatched_at` per test×player), so a restart never double-fires
//! or loses cadence — the same property `next_probe_at` recovery relies on.

use arena_core::entities::{probes, projects, session_scheduler_state, sessions, tasks, tests};
use arena_core::evaluation::{ProbeConfig, ProbeExecutor, ProbeMode, ScheduleOn};
use arena_core::session_status::SessionStatus;
use chrono::Utc;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder};
use uuid::Uuid;

use crate::state::GameServerState;
use crate::ws::player_agent::scheduler::judge_phase_expired;

/// Tick cadence. Probes declare their own intervals; this only bounds how
/// stale a due probe can go unnoticed.
fn tick_interval() -> std::time::Duration {
    let secs = std::env::var("ARENA_PROBE_TICK_SECS")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(10)
        .clamp(2, 300);
    std::time::Duration::from_secs(secs)
}

/// Run the ticker until the process exits.
pub async fn run_probe_scheduler(state: GameServerState) {
    let mut ticker = tokio::time::interval(tick_interval());
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        ticker.tick().await;
        if let Err(e) = tick(&state).await {
            tracing::warn!(error = %e, "probe scheduler: tick failed");
        }
    }
}

/// One scan: run every due server-side probe of every open-ended task
/// currently being worked in a session this server owns, resolve interactive
/// probes whose participant deadline passed, and re-drive judges whose
/// awaited artifact arrived or timed out.
pub async fn tick(state: &GameServerState) -> Result<(), sea_orm::DbErr> {
    // Arrived artifacts resolve first; only then do deadlines time out —
    // a push that lands on the deadline tick counts as arrived.
    crate::artifacts::resolve_pending_artifacts(state).await?;
    resolve_interactive_timeouts(state).await?;
    redrive_waiting_judges(state).await?;

    let running: Vec<sessions::Model> = sessions::Entity::find()
        .filter(sessions::Column::Status.eq(SessionStatus::Running))
        .filter(sessions::Column::GameServerId.eq(state.server_id))
        .all(&state.db)
        .await?;

    for session in running {
        let cursors = session_scheduler_state::Entity::find()
            .filter(session_scheduler_state::Column::SessionIdFk.eq(session.id))
            .filter(session_scheduler_state::Column::TaskId.is_not_null())
            .all(&state.db)
            .await?;
        for cursor in cursors {
            let Some(task_id) = cursor.task_id else {
                continue;
            };
            let Some(task) = tasks::Entity::find_by_id(task_id).one(&state.db).await? else {
                continue;
            };
            if task.evaluation.is_none() {
                continue;
            }
            run_due_probes_for(state, &session, cursor.player_id_fk, &task).await?;
        }
    }
    Ok(())
}

/// Run this player's due server-side probes for one open-ended task.
async fn run_due_probes_for(
    state: &GameServerState,
    session: &sessions::Model,
    player_id: Uuid,
    task: &tasks::Model,
) -> Result<(), sea_orm::DbErr> {
    let test_rows = tests::Entity::find()
        .filter(tests::Column::TaskId.eq(task.id))
        .filter(tests::Column::SessionId.eq(session.id))
        .filter(tests::Column::ProbeConfig.is_not_null())
        .order_by_asc(tests::Column::Ordinal)
        .all(&state.db)
        .await?;
    if test_rows.is_empty() {
        return Ok(());
    }

    // Memory is loaded lazily — only when at least one probe is actually due.
    let mut memory: Option<std::collections::BTreeMap<String, String>> = None;

    for test in &test_rows {
        let Some(config_json) = &test.probe_config else {
            continue;
        };
        let Ok(config) = ProbeConfig::from_json(config_json) else {
            continue;
        };
        if config.effective_executor() != ProbeExecutor::Server {
            continue;
        }
        let Some(schedule) = &config.schedule else {
            continue;
        };
        let wants_interval = schedule.on.contains(&ScheduleOn::Interval);
        let wants_start = schedule.on.contains(&ScheduleOn::Start);
        if !wants_interval && !wants_start {
            continue; // done-only: fired at completion, not here
        }
        let due = match last_dispatch_at(state, test.id, player_id).await? {
            None => true,
            Some(last) => {
                wants_interval && {
                    let interval = schedule.interval_secs.unwrap_or(0).max(1) as i64;
                    (Utc::now() - last).num_seconds() >= interval
                }
            }
        };
        if !due {
            continue;
        }

        let memory = match &memory {
            Some(m) => m,
            None => {
                let schema = projects::Entity::find_by_id(session.project_id_fk)
                    .one(&state.db)
                    .await?
                    .and_then(|p| p.memory_schema);
                memory.insert(
                    crate::session_memory::load_memory_map(
                        &state.db,
                        session.id,
                        player_id,
                        schema.as_deref(),
                    )
                    .await,
                )
            }
        };

        // Interactive probes are participant-driven by definition.
        if config.mode == ProbeMode::Interactive {
            continue;
        }
        let run = {
            let Some(repos_base) = arena_core::git_store::repos_base_dir() else {
                tracing::warn!("probe scheduler: no repos base dir; skipping");
                continue;
            };
            let repo_dir =
                arena_core::git_store::player_repo_path(&repos_base, session.id, player_id);
            crate::probe_exec::run_server_probe(
                state, &repo_dir, session.id, player_id, test, &config, memory,
            )
            .await
        };
        {
            {
                let delta = run.point_delta;
                let output = run.output.clone();
                match crate::probe_exec::record_server_probe(
                    &state.db, session.id, player_id, test.id, &run,
                )
                .await
                {
                    Ok(recorded) => {
                        tracing::info!(
                            session_id = %session.id, player_id = %player_id,
                            test_id = %test.id, outcome = %run.outcome,
                            "probe scheduler: server probe recorded"
                        );
                        state
                            .event_publisher
                            .publish(&arena_core::protocol::ZmqEvent::ProbeFinished {
                                join_code: session.join_code.clone(),
                                player_id,
                                task_id: task.id,
                                probe_id: recorded.id,
                                mode: format!("{:?}", config.mode).to_lowercase(),
                                initiator: test.initiator.clone(),
                                outcome: run.outcome.to_string(),
                                timestamp: Utc::now(),
                            })
                            .await;
                        if delta != 0 {
                            crate::ws::player_agent::scoring::insert_task_result(
                                state,
                                session.id,
                                Some(task.id),
                                delta,
                                &output,
                                player_id,
                            )
                            .await;
                            crate::ws::player_agent::scoring::publish_score_change(
                                state,
                                session.id,
                                player_id,
                                delta as i64,
                                &session.join_code,
                            )
                            .await;
                            crate::ws::player_agent::scoring::broadcast_leaderboard(
                                state,
                                session.id,
                                &session.join_code,
                            )
                            .await;
                        }
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "probe scheduler: record failed");
                    }
                }
            }
        }
    }
    Ok(())
}

/// Interactive probes whose participant deadline passed without an artifact:
/// resolve `no_response`, zero points — the same silence semantics the live
/// loop has always had.
async fn resolve_interactive_timeouts(state: &GameServerState) -> Result<(), sea_orm::DbErr> {
    let overdue = probes::Entity::find()
        .filter(probes::Column::Outcome.is_null())
        .filter(probes::Column::DeadlineAt.lt(Utc::now()))
        .all(&state.db)
        .await?;
    for probe in overdue {
        let Ok(Some(test)) = tests::Entity::find_by_id(probe.test_id)
            .one(&state.db)
            .await
        else {
            continue;
        };
        let is_interactive = test
            .probe_config
            .as_ref()
            .and_then(|c| ProbeConfig::from_json(c).ok())
            .is_some_and(|c| c.mode == ProbeMode::Interactive);
        if !is_interactive {
            continue; // the live loop owns its own probes' deadlines
        }
        let now = Utc::now();
        let _ = probes::Entity::update_many()
            .col_expr(
                probes::Column::Outcome,
                sea_orm::prelude::Expr::value("no_response"),
            )
            .col_expr(
                probes::Column::ResolvedAt,
                sea_orm::prelude::Expr::value(now),
            )
            .col_expr(probes::Column::PointDelta, sea_orm::prelude::Expr::value(0))
            .col_expr(
                probes::Column::UpdatedAt,
                sea_orm::prelude::Expr::value(now),
            )
            .filter(probes::Column::Id.eq(probe.id))
            .filter(probes::Column::Outcome.is_null())
            .exec(&state.db)
            .await;
        tracing::info!(probe_id = %probe.id, "interactive probe timed out (no_response)");
    }
    Ok(())
}

/// Judges suspended on an interactive probe: once every awaited probe of a
/// `waiting` run is resolved (artifact landed, or timed out above), re-drive
/// the judge so it verdicts on the final evidence. A verdict reached after a
/// timed-out artifact is marked `partial`.
async fn redrive_waiting_judges(state: &GameServerState) -> Result<(), sea_orm::DbErr> {
    use arena_core::entities::{artifact_request_watchers, judge_results, task_judges};

    let waiting = judge_results::Entity::find()
        .filter(judge_results::Column::Status.eq("waiting"))
        .all(&state.db)
        .await?;
    for row in waiting {
        let Ok(Some(tj)) = task_judges::Entity::find_by_id(row.task_judge_id)
            .one(&state.db)
            .await
        else {
            continue;
        };
        // This session must be ours to drive.
        let Ok(Some(session)) = sessions::Entity::find_by_id(row.session_id_fk)
            .one(&state.db)
            .await
        else {
            continue;
        };
        if session.game_server_id != Some(state.server_id) {
            continue;
        }
        // The awaited probes: this judge's interactive registrations on the
        // task, plus requests it ATTACHED to (another judge's open ask it
        // agreed with instead of duplicating). Unresolved with a future
        // deadline ⇒ keep waiting.
        let mut registered = tests::Entity::find()
            .filter(tests::Column::SessionId.eq(row.session_id_fk))
            .filter(tests::Column::TaskId.eq(tj.task_id))
            .filter(tests::Column::RegisteredByJudgeId.eq(tj.judge_id))
            .all(&state.db)
            .await?;
        let watched_ids: Vec<Uuid> = artifact_request_watchers::Entity::find()
            .filter(artifact_request_watchers::Column::JudgeId.eq(tj.judge_id))
            .all(&state.db)
            .await?
            .into_iter()
            .map(|w| w.test_id)
            .collect();
        if !watched_ids.is_empty() {
            let watched = tests::Entity::find()
                .filter(tests::Column::SessionId.eq(row.session_id_fk))
                .filter(tests::Column::TaskId.eq(tj.task_id))
                .filter(tests::Column::Id.is_in(watched_ids))
                .all(&state.db)
                .await?;
            for t in watched {
                if !registered.iter().any(|r| r.id == t.id) {
                    registered.push(t);
                }
            }
        }
        // What counts as answered depends on what was asked.
        //
        // An artifact request is answered by a PASS (the file arrived) or by
        // expiry (`no_response`); a failed check there just means the file is
        // not there yet and the regular queue will ask again — keep waiting.
        //
        // A deterministic probe is different: the judge asked the player to
        // RUN something, and "it failed" is the answer, not the absence of
        // one. Waiting for a pass would hold the judge until the session
        // ended every time the tests are red — which is exactly the verdict
        // it was registered to inform.
        //
        // Once the session is no longer running nobody will dispatch or
        // deliver anything, so an unanswered probe counts as expired: the
        // judge verdicts (partial) instead of waiting for ever. The same
        // holds for a request past the judge-phase cap: the queue stopped
        // re-dispatching it and the task moved on, so a failed poll is the
        // final word — waiting for a pass would park the judge until the
        // session ended, blind to whatever did arrive.
        let session_running = session.status == SessionStatus::Running;
        let mut still_waiting = false;
        let mut any_no_response = false;
        for test in &registered {
            let interactive = test
                .probe_config
                .as_ref()
                .and_then(|c| ProbeConfig::from_json(c).ok())
                .is_some_and(|c| c.mode == ProbeMode::Interactive);
            let rows = probes::Entity::find()
                .filter(probes::Column::TestId.eq(test.id))
                .filter(probes::Column::PlayerId.eq(row.player_id_fk))
                .order_by_desc(probes::Column::DispatchedAt)
                .all(&state.db)
                .await?;
            if rows.iter().any(|p| p.outcome.as_deref() == Some("pass")) {
                continue;
            }
            let latest = rows.first().and_then(|p| p.outcome.as_deref());
            match latest {
                Some("no_response") => any_no_response = true,
                // A run that came back — pass, fail or error — has answered
                // a "please run this" ask.
                Some(_) if !interactive => continue,
                _ if !session_running || judge_phase_expired(test) => any_no_response = true,
                _ => still_waiting = true,
            }
        }
        if still_waiting {
            continue;
        }

        tracing::info!(
            session_id = %row.session_id_fk, player_id = %row.player_id_fk,
            task_id = %tj.task_id, judge_id = %tj.judge_id,
            "re-driving waiting judge"
        );
        let redrive_state = state.clone();
        let (session_id, player_id, task_id, judge_id) =
            (row.session_id_fk, row.player_id_fk, tj.task_id, tj.judge_id);
        let (task_judge_id, mark_partial) = (row.task_judge_id, any_no_response);
        tokio::spawn(async move {
            let outcome = crate::judge_queue::enqueue_judge_run(
                &redrive_state,
                &redrive_state.db,
                session_id,
                player_id,
                task_id,
                judge_id,
                false,
            )
            .await;
            if outcome.is_ok() && mark_partial {
                // The artifact never came: the verdict stands on less than
                // the judge asked to see.
                let _ = judge_results::Entity::update_many()
                    .col_expr(
                        judge_results::Column::VerdictKind,
                        sea_orm::prelude::Expr::value(arena_core::evaluation::VERDICT_KIND_PARTIAL),
                    )
                    .filter(judge_results::Column::TaskJudgeId.eq(task_judge_id))
                    .filter(judge_results::Column::PlayerIdFk.eq(player_id))
                    .filter(judge_results::Column::Status.eq("scored"))
                    .exec(&redrive_state.db)
                    .await;
            }
        });
    }
    Ok(())
}

async fn last_dispatch_at(
    state: &GameServerState,
    test_id: Uuid,
    player_id: Uuid,
) -> Result<Option<chrono::DateTime<Utc>>, sea_orm::DbErr> {
    Ok(probes::Entity::find()
        .filter(probes::Column::TestId.eq(test_id))
        .filter(probes::Column::PlayerId.eq(player_id))
        .order_by_desc(probes::Column::DispatchedAt)
        .one(&state.db)
        .await?
        .map(|p| p.dispatched_at))
}
