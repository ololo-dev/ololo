//! Code-health checkpoints on the game server.
//!
//! The ololo agent commits the tree at every probe, scores it and reports
//! (`HealthReport`); the server stores the report as a checkpoint and then
//! scores the same commit itself from the player's pushed repository. The
//! server's number is authoritative, the client's advisory: same crate,
//! same version, same tree ⇒ same score, and any disagreement is flagged
//! rather than argued. When a task closes, the task's final commit
//! (`feat(<task>)`) becomes a checkpoint of its own — the one the health
//! bonus is paid on, so the bonus never depends on the client at all.
//!
//! Verification runs as its own task, never on the probe path: wait for
//! the commit to land (pushes are asynchronous on the client), export it
//! exactly (`read-tree` + `checkout-index`, immune to `export-ignore`),
//! scan under the server-probe concurrency cap, derive the commit's task
//! from the commit messages, store, publish. A restart re-drives what was
//! left `pending`.

use std::path::PathBuf;
use std::time::Duration;

use arena_core::entities::{
    health_checkpoints, probes, session_scheduler_state, sessions, task_results, tasks,
};
use arena_core::health::{
    cap_result, checkpoint_view, commit_exists, composed_score, counted_run_at, export_commit,
    first_parent_log,
};
use arena_core::health_settings::HealthSettings;
use arena_core::protocol::{
    HealthCheckStatus, HealthCheckpointKind, HealthCheckpointView, HealthProbeConfig,
    HealthReportPayload, HealthReportStatus, PushState, ZmqEvent,
};
use arena_core::snapshot_message::task_ranges;
use chrono::Utc;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter, QueryOrder, Set,
};
use uuid::Uuid;

use crate::state::GameServerState;

/// How long the verifier waits for a reported commit to reach the repo.
/// Pushes retry with backoff on the client for up to ~a minute.
const COMMIT_WAIT: Duration = Duration::from_secs(150);
const COMMIT_POLL: Duration = Duration::from_secs(5);
/// How long the task-final verifier waits for the `feat` commit: the agent
/// writes it when it sees the scheduler move on, then pushes.
const TASK_FINAL_WAIT: Duration = Duration::from_secs(240);
/// Commits of the first-parent log read to attribute a commit to a task.
const LOG_LIMIT: usize = 4_000;
/// `commit_sha` of a checkpoint whose report named no commit.
fn no_commit_key(probe_id: Uuid) -> String {
    format!("none:{probe_id}")
}

/// What a `TestPush` needs: the probe's position among the player's probes
/// and, when health is tracked, the client's budget.
pub async fn probe_dispatch_info(
    state: &GameServerState,
    session_id: Uuid,
    player_id: Uuid,
) -> (u32, Option<HealthProbeConfig>) {
    let seq = probes::Entity::find()
        .filter(probes::Column::SessionId.eq(session_id))
        .filter(probes::Column::PlayerId.eq(player_id))
        .count(&state.db)
        .await
        .ok()
        .and_then(|n| u32::try_from(n).ok())
        .unwrap_or(0);
    let settings = HealthSettings::load(&state.db).await.unwrap_or_default();
    let config = settings.enabled.then(|| HealthProbeConfig {
        timeout_secs: u32::try_from(settings.timeout.as_secs()).unwrap_or(u32::MAX),
    });
    (seq, config)
}

