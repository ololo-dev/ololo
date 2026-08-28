//! Campaign progression: which parts of a campaign a user has cleared.
//!
//! A campaign is a parent `projects` row whose children carry
//! `parent_project_id_fk` + `part_ordinal`. Part N+1 opens for a user only
//! once they have *completed* part N, and "completed" is derived from
//! existing rows rather than a dedicated column:
//!
//! a finished session of that project, in which the user holds a
//! non-revoked `players` row whose `session_scheduler_state` reached
//! [`SCHEDULER_STATE_COMPLETED`] — the state the scheduler writes when a
//! player runs out of tasks.
//!
//! Playing a part and abandoning it therefore does not unlock the next one:
//! the follow-up part continues the previous part's codebase, so the gate
//! only opens on a working handover.
//!
//! Note a deliberate corner: a zero-task project leaves no scheduler row at
//! all, so it can never be "completed" here even though
//! [`crate::session_completion`] treats its players as trivially done.
//! Campaign parts are real task-bearing projects and parents cannot host
//! sessions, so the corner stays unreachable.

use crate::entities::{players, session_scheduler_state, sessions, task_results, tasks};
use crate::session_completion::SCHEDULER_STATE_COMPLETED;
use crate::session_status::SessionStatus;
use sea_orm::{
    ColumnTrait, DatabaseConnection, DbErr, EntityTrait, JoinType, QueryFilter, QueryOrder,
    QuerySelect, RelationTrait,
};
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

/// One finished session the user took part in, before the scheduler state is
/// consulted. `finished_at` orders "most recent completing run".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FinishedRun {
    pub project_id: Uuid,
    pub session_id: Uuid,
    pub player_id: Uuid,
    pub finished_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// Projects the user completed, given their finished runs and the set of
/// `(session_id, player_id)` pairs whose scheduler state reached
/// [`SCHEDULER_STATE_COMPLETED`]. Split out from the queries so the
/// progression rule itself is unit-testable.
pub fn completed_projects(
    runs: &[FinishedRun],
    completed_pairs: &HashSet<(Uuid, Uuid)>,
) -> HashSet<Uuid> {
    runs.iter()
        .filter(|r| completed_pairs.contains(&(r.session_id, r.player_id)))
        .map(|r| r.project_id)
        .collect()
}

/// The user's most recent completing run of `project_id`, if any. Ties on a
/// missing `finished_at` (should not happen for finished sessions) sort last.
pub fn latest_completing_run(
    runs: &[FinishedRun],
    completed_pairs: &HashSet<(Uuid, Uuid)>,
    project_id: Uuid,
) -> Option<FinishedRun> {
    runs.iter()
        .filter(|r| r.project_id == project_id)
        .filter(|r| completed_pairs.contains(&(r.session_id, r.player_id)))
        .max_by_key(|r| r.finished_at)
        .copied()
}

/// Finished sessions of `project_ids` the user took part in (non-revoked
/// player rows only).
async fn finished_runs(
    db: &DatabaseConnection,
    project_ids: &[Uuid],
    user_id: Uuid,
) -> Result<Vec<FinishedRun>, DbErr> {
    if project_ids.is_empty() {
        return Ok(Vec::new());
    }
    let rows: Vec<(Uuid, Uuid, Uuid, Option<chrono::DateTime<chrono::Utc>>)> =
        players::Entity::find()
            .join(JoinType::InnerJoin, players::Relation::Session.def())
            .filter(players::Column::UserIdFk.eq(user_id))
            .filter(players::Column::RevokedAt.is_null())
            .filter(sessions::Column::ProjectIdFk.is_in(project_ids.to_vec()))
            .filter(sessions::Column::Status.eq(SessionStatus::Finished))
            .select_only()
            .column(sessions::Column::ProjectIdFk)
            .column(players::Column::SessionIdFk)
            .column(players::Column::Id)
            .column(sessions::Column::FinishedAt)
            .order_by_asc(sessions::Column::FinishedAt)
            .into_tuple()
            .all(db)
            .await?;
    Ok(rows
        .into_iter()
        .map(
            |(project_id, session_id, player_id, finished_at)| FinishedRun {
                project_id,
                session_id,
                player_id,
                finished_at,
            },
        )
        .collect())
}

