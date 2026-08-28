//! Per-player task-completion facts for a session.
//!
//! Answers two questions from existing rows only (no dedicated columns):
//! "has this player completed all of their tasks?" and "have ALL eligible
//! players completed all of their tasks?". Eligible means `players` rows for
//! the session with `revoked_at` IS NULL — a revoked player never blocks
//! completion.
//!
//! A player counts as done when their player-scoped `session_scheduler_state`
//! row has reached [`SCHEDULER_STATE_COMPLETED`] (the scheduler writes that
//! state when `advance_to_next_task` finds no next task). When the session's
//! project has zero tasks every player is trivially done. A joined player
//! without a scheduler row has not started yet and is NOT done.
//!
//! The game server uses [`all_eligible_players_done`] at the finish
//! transition (the LAST finisher ends the session); the per-player facts from
//! [`session_player_completion`] are the reusable base that
//! [`session_player_statuses`] layers judge-aware status reporting on top of.

use crate::entities::{
    judge_results, players, session_scheduler_state, sessions, task_judges, tasks,
};
use crate::protocol::player::PlayerCompletionStatus;
use sea_orm::{
    ColumnTrait, DatabaseConnection, DbErr, EntityTrait, PaginatorTrait, QueryFilter, QuerySelect,
};
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

/// `session_scheduler_state.state` value meaning the player has exhausted
/// their task list. Written by the game server's scheduler; read here.
pub const SCHEDULER_STATE_COMPLETED: &str = "completed";

/// `session_scheduler_state.state` value for an open-ended task whose
/// completion contract has passed and whose judge phase is still settling.
/// Written by the game server's scheduler; the player snapshot reads it to
/// report the task as completed while the judges deliberate.
pub const SCHEDULER_STATE_JUDGING: &str = "judging";

/// `judge_results.status` value for a run that produced a verdict.
pub const JUDGE_RESULT_SCORED: &str = "scored";

/// `judge_results.status` value for a run that gave up without a verdict.
/// Terminal like "scored": a failed judge must never block completion.
pub const JUDGE_RESULT_FAILED: &str = "failed";

/// Completion fact for one eligible (non-revoked) player of a session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlayerCompletion {
    pub player_id: Uuid,
    /// True when the player has completed all tasks (trivially true when the
    /// project has zero tasks).
    pub done: bool,
}

/// Fold per-player facts into the session-level answer. Empty input is
/// `false`: a session with no eligible players is never "all done" — it only
/// ends via time expiry.
pub fn all_done(completions: &[PlayerCompletion]) -> bool {
    !completions.is_empty() && completions.iter().all(|c| c.done)
}

/// Per-player completion facts for every eligible (non-revoked) player of
/// `session_id`. Errors with `RecordNotFound` when the session does not
/// exist.
pub async fn session_player_completion(
    db: &DatabaseConnection,
    session_id: Uuid,
) -> Result<Vec<PlayerCompletion>, DbErr> {
    let session = load_session(db, session_id).await?;
    completion_for_session(db, &session).await
}

async fn load_session(db: &DatabaseConnection, session_id: Uuid) -> Result<sessions::Model, DbErr> {
    sessions::Entity::find_by_id(session_id)
        .one(db)
        .await?
        .ok_or_else(|| DbErr::RecordNotFound(format!("session {session_id}")))
}

/// The project's tasks as THIS session knew them: tasks seeded into the
/// project after the session finished never ran in it, so counting them (or
/// their judges) retroactively wedges a settled session on runs nobody will
/// ever start — session WQ2WOQ froze on "Judges scoring — 7 left" exactly
/// this way after a later `apply-seed` added a third task. A live session
/// has no `finished_at` and sees the full current list.
pub fn session_task_filter(
    query: sea_orm::Select<tasks::Entity>,
    session: &sessions::Model,
) -> sea_orm::Select<tasks::Entity> {
    let query = query.filter(tasks::Column::ProjectIdFk.eq(session.project_id_fk));
    match session.finished_at {
        Some(finished) => query.filter(tasks::Column::CreatedAt.lte(finished)),
        None => query,
    }
}