/// The agent reported its analysis of a probe commit: store the client side
/// of the checkpoint, tell the dashboards, verify in the background.
pub async fn on_report(
    state: GameServerState,
    session_id: Uuid,
    player_id: Uuid,
    join_code: String,
    report: HealthReportPayload,
) {
    let settings = HealthSettings::load(&state.db).await.unwrap_or_default();
    if !settings.enabled {
        tracing::debug!(session_id = %session_id, player_id = %player_id, "health report ignored: health is off");
        return;
    }
    // Work on existing code — a personal project, or a project's own
    // repository cloned before the start — begins from a tree that predates
    // the session: score it once, early, so the chart shows where the player
    // began and the first task's bonus has something to be measured against.
    if arena_core::project_repo::session_starts_from_existing_code(&state.db, session_id)
        .await
        .unwrap_or(false)
    {
        let baseline_state = state.clone();
        let baseline_code = join_code.clone();
        tokio::spawn(async move {
            baseline(&baseline_state, session_id, player_id, &baseline_code).await;
        });
    }
    let now = Utc::now();
    let commit_sha = report
        .commit
        .clone()
        .unwrap_or_else(|| no_commit_key(report.probe_id));
    // The probe row is the source of the task when the report has none —
    // and a report naming a probe this server never dispatched is still
    // stored (as a checkpoint without a probe link), never dropped.
    let probe_row = probes::Entity::find_by_id(report.probe_id)
        .one(&state.db)
        .await
        .ok()
        .flatten();
    if probe_row.is_none() {
        tracing::warn!(session_id = %session_id, player_id = %player_id, probe_id = %report.probe_id, "health report names an unknown probe");
    }
    let probe_task = match report.task_id {
        Some(t) => Some(t),
        None => match &probe_row {
            Some(p) => probe_task_id(&state, p).await,
            None => None,
        },
    };
    let probe_fk = probe_row.as_ref().map(|p| p.id);
    let late = is_late(&state, session_id, player_id, probe_task).await;
    let rewritten = report.push.state == PushState::RejectedNonFastForward;

    let existing = health_checkpoints::Entity::find()
        .filter(health_checkpoints::Column::SessionIdFk.eq(session_id))
        .filter(health_checkpoints::Column::PlayerIdFk.eq(player_id))
        .filter(health_checkpoints::Column::CommitSha.eq(commit_sha.clone()))
        .one(&state.db)
        .await
        .ok()
        .flatten();

    let (client_score, client_grade, client_result) = match &report.result {
        Some(result) => (
            result.score,
            result.grade.map(|g| g.to_string()),
            Some(cap_result(result)),
        ),
        None => (None, None, None),
    };
    let client_error = report.error.clone().or_else(|| {
        (report.push.state != PushState::Pushed)
            .then(|| report.push.error.clone())
            .flatten()
    });
    let version_mismatch = report.jscpd_version != ololo_health::JSCPD_CORE_VERSION
        || report.health_schema != ololo_health::HEALTH_SCHEMA;
    // A commit the client could not make, or one it knows the server will
    // never fast-forward to, has nothing to verify.
    let server_status = if report.commit.is_none() {
        HealthCheckStatus::Failed
    } else if rewritten {
        HealthCheckStatus::Unverified
    } else {
        HealthCheckStatus::Pending
    };
    let server_error = match server_status {
        HealthCheckStatus::Failed => Some("the client made no commit to verify".to_string()),
        HealthCheckStatus::Unverified => {
            Some("the client's push was rejected: the commit is not on the served line".to_string())
        }
        _ => None,
    };

    let id = match existing {
        Some(row) => {
            let id = row.id;
            let mut am: health_checkpoints::ActiveModel = row.into();
            am.client_status = Set(Some(report.status.as_str().to_string()));
            am.client_score = Set(client_score);
            am.client_grade = Set(client_grade);
            am.client_result = Set(client_result);
            am.client_duration_ms =
                Set(Some(i64::try_from(report.duration_ms).unwrap_or(i64::MAX)));
            am.client_jscpd_version = Set(Some(report.jscpd_version.clone()));
            am.client_error = Set(client_error);
            am.client_reported_at = Set(Some(now));
            am.version_mismatch = Set(version_mismatch);
            am.history_rewritten = Set(rewritten);
            am.updated_at = Set(now);
            if let Err(e) = am.update(&state.db).await {
                tracing::warn!(error = %e, "health: checkpoint update failed");
                return;
            }
            id
        }
        None => {
            let id = Uuid::new_v4();
            let am = health_checkpoints::ActiveModel {
                id: Set(id),
                session_id_fk: Set(session_id),
                player_id_fk: Set(player_id),
                task_id_fk: Set(probe_task),
                probe_id_fk: Set(probe_fk),
                kind: Set(HealthCheckpointKind::Probe.as_str().to_string()),
                probe_seq: Set(i32::try_from(report.probe_seq).unwrap_or(i32::MAX)),
                commit_sha: Set(commit_sha),
                derived_task_id: Set(None),
                score_mismatch: Set(false),
                version_mismatch: Set(version_mismatch),
                task_mismatch: Set(false),
                late: Set(late),
                history_rewritten: Set(rewritten),
                client_status: Set(Some(report.status.as_str().to_string())),
                client_score: Set(client_score),
                client_grade: Set(client_grade),
                client_result: Set(client_result),
                client_duration_ms: Set(Some(
                    i64::try_from(report.duration_ms).unwrap_or(i64::MAX),
                )),
                client_jscpd_version: Set(Some(report.jscpd_version.clone())),
                client_error: Set(client_error),
                client_reported_at: Set(Some(now)),
                server_status: Set(server_status.as_str().to_string()),
                server_score: Set(None),
                server_grade: Set(None),
                server_result: Set(None),
                server_duration_ms: Set(None),
                server_jscpd_version: Set(None),
                server_error: Set(server_error),
                server_verified_at: Set(None),
                tests_status: Set(None),
                tests_result: Set(None),
                tests_reported_at: Set(None),
                created_at: Set(now),
                updated_at: Set(now),
            };
            if let Err(e) = am.insert(&state.db).await {
                tracing::warn!(error = %e, "health: checkpoint insert failed");
                return;
            }
            id
        }
    };

    log_checkpoint(&state, session_id, player_id, id, "health_report", &report).await;
    publish(&state, session_id, &join_code, id, &settings).await;
    // Every report is a chance the player's docs changed what runs their
    // tests; reading them is cheap when they did not.
    if settings.tests_on() {
        tokio::spawn(crate::health_tests::refresh(
            state.clone(),
            session_id,
            player_id,
        ));
    }
    if server_status == HealthCheckStatus::Pending {
        let verifier = state.clone();
        tokio::spawn(async move {
            verify(verifier, id, join_code, COMMIT_WAIT).await;
        });
    }
}

