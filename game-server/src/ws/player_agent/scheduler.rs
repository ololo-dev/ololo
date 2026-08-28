use crate::state::GameServerState;
use arena_core::entities::{
    activity_event, players, session_scheduler_state, sessions, tasks, tests,
};
use arena_core::protocol::{ArenaFrame, ZmqEvent};
use arena_core::session_status::SessionStatus;
use axum::extract::ws::WebSocket;
use chrono::Utc;
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, QueryOrder, Set};
use uuid::Uuid;

pub async fn pick_next_adapted_test(
    state: &GameServerState,
    task_row: &tasks::Model,
    session_id: Uuid,
    player_id: Uuid,
) -> Option<Option<tests::Model>> {
    let task_id = task_row.id;

    // Open-ended task: the completion contract replaces the all-sections
    // gate. The completion probe is dispatched until it passes or the work
    // deadline expires; either way the task then completes.
    if task_row.evaluation.is_some()
        && let Some(oe) = open_ended_state(state, task_row, session_id, player_id).await
    {
        // Judge-registered probes ride the same queue as every other probe:
        // the oldest one without a pass is the current probe, dispatched and
        // retried by the regular machinery until it passes (or the judge
        // phase gives up). Nothing else advances while it is open.
        if let Some(judge_test) =
            oldest_unpassed_judge_test(state, task_id, session_id, player_id).await
        {
            return Some(Some(judge_test));
        }
        if oe.completion_passed || oe.deadline_expired {
            return None;
        }
        if let Some(t) = oe.completion_test {
            // A due scheduled participant section (e.g. the TODO report)
            // fires first; otherwise the completion probe is polled at the
            // loop's own cadence.
            if let Some(due) =
                most_overdue_participant_section(state, task_id, session_id, player_id, t.id).await
            {
                return Some(Some(due));
            }
            return Some(Some(t));
        }
        // The contract named a section that produced no tests row — seed
        // validation should have refused this; fall through to the classic
        // rule rather than wedging the task.
        tracing::warn!(%task_id, "open-ended completion probe has no tests row; using classic gating");
    }

    let all_tests: Vec<tests::Model> = tests::Entity::find()
        .filter(tests::Column::TaskId.eq(task_id))
        .filter(tests::Column::SessionId.eq(session_id))
        .order_by_asc(tests::Column::Ordinal)
        .order_by_asc(tests::Column::CreatedAt)
        .all(&state.db)
        .await
        .ok()?;

    if all_tests.is_empty() {
        return Some(None);
    }

    // A task with multiple sections (e.g. general-knowledge has 4 trivia
    // questions) creates one tests row per section. Sections are asked one
    // by one in ordinal order: the first un-passed section is probed until
    // it passes, then the next. Sections that already passed are excluded.
    // Tests rows are shared per session, so passes are checked per player:
    // only this player's own probes count towards their completion.
    for at in &all_tests {
        let has_pass = probes_passed(state, at.id, player_id).await;
        if !has_pass {
            return Some(Some(at.clone()));
        }
    }

    // Every section passed for this player — their task is done.
    None
}

/// Scheduler-state marker for the judge-probe phase: every defined probe is
/// done and the judges are looking. Written by the socket loop's hold,
/// overwritten by the normal advance; the player snapshot reads it to report
/// the task completed (the completion logic matches only `completed`).
pub const SCHEDULER_STATE_JUDGING: &str = arena_core::session_completion::SCHEDULER_STATE_JUDGING;

/// Mark this player's scheduler row `judging` (idempotent, cheap).
pub async fn mark_scheduler_judging(state: &GameServerState, session_id: Uuid, player_id: Uuid) {
    if let Ok(Some(row)) = session_scheduler_state::Entity::find()
        .filter(session_scheduler_state::Column::SessionIdFk.eq(session_id))
        .filter(session_scheduler_state::Column::PlayerIdFk.eq(player_id))
        .one(&state.db)
        .await
        && row.state != SCHEDULER_STATE_JUDGING
        && row.state != arena_core::session_completion::SCHEDULER_STATE_COMPLETED
    {
        let mut am: session_scheduler_state::ActiveModel = row.into();
        am.state = Set(SCHEDULER_STATE_JUDGING.to_string());
        am.updated_at = Set(Utc::now());
        let _ = am.update(&state.db).await;
    }
}

/// How long the socket loop may hold a completed open-ended task in its
/// judge-probe phase: judges investigate the pushed snapshot and may
/// register interactive probes (screenshots, captures); the task advances
/// once those settle. The cap exists so a dead LLM or a saturated queue
/// cannot wedge the whole session on one task.
pub const JUDGE_PHASE_MAX_SECS: u64 = 300;

/// The judge phase gave up on this request: it has been in the queue for
/// longer than [`JUDGE_PHASE_MAX_SECS`] without a pass. A probe that can
/// never pass (a judge-authored validation that is simply wrong) must stop
/// holding the task — before this check the queue re-dispatched it forever
/// and the socket loop's phase cap was unreachable.
fn judge_phase_expired(test: &tests::Model) -> bool {
    (Utc::now() - test.created_at).num_seconds() >= JUDGE_PHASE_MAX_SECS as i64
}