/// The judge attachments as THIS session knew them, mirroring
/// [`session_task_filter`]: a judge attached to a task after the session
/// finished never ran in it (the panel rollouts of 2026-08 attached seven
/// judges to weather-widget tasks retroactively, freezing every earlier
/// session on "Judges scoring"). A live session sees the full current set.
pub fn session_task_judge_filter(
    query: sea_orm::Select<task_judges::Entity>,
    session: &sessions::Model,
) -> sea_orm::Select<task_judges::Entity> {
    match session.finished_at {
        Some(finished) => query.filter(task_judges::Column::CreatedAt.lte(finished)),
        None => query,
    }
}

/// Query body of [`session_player_completion`], reused by
/// [`session_player_statuses`] so the session row is loaded exactly once.
async fn completion_for_session(
    db: &DatabaseConnection,
    session: &sessions::Model,
) -> Result<Vec<PlayerCompletion>, DbErr> {
    let session_id = session.id;
    let eligible = players::Entity::find()
        .filter(players::Column::SessionIdFk.eq(session_id))
        .filter(players::Column::RevokedAt.is_null())
        .all(db)
        .await?;

    let task_count = session_task_filter(tasks::Entity::find(), session)
        .count(db)
        .await?;
    if task_count == 0 {
        // No tasks — every player is trivially done.
        return Ok(eligible
            .iter()
            .map(|p| PlayerCompletion {
                player_id: p.id,
                done: true,
            })
            .collect());
    }

    let scheduler_states: HashMap<Uuid, String> = session_scheduler_state::Entity::find()
        .filter(session_scheduler_state::Column::SessionIdFk.eq(session_id))
        .all(db)
        .await?
        .into_iter()
        .map(|r| (r.player_id_fk, r.state))
        .collect();

    Ok(eligible
        .iter()
        .map(|p| PlayerCompletion {
            player_id: p.id,
            done: scheduler_states
                .get(&p.id)
                .is_some_and(|s| s == SCHEDULER_STATE_COMPLETED),
        })
        .collect())
}

/// True when every eligible (non-revoked) player of `session_id` has
/// completed all of their tasks. `false` for sessions with no eligible
/// players (see [`all_done`]).
pub async fn all_eligible_players_done(
    db: &DatabaseConnection,
    session_id: Uuid,
) -> Result<bool, DbErr> {
    Ok(all_done(&session_player_completion(db, session_id).await?))
}

/// Judge-aware derived status for one eligible (non-revoked) player.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlayerStatus {
    pub player_id: Uuid,
    pub status: PlayerCompletionStatus,
}