/// A task closed for the player: its final tree gets a checkpoint of its
/// own, verified from the `feat(<task>)` commit the agent pushes.
pub async fn on_task_closed(
    state: GameServerState,
    session_id: Uuid,
    player_id: Uuid,
    join_code: String,
    task: tasks::Model,
) {
    let settings = HealthSettings::load(&state.db).await.unwrap_or_default();
    if !settings.enabled {
        return;
    }
    tokio::spawn(async move {
        let repo_dir = repo_dir(session_id, player_id);
        let deadline = tokio::time::Instant::now() + TASK_FINAL_WAIT;
        let sha = loop {
            match arena_core::judging::task_commit::resolve_task_commit(&repo_dir, task.id).await {
                Ok(Some((sha, _subject))) => break Some(sha),
                Ok(None) => {}
                Err(e) => {
                    tracing::debug!(error = %e, "health: task-final commit lookup failed; retrying");
                }
            }
            if tokio::time::Instant::now() >= deadline {
                break None;
            }
            tokio::time::sleep(COMMIT_POLL).await;
        };
        let now = Utc::now();
        let (commit_sha, status, error) = match sha {
            Some(sha) => (sha, HealthCheckStatus::Pending, None),
            None => (
                format!("none:task-final:{}", task.id),
                HealthCheckStatus::CommitMissing,
                Some("the task's final snapshot never reached the server".to_string()),
            ),
        };
        let existing = health_checkpoints::Entity::find()
            .filter(health_checkpoints::Column::SessionIdFk.eq(session_id))
            .filter(health_checkpoints::Column::PlayerIdFk.eq(player_id))
            .filter(health_checkpoints::Column::CommitSha.eq(commit_sha.clone()))
            .one(&state.db)
            .await
            .ok()
            .flatten();
        let id = match existing {
            // The feat commit was already reported as a probe commit?
            // Not possible (probe commits are their own kind), but a
            // repeated close (reconnect) must not duplicate the row.
            Some(row) => {
                if row.kind != HealthCheckpointKind::TaskFinal.as_str() {
                    let mut am: health_checkpoints::ActiveModel = row.clone().into();
                    am.kind = Set(HealthCheckpointKind::TaskFinal.as_str().to_string());
                    am.updated_at = Set(now);
                    let _ = am.update(&state.db).await;
                }
                row.id
            }
            None => {
                let id = Uuid::new_v4();
                let am = health_checkpoints::ActiveModel {
                    id: Set(id),
                    session_id_fk: Set(session_id),
                    player_id_fk: Set(player_id),
                    task_id_fk: Set(Some(task.id)),
                    probe_id_fk: Set(None),
                    kind: Set(HealthCheckpointKind::TaskFinal.as_str().to_string()),
                    probe_seq: Set(0),
                    commit_sha: Set(commit_sha),
                    derived_task_id: Set(None),
                    score_mismatch: Set(false),
                    version_mismatch: Set(false),
                    task_mismatch: Set(false),
                    late: Set(false),
                    history_rewritten: Set(false),
                    client_status: Set(None),
                    client_score: Set(None),
                    client_grade: Set(None),
                    client_result: Set(None),
                    client_duration_ms: Set(None),
                    client_jscpd_version: Set(None),
                    client_error: Set(None),
                    client_reported_at: Set(None),
                    server_status: Set(status.as_str().to_string()),
                    server_score: Set(None),
                    server_grade: Set(None),
                    server_result: Set(None),
                    server_duration_ms: Set(None),
                    server_jscpd_version: Set(None),
                    server_error: Set(error),
                    server_verified_at: Set(None),
                    tests_status: Set(None),
                    tests_result: Set(None),
                    tests_reported_at: Set(None),
                    created_at: Set(now),
                    updated_at: Set(now),
                };
                if let Err(e) = am.insert(&state.db).await {
                    tracing::warn!(error = %e, "health: task-final checkpoint insert failed");
                    return;
                }
                id
            }
        };
        publish(&state, session_id, &join_code, id, &settings).await;
        if status == HealthCheckStatus::Pending {
            verify(state.clone(), id, join_code.clone(), Duration::ZERO).await;
        }
        award_health_bonus(&state, session_id, player_id, &task, &join_code, &settings).await;
    });
}