/// The oldest judge-registered test without a passing probe for this
/// player — the "current" judge probe in the regular queue, or `None`
/// when every judge request is satisfied (or none exist). Requests past
/// the judge-phase cap no longer count: they are expired, not current.
pub async fn oldest_unpassed_judge_test(
    state: &GameServerState,
    task_id: Uuid,
    session_id: Uuid,
    player_id: Uuid,
) -> Option<tests::Model> {
    let registered = tests::Entity::find()
        .filter(tests::Column::TaskId.eq(task_id))
        .filter(tests::Column::SessionId.eq(session_id))
        .filter(tests::Column::RegisteredByJudgeId.is_not_null())
        .order_by_asc(tests::Column::CreatedAt)
        .all(&state.db)
        .await
        .ok()?;
    for test in registered {
        if judge_phase_expired(&test) {
            continue;
        }
        if !probes_passed(state, test.id, player_id).await {
            return Some(test);
        }
    }
    None
}

/// Any judge lifecycle row exists for this (task, player) — the finalize
/// path already fired the judges once (e.g. before a reconnect).
pub async fn judge_runs_started(state: &GameServerState, task_id: Uuid, player_id: Uuid) -> bool {
    use arena_core::entities::{judge_results, task_judges};
    let attached = task_judges::Entity::find()
        .filter(task_judges::Column::TaskId.eq(task_id))
        .all(&state.db)
        .await
        .unwrap_or_default();
    if attached.is_empty() {
        return false;
    }
    judge_results::Entity::find()
        .filter(judge_results::Column::PlayerIdFk.eq(player_id))
        .filter(judge_results::Column::TaskJudgeId.is_in(attached.iter().map(|tj| tj.id)))
        .one(&state.db)
        .await
        .ok()
        .flatten()
        .is_some()
}

/// Whether the judge-probe phase of an open-ended task settled for this
/// player: every attached task-scoped judge's first pass concluded (scored,
/// failed, or `waiting` on an artifact), and every judge-registered probe
/// resolved (artifact landed or timed out). Evaluation itself may still be
/// running — verdicts land asynchronously after the task advances.
pub async fn judge_probes_settled(
    state: &GameServerState,
    task_id: Uuid,
    session_id: Uuid,
    player_id: Uuid,
) -> bool {
    use arena_core::entities::{judge_results, judges, task_judges};
    use std::collections::HashMap;

    let attached = task_judges::Entity::find()
        .filter(task_judges::Column::TaskId.eq(task_id))
        .all(&state.db)
        .await
        .unwrap_or_default();
    // Session-scoped judges run once at session end; they never conclude
    // during a task and must not hold it open.
    let mut task_scoped = Vec::new();
    for tj in attached {
        match judges::Entity::find_by_id(tj.judge_id).one(&state.db).await {
            Ok(Some(j)) if j.scope == arena_core::judging::JUDGE_SCOPE_SESSION => {}
            _ => task_scoped.push(tj),
        }
    }
    if task_scoped.is_empty() {
        return true;
    }

    let rows = judge_results::Entity::find()
        .filter(judge_results::Column::PlayerIdFk.eq(player_id))
        .filter(judge_results::Column::TaskJudgeId.is_in(task_scoped.iter().map(|tj| tj.id)))
        .all(&state.db)
        .await
        .unwrap_or_default();
    let mut latest: HashMap<Uuid, &judge_results::Model> = HashMap::new();
    for r in &rows {
        latest
            .entry(r.task_judge_id)
            .and_modify(|cur| {
                if r.updated_at > cur.updated_at {
                    *cur = r;
                }
            })
            .or_insert(r);
    }
    for tj in &task_scoped {
        match latest.get(&tj.id).map(|r| r.status.as_str()) {
            Some("scored") | Some("failed") | Some("waiting") => {}
            // No row yet, or pending/running: the judge has not had its
            // first pass — it may still register a probe.
            _ => return false,
        }
    }

    // Every probe a judge registered must have delivered (a passing run).
    // Failed attempts are not terminal — the regular queue retries them —
    // so an undelivered request holds the task until the phase cap, after
    // which it counts as settled (the advance path then expires it to
    // `no_response` for the judges' partial verdicts).
    let registered = tests::Entity::find()
        .filter(tests::Column::TaskId.eq(task_id))
        .filter(tests::Column::SessionId.eq(session_id))
        .filter(tests::Column::RegisteredByJudgeId.is_not_null())
        .all(&state.db)
        .await
        .unwrap_or_default();
    for test in &registered {
        if judge_phase_expired(test) {
            continue;
        }
        if !probes_passed(state, test.id, player_id).await {
            return false;
        }
    }
    true
}

/// The judge phase gave up on this task: every judge-registered test still
/// without a pass gets a terminal `no_response` probe row, so the waiting
/// judges re-drive into partial verdicts instead of waiting forever.
pub async fn expire_unpassed_judge_probes(
    state: &GameServerState,
    task_id: Uuid,
    session_id: Uuid,
    player_id: Uuid,
) {
    use arena_core::entities::probes;
    let registered = tests::Entity::find()
        .filter(tests::Column::TaskId.eq(task_id))
        .filter(tests::Column::SessionId.eq(session_id))
        .filter(tests::Column::RegisteredByJudgeId.is_not_null())
        .all(&state.db)
        .await
        .unwrap_or_default();
    let now = Utc::now();
    for test in registered {
        if probes_passed(state, test.id, player_id).await {
            continue;
        }
        let _ = probes::Entity::update_many()
            .col_expr(
                probes::Column::Outcome,
                sea_orm::prelude::Expr::value("no_response"),
            )
            .col_expr(
                probes::Column::ResolvedAt,
                sea_orm::prelude::Expr::value(now),
            )
            .col_expr(
                probes::Column::UpdatedAt,
                sea_orm::prelude::Expr::value(now),
            )
            .filter(probes::Column::TestId.eq(test.id))
            .filter(probes::Column::PlayerId.eq(player_id))
            .filter(probes::Column::Outcome.is_null())
            .exec(&state.db)
            .await;
    }
}

