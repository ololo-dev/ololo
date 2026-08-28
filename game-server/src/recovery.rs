use crate::state::{
    GameServerState, SessionCacheInner, SessionEntry, compute_remaining, finish_session,
};
use crate::ws::session_lifecycle::running_timer;
use arena_core::entities::{projects, sessions};
use arena_core::session_status::SessionStatus;
use chrono::{DateTime, Utc};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use std::sync::{Arc, RwLock};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

/// Milliseconds to sleep between spawning recovered session timers on
/// startup. Read from `ARENA_RESTART_STAGGER_MS`; default 50ms. No hard cap.
pub fn restart_stagger_ms() -> u64 {
    std::env::var("ARENA_RESTART_STAGGER_MS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(50)
}

/// Recovery action for one recovered session. Decided purely from the
/// session's persisted status and recomputed remaining seconds. Pure so it
/// can be tested without a DB or a running tokio runtime.
pub enum RecoveryAction {
    /// Re-arm the running countdown timer with `remaining` seconds.
    SpawnRunning { remaining: i64 },
    /// Session is Paused: spawn a parked task polling the DB every 1s. When
    /// the owner flips status to Running, the parked task hands off to
    /// running_timer with the recomputed remaining.
    SpawnParked,
    /// Remaining already elapsed to zero: transition straight to Finished.
    FinishNow,
}

/// Decide how to recover one session from its persisted status and the
/// FR-012 remaining arithmetic (`max(0, duration - elapsed - paused)`).
pub fn decide_recovery(status: SessionStatus, remaining: i64) -> RecoveryAction {
    match status {
        SessionStatus::Running => {
            if remaining > 0 {
                RecoveryAction::SpawnRunning { remaining }
            } else {
                RecoveryAction::FinishNow
            }
        }
        SessionStatus::Paused => RecoveryAction::SpawnParked,
        // Lobby, Finished, Cancelled: not in the query set; defensive no-op.
        _ => RecoveryAction::FinishNow,
    }
}

/// Restart timers for `Running` sessions and spawn parked pollers for
/// `Paused` sessions owned by this game server. Spawns are staggered by
/// `ARENA_RESTART_STAGGER_MS` (default 50ms). No hard cap.
pub async fn resume_on_startup(state: GameServerState) -> anyhow::Result<()> {
    let server_id = state.server_id;
    let stagger = std::time::Duration::from_millis(restart_stagger_ms());

    let rows = sessions::Entity::find()
        .filter(sessions::Column::GameServerId.eq(server_id))
        .filter(
            sessions::Column::Status
                .eq(SessionStatus::Running)
                .or(sessions::Column::Status.eq(SessionStatus::Paused)),
        )
        .find_also_related(projects::Entity)
        .all(&state.db)
        .await?;

    if rows.is_empty() {
        tracing::info!(server_id = %server_id, "recovery: no running/paused sessions to resume");
        return Ok(());
    }

    tracing::info!(
        server_id = %server_id,
        count = rows.len(),
        stagger_ms = stagger.as_millis() as u64,
        "recovery: resuming sessions",
    );

    let mut first = true;
    for (session, project) in rows {
        if !first {
            tokio::time::sleep(stagger).await;
        }
        first = false;

        let session_id = session.id;
        let join_code = session.join_code.clone();
        let status = session.status;
        let started_at = session.started_at;
        let paused_duration_secs = session.paused_duration_secs;
        let project_id = session.project_id_fk;

        // Duration is project-sourced (projects.default_session_duration_secs).
        let duration_secs = match project {
            Some(p) => p.default_session_duration_secs.max(0) as u64,
            None => {
                // FK is NOT NULL, so this only happens if the project row was
                // deleted out from under the session.
                tracing::error!(
                    session_id = %session_id,
                    join_code = %join_code,
                    "recovery: project row missing, skipping session",
                );
                continue;
            }
        };

        let now = Utc::now();
        let remaining = started_at
            .map(|sa| compute_remaining(duration_secs, sa, now, paused_duration_secs))
            .unwrap_or(duration_secs as i64);

        let action = decide_recovery(status, remaining);
        match action {
            RecoveryAction::SpawnRunning { remaining } => {
                tracing::info!(
                    session_id = %session_id,
                    join_code = %join_code,
                    remaining = remaining,
                    "recovery: spawning running_timer",
                );
                register_session_entry(
                    &state,
                    &join_code,
                    session_id,
                    SessionStatus::Running,
                    started_at,
                );
                // `remaining` decides SpawnRunning vs FinishNow only; running_timer
                // re-reads the project's duration and recomputes remaining from
                // started_at on each tick.
                let timer_state = state.clone();
                let jc = join_code.clone();
                tokio::spawn(running_timer(timer_state, session_id, String::new(), jc));
            }
            RecoveryAction::SpawnParked => {
                tracing::info!(
                    session_id = %session_id,
                    join_code = %join_code,
                    "recovery: spawning parked poller for paused session",
                );
                register_session_entry(
                    &state,
                    &join_code,
                    session_id,
                    SessionStatus::Paused,
                    started_at,
                );
                let timer_state = state.clone();
                let jc = join_code.clone();
                tokio::spawn(parked_poller(
                    timer_state,
                    session_id,
                    project_id,
                    jc,
                    started_at,
                ));
            }
            RecoveryAction::FinishNow => {
                tracing::info!(
                    session_id = %session_id,
                    join_code = %join_code,
                    "recovery: remaining elapsed, transitioning to Finished",
                );
                finish_session(&state, session_id, &join_code, "time_expired").await;
            }
        }
    }

    Ok(())
}

/// How far back the startup sweep looks for finished sessions that never
/// got their Arena Points. Bounded so ancient pre-awards-era sessions are
/// never retro-awarded under today's formula.
pub const UNAWARDED_RECOVERY_WINDOW_HOURS: i64 = 24;

/// Announce settlement for recently finished sessions whose in-memory
/// settle task was lost. The post-expiry flow defers the `SessionSettled`
/// event in an in-memory task until judge runs finish — a deploy or crash
/// inside that window (up to 10 minutes) loses the task and the event with
/// it. This sweep closes that gap on the next startup.
///
/// Per session: when no expected judge runs are pending, settle directly
/// (never re-runs judges — reruns are not deduplicated and would burn LLM
/// tokens); otherwise resume the normal settle-then-announce flow.
pub async fn settle_unsettled_finished_sessions(state: GameServerState) -> anyhow::Result<()> {
    let cutoff = Utc::now() - chrono::Duration::hours(UNAWARDED_RECOVERY_WINDOW_HOURS);
    let finished = sessions::Entity::find()
        .filter(sessions::Column::GameServerId.eq(state.server_id))
        .filter(sessions::Column::Status.eq(SessionStatus::Finished))
        .filter(sessions::Column::FinishedAt.gte(cutoff))
        .all(&state.db)
        .await?;

    for session in finished {
        let pending =
            arena_core::session_completion::expired_session_pending_judges(&state.db, session.id)
                .await
                .unwrap_or(0);
        tracing::info!(
            session_id = %session.id,
            join_code = %session.join_code,
            pending,
            "recovery: re-announcing settlement for a finished session"
        );
        if pending == 0 {
            crate::state::settle_and_publish(&state, session.id, &session.join_code, Utc::now())
                .await;
        } else {
            let st = state.clone();
            let jc = session.join_code.clone();
            let sid = session.id;
            tokio::spawn(async move {
                crate::state::expiry_judges_then_award(
                    &st,
                    sid,
                    &jc,
                    std::time::Duration::from_secs(3),
                    std::time::Duration::from_secs(600),
                )
                .await;
            });
        }
    }
    Ok(())
}

/// Ensure a `SessionEntry` with a fresh `CancellationToken` exists in the
/// registry for a recovered session. Idempotent: leaves an existing entry
/// (e.g. a reconnecting client already created one) untouched.
fn register_session_entry(
    state: &GameServerState,
    join_code: &str,
    session_id: Uuid,
    phase: SessionStatus,
    started_at: Option<DateTime<Utc>>,
) {
    if state.session_registry.contains_key(join_code) {
        return;
    }
    let (tx, _) = tokio::sync::broadcast::channel(256);
    let cancel = CancellationToken::new();
    let cache = Arc::new(RwLock::new(SessionCacheInner::new(
        session_id, phase, started_at,
    )));
    state
        .session_registry
        .insert(join_code.to_string(), SessionEntry { tx, cache, cancel });
}

/// Parked poller for a Paused session. Polls the DB every 1s. On Running,
/// hands off to `running_timer` with recomputed remaining. On a terminal
/// status (Finished/Cancelled) or entry cancellation, exits.
async fn parked_poller(
    state: GameServerState,
    session_id: Uuid,
    _project_id: Uuid,
    join_code: String,
    started_at: Option<DateTime<Utc>>,
) {
    let cancel = state
        .session_registry
        .get(&join_code)
        .map(|e| e.cancel.clone())
        .unwrap_or_default();

    loop {
        let (model, project) = match sessions::Entity::find_by_id(session_id)
            .find_also_related(projects::Entity)
            .one(&state.db)
            .await
        {
            Ok(Some(pair)) => pair,
            Ok(None) => {
                tracing::warn!(session_id = %session_id, "recovery parked: session gone, exiting");
                return;
            }
            Err(e) => {
                tracing::error!(session_id = %session_id, error = %e, "recovery parked: DB error, retrying");
                tokio::select! {
                    _ = cancel.cancelled() => return,
                    _ = tokio::time::sleep(std::time::Duration::from_secs(1)) => {}
                }
                continue;
            }
        };

        match model.status {
            SessionStatus::Running => {
                // Duration is project-sourced (projects.default_session_duration_secs).
                let duration_secs = match project {
                    Some(p) => p.default_session_duration_secs.max(0) as u64,
                    None => {
                        // FK is NOT NULL, so this only happens if the project
                        // row was deleted out from under the session.
                        tracing::error!(session_id = %session_id, "recovery parked: project row missing, exiting");
                        return;
                    }
                };
                let now = Utc::now();
                let remaining = started_at
                    .map(|sa| compute_remaining(duration_secs, sa, now, model.paused_duration_secs))
                    .unwrap_or(duration_secs as i64);
                if remaining <= 0 {
                    finish_session(&state, session_id, &join_code, "time_expired").await;
                    return;
                }
                tracing::info!(
                    session_id = %session_id,
                    join_code = %join_code,
                    remaining = remaining,
                    "recovery parked: owner resumed, handing off to running_timer",
                );
                running_timer(state, session_id, String::new(), join_code).await;
                return;
            }
            SessionStatus::Finished | SessionStatus::Cancelled => {
                tracing::info!(
                    session_id = %session_id,
                    status = %model.status,
                    "recovery parked: terminal status, exiting",
                );
                return;
            }
            _ => {}
        }

        tokio::select! {
            _ = cancel.cancelled() => {
                tracing::info!(session_id = %session_id, "recovery parked: cancelled, exiting");
                return;
            }
            _ = tokio::time::sleep(std::time::Duration::from_secs(1)) => {}
        }
    }
}

/// Judge runs never started, that nobody will ever start again.
///
/// A judge run is scheduled only in memory: the socket spawns
/// `run_task_judges_after_commit`, which first waits up to 90s for the task's
/// snapshot commit. Anything that ends the process inside that window drops
/// every waiting task silently, and no other mechanism re-drives them —
/// [`settle_unsettled_finished_sessions`] recovers *settlement*, not judges. A
/// session can therefore be scored and awarded with its judges stuck at
/// `pending` forever, with nothing raising an alarm. Session VKIBCB reached
/// that state with all 17 of its judges unrun, which is why nothing caught a
/// player whose tasks were answered by a lookup table.
///
/// This sweep enqueues the (player, task_judge) pairs over the tasks each
/// player actually reached — the same set the settle poll waits for, via
/// `reached_tasks_for_player`, so the two cannot disagree. Runs that already
/// produced a terminal row are left alone, which makes the sweep idempotent:
/// once a pair is `scored` or `failed` it is never enqueued again.
///
/// Startup entry point: drives every missing pair immediately (after a restart
/// nothing is running, so nothing can be double-driven).
pub async fn enqueue_missed_judge_runs(state: GameServerState) -> anyhow::Result<()> {
    enqueue_missed_judge_runs_inner(state, None).await
}

/// Periodic entry point: the startup sweep fires once, but a judge that hangs
/// (or crashes) mid-run *after* boot has no re-drive and stays `pending`
/// forever — until the next deploy. Call this on a timer so a stuck judge
/// self-heals within minutes. `min_age` skips sessions that finished too
/// recently, so a judge that is legitimately still running (a `running`/
/// `pending` row is non-terminal, hence "missing" to the sweep) is never
/// double-driven: no judge takes longer than the floor the caller passes.
pub async fn enqueue_missed_judge_runs_older_than(
    state: GameServerState,
    min_age: std::time::Duration,
) -> anyhow::Result<()> {
    enqueue_missed_judge_runs_inner(state, Some(min_age)).await
}

async fn enqueue_missed_judge_runs_inner(
    state: GameServerState,
    min_age: Option<std::time::Duration>,
) -> anyhow::Result<()> {
    use arena_core::entities::{judge_results, players, task_judges};
    use arena_core::session_completion::{JUDGE_RESULT_FAILED, JUDGE_RESULT_SCORED};
    use std::collections::HashSet;

    let now = Utc::now();
    let cutoff = now - chrono::Duration::hours(UNAWARDED_RECOVERY_WINDOW_HOURS);
    let mut query = sessions::Entity::find()
        .filter(sessions::Column::GameServerId.eq(state.server_id))
        .filter(sessions::Column::Status.eq(SessionStatus::Finished))
        .filter(sessions::Column::FinishedAt.gte(cutoff));
    if let Some(age) = min_age {
        let max_finished = now - chrono::Duration::from_std(age).unwrap_or_default();
        query = query.filter(sessions::Column::FinishedAt.lte(max_finished));
    }
    let finished = query.all(&state.db).await?;

    let mut enqueued = 0usize;
    for session in finished {
        // Terminal rows already recorded for this session.
        let terminal: HashSet<(Uuid, Uuid)> = judge_results::Entity::find()
            .filter(judge_results::Column::SessionIdFk.eq(session.id))
            .filter(judge_results::Column::Status.is_in([JUDGE_RESULT_SCORED, JUDGE_RESULT_FAILED]))
            .all(&state.db)
            .await?
            .into_iter()
            .map(|r| (r.player_id_fk, r.task_judge_id))
            .collect();

        let eligible = players::Entity::find()
            .filter(players::Column::SessionIdFk.eq(session.id))
            .filter(players::Column::RevokedAt.is_null())
            .all(&state.db)
            .await?;

        for player in eligible {
            let reached = arena_core::session_completion::reached_tasks_for_player(
                &state.db, session.id, player.id,
            )
            .await
            .unwrap_or_default();
            if reached.is_empty() {
                continue;
            }
            // Attachments created after the finish never ran in this session
            // (a later judge-panel rollout must not spawn retroactive runs).
            let attached = arena_core::session_completion::session_task_judge_filter(
                task_judges::Entity::find(),
                &session,
            )
            .filter(task_judges::Column::TaskId.is_in(reached))
            .all(&state.db)
            .await?;

            for tj in attached {
                if terminal.contains(&(player.id, tj.id)) {
                    continue;
                }
                tracing::warn!(
                    session_id = %session.id,
                    join_code = %session.join_code,
                    player_id = %player.id,
                    task_id = %tj.task_id,
                    judge_id = %tj.judge_id,
                    "recovery: judge run was never recorded; enqueueing"
                );
                match crate::judge_queue::enqueue_judge_run(
                    &state,
                    &state.db,
                    session.id,
                    player.id,
                    tj.task_id,
                    tj.judge_id,
                    false,
                )
                .await
                {
                    Ok(_) => enqueued += 1,
                    Err(e) => {
                        // The failure is persisted by the queue itself, which
                        // makes the pair terminal and stops it coming back.
                        tracing::warn!(
                            session_id = %session.id, player_id = %player.id,
                            task_id = %tj.task_id, error = ?e,
                            "recovery: recovered judge run failed"
                        );
                        enqueued += 1;
                    }
                }
            }
        }
    }
    if enqueued > 0 {
        tracing::info!(
            enqueued,
            "recovery: re-drove judge runs that were never recorded"
        );
    }
    Ok(())
}

/// Re-drive judge runs whose row is stuck at `running` with no process
/// driving it.
///
/// A `running` row is written when a run starts and flipped to
/// `scored`/`failed` when it ends; anything that kills the process mid-run (a
/// deploy, a crash) leaves it `running` forever. The missed-run sweep above
/// covers *finished* sessions (a non-terminal row counts as missing there),
/// but a cancelled session is outside its window on purpose — judging a
/// session nobody finished would burn tokens on abandoned work. A run that
/// had already STARTED is different: its evidence exists and the player page
/// shows "Evaluating…" until a terminal row lands. Session Y3W66Z froze
/// exactly this way — ux-review's artifact re-drive died with the process
/// seconds before the idle sweep cancelled the session.
///
/// `min_age` gates on the row's `updated_at` so a run legitimately executing
/// right now is never double-driven (same contract as the missed-run sweep's
/// floor: no judge takes longer). At startup pass zero — after a restart
/// nothing is running, so every `running` row is an orphan by definition.
pub async fn requeue_orphaned_judge_runs(
    state: GameServerState,
    min_age: std::time::Duration,
) -> anyhow::Result<()> {
    use arena_core::entities::{judge_results, task_judges};

    let stale_before = Utc::now() - chrono::Duration::from_std(min_age).unwrap_or_default();
    let orphans = judge_results::Entity::find()
        .filter(judge_results::Column::Status.eq("running"))
        .filter(judge_results::Column::UpdatedAt.lte(stale_before))
        .all(&state.db)
        .await?;

    let mut enqueued = 0usize;
    for row in orphans {
        let Some(tj) = task_judges::Entity::find_by_id(row.task_judge_id)
            .one(&state.db)
            .await?
        else {
            continue;
        };
        let Some(session) = sessions::Entity::find_by_id(row.session_id_fk)
            .one(&state.db)
            .await?
        else {
            continue;
        };
        if session.game_server_id != Some(state.server_id) {
            continue;
        }
        tracing::warn!(
            session_id = %row.session_id_fk,
            join_code = %session.join_code,
            player_id = %row.player_id_fk,
            task_id = %tj.task_id,
            judge_id = %tj.judge_id,
            "recovery: judge run orphaned mid-flight; re-enqueueing"
        );
        // Failures are persisted by the queue itself (terminal `failed`), so
        // a permanently broken judge does not come back every sweep.
        let _ = crate::judge_queue::enqueue_judge_run(
            &state,
            &state.db,
            row.session_id_fk,
            row.player_id_fk,
            tj.task_id,
            tj.judge_id,
            false,
        )
        .await;
        enqueued += 1;
    }
    if enqueued > 0 {
        tracing::info!(enqueued, "recovery: re-drove orphaned judge runs");
    }
    Ok(())
}

/// Error text on judge_results rows written by [`fail_abandoned_judge_runs`].
pub const ABANDONED_RUN_ERROR: &str = "judge run was lost and its session left the recovery window; settled as failed without a verdict";

/// Terminally close out judge runs that will never happen.
///
/// The re-drive sweeps above only reach sessions finished within
/// [`UNAWARDED_RECOVERY_WINDOW_HOURS`] *and* bound to this game server's id.
/// A run lost outside that window — an old deploy, a server-id change, an
/// infra migration that left the player repos behind — stays `pending`
/// forever: the player page shows "Judges scoring — N left" for a session
/// finished weeks ago (N2H4VA sat that way for nine days). Re-running judges
/// that late is wrong anyway (scores would shift long after the session
/// settled), so this reaper writes a zero-delta terminal `failed` row for
/// every expected-but-missing pair instead.
///
/// Deliberately NOT filtered by `game_server_id`: sessions orphaned by a
/// dead server id are exactly the ones nothing else will ever settle. The
/// `(task_judge_id, player_id)` unique index makes concurrent reapers (or a
/// race with a real late verdict) safe — inserts use on-conflict-do-nothing,
/// so a real row always wins.
pub async fn fail_abandoned_judge_runs(state: GameServerState) -> anyhow::Result<()> {
    use arena_core::entities::judge_results;
    use sea_orm::sea_query::OnConflict;

    let cutoff = Utc::now() - chrono::Duration::hours(UNAWARDED_RECOVERY_WINDOW_HOURS);
    let abandoned = sessions::Entity::find()
        .filter(sessions::Column::Status.eq(SessionStatus::Finished))
        .filter(sessions::Column::FinishedAt.lte(cutoff))
        .all(&state.db)
        .await?;

    let mut settled = 0usize;
    for session in abandoned {
        // A `running` row stuck in an abandoned session is the same lost run
        // wearing a different status (the orphan sweep skips it when the
        // session's server id is dead) — flip it to failed too. The 1-hour
        // age floor keeps a fresh admin-triggered re-run out of reach.
        let stale_running = Utc::now() - chrono::Duration::hours(1);
        judge_results::Entity::update_many()
            .col_expr(
                judge_results::Column::Status,
                sea_orm::sea_query::Expr::value("failed"),
            )
            .col_expr(
                judge_results::Column::Error,
                sea_orm::sea_query::Expr::value(ABANDONED_RUN_ERROR),
            )
            .col_expr(
                judge_results::Column::UpdatedAt,
                sea_orm::sea_query::Expr::value(Utc::now()),
            )
            .filter(judge_results::Column::SessionIdFk.eq(session.id))
            .filter(judge_results::Column::Status.eq("running"))
            .filter(judge_results::Column::UpdatedAt.lte(stale_running))
            .exec(&state.db)
            .await?;

        let pairs = arena_core::session_completion::expired_session_pending_judge_pairs(
            &state.db, session.id,
        )
        .await?;
        for pair in &pairs {
            let now = Utc::now();
            let row = judge_results::ActiveModel {
                id: sea_orm::Set(Uuid::new_v4()),
                session_id_fk: sea_orm::Set(session.id),
                player_id_fk: sea_orm::Set(pair.player_id),
                task_judge_id: sea_orm::Set(pair.task_judge_id),
                rating: sea_orm::Set(serde_json::Value::Null),
                point_delta: sea_orm::Set(0),
                feedback: sea_orm::Set(String::new()),
                model: sea_orm::Set("recovery:reaper".to_string()),
                provider: sea_orm::Set("recovery".to_string()),
                raw_output: sea_orm::Set(String::new()),
                duration_ms: sea_orm::Set(None),
                run_log: sea_orm::Set(None),
                tokens_input: sea_orm::Set(None),
                tokens_output: sea_orm::Set(None),
                tokens_cache_read: sea_orm::Set(None),
                tokens_cache_write: sea_orm::Set(None),
                status: sea_orm::Set("failed".to_string()),
                error: sea_orm::Set(Some(ABANDONED_RUN_ERROR.to_string())),
                verdict_kind: sea_orm::Set(None),
                created_at: sea_orm::Set(now),
                updated_at: sea_orm::Set(now),
            };
            judge_results::Entity::insert(row)
                .on_conflict(
                    OnConflict::columns([
                        judge_results::Column::TaskJudgeId,
                        judge_results::Column::PlayerIdFk,
                    ])
                    .do_nothing()
                    .to_owned(),
                )
                .exec_without_returning(&state.db)
                .await?;
            settled += 1;
        }
        if !pairs.is_empty() {
            tracing::warn!(
                session_id = %session.id,
                join_code = %session.join_code,
                pairs = pairs.len(),
                "recovery: reaped judge runs abandoned outside the recovery window"
            );
        }
    }
    if settled > 0 {
        tracing::info!(settled, "recovery: settled abandoned judge runs as failed");
    }
    Ok(())
}