/// Pay the task's health bonus once, from the server-verified numbers:
/// the task-final checkpoint when it verified, else the last verified probe
/// checkpoint of the task (never a late one), else nothing — with the
/// reason on the row so the breakdown can say why. Tasks that pay no
/// health points get no row at all.
pub async fn award_health_bonus(
    state: &GameServerState,
    session_id: Uuid,
    player_id: Uuid,
    task: &tasks::Model,
    join_code: &str,
    settings: &HealthSettings,
) {
    let weight = task.health_points;
    if weight <= 0 {
        return;
    }
    let already = task_results::Entity::find()
        .filter(task_results::Column::SessionIdFk.eq(session_id))
        .filter(task_results::Column::PlayerIdFk.eq(player_id))
        .filter(task_results::Column::TaskId.eq(task.id))
        .filter(task_results::Column::Kind.eq(task_results::KIND_HEALTH_BONUS))
        .one(&state.db)
        .await
        .ok()
        .flatten();
    if already.is_some() {
        return;
    }
    let verified_rows: Vec<health_checkpoints::Model> = health_checkpoints::Entity::find()
        .filter(health_checkpoints::Column::SessionIdFk.eq(session_id))
        .filter(health_checkpoints::Column::PlayerIdFk.eq(player_id))
        .filter(health_checkpoints::Column::TaskIdFk.eq(task.id))
        .filter(health_checkpoints::Column::ServerStatus.eq(HealthCheckStatus::Ok.as_str()))
        .filter(health_checkpoints::Column::Late.eq(false))
        .order_by_desc(health_checkpoints::Column::CreatedAt)
        .all(&state.db)
        .await
        .unwrap_or_default();
    let task_final = verified_rows
        .iter()
        .find(|r| r.kind == HealthCheckpointKind::TaskFinal.as_str());
    let source = task_final.or_else(|| {
        verified_rows
            .iter()
            .find(|r| r.kind == HealthCheckpointKind::Probe.as_str())
    });
    // The tree's verified score with the project's tests composed in: the
    // last run the player's CLI completed at or before this checkpoint.
    let end = match source {
        Some(row) => {
            let counted = counted_run_at(
                &state.db,
                session_id,
                player_id,
                row.created_at,
                &row.commit_sha,
            )
            .await;
            Some(composed_score(row, counted.as_ref()))
        }
        None => None,
    };
    let score = end.as_ref().and_then(|c| c.score);
    let level = ololo_health::level(score, &settings.thresholds);
    let existing_code =
        arena_core::project_repo::session_starts_from_existing_code(&state.db, session_id)
            .await
            .unwrap_or(false);
    // Work on an existing codebase is paid for where it left the code
    // relative to where it found it; a challenge build, which starts from
    // nothing, on the grade it reached.
    let start = if existing_code && source.is_some() {
        Some(task_start_score(state, session_id, player_id, task, join_code).await)
    } else {
        None
    };
    // A start and an end are compared on the dimensions both have: a start
    // the suite never ran on is compared with the end's tree alone.
    let (start_score, end_score) = match (&start, &end) {
        (Some(start), Some(end)) => ololo_health::compose::comparable(&start.composed, end),
        _ => (None, score),
    };
    let bonus = match &start {
        Some(_) => ololo_health::delta_bonus(start_score, end_score, &settings.thresholds, weight),
        None => ololo_health::bonus(score, &settings.thresholds, weight),
    };
    let note = match (&start, source, end_score) {
        (Some(start), Some(_), Some(end)) => match start_score {
            Some(from) => format!(
                "health-bonus: {from:.1} at {} → {end:.1} ({:+.1}) → {}/{weight}",
                start.label,
                end - from,
                bonus.points
            ),
            None => format!(
                "health-bonus: no code to score at {} → {end:.1} ({level:?}) → {}/{weight}",
                start.label, bonus.points
            ),
        },
        _ => base_note(source, score, level, bonus.points, weight),
    };
    // Name the suite's part when it counted: always for a grade, for a
    // comparison only when the start was measured with it too.
    let counted = |id: &str| -> Option<f64> {
        let end = end.as_ref()?;
        let in_start = start
            .as_ref()
            .is_none_or(|s| s.composed.sub_score(id).is_some());
        end.sub_score(id).filter(|_| in_start)
    };
    let note = match (
        counted(ololo_health::suite::TESTS_ID),
        counted(ololo_health::suite::COVERAGE_ID),
    ) {
        (None, None) => note,
        (tests, coverage) => {
            let part = |name: &str, s: Option<f64>| s.map(|s| format!("{name} {s:.0}"));
            let parts: Vec<String> = [part("tests", tests), part("coverage", coverage)]
                .into_iter()
                .flatten()
                .collect();
            format!("{note} (with {})", parts.join(", "))
        }
    };
    let row = task_results::ActiveModel {
        id: Set(Uuid::new_v4()),
        session_id_fk: Set(session_id),
        player_id_fk: Set(player_id),
        task_id: Set(Some(task.id)),
        answer: Set(note.clone()),
        created_at: Set(Utc::now()),
        point_delta: Set(bonus.points),
        is_bonus: Set(true),
        kind: Set(task_results::KIND_HEALTH_BONUS.to_string()),
    };
    if let Err(e) = row.insert(&state.db).await {
        // The partial unique index (one bonus kind per task) turns a race
        // into "already awarded".
        tracing::info!(error = %e, "health: bonus row not inserted (already awarded?)");
        return;
    }
    tracing::info!(
        session_id = %session_id, player_id = %player_id, task_id = %task.id,
        points = bonus.points, weight, score = ?score, "health: bonus awarded"
    );
    crate::session_log_store::record(
        crate::session_log_store::base_dir(),
        session_id,
        Some(player_id),
        "health_bonus",
        serde_json::json!({
            "task_id": task.id,
            "points": bonus.points,
            "weight": weight,
            "factor": bonus.factor,
            "reason": bonus.reason,
            "score": score,
            "level": level,
            "source_checkpoint": source.map(|r| r.id),
            "start_score": start_score,
            "tests_score": end.as_ref().and_then(|e| e.sub_score(ololo_health::suite::TESTS_ID)),
            "coverage_score": end.as_ref().and_then(|e| e.sub_score(ololo_health::suite::COVERAGE_ID)),
            "note": note,
        }),
    )
    .await;
    if bonus.points != 0 {
        crate::ws::player_agent::scoring::publish_score_change(
            state,
            session_id,
            player_id,
            i64::from(bonus.points),
            join_code,
        )
        .await;
        crate::ws::player_agent::scoring::broadcast_leaderboard(state, session_id, join_code).await;
    }
}