/// `(session_id, player_id)` pairs among `runs` whose scheduler state says
/// the player exhausted their task list — AND who actually earned each task
/// on the way through.
///
/// The scheduler alone is not enough. An open-ended task force-completes when
/// its work window (`completion.deadline_secs`) expires, so the judges can
/// score whatever exists — which means a player who delivered nothing at all
/// still exhausts the task list by sitting the window out. Session TJQJPJ did
/// exactly that: 0/1 tasks passed, and part two of its campaign unlocked
/// anyway.
///
/// What a forced completion never gets is the completion bonus — the game
/// server awards it only when the task completed via its own probe
/// (`completed_via_probe`). So the durable mark of an earned task is its
/// bonus row, and a completing run must carry one for every task the session
/// ran that pays a bonus. Tasks that pay none are exempt (no row is ever
/// written for them), as is a task added to the project after the session
/// finished — the same as-the-session-knew-it rule the settle poll uses.
async fn completed_pairs(
    db: &DatabaseConnection,
    runs: &[FinishedRun],
) -> Result<HashSet<(Uuid, Uuid)>, DbErr> {
    if runs.is_empty() {
        return Ok(HashSet::new());
    }
    let session_ids: Vec<Uuid> = runs
        .iter()
        .map(|r| r.session_id)
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    let rows = session_scheduler_state::Entity::find()
        .filter(session_scheduler_state::Column::SessionIdFk.is_in(session_ids.clone()))
        .filter(session_scheduler_state::Column::State.eq(SCHEDULER_STATE_COMPLETED))
        .all(db)
        .await?;
    let exhausted: HashSet<(Uuid, Uuid)> = rows
        .into_iter()
        .map(|r| (r.session_id_fk, r.player_id_fk))
        .collect();
    if exhausted.is_empty() {
        return Ok(exhausted);
    }

    // The bonus-paying tasks each session ran, as that session knew them.
    let session_rows = sessions::Entity::find()
        .filter(sessions::Column::Id.is_in(session_ids.clone()))
        .all(db)
        .await?;
    let mut required_by_session: HashMap<Uuid, HashSet<Uuid>> = HashMap::new();
    for session in &session_rows {
        let required: Vec<Uuid> =
            crate::session_completion::session_task_filter(tasks::Entity::find(), session)
                .filter(tasks::Column::CompletionBonusPoints.ne(0))
                .select_only()
                .column(tasks::Column::Id)
                .into_tuple()
                .all(db)
                .await?;
        required_by_session.insert(session.id, required.into_iter().collect());
    }

    // The bonuses actually awarded, per (session, player, task).
    let bonus_rows: Vec<(Uuid, Uuid, Option<Uuid>)> = task_results::Entity::find()
        .filter(task_results::Column::SessionIdFk.is_in(session_ids))
        .filter(task_results::Column::IsBonus.eq(true))
        .select_only()
        .column(task_results::Column::SessionIdFk)
        .column(task_results::Column::PlayerIdFk)
        .column(task_results::Column::TaskId)
        .into_tuple()
        .all(db)
        .await?;
    let mut earned: HashMap<(Uuid, Uuid), HashSet<Uuid>> = HashMap::new();
    for (session_id, player_id, task_id) in bonus_rows {
        if let Some(task_id) = task_id {
            earned
                .entry((session_id, player_id))
                .or_default()
                .insert(task_id);
        }
    }

    Ok(exhausted
        .into_iter()
        .filter(|(session_id, player_id)| {
            let Some(required) = required_by_session.get(session_id) else {
                return false;
            };
            let empty = HashSet::new();
            let got = earned.get(&(*session_id, *player_id)).unwrap_or(&empty);
            required.iter().all(|t| got.contains(t))
        })
        .collect())
}

/// Which of `project_ids` the user has completed. One query pair regardless
/// of how many parts a campaign has.
pub async fn user_completed_projects(
    db: &DatabaseConnection,
    project_ids: &[Uuid],
    user_id: Uuid,
) -> Result<HashSet<Uuid>, DbErr> {
    let runs = finished_runs(db, project_ids, user_id).await?;
    let pairs = completed_pairs(db, &runs).await?;
    Ok(completed_projects(&runs, &pairs))
}

/// One finished run with the player it belongs to — the batch shape, for
/// answering "how far did each of these people get" in one pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UserFinishedRun {
    pub user_id: Uuid,
    pub run: FinishedRun,
}