/// Judge-aware derived status for every eligible (non-revoked) player of
/// `session_id`. Errors with `RecordNotFound` when the session does not
/// exist.
///
/// - Tasks remaining ⇒ [`PlayerCompletionStatus::InProgress`], regardless of
///   judge state.
/// - Tasks done but at least one expected judge run not yet terminal ⇒
///   [`PlayerCompletionStatus::AwaitingJudges`]. Expected judge runs are the
///   `task_judges` rows attached to the tasks the player has completed — for
///   a tasks-done player that is every task of the session's project, because
///   the scheduler only ever advances a player past a task once all of its
///   adapted tests pass (which is also the moment the game server enqueues
///   that task's attached judges). A judge run with no `judge_results` row
///   yet (the enqueue-wait window) is non-terminal.
/// - Tasks done and every expected judge run terminal
///   ([`JUDGE_RESULT_SCORED`] or [`JUDGE_RESULT_FAILED`] — a failed judge
///   never blocks completion) ⇒ [`PlayerCompletionStatus::Completed`]. With
///   zero attached judges a tasks-done player is immediately completed.
pub async fn session_player_statuses(
    db: &DatabaseConnection,
    session_id: Uuid,
) -> Result<Vec<PlayerStatus>, DbErr> {
    let session = load_session(db, session_id).await?;
    let completions = completion_for_session(db, &session).await?;

    // Judge state only matters for tasks-done players; skip the judge
    // queries entirely while nobody has finished.
    if completions.iter().all(|c| !c.done) {
        return Ok(completions
            .iter()
            .map(|c| PlayerStatus {
                player_id: c.player_id,
                status: PlayerCompletionStatus::InProgress,
            })
            .collect());
    }

    // Expected judge runs for a tasks-done player: every judge attached to
    // any of the session's tasks. Batched per session — no per-player loops.
    let task_ids: Vec<Uuid> = session_task_filter(tasks::Entity::find(), &session)
        .select_only()
        .column(tasks::Column::Id)
        .into_tuple()
        .all(db)
        .await?;
    let expected_judges: Vec<Uuid> =
        session_task_judge_filter(task_judges::Entity::find(), &session)
            .filter(task_judges::Column::TaskId.is_in(task_ids))
            .select_only()
            .column(task_judges::Column::Id)
            .into_tuple()
            .all(db)
            .await?;

    let mut terminal_by_player: HashMap<Uuid, HashSet<Uuid>> = HashMap::new();
    if !expected_judges.is_empty() {
        let terminal: Vec<(Uuid, Uuid)> = judge_results::Entity::find()
            .filter(judge_results::Column::SessionIdFk.eq(session_id))
            .filter(judge_results::Column::Status.is_in([JUDGE_RESULT_SCORED, JUDGE_RESULT_FAILED]))
            .select_only()
            .column(judge_results::Column::PlayerIdFk)
            .column(judge_results::Column::TaskJudgeId)
            .into_tuple()
            .all(db)
            .await?;
        for (player_id, task_judge_id) in terminal {
            terminal_by_player
                .entry(player_id)
                .or_default()
                .insert(task_judge_id);
        }
    }

    Ok(completions
        .iter()
        .map(|c| {
            let status = if !c.done {
                PlayerCompletionStatus::InProgress
            } else {
                let all_terminal = expected_judges.iter().all(|tj| {
                    terminal_by_player
                        .get(&c.player_id)
                        .is_some_and(|set| set.contains(tj))
                });
                if all_terminal {
                    PlayerCompletionStatus::Completed
                } else {
                    PlayerCompletionStatus::AwaitingJudges
                }
            };
            PlayerStatus {
                player_id: c.player_id,
                status,
            }
        })
        .collect())
}

/// One eligible player's task that was still in progress when the session
/// ended: the scheduler row points at a task and has not reached
/// [`SCHEDULER_STATE_COMPLETED`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InterruptedTask {
    pub player_id: Uuid,
    pub task_id: Uuid,
}

/// The (player, task) pairs interrupted mid-task for `session_id` — eligible
/// (non-revoked) players whose scheduler row has a current `task_id` and is
/// not completed. On time expiry the game server commits whatever the player
/// pushed and runs that task's attached judges anyway.
pub async fn interrupted_tasks(
    db: &DatabaseConnection,
    session_id: Uuid,
) -> Result<Vec<InterruptedTask>, DbErr> {
    let eligible: HashSet<Uuid> = players::Entity::find()
        .filter(players::Column::SessionIdFk.eq(session_id))
        .filter(players::Column::RevokedAt.is_null())
        .select_only()
        .column(players::Column::Id)
        .into_tuple()
        .all(db)
        .await?
        .into_iter()
        .collect();

    Ok(session_scheduler_state::Entity::find()
        .filter(session_scheduler_state::Column::SessionIdFk.eq(session_id))
        .filter(session_scheduler_state::Column::State.ne(SCHEDULER_STATE_COMPLETED))
        .filter(session_scheduler_state::Column::TaskId.is_not_null())
        .all(db)
        .await?
        .into_iter()
        .filter(|r| eligible.contains(&r.player_id_fk))
        .filter_map(|r| {
            r.task_id.map(|task_id| InterruptedTask {
                player_id: r.player_id_fk,
                task_id,
            })
        })
        .collect())
}