/// The note of a bonus paid on the grade the task reached.
fn base_note(
    source: Option<&health_checkpoints::Model>,
    score: Option<f64>,
    level: ololo_health::Level,
    points: i32,
    weight: i32,
) -> String {
    match (source, score) {
        (Some(r), Some(s)) if r.kind == HealthCheckpointKind::TaskFinal.as_str() => {
            format!("health-bonus: final tree scored {s:.1} ({level:?}) → {points}/{weight}")
        }
        (Some(r), Some(s)) => format!(
            "health-bonus: last verified check #{} scored {s:.1} ({level:?}) → {points}/{weight}; the final tree was not verified",
            r.probe_seq
        ),
        (Some(_), None) => {
            format!("health-bonus: the verified tree had no code to score → 0/{weight}")
        }
        (None, _) => format!("health-bonus: no verified checkpoint → 0/{weight}"),
    }
}

/// Where a personal task's health started: the end of the task before it,
/// when that was verified, else the tree the session started from.
struct TaskStart {
    /// The start's score, with the test run it counted composed in.
    composed: ololo_health::compose::Composed,
    /// For the bonus note: "the session start", "the end of task 2".
    label: String,
}

async fn task_start_score(
    state: &GameServerState,
    session_id: Uuid,
    player_id: Uuid,
    task: &tasks::Model,
    join_code: &str,
) -> TaskStart {
    let previous = tasks::Entity::find()
        .filter(tasks::Column::ProjectIdFk.eq(task.project_id_fk))
        .filter(tasks::Column::Ordinal.lt(task.ordinal))
        .order_by_desc(tasks::Column::Ordinal)
        .one(&state.db)
        .await
        .ok()
        .flatten();
    if let Some(previous) = previous {
        let final_row = health_checkpoints::Entity::find()
            .filter(health_checkpoints::Column::SessionIdFk.eq(session_id))
            .filter(health_checkpoints::Column::PlayerIdFk.eq(player_id))
            .filter(health_checkpoints::Column::TaskIdFk.eq(previous.id))
            .filter(health_checkpoints::Column::Kind.eq(HealthCheckpointKind::TaskFinal.as_str()))
            .filter(health_checkpoints::Column::ServerStatus.eq(HealthCheckStatus::Ok.as_str()))
            .one(&state.db)
            .await
            .ok()
            .flatten();
        if let Some(row) = final_row {
            let counted = counted_run_at(
                &state.db,
                session_id,
                player_id,
                row.created_at,
                &row.commit_sha,
            )
            .await;
            return TaskStart {
                composed: composed_score(&row, counted.as_ref()),
                label: format!("the end of task {}", previous.ordinal + 1),
            };
        }
    }
    // The session's first tree: scored before the suite ever ran on it.
    let base = baseline(state, session_id, player_id, join_code).await;
    TaskStart {
        composed: base
            .filter(|r| r.server_status == HealthCheckStatus::Ok.as_str())
            .map(|r| composed_score(&r, None))
            .unwrap_or_default(),
        label: "the session start".to_string(),
    }
}

