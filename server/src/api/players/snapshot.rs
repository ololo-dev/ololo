use crate::auth::jwt::AccessClaims;
use crate::state::AppState;
use arena_core::entities::{
    judge_results, judges, players, probes, projects, session_scheduler_state, sessions,
    task_judges, task_results, tasks, tests as entity_tests, users,
};
use arena_core::protocol::{
    PlayerJudgeScoredPayload, PlayerProbeEntry, PlayerProbeResult, PlayerSchedulerState,
    PlayerSnapshotData, PlayerSnapshotPayload, PlayerTaskResult, PlayerTaskSummaryEntry,
    ProbeState,
};
use axum::Json;
use axum::extract::{Path, State};
use chrono::Utc;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder};
use std::collections::HashMap;
use uuid::Uuid;

use super::error::PlayerError;

pub async fn get_player_snapshot(
    Path((code, player_param)): Path<(String, String)>,
    State(state): State<AppState>,
    claims: AccessClaims,
) -> Result<Json<PlayerSnapshotPayload>, PlayerError> {
    let user_id = claims.user_id().map_err(|_| PlayerError::Forbidden)?;

    // Look up session by join code
    let session = sessions::Entity::find()
        .filter(sessions::Column::JoinCode.eq(code.to_uppercase()))
        .one(&state.db)
        .await
        .map_err(|e| PlayerError::Internal(e.to_string()))?
        .ok_or(PlayerError::NotFound)?;

    // Resolve the player within the session (UUID or display name).
    let player = super::find_player_by_param(&state.db, session.id, &player_param)
        .await
        .map_err(|e| PlayerError::Internal(e.to_string()))?
        .ok_or(PlayerError::NotFound)?;
    let player_id = player.id;

    // Verify the player belongs to the authenticated user; admins may view
    // any player's page.
    if player.user_id_fk != Some(user_id)
        && !crate::auth::is_user_admin(&state.db, user_id)
            .await
            .map_err(|e| PlayerError::Internal(e.to_string()))?
    {
        return Err(PlayerError::Forbidden);
    }

    let last_seq = state
        .player_registry
        .get(&player_id)
        .map(|ch| ch.seq.load(std::sync::atomic::Ordering::SeqCst))
        .unwrap_or(0);

    let snapshot = build_snapshot(
        &state,
        player_id,
        session.id,
        &player.display_name,
        last_seq,
    )
    .await;
    Ok(Json(snapshot))
}

pub(crate) async fn build_snapshot(
    state: &AppState,
    player_id: Uuid,
    session_id: Uuid,
    display_name: &str,
    last_seq: u64,
) -> PlayerSnapshotPayload {
    build_snapshot_from_parts(
        &state.db,
        &state.session_registry,
        player_id,
        session_id,
        display_name,
        last_seq,
    )
    .await
}