/// The tasks a player reached, given the project's `task_id -> ordinal` map
/// and their `session_scheduler_state` row as `(state, current_task_id)`.
///
/// "Reached" means: every task when the scheduler row is
/// [`SCHEDULER_STATE_COMPLETED`]; tasks at ordinals up to and INCLUDING the
/// current one when the player was interrupted mid-task; nothing when they
/// never got a scheduler row (or it points at a task outside this project).
///
/// This is the single definition of "which tasks does this player get judged
/// on". [`expired_session_pending_judges`] counts the judge runs it must wait
/// for from this set, and session-scoped judges must produce a terminal
/// `judge_results` row for exactly this set — a second, drifting definition
/// would leave the settle poll waiting for rows nobody writes, delaying Arena
/// Points until the deadline fires.
pub fn reached_task_ids(
    task_ordinals: &HashMap<Uuid, i32>,
    scheduler_row: Option<&(String, Option<Uuid>)>,
) -> Vec<Uuid> {
    match scheduler_row {
        None => Vec::new(),
        Some((state, _)) if state == SCHEDULER_STATE_COMPLETED => {
            task_ordinals.keys().copied().collect()
        }
        Some((_, Some(current))) => match task_ordinals.get(current) {
            Some(current_ordinal) => task_ordinals
                .iter()
                .filter(|(_, ord)| *ord <= current_ordinal)
                .map(|(id, _)| *id)
                .collect(),
            None => Vec::new(),
        },
        Some((_, None)) => Vec::new(),
    }
}

/// Single-player [`reached_task_ids`], loading the two inputs itself.
///
/// The batch path ([`expired_session_pending_judges`]) preloads both maps once
/// for every player; this is for callers judging one player at a time.
pub async fn reached_tasks_for_player(
    db: &DatabaseConnection,
    session_id: Uuid,
    player_id: Uuid,
) -> Result<Vec<Uuid>, DbErr> {
    let session = load_session(db, session_id).await?;
    let task_ordinals = session_task_ordinals(db, &session).await?;
    if task_ordinals.is_empty() {
        return Ok(Vec::new());
    }
    let row = session_scheduler_state::Entity::find()
        .filter(session_scheduler_state::Column::SessionIdFk.eq(session_id))
        .filter(session_scheduler_state::Column::PlayerIdFk.eq(player_id))
        .one(db)
        .await?
        .map(|r| (r.state, r.task_id));
    Ok(reached_task_ids(&task_ordinals, row.as_ref()))
}

/// `task_id -> ordinal` for every task the session ran with (see
/// [`session_task_filter`]).
async fn session_task_ordinals(
    db: &DatabaseConnection,
    session: &sessions::Model,
) -> Result<HashMap<Uuid, i32>, DbErr> {
    Ok(session_task_filter(tasks::Entity::find(), session)
        .select_only()
        .column(tasks::Column::Id)
        .column(tasks::Column::Ordinal)
        .into_tuple()
        .all(db)
        .await?
        .into_iter()
        .collect())
}

/// Number of judge runs still expected for an EXPIRED session before Arena
/// Points may be awarded. Zero means every expected run is terminal
/// ([`JUDGE_RESULT_SCORED`] or [`JUDGE_RESULT_FAILED`]).
///
/// Expected runs per eligible player are the `task_judges` attached to:
/// - every project task, when the player's scheduler row is completed;
/// - tasks at ordinals up to and INCLUDING the current one, when the player
///   was interrupted mid-task (the game server fires the interrupted task's
///   judges at expiry too);
/// - nothing, when the player never got a scheduler row.
///
/// A pair with no `judge_results` row yet (commit-wait window) counts as
/// pending. Callers poll this under a deadline: a run lost before row
/// creation (e.g. game-server restart) would otherwise pend forever.
pub async fn expired_session_pending_judges(
    db: &DatabaseConnection,
    session_id: Uuid,
) -> Result<usize, DbErr> {
    Ok(expired_session_pending_judge_pairs(db, session_id)
        .await?
        .len())
}