/// The open-ended view of a task's progress for one player.
pub struct OpenEndedState {
    pub contract: arena_core::evaluation::EvaluationContract,
    /// The tests row of the completion probe, when it materialized.
    pub completion_test: Option<tests::Model>,
    /// The completion probe has a passing run for this player.
    pub completion_passed: bool,
    /// The work window (`completion.deadline_secs` since `task_started`)
    /// has expired — the task is force-evaluated on whatever exists.
    pub deadline_expired: bool,
}

/// Assemble the open-ended state: contract, completion-probe row (matched by
/// the section title's position in the task DSL), pass status, and deadline.
/// `None` when the evaluation column does not parse (hand-edited row — the
/// caller falls back to classic behavior).
pub async fn open_ended_state(
    state: &GameServerState,
    task_row: &tasks::Model,
    session_id: Uuid,
    player_id: Uuid,
) -> Option<OpenEndedState> {
    let contract = arena_core::evaluation::EvaluationContract::from_json(
        task_row.evaluation.as_ref()?,
    )
    .map_err(|e| {
        tracing::warn!(task_id = %task_row.id, error = %e, "unparseable evaluation contract");
        e
    })
    .ok()?;

    // The completion probe is named by section title; tests rows are keyed by
    // the section's position in the parsed DSL (the same enumeration that
    // materialized them).
    let completion_ordinal: Option<i32> = serde_json::from_value::<
        arena_core::task_template::TestTemplate,
    >(task_row.test_template.clone())
    .ok()
    .and_then(|tpl| {
        arena_core::task_template::parse_structured_markdown_tests(&tpl.command_template)
            .iter()
            .position(|s| s.title.trim() == contract.completion.probe.trim())
            .map(|i| i as i32)
    });

    let completion_test = match completion_ordinal {
        Some(ordinal) => tests::Entity::find()
            .filter(tests::Column::TaskId.eq(task_row.id))
            .filter(tests::Column::SessionId.eq(session_id))
            .filter(tests::Column::Ordinal.eq(ordinal))
            .one(&state.db)
            .await
            .ok()
            .flatten(),
        None => None,
    };

    let completion_passed = match &completion_test {
        Some(t) => probes_passed(state, t.id, player_id).await,
        None => false,
    };

    let deadline_expired = match task_started_at(state, session_id, player_id, task_row.id).await {
        Some(started) => {
            Utc::now() >= started + chrono::Duration::seconds(contract.completion.deadline_secs)
        }
        // No recorded start: the window has not opened, so it cannot have
        // closed.
        None => false,
    };

    Some(OpenEndedState {
        contract,
        completion_test,
        completion_passed,
        deadline_expired,
    })
}

/// The most-overdue *due* scheduled participant section of an open-ended
/// task, excluding the completion probe (the caller's fallback). "Due" means
/// the section declares `schedule.on: [interval]` and its interval elapsed
/// since this player's last dispatch — or it never ran and declares `start`
/// or `interval`. Sections with a server executor ride the probe ticker, not
/// the agent loop.
pub async fn most_overdue_participant_section(
    state: &GameServerState,
    task_id: Uuid,
    session_id: Uuid,
    player_id: Uuid,
    completion_test_id: Uuid,
) -> Option<tests::Model> {
    use arena_core::evaluation::{ProbeConfig, ProbeExecutor, ScheduleOn};

    let rows = tests::Entity::find()
        .filter(tests::Column::TaskId.eq(task_id))
        .filter(tests::Column::SessionId.eq(session_id))
        .order_by_asc(tests::Column::Ordinal)
        .all(&state.db)
        .await
        .ok()?;

    let now = Utc::now();
    let mut best: Option<(f64, tests::Model)> = None;
    for row in rows {
        // The completion probe competes here too, on its own schedule.
        // Excluding it starved it: with the loop cadence backed off to the
        // report probe's interval, the report was due on every wake and the
        // completion probe never ran again — a finished TODO list could not
        // be noticed until the window deadline (session RQS57S).
        let is_completion = row.id == completion_test_id;
        let Some(config_json) = &row.probe_config else {
            continue;
        };
        let Ok(config) = ProbeConfig::from_json(config_json) else {
            continue;
        };
        if config.effective_executor() != ProbeExecutor::Participant {
            continue;
        }
        let Some(schedule) = &config.schedule else {
            continue;
        };
        let wants_interval = schedule.on.contains(&ScheduleOn::Interval);
        let wants_start = schedule.on.contains(&ScheduleOn::Start);
        if !wants_interval && !wants_start {
            continue;
        }
        let last = last_dispatch_at(state, row.id, player_id).await;
        let overdue_ratio = match last {
            // Never ran: a start or interval probe fires immediately. The
            // completion probe's first run is the caller's fallback.
            None => {
                if is_completion {
                    continue;
                }
                f64::MAX
            }
            Some(last) => {
                if !wants_interval {
                    continue; // start-only, already fired
                }
                let interval = schedule.interval_secs.unwrap_or(0).max(1) as f64;
                (now - last).num_seconds() as f64 / interval
            }
        };
        if overdue_ratio >= 1.0
            && best
                .as_ref()
                .map(|(r, _)| overdue_ratio > *r)
                .unwrap_or(true)
        {
            best = Some((overdue_ratio, row));
        }
    }
    best.map(|(_, row)| row)
}

