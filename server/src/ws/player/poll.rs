//! Player WebSocket poll task: watches probes/scheduler state and fans out frames.

use crate::scoring::broadcast_leaderboard;
use crate::state::{PlayerChannel, PlayerRegistry, SessionRegistry};
use arena_core::entities::{
    probes, session_scheduler_state, sessions, tasks, tests as entity_tests,
};
use arena_core::protocol::{
    PlayerDeltaPayload, PlayerFrame, PlayerSchedulerState, PlayerTaskResult,
    PlayerTaskSummaryEntry, ProbeState, ProbeUpdatedPayload, TaskRevealedPayload,
};
use arena_core::scoring::read_score_rank;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use uuid::Uuid;

pub(crate) async fn db_poll_task(
    db: sea_orm::DatabaseConnection,
    channel: Arc<PlayerChannel>,
    player_id: Uuid,
    session_id: Uuid,
    session_registry: SessionRegistry,
    player_registry: PlayerRegistry,
    join_code: String,
) {
    let mut last_poll_ts = chrono::Utc::now();
    let mut interval = tokio::time::interval(Duration::from_secs(1));
    interval.tick().await;
    let mut last_current_task_id: Option<Uuid> = None;
    let mut last_sched_state: Option<String> = None;

    // Pre-load session tasks and adapted tests
    let session = sessions::Entity::find_by_id(session_id)
        .one(&db)
        .await
        .ok()
        .flatten();
    let (task_rows, all_adapted_rows, total_tasks) = match &session {
        Some(s) => {
            let tasks_result = tasks::Entity::find()
                .filter(tasks::Column::ProjectIdFk.eq(s.project_id_fk))
                .order_by_asc(tasks::Column::Ordinal)
                .all(&db)
                .await
                .unwrap_or_default();
            let total = tasks_result.len();
            let adapted = entity_tests::Entity::find()
                .filter(entity_tests::Column::SessionId.eq(session_id))
                .order_by_asc(entity_tests::Column::Ordinal)
                .all(&db)
                .await
                .unwrap_or_default();
            (tasks_result, adapted, total)
        }
        None => (vec![], vec![], 0),
    };

    let ordinal_by_task: HashMap<Uuid, i32> = task_rows.iter().map(|t| (t.id, t.ordinal)).collect();
    let title_by_task: HashMap<Uuid, String> =
        task_rows.iter().map(|t| (t.id, t.title.clone())).collect();
    let mut snippet_by_task: HashMap<Uuid, String> = HashMap::new();
    for a in &all_adapted_rows {
        snippet_by_task
            .entry(a.task_id)
            .or_insert_with(|| a.command_template.clone());
    }

    loop {
        interval.tick().await;

        // A. Read scheduler state every tick
        let sched_row = session_scheduler_state::Entity::find()
            .filter(session_scheduler_state::Column::PlayerIdFk.eq(player_id))
            .filter(session_scheduler_state::Column::SessionIdFk.eq(session_id))
            .one(&db)
            .await
            .ok()
            .flatten();
        let current_task_id = sched_row.as_ref().and_then(|s| s.task_id);
        let next_probe_at = sched_row.as_ref().and_then(|s| s.next_probe_at);

        // A2. The scheduler also changes state on the SAME task — "judging"
        // is the hold after an accepted delivery while the panel works, and
        // a live page must narrate it (the snapshot only carries it on
        // reload). Emit a delta with the fresh state; a delta without a
        // result leaves the task's result untouched client-side.
        let sched_state = sched_row.as_ref().map(|s| s.state.clone());
        if let Some((task_id, state)) = sched_state_change(
            current_task_id,
            last_current_task_id,
            &sched_state,
            &last_sched_state,
        ) {
            let (score, rank) = registry_score_rank(&session_registry, session_id, player_id);
            let delta = PlayerDeltaPayload {
                seq: 0,
                task_id,
                result: None,
                scheduler_state: Some(PlayerSchedulerState {
                    state,
                    activated_at: None,
                    deadline_at: None,
                }),
                score,
                rank,
            };
            if channel
                .send_sequenced(|seq| {
                    PlayerFrame::PlayerTaskDelta(PlayerDeltaPayload { seq, ..delta })
                })
                .is_err()
            {
                tracing::debug!("db_poll_task: broadcast channel closed, exiting");
                return;
            }
        }
        last_sched_state = sched_state;

        // B. Emit TaskRevealed on task transition
        if current_task_id != last_current_task_id {
            // Mark the previous task as completed via PlayerTaskDelta.
            if let Some(old_id) = last_current_task_id {
                let (score, rank) = registry_score_rank(&session_registry, session_id, player_id);
                let delta = PlayerDeltaPayload {
                    seq: 0,
                    task_id: old_id,
                    result: Some(PlayerTaskResult {
                        status: "completed".to_string(),
                        submitted_answer: None,
                        correct_answer: None,
                        score_delta: 0,
                        evaluated_at: None,
                    }),
                    scheduler_state: None,
                    score,
                    rank,
                };
                if channel
                    .send_sequenced(|seq| {
                        PlayerFrame::PlayerTaskDelta(PlayerDeltaPayload { seq, ..delta })
                    })
                    .is_err()
                {
                    tracing::debug!("db_poll_task: broadcast channel closed, exiting");
                    return;
                }
            }
            if let Some(new_id) = current_task_id
                && let Some(task) = task_rows.iter().find(|t| t.id == new_id)
            {
                let adapted_content = snippet_by_task.get(&task.id).cloned().unwrap_or_default();
                let sched_state = sched_row.as_ref().map(|s| PlayerSchedulerState {
                    state: s.state.clone(),
                    activated_at: None,
                    deadline_at: None,
                });
                let task_entry = PlayerTaskSummaryEntry {
                    task_id: task.id,
                    ordinal: task.ordinal,
                    title: task.title.clone(),
                    content: task.content.clone(),
                    tags: serde_json::from_str::<Vec<String>>(&task.tags).unwrap_or_default(),
                    adapted_content,
                    result: None,
                    scheduler_state: sched_state,
                    total_points: 0,
                    bonus_points: 0,
                };
                let payload = TaskRevealedPayload {
                    seq: 0,
                    task: task_entry,
                    total_tasks,
                };
                if channel
                    .send_sequenced(|seq| {
                        PlayerFrame::TaskRevealed(TaskRevealedPayload { seq, ..payload })
                    })
                    .is_err()
                {
                    tracing::debug!("db_poll_task: broadcast channel closed, exiting");
                    return;
                }
                // NOTE: no activity_event insert / observer TaskStarted broadcast here.
                // game-server's emit_task_started is the single authoritative emitter
                // (persists the row and publishes ZmqEvent::TaskStarted, which zmq_sub
                // bridges to observers). This poll task runs per player WS connection
                // and fires on every reconnect, so emitting here duplicated log rows.
            }
            last_current_task_id = current_task_id;
        }

        // C. Query changed probes
        let changed_probes = match probes::Entity::find()
            .filter(probes::Column::SessionId.eq(session_id))
            .filter(probes::Column::PlayerId.eq(player_id))
            .filter(probes::Column::UpdatedAt.gt(last_poll_ts))
            .order_by_asc(probes::Column::UpdatedAt)
            .all(&db)
            .await
        {
            Ok(rows) => rows,
            Err(e) => {
                tracing::warn!(error = %e, "db_poll_task: query failed, retrying next tick");
                continue;
            }
        };

        if changed_probes.is_empty() {
            last_poll_ts = chrono::Utc::now();
            continue;
        }

        // D. Build adapted_test maps for changed probes
        let test_ids: Vec<Uuid> = changed_probes
            .iter()
            .map(|p| p.test_id)
            .collect::<std::collections::HashSet<_>>()
            .into_iter()
            .collect();

        let adapted_rows = match entity_tests::Entity::find()
            .filter(entity_tests::Column::Id.is_in(test_ids))
            .all(&db)
            .await
        {
            Ok(rows) => rows,
            Err(e) => {
                tracing::warn!(error = %e, "db_poll_task: adapted_tests query failed, skipping this tick");
                last_poll_ts = chrono::Utc::now();
                continue;
            }
        };

        let mut title_by_adapted_test: HashMap<Uuid, String> = HashMap::new();
        let mut command_by_adapted_test: HashMap<Uuid, String> = HashMap::new();
        for a in &adapted_rows {
            if let Some(title) = title_by_task.get(&a.task_id) {
                title_by_adapted_test.insert(a.id, title.clone());
            }
            command_by_adapted_test.insert(a.id, a.command_template.clone());
        }

        // E. Broadcast leaderboard when any probe has been resolved — this updates
        // the session cache and fans out a LeaderboardUpdate to all session observers
        // (the session page) and ScoreRankUpdated to all connected player channels.
        // Must happen before reading score/rank so the values below are fresh.
        let has_resolved = changed_probes.iter().any(|p| p.outcome.is_some());
        if has_resolved {
            broadcast_leaderboard(
                &db,
                &session_registry,
                &player_registry,
                &join_code,
                session_id,
            )
            .await;
        }

        // F. Compute score/rank from the (now-updated) session cache
        let (score, rank) = registry_score_rank(&session_registry, session_id, player_id);

        // G. Emit ProbeUpdated for each changed probe
        for probe in &changed_probes {
            let adapted_test = adapted_rows.iter().find(|a| a.id == probe.test_id);
            let task_id = match adapted_test {
                Some(at) => at.task_id,
                None => {
                    tracing::warn!(probe_id = %probe.id, "db_poll_task: adapted_test not found, skipping probe");
                    continue;
                }
            };
            let task_title = title_by_adapted_test
                .get(&probe.test_id)
                .cloned()
                .unwrap_or_default();
            let task_ordinal = ordinal_by_task.get(&task_id).copied().unwrap_or(0);
            let test_command = command_by_adapted_test
                .get(&probe.test_id)
                .cloned()
                .unwrap_or_default();
            let probe_state = if probe.outcome.is_none() {
                ProbeState::Dispatched
            } else {
                ProbeState::Resolved
            };

            // Parse secret metadata to suppress the expected value from
            // the client-visible payload. fixture_values was redacted at
            // persist time by the game-server; only expected needs work.
            let secret_meta = arena_core::probe_engine::SecretMeta::parse(&probe.secret_meta);
            let secret_expected = secret_meta.as_ref().map(|m| m.expected).unwrap_or(false);
            let suppressed_expected = if secret_expected {
                None
            } else {
                probe
                    .resolved_answer
                    .clone()
                    .or_else(|| probe.expected_answer.clone())
            };
            let payload = ProbeUpdatedPayload {
                seq: 0,
                probe_id: probe.id,
                task_id,
                task_title,
                task_ordinal,
                adapted_test_id: probe.test_id,
                test_ordinal: adapted_test.map(|at| at.ordinal),
                label: adapted_test.and_then(|at| {
                    arena_core::protocol::test_display_label(&at.prompt).map(str::to_string)
                }),
                description: adapted_test.and_then(|at| {
                    at.description
                        .as_deref()
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(str::to_string)
                }),
                test_command,
                attempt: probe.attempt,
                rendered_command: probe.rendered_command.clone(),
                fixture_values: Some(probe.fixture_values.clone()),
                expected_answer: if secret_expected {
                    None
                } else {
                    probe.expected_answer.clone()
                },
                state: probe_state,
                outcome: probe.outcome.clone(),
                actual: probe.output.clone(),
                expected: suppressed_expected,
                exit_code: probe.exit_code,
                duration_ms: probe.duration_ms,
                dispatched_at: Some(probe.dispatched_at),
                deadline_at: Some(probe.deadline_at),
                resolved_at: probe.resolved_at,
                point_delta: probe.point_delta.unwrap_or(0),
                score,
                rank,
                updated_at: probe.updated_at.unwrap_or(probe.dispatched_at),
                next_probe_at,
            };

            if channel
                .send_sequenced(|seq| {
                    PlayerFrame::ProbeUpdated(ProbeUpdatedPayload { seq, ..payload })
                })
                .is_err()
            {
                tracing::debug!("db_poll_task: broadcast channel closed, exiting");
                return;
            }
        }

        last_poll_ts = chrono::Utc::now();
    }
}