/// Every *completing* run of `project_ids` by any of `user_ids`. Two queries
/// for the whole set, however many players and parts it holds — the per-user
/// [`user_completed_projects`] would be two queries *each*.
///
/// Callers that only need "did they clear it" want
/// [`completed_projects_for_users`]; this one keeps the run itself, for
/// linking to the session a part was cleared in.
pub async fn completing_runs_for_users(
    db: &DatabaseConnection,
    project_ids: &[Uuid],
    user_ids: &[Uuid],
) -> Result<Vec<UserFinishedRun>, DbErr> {
    if project_ids.is_empty() || user_ids.is_empty() {
        return Ok(Vec::new());
    }
    /// `(user, project, session, player, finished_at)` — one clearing
    /// candidate, before the scheduler state is consulted.
    type RunRow = (
        Uuid,
        Uuid,
        Uuid,
        Uuid,
        Option<chrono::DateTime<chrono::Utc>>,
    );
    let rows: Vec<RunRow> = players::Entity::find()
        .join(JoinType::InnerJoin, players::Relation::Session.def())
        .filter(players::Column::UserIdFk.is_in(user_ids.to_vec()))
        .filter(players::Column::RevokedAt.is_null())
        .filter(sessions::Column::ProjectIdFk.is_in(project_ids.to_vec()))
        .filter(sessions::Column::Status.eq(SessionStatus::Finished))
        .select_only()
        .column(players::Column::UserIdFk)
        .column(sessions::Column::ProjectIdFk)
        .column(players::Column::SessionIdFk)
        .column(players::Column::Id)
        .column(sessions::Column::FinishedAt)
        .order_by_asc(sessions::Column::FinishedAt)
        .into_tuple()
        .all(db)
        .await?;

    let runs: Vec<UserFinishedRun> = rows
        .into_iter()
        .map(
            |(user_id, project_id, session_id, player_id, finished_at)| UserFinishedRun {
                user_id,
                run: FinishedRun {
                    project_id,
                    session_id,
                    player_id,
                    finished_at,
                },
            },
        )
        .collect();
    let plain: Vec<FinishedRun> = runs.iter().map(|r| r.run).collect();
    let pairs = completed_pairs(db, &plain).await?;
    Ok(runs
        .into_iter()
        .filter(|r| pairs.contains(&(r.run.session_id, r.run.player_id)))
        .collect())
}

/// Which of `project_ids` each of `user_ids` completed.
pub async fn completed_projects_for_users(
    db: &DatabaseConnection,
    project_ids: &[Uuid],
    user_ids: &[Uuid],
) -> Result<HashMap<Uuid, HashSet<Uuid>>, DbErr> {
    let runs = completing_runs_for_users(db, project_ids, user_ids).await?;
    let mut by_user: HashMap<Uuid, HashSet<Uuid>> = HashMap::new();
    for r in runs {
        by_user
            .entry(r.user_id)
            .or_default()
            .insert(r.run.project_id);
    }
    Ok(by_user)
}

/// True when the user has a completing run of `project_id`.
pub async fn user_completed_project(
    db: &DatabaseConnection,
    project_id: Uuid,
    user_id: Uuid,
) -> Result<bool, DbErr> {
    Ok(user_completed_projects(db, &[project_id], user_id)
        .await?
        .contains(&project_id))
}

/// The user's most recent completing session of `project_id`, as
/// `(session_id, player_id)` — the snapshot repo the next part continues
/// from.
pub async fn latest_completing_session(
    db: &DatabaseConnection,
    project_id: Uuid,
    user_id: Uuid,
) -> Result<Option<(Uuid, Uuid)>, DbErr> {
    let runs = finished_runs(db, &[project_id], user_id).await?;
    let pairs = completed_pairs(db, &runs).await?;
    Ok(latest_completing_run(&runs, &pairs, project_id).map(|r| (r.session_id, r.player_id)))
}

/// Projects the user currently has a live (lobby/running/paused) player row
/// in, restricted to `project_ids`. Feeds the "in progress" state of a
/// campaign parts list.
pub async fn user_live_projects(
    db: &DatabaseConnection,
    project_ids: &[Uuid],
    user_id: Uuid,
) -> Result<HashSet<Uuid>, DbErr> {
    if project_ids.is_empty() {
        return Ok(HashSet::new());
    }
    let rows: Vec<Uuid> = players::Entity::find()
        .join(JoinType::InnerJoin, players::Relation::Session.def())
        .filter(players::Column::UserIdFk.eq(user_id))
        .filter(players::Column::RevokedAt.is_null())
        .filter(sessions::Column::ProjectIdFk.is_in(project_ids.to_vec()))
        .filter(sessions::Column::Status.is_in(vec![
            SessionStatus::Lobby,
            SessionStatus::Running,
            SessionStatus::Paused,
        ]))
        .select_only()
        .column(sessions::Column::ProjectIdFk)
        .into_tuple()
        .all(db)
        .await?;
    Ok(rows.into_iter().collect())
}

/// Per-caller state of one campaign part.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PartState {
    /// The caller cleared this part.
    Completed,
    /// The caller has a live session on this part right now.
    InProgress,
    /// Open to start: the first part, or the previous part is completed.
    Available,
    /// The previous part is not completed yet.
    Locked,
}