/// This player's most recent dispatch of `test_id`, if any.
async fn last_dispatch_at(
    state: &GameServerState,
    test_id: Uuid,
    player_id: Uuid,
) -> Option<chrono::DateTime<Utc>> {
    use arena_core::entities::probes;
    probes::Entity::find()
        .filter(probes::Column::TestId.eq(test_id))
        .filter(probes::Column::PlayerId.eq(player_id))
        .order_by_desc(probes::Column::DispatchedAt)
        .one(&state.db)
        .await
        .ok()
        .flatten()
        .map(|p| p.dispatched_at)
}

/// When this player's task started, from the `task_started` activity event
/// the scheduler emits on bootstrap/advance.
async fn task_started_at(
    state: &GameServerState,
    session_id: Uuid,
    player_id: Uuid,
    task_id: Uuid,
) -> Option<chrono::DateTime<Utc>> {
    activity_event::Entity::find()
        .filter(activity_event::Column::SessionIdFk.eq(session_id))
        .filter(activity_event::Column::PlayerIdFk.eq(player_id))
        .filter(activity_event::Column::TaskIdFk.eq(task_id))
        .filter(activity_event::Column::EventKind.eq("task_started"))
        .order_by_asc(activity_event::Column::Timestamp)
        .one(&state.db)
        .await
        .ok()
        .flatten()
        .map(|e| e.timestamp)
}

/// 1-based position of `test_id` within the task's ordinal-ordered test
/// list, plus the total test count. `(0, 0)` when the test is not found
/// (defensive — the caller just picked it from this list).
pub async fn test_position(
    state: &GameServerState,
    task_id: Uuid,
    session_id: Uuid,
    test_id: Uuid,
) -> (i32, i32) {
    let ids: Vec<Uuid> = tests::Entity::find()
        .filter(tests::Column::TaskId.eq(task_id))
        .filter(tests::Column::SessionId.eq(session_id))
        .order_by_asc(tests::Column::Ordinal)
        .order_by_asc(tests::Column::CreatedAt)
        .all(&state.db)
        .await
        .map(|rows| rows.into_iter().map(|t| t.id).collect())
        .unwrap_or_default();
    match ids.iter().position(|id| *id == test_id) {
        Some(i) => ((i + 1) as i32, ids.len() as i32),
        None => (0, 0),
    }
}

async fn probes_passed(state: &GameServerState, test_id: Uuid, player_id: Uuid) -> bool {
    use arena_core::entities::probes;
    probes::Entity::find()
        .filter(probes::Column::TestId.eq(test_id))
        .filter(probes::Column::PlayerId.eq(player_id))
        .filter(probes::Column::Outcome.eq("pass"))
        .one(&state.db)
        .await
        .ok()
        .flatten()
        .is_some()
}

/// Resolve the currently scheduled task id for the session, bootstrapping the
/// scheduler state (and starting the session) on first dispatch. Returns `None`
/// when there are no tasks — the caller should then finish the session and break.
pub async fn resolve_current_task_id(
    state: &GameServerState,
    session_id: Uuid,
    player_id: Uuid,
    join_code: &str,
    socket: &mut WebSocket,
) -> Option<Uuid> {
    let row = match session_scheduler_state::Entity::find()
        .filter(session_scheduler_state::Column::SessionIdFk.eq(session_id))
        .filter(session_scheduler_state::Column::PlayerIdFk.eq(player_id))
        .one(&state.db)
        .await
    {
        Ok(r) => r,
        Err(e) => {
            tracing::error!(session_id = %session_id, player_id = %player_id, error = %e, "player_agent: DB error loading scheduler state");
            return None;
        }
    };
    // A completed player reconnecting must NOT be re-bootstrapped onto the
    // first task — acknowledge their per-player completion and let the
    // caller close the socket.
    if row
        .as_ref()
        .is_some_and(|r| r.state == arena_core::session_completion::SCHEDULER_STATE_COMPLETED)
    {
        tracing::info!(session_id = %session_id, player_id = %player_id, "player_agent: reconnect after completion, not re-bootstrapping");
        let frame = arena_core::protocol::PlayerAgentFrame::SessionComplete {
            session_id,
            reason: Some(
                arena_core::protocol::SESSION_COMPLETE_REASON_PLAYER_TASKS_COMPLETED.to_string(),
            ),
        };
        let json = serde_json::to_string(&frame).unwrap_or_default();
        let _ = socket.send(axum::extract::ws::Message::Text(json)).await;
        return None;
    }
    let existing = row.and_then(|r| r.task_id);
    if existing.is_none() {
        match bootstrap_scheduler_state(state, session_id, player_id, join_code).await {
            Some(id) => {
                ensure_session_running(state, session_id).await;
                Some(id)
            }
            None => {
                tracing::info!(session_id = %session_id, "player_agent: no tasks for session, finishing");
                finish_no_tasks(state, session_id, player_id, join_code, socket).await;
                None
            }
        }
    } else {
        existing
    }
}