/// The player's baseline checkpoint — the session-start tree, scored —
/// created and verified on first use. `None` when the repository has no
/// line yet or the row cannot be written.
async fn baseline(
    state: &GameServerState,
    session_id: Uuid,
    player_id: Uuid,
    join_code: &str,
) -> Option<health_checkpoints::Model> {
    let find = || {
        health_checkpoints::Entity::find()
            .filter(health_checkpoints::Column::SessionIdFk.eq(session_id))
            .filter(health_checkpoints::Column::PlayerIdFk.eq(player_id))
            .filter(health_checkpoints::Column::Kind.eq(HealthCheckpointKind::Baseline.as_str()))
            .one(&state.db)
    };
    let row = match find().await.ok().flatten() {
        Some(row) => row,
        None => {
            // The session-start snapshot is the root of the line; the CLI
            // pushes it before the first probe.
            let log = first_parent_log(&repo_dir(session_id, player_id), LOG_LIMIT)
                .await
                .ok()?;
            if log.len() >= LOG_LIMIT {
                return None;
            }
            let root = log.first()?;
            let settings = HealthSettings::load(&state.db).await.unwrap_or_default();
            let started_at = sessions::Entity::find_by_id(session_id)
                .one(&state.db)
                .await
                .ok()
                .flatten()
                .and_then(|s| s.started_at);
            let now = Utc::now();
            let am = health_checkpoints::ActiveModel {
                id: Set(Uuid::new_v4()),
                session_id_fk: Set(session_id),
                player_id_fk: Set(player_id),
                task_id_fk: Set(None),
                probe_id_fk: Set(None),
                kind: Set(HealthCheckpointKind::Baseline.as_str().to_string()),
                probe_seq: Set(0),
                commit_sha: Set(root.sha.clone()),
                derived_task_id: Set(None),
                score_mismatch: Set(false),
                version_mismatch: Set(false),
                task_mismatch: Set(false),
                late: Set(false),
                history_rewritten: Set(false),
                client_status: Set(None),
                client_score: Set(None),
                client_grade: Set(None),
                client_result: Set(None),
                client_duration_ms: Set(None),
                client_jscpd_version: Set(None),
                client_error: Set(None),
                client_reported_at: Set(None),
                server_status: Set(HealthCheckStatus::Pending.as_str().to_string()),
                server_score: Set(None),
                server_grade: Set(None),
                server_result: Set(None),
                server_duration_ms: Set(None),
                server_jscpd_version: Set(None),
                server_error: Set(None),
                server_verified_at: Set(None),
                tests_status: Set(None),
                tests_result: Set(None),
                tests_reported_at: Set(None),
                // On the chart the baseline sits where the session began.
                created_at: Set(started_at.unwrap_or(now)),
                updated_at: Set(now),
            };
            match am.insert(&state.db).await {
                Ok(row) => {
                    publish(state, session_id, join_code, row.id, &settings).await;
                    row
                }
                // A concurrent first report won the race (unique commit).
                Err(_) => find().await.ok().flatten()?,
            }
        }
    };
    if row.server_status != HealthCheckStatus::Pending.as_str() {
        return Some(row);
    }
    verify(state.clone(), row.id, join_code.to_string(), Duration::ZERO).await;
    health_checkpoints::Entity::find_by_id(row.id)
        .one(&state.db)
        .await
        .ok()
        .flatten()
}

/// Re-drive every checkpoint a restart left `pending`.
pub async fn resume_pending(state: GameServerState) -> Result<usize, sea_orm::DbErr> {
    let settings = HealthSettings::load(&state.db).await?;
    if !settings.enabled {
        return Ok(0);
    }
    let rows = health_checkpoints::Entity::find()
        .filter(health_checkpoints::Column::ServerStatus.eq(HealthCheckStatus::Pending.as_str()))
        .all(&state.db)
        .await?;
    let mut n = 0;
    for row in rows {
        let Some(session) = sessions::Entity::find_by_id(row.session_id_fk)
            .one(&state.db)
            .await?
        else {
            continue;
        };
        // Only sessions this game server owns.
        if session.game_server_id != Some(state.server_id) {
            continue;
        }
        n += 1;
        let verifier = state.clone();
        let join_code = session.join_code.clone();
        tokio::spawn(async move {
            verify(verifier, row.id, join_code, COMMIT_WAIT).await;
        });
    }
    Ok(n)
}