pub(crate) async fn build_snapshot_from_parts(
    db: &sea_orm::DatabaseConnection,
    session_registry: &crate::state::SessionRegistry,
    player_id: Uuid,
    session_id: Uuid,
    display_name: &str,
    last_seq: u64,
) -> PlayerSnapshotPayload {
    let session = sessions::Entity::find_by_id(session_id)
        .one(db)
        .await
        .ok()
        .flatten();

    // Session duration is sourced from the project row at request time so a
    // project edit is reflected immediately in `session_ends_at`.
    let session_duration_secs: i64 = match &session {
        Some(s) => projects::Entity::find_by_id(s.project_id_fk)
            .one(db)
            .await
            .ok()
            .flatten()
            .map(|p| p.default_session_duration_secs)
            .unwrap_or(3600),
        None => 3600,
    };

    // Best-effort: read the player row to surface their chosen AI coding
    // agent name and the owning account's avatar. A missing row or malformed
    // metadata yields None — the rest of the snapshot is unaffected.
    let player_row = players::Entity::find_by_id(player_id)
        .one(db)
        .await
        .ok()
        .flatten();
    let agent_display_name = player_row
        .as_ref()
        .and_then(|p| arena_core::scoring::parse_agent_display_name(p.metadata_json.as_deref()));
    let avatar_url = match player_row.as_ref().and_then(|p| p.user_id_fk) {
        Some(uid) => users::Entity::find_by_id(uid)
            .one(db)
            .await
            .ok()
            .flatten()
            .and_then(|u| u.avatar_url),
        None => None,
    };

    let task_rows = if let Some(sess) = &session {
        // Only the tasks this session ran with: a task seeded into the
        // project after the session finished never ran here, and rendering
        // it (or expecting its judges below) would show a settled session
        // as waiting on judge runs nobody will ever start.
        let mut query =
            tasks::Entity::find().filter(tasks::Column::ProjectIdFk.eq(sess.project_id_fk));
        if let Some(finished) = sess.finished_at {
            query = query.filter(tasks::Column::CreatedAt.lte(finished));
        }
        query
            .order_by_asc(tasks::Column::Ordinal)
            .all(db)
            .await
            .unwrap_or_default()
    } else {
        vec![]
    };

    let adapted_tests_rows = entity_tests::Entity::find()
        .filter(entity_tests::Column::SessionId.eq(session_id))
        .order_by_asc(entity_tests::Column::Ordinal)
        .all(db)
        .await
        .unwrap_or_default();
    let mut task_id_by_test_id: HashMap<Uuid, Uuid> = HashMap::new();
    let mut command_by_test_id: HashMap<Uuid, String> = HashMap::new();
    let mut ordinal_by_test_id: HashMap<Uuid, i32> = HashMap::new();
    let mut label_by_test_id: HashMap<Uuid, String> = HashMap::new();
    let mut description_by_test_id: HashMap<Uuid, String> = HashMap::new();
    let mut snippet_by_task: HashMap<Uuid, String> = HashMap::new();
    for row in &adapted_tests_rows {
        task_id_by_test_id.insert(row.id, row.task_id);
        command_by_test_id.insert(row.id, row.command_template.clone());
        ordinal_by_test_id.insert(row.id, row.ordinal);
        if let Some(label) = arena_core::protocol::test_display_label(&row.prompt) {
            label_by_test_id.insert(row.id, label.to_string());
        }
        if let Some(desc) = row
            .description
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            description_by_test_id.insert(row.id, desc.to_string());
        }
        snippet_by_task
            .entry(row.task_id)
            .or_insert_with(|| row.command_template.clone());
    }

    // Chronological: artifact dedupe downstream keeps the newest delivery.
    let probe_rows = probes::Entity::find()
        .filter(probes::Column::SessionId.eq(session_id))
        .filter(probes::Column::PlayerId.eq(player_id))
        .order_by_asc(probes::Column::DispatchedAt)
        .all(db)
        .await
        .unwrap_or_default();
    let mut latest_probe_by_task: HashMap<Uuid, probes::Model> = HashMap::new();
    for probe in &probe_rows {
        if let Some(task_id) = task_id_by_test_id.get(&probe.test_id).copied() {
            match latest_probe_by_task.get(&task_id) {
                Some(existing) if existing.dispatched_at >= probe.dispatched_at => {}
                _ => {
                    latest_probe_by_task.insert(task_id, probe.clone());
                }
            }
        }
    }

    let title_by_task: HashMap<Uuid, String> =
        task_rows.iter().map(|t| (t.id, t.title.clone())).collect();
    let ordinal_by_task: HashMap<Uuid, i32> = task_rows.iter().map(|t| (t.id, t.ordinal)).collect();

    let probe_entries: Vec<PlayerProbeEntry> = probe_rows
        .iter()
        .map(|probe| {
            let task_id = task_id_by_test_id
                .get(&probe.test_id)
                .copied()
                .unwrap_or(probe.session_id);
            let task_title = title_by_task.get(&task_id).cloned().unwrap_or_default();
            let task_ordinal = ordinal_by_task.get(&task_id).copied().unwrap_or(0);
            let test_command = command_by_test_id
                .get(&probe.test_id)
                .cloned()
                .unwrap_or_default();

            // Parse secret metadata to suppress secret fixture keys and the
            // expected value from the client-visible probe entry. The DB
            // row keeps the raw values for grading; only the wire payload
            // is redacted. fixture_values was already redacted at persist
            // time by the game-server, so we only suppress expected here.
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
            PlayerProbeEntry {
                id: probe.id,
                task_id,
                task_title,
                task_ordinal,
                adapted_test_id: probe.test_id,
                test_ordinal: ordinal_by_test_id.get(&probe.test_id).copied(),
                label: label_by_test_id.get(&probe.test_id).cloned(),
                description: description_by_test_id.get(&probe.test_id).cloned(),
                test_command,
                attempt: probe.attempt,
                rendered_command: probe.rendered_command.clone(),
                fixture_values: Some(probe.fixture_values.clone()),
                // expected_answer holds the raw predicate template. Suppress
                // when the task marked the expected value secret.
                expected_answer: if secret_expected {
                    None
                } else {
                    probe.expected_answer.clone()
                },
                state: if probe.outcome.is_none() {
                    ProbeState::Dispatched
                } else {
                    ProbeState::Resolved
                },
                outcome: probe.outcome.clone(),
                actual: probe.output.clone(),
                expected: suppressed_expected.clone(),
                exit_code: probe.exit_code,
                duration_ms: probe.duration_ms,
                dispatched_at: Some(probe.dispatched_at),
                deadline_at: Some(probe.deadline_at),
                resolved_at: probe.resolved_at,
                point_delta: probe.point_delta.unwrap_or(0),
                updated_at: probe.updated_at.unwrap_or(probe.dispatched_at),
                result: probe.outcome.as_ref().map(|outcome| PlayerProbeResult {
                    status: match outcome.as_str() {
                        "pass" => "passed".to_string(),
                        "error" => "failed".to_string(),
                        "no_response" => "no_response".to_string(),
                        _ => "pending".to_string(),
                    },
                    expected: suppressed_expected.clone(),
                    actual: probe.output.clone(),
                }),
            }
        })
        .collect();

    let result_rows = task_results::Entity::find()
        .filter(task_results::Column::SessionIdFk.eq(session_id))
        .filter(task_results::Column::PlayerIdFk.eq(player_id))
        .order_by_desc(task_results::Column::CreatedAt)
        .all(db)
        .await
        .unwrap_or_default();
    let mut latest_result_by_task: HashMap<Uuid, task_results::Model> = HashMap::new();
    let mut points_by_task: HashMap<Uuid, i64> = HashMap::new();
    let mut bonus_by_task: HashMap<Uuid, i64> = HashMap::new();
    for row in result_rows {
        let Some(task_id) = row.task_id else {
            continue;
        };
        *points_by_task.entry(task_id).or_insert(0) += row.point_delta as i64;
        if row.is_bonus {
            *bonus_by_task.entry(task_id).or_insert(0) += row.point_delta as i64;
        }
        latest_result_by_task.entry(task_id).or_insert(row);
    }

    let sched_row = session_scheduler_state::Entity::find()
        .filter(session_scheduler_state::Column::PlayerIdFk.eq(player_id))
        .filter(session_scheduler_state::Column::SessionIdFk.eq(session_id))
        .one(db)
        .await
        .ok()
        .flatten();
    let current_task_id = sched_row.as_ref().and_then(|row| row.task_id);
    let current_ordinal = current_task_id
        .and_then(|tid| ordinal_by_task.get(&tid).copied())
        .unwrap_or(i32::MAX);

    // Persisted judge verdicts for this player, resolved to task + judge
    // identity via task_judges/judges. Best-effort: failures yield an empty
    // list without affecting the rest of the snapshot. Computed before the
    // task entries so judge deltas fold into each task's total_points.
    let judge_result_rows = judge_results::Entity::find()
        .filter(judge_results::Column::SessionIdFk.eq(session_id))
        .filter(judge_results::Column::PlayerIdFk.eq(player_id))
        .order_by_asc(judge_results::Column::CreatedAt)
        .all(db)
        .await
        .unwrap_or_default();
    let task_judge_rows = task_judges::Entity::find()
        .all(db)
        .await
        .unwrap_or_default();
    let judge_rows = judges::Entity::find().all(db).await.unwrap_or_default();
    let task_judge_by_id: HashMap<Uuid, &task_judges::Model> =
        task_judge_rows.iter().map(|tj| (tj.id, tj)).collect();
    let judge_by_id: HashMap<Uuid, &judges::Model> = judge_rows.iter().map(|j| (j.id, j)).collect();
    // Only "scored" rows are verdicts; "running"/"failed" rows carry no
    // rating and must not fold into task points.
    // The session report is a debrief, not a verdict: it is lifted out of the
    // verdict list so it renders as prose on its own, instead of appearing as
    // one more judge card on whichever task it happened to be attached to.
    let session_report = judge_result_rows
        .iter()
        .filter(|r| r.status == "scored")
        .filter_map(|r| {
            let tj = task_judge_by_id.get(&r.task_judge_id)?;
            let judge = judge_by_id.get(&tj.judge_id)?;
            if judge.kind != arena_core::judging::JUDGE_KIND_REPORT {
                return None;
            }
            Some(arena_core::protocol::player::PlayerSessionReport {
                judge_name: judge.name.clone(),
                judge_slug: judge.slug.clone(),
                markdown: r.feedback.clone(),
                document: arena_core::judging::report::parse_report_doc(&r.feedback),
                created_at: r.created_at,
            })
        })
        .next_back();

    let judge_result_entries: Vec<PlayerJudgeScoredPayload> = judge_result_rows
        .iter()
        .filter(|r| r.status == "scored")
        .filter_map(|r| {
            let tj = task_judge_by_id.get(&r.task_judge_id)?;
            let judge = judge_by_id.get(&tj.judge_id)?;
            if judge.kind == arena_core::judging::JUDGE_KIND_REPORT {
                return None;
            }
            Some(PlayerJudgeScoredPayload {
                task_id: tj.task_id,
                judge_slug: judge.slug.clone(),
                judge_name: judge.name.clone(),
                rating: arena_core::judging::rating_scalar(&r.rating),
                feedback: r.feedback.clone(),
                point_delta: r.point_delta,
                created_at: r.created_at,
                duration_ms: r.duration_ms,
            })
        })
        .collect();
    for j in &judge_result_entries {
        *points_by_task.entry(j.task_id).or_insert(0) += j.point_delta as i64;
    }

    // Lifecycle of every judge attached to a revealed task: attachments with
    // no judge_results row are "pending"; otherwise the row's status
    // ("running" | "scored" | "failed") is authoritative.
    let result_by_task_judge: HashMap<Uuid, &judge_results::Model> = judge_result_rows
        .iter()
        .map(|r| (r.task_judge_id, r))
        .collect();
    let mut judge_status_entries: Vec<arena_core::protocol::PlayerJudgeStatusPayload> =
        task_judge_rows
            .iter()
            .filter(|tj| {
                ordinal_by_task
                    .get(&tj.task_id)
                    .is_some_and(|ord| *ord <= current_ordinal)
            })
            // A judge attached to the task after the session finished never
            // ran here (same reasoning as the task filter above) — without
            // this, a later judge-panel rollout freezes every earlier
            // session's page on "Judges scoring — N left".
            .filter(|tj| match session.as_ref().and_then(|s| s.finished_at) {
                Some(finished) => tj.created_at <= finished,
                None => true,
            })
            .filter_map(|tj| {
                let judge = judge_by_id.get(&tj.judge_id)?;
                let row = result_by_task_judge.get(&tj.id);
                Some(arena_core::protocol::PlayerJudgeStatusPayload {
                    task_id: tj.task_id,
                    judge_slug: judge.slug.clone(),
                    judge_name: judge.name.clone(),
                    status: row
                        .map(|r| r.status.clone())
                        .unwrap_or_else(|| "pending".to_string()),
                    // Public surface: never leak the raw provider/system
                    // error — admins fetch it via the judge-result details
                    // endpoint (keyed by judge_result_id). One exception:
                    // a quota denial is the player's own account state, and
                    // hiding it left a whole session of failed judges with
                    // no visible reason (KN5JHB).
                    error: row.and_then(|r| {
                        r.error.as_ref().map(|e| {
                            if arena_core::judging::JudgeError::message_is_quota_denial(e) {
                                "Monthly judge-run limit reached for this account — reviews \
                                 resume next month or when the account's limit is raised."
                                    .to_string()
                            } else {
                                "Judge evaluation failed. A session operator can view the details."
                                    .to_string()
                            }
                        })
                    }),
                    updated_at: row.map(|r| r.updated_at),
                    judge_result_id: row.map(|r| r.id),
                })
            })
            .collect();
    judge_status_entries.sort_by_key(|s| {
        (
            ordinal_by_task.get(&s.task_id).copied().unwrap_or(i32::MAX),
            s.judge_slug.clone(),
        )
    });

    let task_entries: Vec<PlayerTaskSummaryEntry> = task_rows
        .iter()
        .filter(|task| task.ordinal <= current_ordinal)
        .map(|task| {
            let adapted_content = snippet_by_task.get(&task.id).cloned().unwrap_or_default();

            let result = latest_result_by_task
                .get(&task.id)
                .map(|r| PlayerTaskResult {
                    status: if r.point_delta > 0 {
                        "completed".to_string()
                    } else if task.evaluation.is_some() && current_task_id == Some(task.id) {
                        // An open-ended task the scheduler still holds: a
                        // zero-point completion poll is progress, not failure
                        // (the page rendered it as "attempt failed — retry
                        // scheduled" to a player who had done nothing wrong).
                        // Once the scheduler marks the row `judging`, the
                        // completion contract has been accepted and only the
                        // judge verdicts are outstanding — that is completed.
                        if sched_row.as_ref().is_some_and(|s| {
                            s.state == arena_core::session_completion::SCHEDULER_STATE_JUDGING
                        }) {
                            "completed".to_string()
                        } else {
                            "pending".to_string()
                        }
                    } else {
                        "failed".to_string()
                    },
                    submitted_answer: Some(r.answer.clone()),
                    correct_answer: None,
                    score_delta: r.point_delta,
                    evaluated_at: Some(r.created_at),
                })
                .or_else(|| {
                    latest_probe_by_task.get(&task.id).and_then(|probe| {
                        probe.outcome.as_ref().map(|outcome| PlayerTaskResult {
                            status: match outcome.as_str() {
                                "pass" => "completed".to_string(),
                                // An open-ended task is completed by its
                                // contract, not by its latest probe: while it
                                // runs, a non-passing measurement is progress,
                                // not failure.
                                "error" if task.evaluation.is_some() => "pending".to_string(),
                                "error" => "failed".to_string(),
                                "no_response" => "no_response".to_string(),
                                _ => "pending".to_string(),
                            },
                            submitted_answer: probe.output.clone(),
                            correct_answer: probe.expected_answer.clone(),
                            score_delta: probe.point_delta.unwrap_or(0),
                            evaluated_at: probe.resolved_at,
                        })
                    })
                });

            let scheduler_state = if current_task_id == Some(task.id) {
                sched_row.as_ref().map(|s| PlayerSchedulerState {
                    state: s.state.clone(),
                    activated_at: None,
                    deadline_at: latest_probe_by_task.get(&task.id).map(|p| p.deadline_at),
                })
            } else {
                None
            };

            PlayerTaskSummaryEntry {
                task_id: task.id,
                ordinal: task.ordinal,
                title: task.title.clone(),
                content: task.content.clone(),
                tags: serde_json::from_str::<Vec<String>>(&task.tags).unwrap_or_default(),
                adapted_content,
                result,
                scheduler_state,
                total_points: points_by_task.get(&task.id).copied().unwrap_or(0),
                bonus_points: bonus_by_task.get(&task.id).copied().unwrap_or(0),
            }
        })
        .collect();

    // Score/rank: prefer the live registry cache (kept fresh by probe and
    // judge events), but recompute from the DB when the cache can't answer.
    // Finished sessions are not hydrated into the registry at startup, so
    // after a server restart the cache-only read served the (0, 0) sentinel
    // — score 0 and "rank #0" — while the session report page computed the
    // real leaderboard from the DB.
    let cached_score_rank = session_registry
        .iter()
        .find(|e| {
            let cache = e.cache.read().unwrap_or_else(|e2| e2.into_inner());
            cache.session_id == session_id
        })
        .map(|entry| {
            let cache = entry.cache.read().unwrap_or_else(|e2| e2.into_inner());
            arena_core::scoring::read_score_rank(&cache.leaderboard, player_id)
        });
    let (score, rank) = match cached_score_rank {
        Some((score, rank)) if rank > 0 => (score, rank),
        _ => crate::scoring::compute_leaderboard(db, session_id)
            .await
            .map(|entries| arena_core::scoring::read_score_rank(&entries, player_id))
            .unwrap_or((0, 0)),
    };

    let next_probe_at = sched_row.as_ref().and_then(|s| s.next_probe_at);

    // Derived completion status — this is a single-player detail path, so
    // the judge-aware aggregate is computed here (one batched call per
    // snapshot; list endpoints never compute it). Best-effort: a failure or
    // a missing entry (revoked player) yields None, which is skip-serialized.
    let completion_status = arena_core::session_completion::session_player_statuses(db, session_id)
        .await
        .ok()
        .and_then(|statuses| {
            statuses
                .iter()
                .find(|s| s.player_id == player_id)
                .map(|s| s.status)
        });

    let (session_started_at, session_ends_at) = match &session {
        Some(s) => {
            let started = s.started_at;
            let ends = s.started_at.map(|st| {
                let mut base = st + chrono::Duration::seconds(session_duration_secs);
                if let Some(paused_secs) = s.paused_duration_secs {
                    base += chrono::Duration::seconds(paused_secs);
                }
                if let Some(paused_at) = s.paused_at {
                    base +=
                        chrono::Duration::seconds((Utc::now() - paused_at).num_seconds().max(0));
                }
                base
            });
            (started, ends)
        }
        None => (None, None),
    };

    let agent_connected = player_row.as_ref().map(|p| p.agent_connected);

    // Open-ended evaluation blocks: contract criteria + panel scores +
    // latest TODO report + pending artifact requests + measurement history.
    let evaluations = build_evaluations(
        db,
        &task_rows,
        &adapted_tests_rows,
        &probe_rows,
        player_id,
        session_id,
    )
    .await;

    // The copy/paste validation's result: the persisted report names the
    // percentage and the matched sources (clean runs included); sessions
    // scanned before the reports table fall back to the penalty row's note.
    let similarity_adjustment = match arena_core::entities::similarity_reports::Entity::find()
        .filter(arena_core::entities::similarity_reports::Column::SessionIdFk.eq(session_id))
        .filter(arena_core::entities::similarity_reports::Column::PlayerIdFk.eq(player_id))
        .one(db)
        .await
        .ok()
        .flatten()
    {
        Some(row) => {
            // A passed check names nobody: under the threshold the overlap
            // is honest convergence, and pointing at specific sessions and
            // players would read as an accusation. Sources are shown only
            // when a penalty was actually charged.
            let sources: Vec<arena_core::protocol::PlayerSimilaritySource> = if row.penalty < 0 {
                serde_json::from_value(row.sources_json.clone()).unwrap_or_default()
            } else {
                Vec::new()
            };
            let note = match sources.first() {
                Some(src) if src.player == *display_name => format!(
                    "{:.0}% of the delivered code matches your own code from session {}",
                    row.duplicated_pct, src.join_code
                ),
                Some(src) => format!(
                    "{:.0}% of the delivered code matches {}'s code from session {}",
                    row.duplicated_pct, src.player, src.join_code
                ),
                None if row.duplicated_pct > 0.0 => format!(
                    "similarity check passed: {:.0}% overlap with {} earlier session repos, \
                     under the penalty threshold",
                    row.duplicated_pct, row.corpus_repos
                ),
                None => format!(
                    "no copy/paste found across {} earlier session repos",
                    row.corpus_repos
                ),
            };
            Some(arena_core::protocol::PlayerSimilarityAdjustment {
                note,
                point_delta: row.penalty as i32,
                duplicated_pct: row.duplicated_pct,
                sources,
            })
        }
        None => arena_core::entities::task_results::Entity::find()
            .filter(arena_core::entities::task_results::Column::SessionIdFk.eq(session_id))
            .filter(arena_core::entities::task_results::Column::PlayerIdFk.eq(player_id))
            .filter(arena_core::entities::task_results::Column::TaskId.is_null())
            .filter(arena_core::entities::task_results::Column::IsBonus.eq(true))
            .all(db)
            .await
            .unwrap_or_default()
            .into_iter()
            .find(|r| r.answer.starts_with("similarity-penalty"))
            .map(|r| arena_core::protocol::PlayerSimilarityAdjustment {
                note: r
                    .answer
                    .split_once(": ")
                    .map(|(_, rest)| rest.to_string())
                    .unwrap_or(r.answer),
                point_delta: r.point_delta,
                duplicated_pct: 0.0,
                sources: Vec::new(),
            }),
    };

    PlayerSnapshotData {
        player_id,
        display_name: display_name.to_string(),
        avatar_url,
        agent_display_name,
        score,
        rank,
        last_seq,
        probes: probe_entries,
        tasks: task_entries,
        total_tasks: task_rows.len(),
        next_probe_at,
        session_started_at,
        session_ends_at,
        session_status: session
            .as_ref()
            .map(|s| s.status)
            .unwrap_or(arena_core::session_status::SessionStatus::Lobby),
        agent_connected,
        completion_status,
        judge_results: judge_result_entries,
        judge_statuses: judge_status_entries,
        session_report,
        evaluations,
        similarity_adjustment,
    }
    .into()
}