pub async fn advance_to_next_task(
    state: &GameServerState,
    session_id: Uuid,
    player_id: Uuid,
    join_code: &str,
    current_task_id: Uuid,
) -> bool {
    let current_task = match tasks::Entity::find_by_id(current_task_id)
        .one(&state.db)
        .await
    {
        Ok(Some(t)) => t,
        _ => return false,
    };

    let next_task = tasks::Entity::find()
        .filter(tasks::Column::ProjectIdFk.eq(current_task.project_id_fk))
        .filter(tasks::Column::Ordinal.gt(current_task.ordinal))
        .order_by_asc(tasks::Column::Ordinal)
        .one(&state.db)
        .await;

    let new_task_id = match &next_task {
        Ok(Some(t)) => Some(t.id),
        _ => None,
    };

    if let Some(sched) = session_scheduler_state::Entity::find()
        .filter(session_scheduler_state::Column::SessionIdFk.eq(session_id))
        .filter(session_scheduler_state::Column::PlayerIdFk.eq(player_id))
        .one(&state.db)
        .await
        .ok()
        .flatten()
    {
        let mut am: session_scheduler_state::ActiveModel = sched.into();
        am.task_id = Set(new_task_id);
        am.updated_at = Set(Utc::now());
        am.state = Set(new_task_id.map_or_else(
            || arena_core::session_completion::SCHEDULER_STATE_COMPLETED.to_string(),
            |_| "idle".to_string(),
        ));
        let _ = am.update(&state.db).await;
        if let Some(t) = next_task.ok().flatten() {
            emit_task_started(state, session_id, player_id, join_code, &t).await;
            true
        } else {
            false
        }
    } else {
        false
    }
}

pub async fn ensure_adapted_test(
    state: &GameServerState,
    task_id: Uuid,
    session_id: Uuid,
) -> Result<Option<tests::Model>, sea_orm::DbErr> {
    let existing = tests::Entity::find()
        .filter(tests::Column::TaskId.eq(task_id))
        .filter(tests::Column::SessionId.eq(session_id))
        .order_by_asc(tests::Column::CreatedAt)
        .one(&state.db)
        .await?;

    if let Some(t) = existing {
        return Ok(Some(t));
    }

    let task_row = match tasks::Entity::find_by_id(task_id).one(&state.db).await? {
        Some(r) => r,
        None => return Ok(None),
    };

    let template: arena_core::task_template::TestTemplate =
        match serde_json::from_value(task_row.test_template.clone()) {
            Ok(t) => t,
            Err(_) => return Ok(None),
        };

    // Parse structured markdown tests from the command_template field.
    // Every `##` section becomes its own tests row (probe type) — mirrors
    // the server's passthrough adaptation. The scheduler then asks them
    // one by one in ordinal order.
    let structured =
        arena_core::task_template::parse_structured_markdown_tests(&template.command_template);
    if !structured.is_empty() {
        let mut first_model: Option<tests::Model> = None;
        for (ordinal, section) in structured.iter().enumerate() {
            // Seed validation already rejected malformed fences; a parse
            // failure here is a hand-edited row, and the safe reading is
            // "legacy probe" — the config narrows behavior, never gates it.
            let probe_config = match section.parsed_probe_config() {
                Ok(cfg) => cfg,
                Err(e) => {
                    tracing::warn!(%task_id, ordinal, error = %e,
                        "yaml probe fence unparseable; treating section as legacy probe");
                    None
                }
            };
            let new_test = tests::ActiveModel {
                id: Set(Uuid::new_v4()),
                command_template: Set(section.command_template.clone()),
                answer_template: Set(section.answer_template.clone()),
                fixture_definitions: Set(section.fixture_definitions.clone()),
                created_at: Set(Utc::now()),
                session_id: Set(session_id),
                task_id: Set(task_id),
                ordinal: Set(ordinal as i32),
                // The section's `## ` heading and prose — mirrors the server's
                // passthrough adaptation, which this path used to diverge from
                // (it wrote the placeholder unconditionally, so probes created
                // here rendered with no label at all).
                prompt: Set(if section.title.trim().is_empty() {
                    format!("Structured markdown test {ordinal}")
                } else {
                    section.title.trim().to_string()
                }),
                description: Set(Some(section.description.clone()).filter(|s| !s.is_empty())),
                probe_config: Set(probe_config
                    .as_ref()
                    .and_then(|c| serde_json::to_value(c).ok())),
                initiator: Set(arena_core::evaluation::INITIATOR_SYSTEM.to_string()),
                registered_by_judge_id: Set(None),
            };
            let model = new_test.insert(&state.db).await?;
            if first_model.is_none() {
                first_model = Some(model);
            }
        }
        return Ok(first_model);
    }

    let fixture_defs_json =
        serde_json::to_string(&template.fixtures).unwrap_or_else(|_| "[]".to_string());
    let answer_template = template.answer_template.unwrap_or_default();

    let new_test = tests::ActiveModel {
        id: Set(Uuid::new_v4()),
        command_template: Set(template.command_template.clone()),
        answer_template: Set(answer_template),
        fixture_definitions: Set(fixture_defs_json),
        created_at: Set(Utc::now()),
        session_id: Set(session_id),
        task_id: Set(task_id),
        ordinal: Set(0),
        prompt: Set(String::new()),
        description: Set(None),
        probe_config: Set(None),
        initiator: Set(arena_core::evaluation::INITIATOR_SYSTEM.to_string()),
        registered_by_judge_id: Set(None),
    };

    let model = match new_test.insert(&state.db).await {
        Ok(m) => m,
        Err(e) => {
            let is_unique = e.to_string().to_lowercase().contains("unique");
            if is_unique {
                return tests::Entity::find()
                    .filter(tests::Column::TaskId.eq(task_id))
                    .filter(tests::Column::SessionId.eq(session_id))
                    .order_by_asc(tests::Column::CreatedAt)
                    .one(&state.db)
                    .await;
            }
            return Err(e);
        }
    };
    Ok(Some(model))
}