/// Score the checkpoint's commit from the player's repository and settle
/// the row. `wait` bounds how long the commit may take to arrive.
pub async fn verify(
    state: GameServerState,
    checkpoint_id: Uuid,
    join_code: String,
    wait: Duration,
) {
    let Ok(Some(row)) = health_checkpoints::Entity::find_by_id(checkpoint_id)
        .one(&state.db)
        .await
    else {
        return;
    };
    if HealthCheckStatus::parse(&row.server_status) != Some(HealthCheckStatus::Pending) {
        return;
    }
    let settings = HealthSettings::load(&state.db).await.unwrap_or_default();
    let session_id = row.session_id_fk;
    let player_id = row.player_id_fk;
    let repo_dir = repo_dir(session_id, player_id);
    let started = std::time::Instant::now();

    // 1. The commit has to be there. Pushes are asynchronous on the
    //    client, so give it time — but not forever.
    let deadline = tokio::time::Instant::now() + wait;
    let present = loop {
        match commit_exists(&repo_dir, &row.commit_sha).await {
            Ok(true) => break true,
            Ok(false) => {}
            Err(e) => tracing::debug!(error = %e, "health: commit lookup failed; retrying"),
        }
        if tokio::time::Instant::now() >= deadline {
            break false;
        }
        tokio::time::sleep(COMMIT_POLL.min(wait.max(Duration::from_millis(50)))).await;
    };
    if !present {
        settle(
            &state,
            &row,
            Settled::failed(
                HealthCheckStatus::CommitMissing,
                "the reported commit never reached the server",
            ),
            &settings,
            started,
        )
        .await;
        publish(&state, session_id, &join_code, row.id, &settings).await;
        return;
    }

    // 2. Where the history puts the commit: the task it belongs to, and
    //    whether it is still on the served line at all.
    let derived_task = match first_parent_log(&repo_dir, LOG_LIMIT).await {
        Ok(log) => task_ranges(&log).task_of(&row.commit_sha),
        Err(e) => {
            tracing::debug!(error = %e, "health: log read failed");
            None
        }
    };
    let on_line = arena_core::health::on_main_line(&repo_dir, &row.commit_sha)
        .await
        .unwrap_or(true);
    if !on_line {
        settle(
            &state,
            &row,
            Settled {
                status: HealthCheckStatus::Unverified,
                result: None,
                error: Some("the commit is not on the served line (history was rewritten)".into()),
                derived_task,
                history_rewritten: true,
            },
            &settings,
            started,
        )
        .await;
        publish(&state, session_id, &join_code, row.id, &settings).await;
        return;
    }

    // 3. Export exactly and scan under the server-probe cap.
    let scan = async {
        let dir = tempfile::Builder::new()
            .prefix("ololo-health-verify-")
            .tempdir()
            .map_err(|e| format!("scratch dir: {e}"))?;
        export_commit(&repo_dir, &row.commit_sha, dir.path())
            .await
            .map_err(|e| format!("export: {e}"))?;
        let _permit = crate::probe_exec::acquire_permit().await;
        ololo_health::analyze_async(dir.path().to_path_buf(), settings.scan_config())
            .await
            .map_err(|e| match e {
                ololo_health::HealthError::Timeout(_) => "timeout".to_string(),
                other => other.to_string(),
            })
    };
    let settled = match scan.await {
        Ok(result) => Settled {
            status: HealthCheckStatus::Ok,
            result: Some(result),
            error: None,
            derived_task,
            history_rewritten: false,
        },
        Err(e) if e == "timeout" => Settled {
            status: HealthCheckStatus::Timeout,
            result: None,
            error: Some(format!("analysis exceeded {}s", settings.timeout.as_secs())),
            derived_task,
            history_rewritten: false,
        },
        Err(e) => Settled {
            status: HealthCheckStatus::Failed,
            result: None,
            error: Some(e),
            derived_task,
            history_rewritten: false,
        },
    };
    settle(&state, &row, settled, &settings, started).await;
    publish(&state, session_id, &join_code, row.id, &settings).await;
}

struct Settled {
    status: HealthCheckStatus,
    result: Option<ololo_health::HealthResult>,
    error: Option<String>,
    derived_task: Option<Uuid>,
    history_rewritten: bool,
}

impl Settled {
    fn failed(status: HealthCheckStatus, error: &str) -> Self {
        Settled {
            status,
            result: None,
            error: Some(error.to_string()),
            derived_task: None,
            history_rewritten: false,
        }
    }
}

async fn settle(
    state: &GameServerState,
    row: &health_checkpoints::Model,
    settled: Settled,
    settings: &HealthSettings,
    started: std::time::Instant,
) {
    let now = Utc::now();
    let server_score = settled.result.as_ref().and_then(|r| r.score);
    let score_mismatch = settled.status == HealthCheckStatus::Ok
        && row.client_status.as_deref() == Some(HealthReportStatus::Ok.as_str())
        && !ololo_health::scores_match(row.client_score, server_score, settings.tolerance);
    let task_mismatch = matches!(
        (settled.derived_task, row.task_id_fk),
        (Some(derived), Some(expected)) if derived != expected
    );
    let mut am: health_checkpoints::ActiveModel = row.clone().into();
    am.server_status = Set(settled.status.as_str().to_string());
    am.server_score = Set(server_score);
    am.server_grade = Set(settled
        .result
        .as_ref()
        .and_then(|r| r.grade.map(|g| g.to_string())));
    am.server_result = Set(settled.result.as_ref().map(cap_result));
    am.server_duration_ms = Set(Some(
        i64::try_from(started.elapsed().as_millis()).unwrap_or(i64::MAX),
    ));
    am.server_jscpd_version = Set(Some(ololo_health::JSCPD_CORE_VERSION.to_string()));
    am.server_error = Set(settled.error.clone());
    am.server_verified_at = Set(Some(now));
    am.derived_task_id = Set(settled.derived_task);
    am.score_mismatch = Set(score_mismatch);
    am.task_mismatch = Set(task_mismatch);
    if settled.history_rewritten {
        am.history_rewritten = Set(true);
    }
    am.updated_at = Set(now);
    if let Err(e) = am.update(&state.db).await {
        tracing::warn!(error = %e, "health: checkpoint settle failed");
        return;
    }
    tracing::info!(
        session_id = %row.session_id_fk, player_id = %row.player_id_fk,
        commit = %row.commit_sha, status = settled.status.as_str(),
        score = ?server_score, score_mismatch, task_mismatch,
        "health: checkpoint verified"
    );
    crate::session_log_store::record(
        crate::session_log_store::base_dir(),
        row.session_id_fk,
        Some(row.player_id_fk),
        "health_checkpoint",
        serde_json::json!({
            "checkpoint_id": row.id,
            "kind": row.kind,
            "commit": row.commit_sha,
            "probe_id": row.probe_id_fk,
            "probe_seq": row.probe_seq,
            "task_id": row.task_id_fk,
            "derived_task_id": settled.derived_task,
            "server_status": settled.status.as_str(),
            "server_score": server_score,
            "client_score": row.client_score,
            "score_mismatch": score_mismatch,
            "version_mismatch": row.version_mismatch,
            "task_mismatch": task_mismatch,
            "late": row.late,
            "history_rewritten": row.history_rewritten || settled.history_rewritten,
            "error": settled.error,
            "duration_ms": started.elapsed().as_millis() as u64,
        }),
    )
    .await;
}