/// One judge run still expected of an expired session: the attachment has no
/// terminal `judge_results` row for this player yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PendingJudgePair {
    pub player_id: Uuid,
    pub task_id: Uuid,
    pub task_judge_id: Uuid,
}

/// The expected-but-not-terminal judge runs behind
/// [`expired_session_pending_judges`], enumerated. The recovery reaper uses
/// this exact set to settle sessions whose runs were lost for good — sharing
/// the definition keeps "what the settle poll waits for" and "what the reaper
/// closes out" from drifting apart.
pub async fn expired_session_pending_judge_pairs(
    db: &DatabaseConnection,
    session_id: Uuid,
) -> Result<Vec<PendingJudgePair>, DbErr> {
    let session = load_session(db, session_id).await?;

    let eligible: Vec<Uuid> = players::Entity::find()
        .filter(players::Column::SessionIdFk.eq(session_id))
        .filter(players::Column::RevokedAt.is_null())
        .select_only()
        .column(players::Column::Id)
        .into_tuple()
        .all(db)
        .await?;
    if eligible.is_empty() {
        return Ok(Vec::new());
    }

    let task_ordinals = session_task_ordinals(db, &session).await?;
    if task_ordinals.is_empty() {
        return Ok(Vec::new());
    }

    let judges_by_task: HashMap<Uuid, Vec<Uuid>> =
        session_task_judge_filter(task_judges::Entity::find(), &session)
            .filter(task_judges::Column::TaskId.is_in(task_ordinals.keys().copied()))
            .select_only()
            .column(task_judges::Column::TaskId)
            .column(task_judges::Column::Id)
            .into_tuple()
            .all(db)
            .await?
            .into_iter()
            .fold(
                HashMap::new(),
                |mut m: HashMap<Uuid, Vec<Uuid>>, (task_id, judge_id): (Uuid, Uuid)| {
                    m.entry(task_id).or_default().push(judge_id);
                    m
                },
            );
    if judges_by_task.is_empty() {
        return Ok(Vec::new());
    }

    let scheduler_rows: HashMap<Uuid, (String, Option<Uuid>)> =
        session_scheduler_state::Entity::find()
            .filter(session_scheduler_state::Column::SessionIdFk.eq(session_id))
            .all(db)
            .await?
            .into_iter()
            .map(|r| (r.player_id_fk, (r.state, r.task_id)))
            .collect();

    let terminal: HashSet<(Uuid, Uuid)> = judge_results::Entity::find()
        .filter(judge_results::Column::SessionIdFk.eq(session_id))
        .filter(judge_results::Column::Status.is_in([JUDGE_RESULT_SCORED, JUDGE_RESULT_FAILED]))
        .select_only()
        .column(judge_results::Column::PlayerIdFk)
        .column(judge_results::Column::TaskJudgeId)
        .into_tuple()
        .all(db)
        .await?
        .into_iter()
        .collect();

    let mut pending = Vec::new();
    for player_id in eligible {
        let judged_tasks = reached_task_ids(&task_ordinals, scheduler_rows.get(&player_id));
        for task_id in judged_tasks {
            for task_judge_id in judges_by_task.get(&task_id).into_iter().flatten() {
                if !terminal.contains(&(player_id, *task_judge_id)) {
                    pending.push(PendingJudgePair {
                        player_id,
                        task_id,
                        task_judge_id: *task_judge_id,
                    });
                }
            }
        }
    }
    Ok(pending)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fact(done: bool) -> PlayerCompletion {
        PlayerCompletion {
            player_id: Uuid::new_v4(),
            done,
        }
    }

    #[test]
    fn all_done_is_false_for_no_players() {
        assert!(!all_done(&[]));
    }

    #[test]
    fn all_done_is_false_while_any_player_is_pending() {
        assert!(!all_done(&[fact(true), fact(false)]));
    }

    #[test]
    fn all_done_is_true_when_every_player_finished() {
        assert!(all_done(&[fact(true), fact(true)]));
    }
}