/// The open-ended evaluation state of every open-ended task in the project.
async fn build_evaluations(
    db: &sea_orm::DatabaseConnection,
    task_rows: &[tasks::Model],
    adapted_tests_rows: &[arena_core::entities::tests::Model],
    probe_rows: &[probes::Model],
    player_id: Uuid,
    session_id: Uuid,
) -> Vec<arena_core::protocol::PlayerTaskEvaluation> {
    use arena_core::evaluation::{EvaluationContract, ProbeConfig, ProbeMode};
    use arena_core::protocol::{
        PlayerArtifactRef, PlayerArtifactRequest, PlayerCriterionScore, PlayerCriterionState,
        PlayerMeasurementPoint, PlayerTaskEvaluation,
    };

    let mut out = Vec::new();
    for task in task_rows {
        let Some(evaluation) = &task.evaluation else {
            continue;
        };
        let Ok(contract) = EvaluationContract::from_json(evaluation) else {
            continue;
        };

        // The panel's criteria scores, read out of the stored sheets.
        let attached = arena_core::entities::task_judges::Entity::find()
            .filter(arena_core::entities::task_judges::Column::TaskId.eq(task.id))
            .all(db)
            .await
            .unwrap_or_default();
        let mut judge_slug_by_id: HashMap<Uuid, String> = HashMap::new();
        let judge_slug_by_tj: HashMap<Uuid, String> = {
            let mut m = HashMap::new();
            for tj in &attached {
                if let Ok(Some(j)) = judges::Entity::find_by_id(tj.judge_id).one(db).await {
                    judge_slug_by_id.insert(j.id, j.slug.clone());
                    m.insert(tj.id, j.slug);
                }
            }
            m
        };
        let verdicts = judge_results::Entity::find()
            .filter(judge_results::Column::TaskJudgeId.is_in(attached.iter().map(|tj| tj.id)))
            .filter(judge_results::Column::PlayerIdFk.eq(player_id))
            .filter(judge_results::Column::Status.eq("scored"))
            .all(db)
            .await
            .unwrap_or_default();

        let criteria = contract
            .criteria
            .iter()
            .map(|c| {
                let mut scores = Vec::new();
                for verdict in &verdicts {
                    let Some(sheet) = verdict.rating.get("criteria").and_then(|v| v.as_array())
                    else {
                        continue;
                    };
                    for row in sheet {
                        if row["key"].as_str().map(str::trim) != Some(c.key.trim()) {
                            continue;
                        }
                        scores.push(PlayerCriterionScore {
                            judge_slug: judge_slug_by_tj
                                .get(&verdict.task_judge_id)
                                .cloned()
                                .unwrap_or_default(),
                            score: row["score"].as_f64(),
                            rationale: row["rationale"].as_str().unwrap_or("").to_string(),
                        });
                    }
                }
                PlayerCriterionState {
                    key: c.key.clone(),
                    title: if c.title.is_empty() {
                        c.key.clone()
                    } else {
                        c.title.clone()
                    },
                    weight: c.weight,
                    scores,
                }
            })
            .collect();

        // Config-carrying tests of this task, for todo/measurements/artifacts.
        let mut todo: Option<(chrono::DateTime<Utc>, serde_json::Value)> = None;
        let mut pending_artifacts = Vec::new();
        let mut artifacts = Vec::new();
        let mut measurements = Vec::new();
        for test in adapted_tests_rows.iter().filter(|t| t.task_id == task.id) {
            let Some(config) = test
                .probe_config
                .as_ref()
                .and_then(|c| ProbeConfig::from_json(c).ok())
            else {
                continue;
            };
            for probe in probe_rows.iter().filter(|p| p.test_id == test.id) {
                if let Some(rj) = &probe.result_json {
                    if let Some(t) = rj.get("todo")
                        && todo
                            .as_ref()
                            .is_none_or(|(at, _)| probe.dispatched_at > *at)
                    {
                        todo = Some((probe.dispatched_at, t.clone()));
                    }
                    measurements.push(PlayerMeasurementPoint {
                        test_ordinal: test.ordinal,
                        label: test.prompt.clone(),
                        at: probe.dispatched_at,
                        outcome: probe.outcome.clone(),
                        result_json: rj.clone(),
                    });
                }
                if config.mode == ProbeMode::Interactive && probe.outcome.is_none() {
                    pending_artifacts.push(PlayerArtifactRequest {
                        probe_id: probe.id,
                        instruction: config.instruction.clone().unwrap_or_default(),
                        path: config
                            .artifact
                            .as_ref()
                            .and_then(|a| a.path.clone())
                            .unwrap_or_default(),
                        deadline_at: probe.deadline_at,
                    });
                }
                if config.mode == ProbeMode::Interactive
                    && let Some(reference) = &probe.artifact_path
                {
                    // Every delivered file, by repo path, in `?i=N` order —
                    // the gallery captions each one by its own name.
                    let files: Vec<String> = probe
                        .result_json
                        .as_ref()
                        .and_then(|r| r.get("artifact")?.get("files")?.as_array().cloned())
                        .unwrap_or_default()
                        .iter()
                        .filter_map(|f| Some(f.get("path")?.as_str()?.to_string()))
                        .collect();
                    let file_count = if files.is_empty() {
                        1
                    } else {
                        files.len() as u32
                    };
                    let label = reference
                        .split_once(':')
                        .map(|(_, p)| p.to_string())
                        .unwrap_or_else(|| reference.clone());
                    artifacts.push(PlayerArtifactRef {
                        probe_id: probe.id,
                        // The delivered file decides how the gallery renders
                        // (a video/webm request answered with a .gif must not
                        // end up in <video>); the declared type is only the
                        // fallback for unrecognized extensions.
                        content_type: arena_core::evaluation::artifact_content_type_for_path(
                            &label,
                        )
                        .map(str::to_string)
                        .or_else(|| config.artifact.as_ref().map(|a| a.content_type.clone()))
                        .unwrap_or_else(|| "application/octet-stream".to_string()),
                        label,
                        judge_slug: test
                            .registered_by_judge_id
                            .and_then(|id| judge_slug_by_id.get(&id).cloned()),
                        file_count,
                        files,
                    });
                }
            }
        }
        measurements.sort_by_key(|m| m.at);

        // The work window, when the task has started for this player.
        let deadline_at = arena_core::entities::activity_event::Entity::find()
            .filter(arena_core::entities::activity_event::Column::SessionIdFk.eq(session_id))
            .filter(arena_core::entities::activity_event::Column::PlayerIdFk.eq(player_id))
            .filter(arena_core::entities::activity_event::Column::TaskIdFk.eq(task.id))
            .filter(arena_core::entities::activity_event::Column::EventKind.eq("task_started"))
            .order_by_asc(arena_core::entities::activity_event::Column::Timestamp)
            .one(db)
            .await
            .ok()
            .flatten()
            .map(|e| e.timestamp + chrono::Duration::seconds(contract.completion.deadline_secs));

        out.push(PlayerTaskEvaluation {
            task_id: task.id,
            criteria,
            todo: todo.map(|(_, t)| t),
            pending_artifacts,
            deadline_at,
            measurements,
            artifacts,
            repo_images: Vec::new(),
        });
    }

    // Committed screenshots: participants often save captures into the repo
    // on their own initiative (not through a judge's request). Those belong
    // in the gallery too — one HEAD scan, shared by every open-ended task.
    if !out.is_empty() {
        let images = repo_image_paths(session_id, player_id).await;
        for ev in &mut out {
            ev.repo_images = images.clone();
        }
    }
    out
}