impl PartState {
    pub fn as_str(self) -> &'static str {
        match self {
            PartState::Completed => "completed",
            PartState::InProgress => "in_progress",
            PartState::Available => "available",
            PartState::Locked => "locked",
        }
    }
}

/// State of each part, in the given (ordinal) order. The first part is always
/// startable; every later part needs its predecessor completed. An anonymous
/// caller passes empty sets and sees part one available, the rest locked.
pub fn part_states(
    ordered_part_ids: &[Uuid],
    completed: &HashSet<Uuid>,
    live: &HashSet<Uuid>,
) -> Vec<PartState> {
    let mut prev_completed = true;
    let mut out = Vec::with_capacity(ordered_part_ids.len());
    for id in ordered_part_ids {
        let is_completed = completed.contains(id);
        let state = if is_completed {
            PartState::Completed
        } else if live.contains(id) {
            PartState::InProgress
        } else if prev_completed {
            PartState::Available
        } else {
            PartState::Locked
        };
        out.push(state);
        prev_completed = is_completed;
    }
    out
}

/// The part immediately before `part_ordinal` in the same campaign — the one
/// whose completion unlocks it. Written as "greatest ordinal below" rather
/// than `ordinal - 1` so a campaign that lost a part to a seed edit still
/// chains instead of dead-locking.
pub fn previous_part<T>(
    siblings: &[T],
    part_ordinal: i32,
    ordinal_of: impl Fn(&T) -> i32,
) -> Option<&T> {
    siblings
        .iter()
        .filter(|s| ordinal_of(s) < part_ordinal)
        .max_by_key(|s| ordinal_of(s))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};

    fn uid(n: u8) -> Uuid {
        Uuid::from_bytes([n; 16])
    }

    fn run(project: u8, session: u8, player: u8, finished_secs: i64) -> FinishedRun {
        FinishedRun {
            project_id: uid(project),
            session_id: uid(session),
            player_id: uid(player),
            finished_at: Some(Utc.timestamp_opt(finished_secs, 0).unwrap()),
        }
    }

    #[test]
    fn a_finished_session_without_a_completed_scheduler_row_does_not_count() {
        let runs = vec![run(1, 10, 20, 100)];
        let completed = completed_projects(&runs, &HashSet::new());
        assert!(completed.is_empty());
    }

    #[test]
    fn a_completing_run_marks_its_project_completed() {
        let runs = vec![run(1, 10, 20, 100)];
        let pairs: HashSet<_> = [(uid(10), uid(20))].into_iter().collect();
        assert_eq!(
            completed_projects(&runs, &pairs),
            [uid(1)].into_iter().collect::<HashSet<_>>()
        );
    }

    #[test]
    fn the_newest_completing_run_wins() {
        let runs = vec![
            run(1, 10, 20, 100),
            run(1, 11, 21, 500),
            run(1, 12, 22, 300),
        ];
        // The middle one is the only non-completing run, to prove filtering
        // happens before the recency pick.
        let pairs: HashSet<_> = [(uid(10), uid(20)), (uid(12), uid(22))]
            .into_iter()
            .collect();
        let latest = latest_completing_run(&runs, &pairs, uid(1)).expect("a completing run");
        assert_eq!(latest.session_id, uid(12));
        assert_eq!(latest.player_id, uid(22));
    }

    #[test]
    fn parts_unlock_one_at_a_time() {
        let parts = vec![uid(1), uid(2), uid(3)];
        let completed: HashSet<_> = [uid(1)].into_iter().collect();
        assert_eq!(
            part_states(&parts, &completed, &HashSet::new()),
            vec![
                PartState::Completed,
                PartState::Available,
                PartState::Locked
            ]
        );
    }

    #[test]
    fn an_anonymous_caller_sees_only_the_first_part_open() {
        let parts = vec![uid(1), uid(2)];
        assert_eq!(
            part_states(&parts, &HashSet::new(), &HashSet::new()),
            vec![PartState::Available, PartState::Locked]
        );
    }

    #[test]
    fn a_live_session_shows_as_in_progress_without_unlocking_the_next_part() {
        let parts = vec![uid(1), uid(2)];
        let live: HashSet<_> = [uid(1)].into_iter().collect();
        assert_eq!(
            part_states(&parts, &HashSet::new(), &live),
            vec![PartState::InProgress, PartState::Locked]
        );
    }

    #[test]
    fn a_gap_in_ordinals_still_chains() {
        let siblings = vec![0, 1, 4];
        assert_eq!(previous_part(&siblings, 7, |o| *o), Some(&4));
        assert_eq!(previous_part(&siblings, 0, |o| *o), None);
    }
}