pub async fn bootstrap_scheduler_state(
    state: &GameServerState,
    session_id: Uuid,
    player_id: Uuid,
    join_code: &str,
) -> Option<Uuid> {
    let session = sessions::Entity::find_by_id(session_id)
        .one(&state.db)
        .await
        .ok()
        .flatten()?;

    let first_task = tasks::Entity::find()
        .filter(tasks::Column::ProjectIdFk.eq(session.project_id_fk))
        .order_by_asc(tasks::Column::Ordinal)
        .one(&state.db)
        .await
        .ok()
        .flatten()?;

    let existing = session_scheduler_state::Entity::find()
        .filter(session_scheduler_state::Column::SessionIdFk.eq(session_id))
        .filter(session_scheduler_state::Column::PlayerIdFk.eq(player_id))
        .one(&state.db)
        .await
        .ok()
        .flatten();

    if let Some(sched) = existing {
        let now = Utc::now();
        let mut am: session_scheduler_state::ActiveModel = sched.into();
        am.task_id = Set(Some(first_task.id));
        am.state = Set("idle".to_string());
        am.next_probe_at = Set(None);
        am.updated_at = Set(now);
        if let Err(e) = am.update(&state.db).await {
            tracing::error!(session_id = %session_id, error = %e, "player_agent: failed to update scheduler state");
            return None;
        }
    } else {
        let now = Utc::now();
        let am = session_scheduler_state::ActiveModel {
            id: Set(Uuid::new_v4()),
            session_id_fk: Set(session_id),
            player_id_fk: Set(player_id),
            task_id: Set(Some(first_task.id)),
            state: Set("idle".to_string()),
            next_probe_at: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
        };
        if let Err(e) = am.insert(&state.db).await {
            tracing::error!(session_id = %session_id, error = %e, "player_agent: failed to insert scheduler state");
            return None;
        }
    }

    tracing::info!(session_id = %session_id, task_id = %first_task.id, join_code = %join_code, "player_agent: bootstrapped scheduler state with first task");
    emit_task_started(state, session_id, player_id, join_code, &first_task).await;
    Some(first_task.id)
}

pub async fn ensure_session_running(state: &GameServerState, session_id: Uuid) {
    if let Ok(Some(model)) = sessions::Entity::find_by_id(session_id)
        .one(&state.db)
        .await
        && model.status != SessionStatus::Running
    {
        let now = Utc::now();
        let mut am: sessions::ActiveModel = model.into();
        am.status = Set(SessionStatus::Running);
        am.started_at = Set(Some(now));
        if let Err(e) = am.update(&state.db).await {
            tracing::error!(session_id = %session_id, error = %e, "player_agent: failed to set session to running");
        } else {
            tracing::info!(session_id = %session_id, "player_agent: session status set to running");
        }
    }
}

pub async fn update_next_probe_at(
    state: &GameServerState,
    session_id: Uuid,
    player_id: Uuid,
    next_probe_at: Option<chrono::DateTime<chrono::Utc>>,
) {
    if let Some(sched) = session_scheduler_state::Entity::find()
        .filter(session_scheduler_state::Column::SessionIdFk.eq(session_id))
        .filter(session_scheduler_state::Column::PlayerIdFk.eq(player_id))
        .one(&state.db)
        .await
        .ok()
        .flatten()
    {
        let now = Utc::now();
        let mut am: session_scheduler_state::ActiveModel = sched.into();
        am.next_probe_at = Set(next_probe_at);
        am.updated_at = Set(now);
        if let Err(e) = am.update(&state.db).await {
            tracing::warn!(session_id = %session_id, player_id = %player_id, error = %e, "player_agent: failed to update next_probe_at");
        }
    }
}

pub async fn finish_no_tasks(
    state: &GameServerState,
    session_id: Uuid,
    player_id: Uuid,
    join_code: &str,
    socket: &mut axum::extract::ws::WebSocket,
) {
    // With zero tasks every eligible player is trivially done, so the
    // all-done check finishes the session immediately on first connect
    // (preserving the historical behavior). The guard only bites in
    // degenerate states (e.g. session row vanished), where finishing would
    // be wrong anyway.
    let finished = crate::state::finish_session_if_all_done(state, session_id, join_code).await;
    update_next_probe_at(state, session_id, player_id, None).await;
    let reason = if finished {
        "all_tasks_completed".to_string()
    } else {
        arena_core::protocol::SESSION_COMPLETE_REASON_PLAYER_TASKS_COMPLETED.to_string()
    };
    let frame = arena_core::protocol::PlayerAgentFrame::SessionComplete {
        session_id,
        reason: Some(reason),
    };
    let json = serde_json::to_string(&frame).unwrap_or_default();
    let _ = socket.send(axum::extract::ws::Message::Text(json)).await;
}