/// Image files at the player's repo HEAD, screenshot-looking paths first,
/// capped. Empty on any failure — the gallery just shows less.
async fn repo_image_paths(session_id: Uuid, player_id: Uuid) -> Vec<String> {
    const MAX_IMAGES: usize = 12;
    let Some(base) = arena_core::git_store::repos_base_dir() else {
        return Vec::new();
    };
    let repo_dir = arena_core::git_store::player_repo_path(&base, session_id, player_id);
    tokio::task::spawn_blocking(move || {
        let git_bin = which::which("git").ok()?;
        let out = std::process::Command::new(git_bin)
            .arg("-C")
            .arg(&repo_dir)
            .arg("ls-tree")
            .arg("-r")
            .arg("HEAD")
            .arg("--name-only")
            .output()
            .ok()?;
        if !out.status.success() {
            return None;
        }
        let mut paths: Vec<String> = String::from_utf8_lossy(&out.stdout)
            .lines()
            .filter(|p| {
                let lower = p.to_ascii_lowercase();
                [".png", ".jpg", ".jpeg", ".webp", ".gif"]
                    .iter()
                    .any(|ext| lower.ends_with(ext))
                    && !lower.contains("node_modules/")
                    // Probe-delivered files already reach the gallery as
                    // artifact refs; listing them here doubles every capture.
                    && !lower.starts_with(".ololo/artifacts/")
            })
            .map(str::to_string)
            .collect();
        // Screenshot-looking paths first, then stable order.
        paths.sort_by_key(|p| {
            let lower = p.to_ascii_lowercase();
            let looks = lower.contains("screenshot") || lower.contains(".ololo/");
            (!looks, p.clone())
        });
        paths.truncate(MAX_IMAGES);
        Some(paths)
    })
    .await
    .ok()
    .flatten()
    .unwrap_or_default()
}

#[cfg(test)]
mod tests;