/// Whether this tick must announce a scheduler-state change: the task is
/// unchanged but its state differs from the last tick. Task transitions are
/// not state changes — TaskRevealed carries the fresh state itself — and a
/// vanished scheduler row announces nothing (the transition delta owns the
/// "cleared" story).
fn sched_state_change(
    current_task_id: Option<Uuid>,
    last_task_id: Option<Uuid>,
    state: &Option<String>,
    last_state: &Option<String>,
) -> Option<(Uuid, String)> {
    if current_task_id != last_task_id || state == last_state {
        return None;
    }
    match (current_task_id, state) {
        (Some(task_id), Some(state)) => Some((task_id, state.clone())),
        _ => None,
    }
}

/// The player's score and rank from the in-memory session cache.
fn registry_score_rank(
    session_registry: &SessionRegistry,
    session_id: Uuid,
    player_id: Uuid,
) -> (i64, usize) {
    session_registry
        .iter()
        .find(|e| {
            let cache = e.cache.read().unwrap_or_else(|e2| e2.into_inner());
            cache.session_id == session_id
        })
        .map(|entry| {
            let cache = entry.cache.read().unwrap_or_else(|e2| e2.into_inner());
            read_score_rank(&cache.leaderboard, player_id)
        })
        .unwrap_or((0, 0))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &str) -> Option<String> {
        Some(v.to_string())
    }

    #[test]
    fn same_task_new_state_announces() {
        let t = Some(Uuid::new_v4());
        let got = sched_state_change(t, t, &s("judging"), &s("active"));
        assert_eq!(got, Some((t.unwrap(), "judging".to_string())));
        // First observation of a state on this task counts too.
        assert!(sched_state_change(t, t, &s("active"), &None).is_some());
    }

    #[test]
    fn unchanged_state_stays_silent() {
        let t = Some(Uuid::new_v4());
        assert_eq!(sched_state_change(t, t, &s("judging"), &s("judging")), None);
        assert_eq!(sched_state_change(None, None, &None, &None), None);
    }

    #[test]
    fn task_transitions_are_not_state_changes() {
        // TaskRevealed carries the fresh state itself; announcing here too
        // would double-fire on every advance.
        let old = Some(Uuid::new_v4());
        let new = Some(Uuid::new_v4());
        assert_eq!(
            sched_state_change(new, old, &s("active"), &s("judging")),
            None
        );
        assert_eq!(sched_state_change(new, None, &s("active"), &None), None);
    }

    #[test]
    fn a_vanished_scheduler_row_announces_nothing() {
        let t = Some(Uuid::new_v4());
        assert_eq!(sched_state_change(t, t, &None, &s("judging")), None);
        assert_eq!(sched_state_change(None, None, &None, &s("judging")), None);
    }
}