/// Persist a `task_started` activity_event row and publish `ZmqEvent::TaskStarted`
/// so the arena server can bridge it into `ArenaFrame::TaskStarted` for dashboard
/// observers. Called on every task transition (bootstrap first task + advance).
pub(crate) async fn emit_task_started(
    state: &GameServerState,
    session_id: Uuid,
    player_id: Uuid,
    join_code: &str,
    task: &tasks::Model,
) {
    let display_name = players::Entity::find_by_id(player_id)
        .one(&state.db)
        .await
        .ok()
        .flatten()
        .map(|p| p.display_name)
        .unwrap_or_default();
    let version = state
        .session_registry
        .get(join_code)
        .and_then(|e| e.cache.read().ok().map(|c| c.version))
        .unwrap_or(0);
    let now = Utc::now();

    let activity_row = activity_event::ActiveModel {
        id: Set(Uuid::new_v4()),
        session_id_fk: Set(session_id),
        player_id_fk: Set(player_id),
        task_id_fk: Set(task.id),
        event_kind: Set("task_started".to_string()),
        task_ordinal: Set(task.ordinal),
        task_title: Set(task.title.clone()),
        player_display_name: Set(display_name.clone()),
        judge_name: Set(None),
        point_delta: Set(None),
        timestamp: Set(now),
        version: Set(version as i64),
        detail: Set(None),
    };
    if let Err(e) = activity_event::Entity::insert(activity_row)
        .exec(&state.db)
        .await
    {
        tracing::warn!(error = %e, "emit_task_started: failed to persist activity_event, skipping publish");
        return;
    }

    let event = ZmqEvent::TaskStarted {
        join_code: join_code.to_string(),
        player_id,
        player_display_name: display_name.clone(),
        task_id: task.id,
        task_ordinal: task.ordinal,
        task_title: task.title.clone(),
        timestamp: now,
        version,
    };
    state.event_publisher.publish(&event).await;

    // Also broadcast directly to game-server observer clients.
    let frame = ArenaFrame::TaskStarted {
        player_id,
        player_display_name: display_name,
        task_id: task.id,
        task_ordinal: task.ordinal,
        task_title: task.title.clone(),
        timestamp: now,
        version,
    };
    if let Some(entry) = state.session_registry.get(join_code) {
        let _ = entry.tx.send(frame);
    }
}

#[cfg(test)]
mod judge_phase_tests {
    //! The gate that decides when a delivered task may advance: the exact
    //! logic that held session DIWHYR for 3.5 minutes while every verdict
    //! was already in and a judge-registered check kept failing.
    use super::*;
    use crate::test_fixtures::{
        attach_judge, extra_task, insert_judge_result, insert_judge_test, insert_probe,
        insert_scheduler_state, mem_db, session_with_player, task_with_test, test_state,
    };
    use arena_core::entities::probes;

    #[tokio::test]
    async fn a_task_without_judges_is_settled_immediately() {
        let db = mem_db().await;
        let fx = session_with_player(&db).await;
        let (task_id, _t) = task_with_test(&db, &fx).await;
        let state = test_state(db.clone());
        assert!(judge_probes_settled(&state, task_id, fx.session_id, fx.player_id).await);
    }

    #[tokio::test]
    async fn a_judge_that_has_not_run_holds_the_task() {
        let db = mem_db().await;
        let fx = session_with_player(&db).await;
        let (task_id, _t) = task_with_test(&db, &fx).await;
        let (_judge, tj) = attach_judge(&db, task_id, "correctness", "task").await;
        let state = test_state(db.clone());
        // No row at all: the judge has not had its first pass.
        assert!(!judge_probes_settled(&state, task_id, fx.session_id, fx.player_id).await);
        // Still running: same story.
        insert_judge_result(&db, &fx, tj, "running").await;
        assert!(!judge_probes_settled(&state, task_id, fx.session_id, fx.player_id).await);
    }

    #[tokio::test]
    async fn session_scoped_judges_never_hold_a_task() {
        // They run once at session end; waiting for them here would wedge
        // every task.
        let db = mem_db().await;
        let fx = session_with_player(&db).await;
        let (task_id, _t) = task_with_test(&db, &fx).await;
        attach_judge(&db, task_id, "anti-cheat", "session").await;
        let state = test_state(db.clone());
        assert!(judge_probes_settled(&state, task_id, fx.session_id, fx.player_id).await);
    }

    #[tokio::test]
    async fn an_unpassed_judge_check_holds_the_task_until_the_phase_cap() {
        let db = mem_db().await;
        let fx = session_with_player(&db).await;
        let (task_id, _t) = task_with_test(&db, &fx).await;
        let (judge_id, tj) = attach_judge(&db, task_id, "test-quality", "task").await;
        insert_judge_result(&db, &fx, tj, "scored").await;
        let state = test_state(db.clone());

        // The judge scored, but its extra check has not passed — DIWHYR.
        let fresh = insert_judge_test(&db, &fx, task_id, judge_id, 0).await;
        insert_probe(&db, &fx, fresh, Some("error")).await;
        assert!(
            !judge_probes_settled(&state, task_id, fx.session_id, fx.player_id).await,
            "a failing judge check must hold the advance"
        );
        assert_eq!(
            oldest_unpassed_judge_test(&state, task_id, fx.session_id, fx.player_id)
                .await
                .map(|t| t.id),
            Some(fresh),
            "the queue re-dispatches exactly that check"
        );

        // Past the cap the request is expired, not current: the task moves on.
        let aged =
            insert_judge_test(&db, &fx, task_id, judge_id, JUDGE_PHASE_MAX_SECS as i64 + 1).await;
        insert_probe(&db, &fx, aged, Some("error")).await;
        assert!(judge_phase_expired(
            &tests::Entity::find_by_id(aged)
                .one(&db)
                .await
                .expect("query")
                .expect("test")
        ));
    }