async fn log_checkpoint(
    state: &GameServerState,
    session_id: Uuid,
    player_id: Uuid,
    checkpoint_id: Uuid,
    kind: &str,
    report: &HealthReportPayload,
) {
    let _ = state;
    crate::session_log_store::record(
        crate::session_log_store::base_dir(),
        session_id,
        Some(player_id),
        kind,
        serde_json::json!({
            "checkpoint_id": checkpoint_id,
            "probe_id": report.probe_id,
            "probe_seq": report.probe_seq,
            "task_id": report.task_id,
            "commit": report.commit,
            "status": report.status.as_str(),
            "score": report.result.as_ref().and_then(|r| r.score),
            "jscpd_version": report.jscpd_version,
            "push": report.push,
            "error": report.error,
            "duration_ms": report.duration_ms,
        }),
    )
    .await;
}

/// Publish the checkpoint's current view to the dashboards (for the test
/// reports, which rescore checkpoints after the fact).
pub(crate) async fn publish_view(
    state: &GameServerState,
    session_id: Uuid,
    join_code: &str,
    checkpoint_id: Uuid,
    settings: &HealthSettings,
) {
    publish(state, session_id, join_code, checkpoint_id, settings).await;
}

/// Publish the checkpoint's current view to the dashboards.
async fn publish(
    state: &GameServerState,
    session_id: Uuid,
    join_code: &str,
    checkpoint_id: Uuid,
    settings: &HealthSettings,
) {
    let Some(view) = view_of(state, session_id, checkpoint_id, settings).await else {
        return;
    };
    let version = state
        .session_registry
        .get(join_code)
        .and_then(|e| e.cache.read().ok().map(|c| c.version))
        .unwrap_or(0);
    let player_id = health_checkpoints::Entity::find_by_id(checkpoint_id)
        .one(&state.db)
        .await
        .ok()
        .flatten()
        .map(|r| r.player_id_fk)
        .unwrap_or_default();
    state
        .event_publisher
        .publish(&ZmqEvent::HealthUpdated {
            join_code: join_code.to_string(),
            player_id,
            checkpoint: Box::new(view),
            timestamp: Utc::now(),
            version,
        })
        .await;
}

/// The browser's view of one checkpoint row.
pub async fn view_of(
    state: &GameServerState,
    session_id: Uuid,
    checkpoint_id: Uuid,
    settings: &HealthSettings,
) -> Option<HealthCheckpointView> {
    let row = health_checkpoints::Entity::find_by_id(checkpoint_id)
        .one(&state.db)
        .await
        .ok()
        .flatten()?;
    let started_at = sessions::Entity::find_by_id(session_id)
        .one(&state.db)
        .await
        .ok()
        .flatten()
        .and_then(|s| s.started_at);
    let title = match row.task_id_fk {
        Some(task_id) => tasks::Entity::find_by_id(task_id)
            .one(&state.db)
            .await
            .ok()
            .flatten()
            .map(|t| t.title),
        None => None,
    };
    let counted = counted_run_at(
        &state.db,
        row.session_id_fk,
        row.player_id_fk,
        row.created_at,
        &row.commit_sha,
    )
    .await;
    Some(checkpoint_view(
        &row,
        title,
        started_at,
        &settings.thresholds,
        counted.as_ref(),
    ))
}

fn repo_dir(session_id: Uuid, player_id: Uuid) -> PathBuf {
    let base = arena_core::git_store::repos_base_dir().unwrap_or_else(|| PathBuf::from("repos"));
    arena_core::git_store::player_repo_path(&base, session_id, player_id)
}

async fn probe_task_id(state: &GameServerState, probe: &probes::Model) -> Option<Uuid> {
    arena_core::entities::tests::Entity::find_by_id(probe.test_id)
        .one(&state.db)
        .await
        .ok()
        .flatten()
        .map(|t| t.task_id)
}

/// Whether the player's scheduler has already moved past `task`.
async fn is_late(
    state: &GameServerState,
    session_id: Uuid,
    player_id: Uuid,
    task: Option<Uuid>,
) -> bool {
    let Some(task) = task else {
        return false;
    };
    let Ok(Some(sched)) = session_scheduler_state::Entity::find()
        .filter(session_scheduler_state::Column::SessionIdFk.eq(session_id))
        .filter(session_scheduler_state::Column::PlayerIdFk.eq(player_id))
        .one(&state.db)
        .await
    else {
        return false;
    };
    match sched.task_id {
        Some(current) => current != task,
        // No current task: the player is done with every task.
        None => sched.state != "waiting",
    }
}