    #[tokio::test]
    async fn a_passing_judge_check_releases_the_task() {
        let db = mem_db().await;
        let fx = session_with_player(&db).await;
        let (task_id, _t) = task_with_test(&db, &fx).await;
        let (judge_id, tj) = attach_judge(&db, task_id, "test-quality", "task").await;
        insert_judge_result(&db, &fx, tj, "scored").await;
        let check = insert_judge_test(&db, &fx, task_id, judge_id, 0).await;
        insert_probe(&db, &fx, check, Some("pass")).await;
        let state = test_state(db.clone());
        assert!(judge_probes_settled(&state, task_id, fx.session_id, fx.player_id).await);
        assert!(
            oldest_unpassed_judge_test(&state, task_id, fx.session_id, fx.player_id)
                .await
                .is_none()
        );
    }

    #[tokio::test]
    async fn expiring_marks_only_the_undelivered_checks() {
        let db = mem_db().await;
        let fx = session_with_player(&db).await;
        let (task_id, _t) = task_with_test(&db, &fx).await;
        let (judge_id, _tj) = attach_judge(&db, task_id, "ux-review", "task").await;
        let unpassed = insert_judge_test(&db, &fx, task_id, judge_id, 0).await;
        let open_probe = insert_probe(&db, &fx, unpassed, None).await;
        let passed = insert_judge_test(&db, &fx, task_id, judge_id, 0).await;
        let passed_probe = insert_probe(&db, &fx, passed, Some("pass")).await;
        let state = test_state(db.clone());

        expire_unpassed_judge_probes(&state, task_id, fx.session_id, fx.player_id).await;

        let open = probes::Entity::find_by_id(open_probe)
            .one(&db)
            .await
            .expect("query")
            .expect("probe");
        assert_eq!(
            open.outcome.as_deref(),
            Some("no_response"),
            "an undelivered request becomes terminal so the judges re-drive"
        );
        let kept = probes::Entity::find_by_id(passed_probe)
            .one(&db)
            .await
            .expect("query")
            .expect("probe");
        assert_eq!(
            kept.outcome.as_deref(),
            Some("pass"),
            "a delivered check keeps its verdict"
        );
    }

    #[tokio::test]
    async fn judging_mark_does_not_clobber_a_completed_player() {
        let db = mem_db().await;
        let fx = session_with_player(&db).await;
        let (task_id, _t) = task_with_test(&db, &fx).await;
        let state = test_state(db.clone());

        insert_scheduler_state(&db, &fx, Some(task_id), "idle").await;
        mark_scheduler_judging(&state, fx.session_id, fx.player_id).await;
        let row = session_scheduler_state::Entity::find()
            .filter(session_scheduler_state::Column::PlayerIdFk.eq(fx.player_id))
            .one(&db)
            .await
            .expect("query")
            .expect("row");
        assert_eq!(row.state, SCHEDULER_STATE_JUDGING);

        // A player who finished every task must not be dragged back.
        let mut am: session_scheduler_state::ActiveModel = row.into();
        am.state = Set(arena_core::session_completion::SCHEDULER_STATE_COMPLETED.to_string());
        am.update(&db).await.expect("update");
        mark_scheduler_judging(&state, fx.session_id, fx.player_id).await;
        let row = session_scheduler_state::Entity::find()
            .filter(session_scheduler_state::Column::PlayerIdFk.eq(fx.player_id))
            .one(&db)
            .await
            .expect("query")
            .expect("row");
        assert_eq!(
            row.state,
            arena_core::session_completion::SCHEDULER_STATE_COMPLETED
        );
    }

    #[tokio::test]
    async fn advance_moves_to_the_next_ordinal_then_completes() {
        let db = mem_db().await;
        let fx = session_with_player(&db).await;
        let (task0, _t) = task_with_test(&db, &fx).await;
        let task1 = extra_task(&db, &fx, 1).await;
        insert_scheduler_state(&db, &fx, Some(task0), "idle").await;
        let state = test_state(db.clone());

        assert!(
            advance_to_next_task(&state, fx.session_id, fx.player_id, "JOIN01", task0).await,
            "task 0 advances to task 1"
        );
        let row = session_scheduler_state::Entity::find()
            .filter(session_scheduler_state::Column::PlayerIdFk.eq(fx.player_id))
            .one(&db)
            .await
            .expect("query")
            .expect("row");
        assert_eq!(row.task_id, Some(task1));
        assert_eq!(row.state, "idle");

        assert!(
            !advance_to_next_task(&state, fx.session_id, fx.player_id, "JOIN01", task1).await,
            "the last task has nowhere to advance"
        );
        let row = session_scheduler_state::Entity::find()
            .filter(session_scheduler_state::Column::PlayerIdFk.eq(fx.player_id))
            .one(&db)
            .await
            .expect("query")
            .expect("row");
        assert_eq!(row.task_id, None);
        assert_eq!(
            row.state,
            arena_core::session_completion::SCHEDULER_STATE_COMPLETED
        );
    }
}
