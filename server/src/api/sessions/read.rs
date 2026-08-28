use crate::auth::jwt::AccessClaims;
use crate::state::AppState;
use arena_core::entities::{
    activity_event, players, probes, projects, session_scheduler_state, sessions, task_agent_stats,
    task_results, tasks, users,
};
use arena_core::session_status::SessionStatus;
use axum::Json;
use axum::extract::Path;
use axum::extract::{ConnectInfo, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use chrono::Utc;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, QueryOrder,
    QuerySelect, Set,
};
use std::collections::{HashMap, HashSet};
use std::net::SocketAddr;
use uuid::Uuid;

use super::common::*;

pub async fn get_list(
    State(state): State<AppState>,
    claims: AccessClaims,
) -> Result<Response, SessionError> {
    let user_id = parse_user_id(&claims)?;

    // Sessions where caller is owner.
    let owned = sessions::Entity::find()
        .filter(sessions::Column::OwnerIdFk.eq(user_id))
        .all(&state.db)
        .await?;

    // Sessions where caller is a member.
    let member_ids: Vec<Uuid> = players::Entity::find()
        .filter(players::Column::UserIdFk.eq(user_id))
        .all(&state.db)
        .await?
        .into_iter()
        .map(|m| m.session_id_fk)
        .collect();
    let member_sessions = if member_ids.is_empty() {
        Vec::new()
    } else {
        sessions::Entity::find()
            .filter(sessions::Column::Id.is_in(member_ids))
            .all(&state.db)
            .await?
    };

    // Merge dedup by id.
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for m in owned.into_iter().chain(member_sessions) {
        if seen.insert(m.id) {
            let jc = Some(m.join_code.clone());
            out.push(to_summary(m, jc));
        }
    }
    Ok(Json(SessionListResp { sessions: out }).into_response())
}
pub async fn get_one(
    State(state): State<AppState>,
    claims: AccessClaims,
    Path(id): Path<Uuid>,
) -> Result<Response, SessionError> {
    let user_id = parse_user_id(&claims)?;
    let row = load_visible(&state.db, id, user_id).await?;
    let jc = Some(row.join_code.clone());
    Ok(Json(to_summary(row, jc)).into_response())
}

pub async fn get_report(
    State(state): State<AppState>,
    claims: Option<AccessClaims>,
    Path(id): Path<Uuid>,
) -> Result<Response, SessionError> {
    let session = sessions::Entity::find_by_id(id)
        .one(&state.db)
        .await?
        .ok_or(SessionError::NotFound)?;
    authorize_session_view(&state.db, &session, claims.as_ref()).await?;

    if session.status != SessionStatus::Finished && session.status != SessionStatus::Cancelled {
        return Err(SessionError::ReportNotReady);
    }

    let active_players = players::Entity::find()
        .filter(players::Column::SessionIdFk.eq(id))
        .filter(players::Column::RevokedAt.is_null())
        .all(&state.db)
        .await?;

    let task_rows = tasks::Entity::find()
        .filter(tasks::Column::ProjectIdFk.eq(session.project_id_fk))
        .all(&state.db)
        .await?;
    let task_title_by_id: std::collections::HashMap<Uuid, String> =
        task_rows.into_iter().map(|t| (t.id, t.title)).collect();

    let result_rows = task_results::Entity::find()
        .filter(task_results::Column::SessionIdFk.eq(id))
        .order_by_asc(task_results::Column::CreatedAt)
        .all(&state.db)
        .await?;

    let player_name_by_id: std::collections::HashMap<Uuid, String> = active_players
        .iter()
        .map(|p| (p.id, p.display_name.clone()))
        .collect();

    // Avatar/username per player, resolved through the linked user account.
    // Finished sessions have no live WS snapshot to supply these.
    let linked_user_ids: Vec<Uuid> = active_players.iter().filter_map(|p| p.user_id_fk).collect();
    let user_by_id: std::collections::HashMap<Uuid, users::Model> = if linked_user_ids.is_empty() {
        std::collections::HashMap::new()
    } else {
        users::Entity::find()
            .filter(users::Column::Id.is_in(linked_user_ids))
            .all(&state.db)
            .await
            .unwrap_or_default()
            .into_iter()
            .map(|u| (u.id, u))
            .collect()
    };
    let report_players: Vec<ReportPlayerDto> = active_players
        .iter()
        .map(|p| {
            let user = p.user_id_fk.and_then(|uid| user_by_id.get(&uid));
            ReportPlayerDto {
                player_id: p.id,
                user_id: p.user_id_fk,
                display_name: p.display_name.clone(),
                avatar_url: user.and_then(|u| u.avatar_url.clone()),
                username: user.and_then(|u| u.username.clone()),
            }
        })
        .collect();

    let leaderboard = crate::scoring::compute_leaderboard(&state.db, id)
        .await
        .unwrap_or_default();

    let timeline = result_rows
        .into_iter()
        .map(|r| SessionReportTimelineItem {
            task_id: r.task_id.unwrap_or_else(Uuid::nil).to_string(),
            task_title: r
                .task_id
                .and_then(|tid| task_title_by_id.get(&tid).cloned())
                .unwrap_or_else(|| "Unknown task".to_string()),
            player_id: r.player_id_fk,
            player_display_name: player_name_by_id
                .get(&r.player_id_fk)
                .cloned()
                .unwrap_or_else(|| r.player_id_fk.to_string()),
            point_delta: r.point_delta,
            answer: r.answer,
            created_at: r.created_at,
        })
        .collect();

    // Activity events: newest 500, reversed to ASC for chronological display.
    let mut activity_rows = activity_event::Entity::find()
        .filter(activity_event::Column::SessionIdFk.eq(id))
        .order_by_desc(activity_event::Column::Timestamp)
        .limit(500)
        .all(&state.db)
        .await
        .unwrap_or_default();
    activity_rows.reverse();
    dedupe_activity_feed(&mut activity_rows);
    let activity_events: Vec<ActivityEventDto> = activity_rows
        .into_iter()
        .map(|r| ActivityEventDto {
            event_kind: r.event_kind,
            player_id: r.player_id_fk,
            player_display_name: r.player_display_name,
            task_id: r.task_id_fk,
            task_ordinal: r.task_ordinal,
            task_title: r.task_title,
            judge_name: r.judge_name,
            point_delta: r.point_delta,
            detail: r.detail,
            timestamp: r.timestamp,
            version: r.version,
        })
        .collect();

    let score_history = arena_core::scoring::build_score_history(&state.db, id, session.started_at)
        .await
        .unwrap_or(None);

    // The same signal the award settle poll waits on: judge runs still owed
    // for tasks the players reached. Only meaningful once the timer stopped.
    let judges_pending = if session.status == SessionStatus::Finished {
        arena_core::session_completion::expired_session_pending_judges(&state.db, id)
            .await
            .unwrap_or(0) as u64
    } else {
        0
    };

    Ok(Json(SessionReportResp {
        session_id: id.to_string(),
        status: session.status.to_string(),
        leaderboard,
        timeline,
        activity_events,
        players: report_players,
        score_history,
        judges_pending,
    })
    .into_response())
}

/// `GET /api/sessions/:id/player-stats` — per-player statistics for the
/// session page: solved tasks, probes dispatched, agents/models used
/// (client-reported), and aggregated token usage. Public, works for
/// running and finished sessions alike.
pub async fn get_player_stats(
    State(state): State<AppState>,
    claims: Option<AccessClaims>,
    Path(id): Path<Uuid>,
) -> Result<Response, SessionError> {
    use sea_orm::PaginatorTrait;

    let session = sessions::Entity::find_by_id(id)
        .one(&state.db)
        .await?
        .ok_or(SessionError::NotFound)?;
    authorize_session_view(&state.db, &session, claims.as_ref()).await?;

    let total_tasks = tasks::Entity::find()
        .filter(tasks::Column::ProjectIdFk.eq(session.project_id_fk))
        .count(&state.db)
        .await?;

    let active_players = players::Entity::find()
        .filter(players::Column::SessionIdFk.eq(id))
        .filter(players::Column::RevokedAt.is_null())
        .all(&state.db)
        .await?;
    let linked_user_ids: Vec<Uuid> = active_players.iter().filter_map(|p| p.user_id_fk).collect();
    let user_by_id: std::collections::HashMap<Uuid, users::Model> = if linked_user_ids.is_empty() {
        std::collections::HashMap::new()
    } else {
        users::Entity::find()
            .filter(users::Column::Id.is_in(linked_user_ids))
            .all(&state.db)
            .await
            .unwrap_or_default()
            .into_iter()
            .map(|u| (u.id, u))
            .collect()
    };

    // Session score per player (probes + judges), used for the rank
    // tie-break; the displayed total is decomposed below.
    let scores = arena_core::scoring::aggregate_scores(&state.db, id)
        .await
        .unwrap_or_default();

    // Point decomposition + solved tasks from task_results/judge_results.
    let result_rows = task_results::Entity::find()
        .filter(task_results::Column::SessionIdFk.eq(id))
        .all(&state.db)
        .await
        .unwrap_or_default();
    let mut probe_pts: std::collections::HashMap<Uuid, i64> = std::collections::HashMap::new();
    let mut bonus_pts: std::collections::HashMap<Uuid, i64> = std::collections::HashMap::new();
    let mut solved: std::collections::HashMap<Uuid, std::collections::HashSet<Uuid>> =
        std::collections::HashMap::new();
    for r in &result_rows {
        if r.is_bonus {
            *bonus_pts.entry(r.player_id_fk).or_insert(0) += r.point_delta as i64;
            if let Some(tid) = r.task_id {
                solved.entry(r.player_id_fk).or_default().insert(tid);
            }
        } else {
            *probe_pts.entry(r.player_id_fk).or_insert(0) += r.point_delta as i64;
        }
    }
    let judge_rows = arena_core::entities::judge_results::Entity::find()
        .filter(arena_core::entities::judge_results::Column::SessionIdFk.eq(id))
        .all(&state.db)
        .await
        .unwrap_or_default();
    let mut judge_pts: std::collections::HashMap<Uuid, i64> = std::collections::HashMap::new();
    for r in &judge_rows {
        *judge_pts.entry(r.player_id_fk).or_insert(0) += r.point_delta as i64;
    }

    // Competition ranking among active players: 1 + number strictly better
    // by the leaderboard sort key (total points, then tests passed).
    let key = |pid: Uuid| {
        scores
            .get(&pid)
            .map(|d| (d.total_points, d.tests_passed))
            .unwrap_or((0, 0))
    };
    let rank_of = |pid: Uuid| -> u32 {
        let me = key(pid);
        1 + active_players
            .iter()
            .filter(|o| o.id != pid && key(o.id) > me)
            .count() as u32
    };

    // Probe counts per player.
    let probe_rows: Vec<(Uuid, i64)> = probes::Entity::find()
        .filter(probes::Column::SessionId.eq(id))
        .select_only()
        .column(probes::Column::PlayerId)
        .column_as(probes::Column::Id.count(), "n")
        .group_by(probes::Column::PlayerId)
        .into_tuple()
        .all(&state.db)
        .await
        .unwrap_or_default();
    let probes_by_player: std::collections::HashMap<Uuid, i64> = probe_rows.into_iter().collect();

    // Client-reported per-task agent stats: aggregate tokens and collect
    // the distinct agents/models actually observed.
    let stat_rows = task_agent_stats::Entity::find()
        .filter(task_agent_stats::Column::SessionIdFk.eq(id))
        .all(&state.db)
        .await
        .unwrap_or_default();

    #[derive(Default)]
    struct Agg {
        input: i64,
        output: i64,
        cache_read: i64,
        cache_write: i64,
        reasoning: i64,
        cost: f64,
        has_cost: bool,
        tool_calls: i64,
        agents: std::collections::BTreeSet<String>,
        models: std::collections::BTreeSet<String>,
    }
    let mut agg: std::collections::HashMap<Uuid, Agg> = std::collections::HashMap::new();
    for row in &stat_rows {
        let a = agg.entry(row.player_id_fk).or_default();
        a.input += row.input_tokens;
        a.output += row.output_tokens;
        a.cache_read += row.cache_read_tokens;
        a.cache_write += row.cache_write_tokens;
        a.reasoning += row.reasoning_tokens;
        if let Some(c) = row.cost {
            a.cost += c;
            a.has_cost = true;
        }
        a.tool_calls += row.tool_calls;
        if let Ok(sessions) =
            serde_json::from_str::<Vec<arena_core::protocol::AgentSessionStats>>(&row.agents_json)
        {
            for s in sessions {
                a.agents.insert(s.agent);
                if let Some(m) = s.model {
                    a.models.insert(m);
                }
            }
        }
    }

    let players_out: Vec<SessionPlayerStatsDto> = active_players
        .iter()
        .map(|p| {
            let user = p.user_id_fk.and_then(|uid| user_by_id.get(&uid));
            let a = agg.remove(&p.id).unwrap_or_default();
            SessionPlayerStatsDto {
                player_id: p.id,
                user_id: p.user_id_fk,
                display_name: p.display_name.clone(),
                avatar_url: user.and_then(|u| u.avatar_url.clone()),
                username: user.and_then(|u| u.username.clone()),
                agent_display_name: arena_core::scoring::parse_agent_display_name(
                    p.metadata_json.as_deref(),
                ),
                rank: rank_of(p.id),
                game_points: probe_pts.get(&p.id).copied().unwrap_or(0)
                    + bonus_pts.get(&p.id).copied().unwrap_or(0)
                    + judge_pts.get(&p.id).copied().unwrap_or(0),
                probe_points: probe_pts.get(&p.id).copied().unwrap_or(0),
                bonus_points: bonus_pts.get(&p.id).copied().unwrap_or(0),
                judge_points: judge_pts.get(&p.id).copied().unwrap_or(0),
                solved_tasks: solved.get(&p.id).map(|t| t.len() as u64).unwrap_or(0),
                probes: probes_by_player.get(&p.id).copied().unwrap_or(0).max(0) as u64,
                agents: a.agents.into_iter().collect(),
                models: a.models.into_iter().collect(),
                input_tokens: a.input,
                output_tokens: a.output,
                cache_read_tokens: a.cache_read,
                cache_write_tokens: a.cache_write,
                reasoning_tokens: a.reasoning,
                cost: a.has_cost.then_some(a.cost),
                tool_calls: a.tool_calls,
            }
        })
        .collect();

    Ok(Json(SessionPlayerStatsResp {
        total_tasks,
        players: players_out,
    })
    .into_response())
}

pub async fn get_activity(
    State(state): State<AppState>,
    claims: Option<AccessClaims>,
    Path(id): Path<Uuid>,
) -> Result<Response, SessionError> {
    let session = sessions::Entity::find_by_id(id)
        .one(&state.db)
        .await?
        .ok_or(SessionError::NotFound)?;
    authorize_session_view(&state.db, &session, claims.as_ref()).await?;

    let mut activity_rows = activity_event::Entity::find()
        .filter(activity_event::Column::SessionIdFk.eq(id))
        .order_by_desc(activity_event::Column::Timestamp)
        .limit(500)
        .all(&state.db)
        .await
        .unwrap_or_default();
    activity_rows.reverse();
    dedupe_activity_feed(&mut activity_rows);
    let events: Vec<ActivityEventDto> = activity_rows
        .into_iter()
        .map(|r| ActivityEventDto {
            event_kind: r.event_kind,
            player_id: r.player_id_fk,
            player_display_name: r.player_display_name,
            task_id: r.task_id_fk,
            task_ordinal: r.task_ordinal,
            task_title: r.task_title,
            judge_name: r.judge_name,
            point_delta: r.point_delta,
            detail: r.detail,
            timestamp: r.timestamp,
            version: r.version,
        })
        .collect();

    Ok(Json(ActivityListResp { events }).into_response())
}

/// Collapse the activity feed's repeated rows. Rows must be in ascending
/// timestamp order (oldest first).
///
/// Three shapes of noise, three rules:
///
/// - `task_started` and a probe-pass `task_scored` (the "implemented Task N"
///   line, distinguished by having no `judge_name`) are re-emitted on every
///   probe cycle, not once per task: a task that keeps passing re-emits the
///   same line each cycle, and — for sessions before the single-flight fix — a
///   duplicated socket emitted several within the same second. None of these
///   repeats is a new award (scoring is deduped in `task_results`; the
///   leaderboard is unaffected), so keep the EARLIEST per (player, task, kind).
///
/// - A judge verdict (`task_scored` WITH a `judge_name`) is written once per
///   run, but a re-run — the recovery sweep, or a manual re-judge — appends a
///   fresh row while the single `judge_result` is upserted, so earlier verdict
///   rows are stale. Keep the LATEST per (player, task, judge). Different
///   judges on one task keep their own rows (distinct `judge_name`), and a
///   judge that legitimately scores several tasks keeps one row per task.
pub(crate) fn dedupe_activity_feed(rows: &mut Vec<activity_event::Model>) {
    // First pass: for each judge verdict key, remember the index of its LAST
    // (newest) row — that is the one to keep.
    let mut last_judge_idx = std::collections::HashMap::new();
    for (i, r) in rows.iter().enumerate() {
        if r.event_kind == "task_scored"
            && let Some(judge) = &r.judge_name
        {
            last_judge_idx.insert((r.player_id_fk, r.task_id_fk, judge.clone()), i);
        }
    }

    // Second pass: drop superseded judge verdicts, and collapse the repeatable
    // kinds to their earliest occurrence.
    let mut seen = std::collections::HashSet::new();
    let mut idx = 0usize;
    rows.retain(|r| {
        let i = idx;
        idx += 1;
        if r.event_kind == "task_scored"
            && let Some(judge) = &r.judge_name
        {
            return last_judge_idx.get(&(r.player_id_fk, r.task_id_fk, judge.clone())) == Some(&i);
        }
        let collapsible = r.event_kind == "task_started"
            || (r.event_kind == "task_scored" && r.judge_name.is_none());
        !collapsible || seen.insert((r.player_id_fk, r.task_id_fk, r.event_kind.clone()))
    });
}

/// Load up to 500 activity events for a session, oldest first, as protocol
/// `SessionActivityEvent` values. Used by the WS snapshot build sites.
/// Returns an empty vec on DB error — the snapshot must still be sent.
pub(crate) async fn load_session_activity(
    db: &DatabaseConnection,
    session_id: Uuid,
) -> Vec<arena_core::protocol::SessionActivityEvent> {
    let mut activity_rows = activity_event::Entity::find()
        .filter(activity_event::Column::SessionIdFk.eq(session_id))
        .order_by_desc(activity_event::Column::Timestamp)
        .limit(500)
        .all(db)
        .await
        .unwrap_or_default();
    activity_rows.reverse();
    dedupe_activity_feed(&mut activity_rows);
    activity_rows
        .into_iter()
        .map(|r| arena_core::protocol::SessionActivityEvent {
            event_kind: r.event_kind,
            player_id: r.player_id_fk,
            player_display_name: r.player_display_name,
            task_id: r.task_id_fk,
            task_ordinal: r.task_ordinal,
            task_title: r.task_title,
            judge_name: r.judge_name,
            point_delta: r.point_delta,
            detail: r.detail,
            timestamp: r.timestamp,
            version: r.version,
        })
        .collect()
}

#[tracing::instrument(level = "info", skip_all, fields(session_id = %id))]
pub async fn patch_one(
    State(state): State<AppState>,
    claims: AccessClaims,
    Path(id): Path<Uuid>,
    Json(req): Json<PatchSessionReq>,
) -> Result<Response, SessionError> {
    let user_id = parse_user_id(&claims)?;
    let is_admin = crate::auth::is_user_admin(&state.db, user_id).await?;
    let row = if is_admin {
        load_any(&state.db, id).await?
    } else {
        load_for_owner(&state.db, id, user_id).await?
    };

    let new_name = match &req.name {
        Some(n) => Some(validate_name(n)?),
        None => None,
    };
    let new_status = match &req.status {
        Some(s) => {
            validate_transition(&row.status, s)?;
            Some(*s)
        }
        None => None,
    };

    if new_name.is_none() && new_status.is_none() {
        let jc = Some(row.join_code.clone());
        return Ok(Json(to_summary(row, jc)).into_response());
    }

    // A user-initiated cancel records who did it, for the end notification.
    let canceller_name = if new_status == Some(SessionStatus::Cancelled) {
        users::Entity::find_by_id(user_id)
            .one(&state.db)
            .await?
            .map(|u| u.display_name)
    } else {
        None
    };

    let now = Utc::now();
    let mut am: sessions::ActiveModel = row.clone().into();
    if let Some(n) = new_name {
        am.name = Set(n);
    }
    if let Some(s) = new_status {
        // Audit timestamps (NFR-005): record when the session entered
        // its `running` and terminal states.
        if s == SessionStatus::Running && row.started_at.is_none() {
            am.started_at = Set(Some(now));
        }
        if (s == SessionStatus::Finished || s == SessionStatus::Cancelled)
            && row.finished_at.is_none()
        {
            am.finished_at = Set(Some(now));
        }
        // Pause / resume / cancel timestamp bookkeeping.
        if s == SessionStatus::Paused {
            am.paused_at = Set(Some(now));
        }
        if s == SessionStatus::Running && row.status == SessionStatus::Paused {
            if let Some(paused_at) = row.paused_at {
                let extra = (now - paused_at).num_seconds().max(0);
                let acc = row.paused_duration_secs.unwrap_or(0) + extra;
                am.paused_duration_secs = Set(Some(acc));
            }
            am.paused_at = Set(None);
        }
        if s == SessionStatus::Cancelled && row.paused_at.is_some() {
            am.paused_at = Set(None);
        }
        if s == SessionStatus::Cancelled {
            am.cancel_reason = Set(Some("user".to_string()));
            am.cancelled_by = Set(canceller_name.clone());
        }
        am.status = Set(s);
    }
    let updated = am.update(&state.db).await?;
    // Keep session registry in sync with lifecycle.
    if row.status != updated.status {
        let lifecycle_frame = crate::protocol::ArenaFrame::ProjectSessionUpdate {
            session_id: updated.id,
            name: updated.name.clone(),
            status: updated.status,
            player_count: crate::api::sessions::common::count_players(&state.db, updated.id).await,
            project_id: updated.project_id_fk,
            join_code: Some(updated.join_code.clone()),
            created_at: updated.created_at,
            cancel_reason: updated.cancel_reason.clone(),
            cancelled_by: updated.cancelled_by.clone(),
        };
        // Broadcast to project observers AND session observers so the
        // /s/[code] page transitions phase without a reload.
        fan_session_update(
            &state.session_registry,
            &updated.join_code,
            lifecycle_frame.clone(),
        );
        fan_project_update(
            &state.project_registry,
            updated.project_id_fk,
            lifecycle_frame.clone(),
        );
        fan_landing_update(&state, updated.project_id_fk, lifecycle_frame).await;
        match updated.status {
            SessionStatus::Running => {
                // Update cache (timer is now owned by game-server).
                if let Some(entry) = state.session_registry.get(&updated.join_code)
                    && let Ok(mut cache) = entry.cache.write()
                {
                    cache.phase = SessionStatus::Running;
                    cache.started_at = Some(now);
                    cache.version = cache.version.saturating_add(1);
                }
            }
            SessionStatus::Paused => {
                if let Some(entry) = state.session_registry.get(&updated.join_code)
                    && let Ok(mut cache) = entry.cache.write()
                {
                    cache.phase = SessionStatus::Paused;
                    cache.version = cache.version.saturating_add(1);
                }
            }
            SessionStatus::Cancelled => {
                // Update cache: transition to cancelled, then evict.
                if let Some(entry) = state.session_registry.get(&updated.join_code)
                    && let Ok(mut cache) = entry.cache.write()
                {
                    cache.phase = SessionStatus::Cancelled;
                    cache.version = cache.version.saturating_add(1);
                }
                // The player page reacts to SessionStatusChange, which only
                // reaches it through the zmq bridge — the lifecycle frame above
                // targets dashboard/project observers, not the per-player WS.
                // Carry the reason so the player's end banner can name it.
                let _ = state
                    .zmq_events_tx
                    .send(arena_core::protocol::ZmqEvent::SessionStatus {
                        join_code: updated.join_code.clone(),
                        status: SessionStatus::Cancelled.to_string(),
                        version: 0,
                        cancel_reason: updated.cancel_reason.clone(),
                        cancelled_by: updated.cancelled_by.clone(),
                    });
                // Evict from session registry so dashboard WS clients disconnect.
                // Best-effort: missing entry is silently ignored (session may
                // have never had a timer, e.g. created and cancelled immediately).
                state.session_registry.remove(&updated.join_code);
                state.admin_registry.remove(&updated.id);
            }
            SessionStatus::Finished => {
                state.session_registry.remove(&updated.join_code);
                state.admin_registry.remove(&updated.id);
            }
            _ => {}
        }
    }
    let jc = Some(updated.join_code.clone());
    Ok(Json(to_summary(updated, jc)).into_response())
}

pub async fn delete_one(
    State(state): State<AppState>,
    claims: AccessClaims,
    Path(id): Path<Uuid>,
) -> Result<Response, SessionError> {
    let user_id = parse_user_id(&claims)?;
    // Admins reach any session, as they already do for `patch_one`. Without
    // this an admin could cancel a session from the settings registry but not
    // remove it, which reads as a broken button rather than a policy.
    let is_admin = crate::auth::is_user_admin(&state.db, user_id).await?;
    let row = if is_admin {
        load_any(&state.db, id).await?
    } else {
        load_for_owner(&state.db, id, user_id).await?
    };

    // Explicit removal of players rows. The migration declares
    // ON DELETE CASCADE for the FK, but we delete here defensively so
    // behaviour is identical across SQLite (which honours FK cascade
    // only when `PRAGMA foreign_keys=ON` is set on the connection) and
    // PostgreSQL. FR-038 cascade for `agents`/`tasks`/`invites` is
    // handled by their respective FK declarations in later migrations.
    players::Entity::delete_many()
        .filter(players::Column::SessionIdFk.eq(row.id))
        .exec(&state.db)
        .await?;

    sessions::Entity::delete_by_id(row.id)
        .exec(&state.db)
        .await?;

    Ok(StatusCode::NO_CONTENT.into_response())
}

/// `GET /api/projects/:project_id/sessions`
///
/// Lists all sessions for a project. Project owners see every session;
/// other authenticated users see only sessions they own or are a member of.
pub async fn get_project_sessions(
    State(state): State<AppState>,
    claims: Option<AccessClaims>,
    Path(project_id): Path<Uuid>,
) -> Result<Response, SessionError> {
    // Check project exists.
    let project = projects::Entity::find_by_id(project_id)
        .one(&state.db)
        .await?
        .ok_or(SessionError::ProjectNotFound)?;

    // A public project's sessions are public history: visible to everyone,
    // authenticated or not. A non-public project stays restricted to the
    // project owner (all sessions) or a session owner/member (their own);
    // anonymous callers on a private project see nothing.
    let user_id = match claims.as_ref() {
        Some(c) => Some(parse_user_id(c)?),
        None => None,
    };
    let all_sessions = if project.public || user_id == Some(project.owner_user_id_fk) {
        // Public project (anyone) or the project owner: every session.
        sessions::Entity::find()
            .filter(sessions::Column::ProjectIdFk.eq(project_id))
            .all(&state.db)
            .await?
    } else if let Some(user_id) = user_id {
        // Private project, non-owner: sessions they own or are a member of.
        let owned = sessions::Entity::find()
            .filter(sessions::Column::ProjectIdFk.eq(project_id))
            .filter(sessions::Column::OwnerIdFk.eq(user_id))
            .all(&state.db)
            .await?;

        let member_session_ids: Vec<Uuid> = players::Entity::find()
            .filter(players::Column::UserIdFk.eq(user_id))
            .all(&state.db)
            .await?
            .into_iter()
            .map(|m| m.session_id_fk)
            .collect();

        let member_sessions = if member_session_ids.is_empty() {
            Vec::new()
        } else {
            sessions::Entity::find()
                .filter(sessions::Column::ProjectIdFk.eq(project_id))
                .filter(sessions::Column::Id.is_in(member_session_ids))
                .all(&state.db)
                .await?
        };

        let mut seen = std::collections::HashSet::new();
        let mut merged = Vec::new();
        for m in owned.into_iter().chain(member_sessions) {
            if seen.insert(m.id) {
                merged.push(m);
            }
        }
        merged
    } else {
        // Private project, anonymous caller: nothing.
        Vec::new()
    };

    // The project page shows a player count on every row, and a winner on the
    // finished ones. Both are aggregated in bulk — a fixed handful of queries
    // for the whole list rather than a few per session.
    let session_ids: Vec<Uuid> = all_sessions.iter().map(|m| m.id).collect();
    let counts = arena_core::scoring::player_counts(&state.db, &session_ids)
        .await
        .unwrap_or_default();
    let finished_ids: Vec<Uuid> = all_sessions
        .iter()
        .filter(|m| m.status == SessionStatus::Finished)
        .map(|m| m.id)
        .collect();
    let winners = arena_core::scoring::session_winners(&state.db, &finished_ids)
        .await
        .unwrap_or_default();

    let sessions = all_sessions
        .into_iter()
        .map(|m| {
            let jc = Some(m.join_code.clone());
            let id = m.id;
            let winner = winners.get(&id);
            let mut summary = to_summary(m, jc);
            summary.player_count = counts.get(&id).copied().unwrap_or(0);
            // Shown whatever the sign: judges can dock a whole field below
            // zero, and the top of that leaderboard still won the session.
            if let Some(w) = winner {
                summary.best_player = Some(w.display_name.clone());
                summary.best_score = Some(w.total_points);
            }
            summary
        })
        .collect();

    Ok(Json(SessionListResp { sessions }).into_response())
}

/// `GET /api/sessions/by-code/:join_code`
///
/// Unauthenticated lookup of a session by its join code. Returns a minimal
/// public view: `{ id, join_code, state, project_id, project_name, project_slug }`.
/// Rate-limited by IP.
pub async fn get_by_code(
    conn_info: Option<ConnectInfo<SocketAddr>>,
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(code): Path<String>,
) -> Response {
    let ip = extract_ip(&headers, conn_info.as_ref());
    if !state.rate_limiter.check_and_record(&ip) {
        return SessionError::RateLimited.into_response();
    }
    let row = match sessions::Entity::find()
        .filter(sessions::Column::JoinCode.eq(code.to_uppercase()))
        .one(&state.db)
        .await
    {
        Ok(Some(r)) => r,
        Ok(None) => return SessionError::NotFound.into_response(),
        Err(e) => return SessionError::Db(e).into_response(),
    };
    // Fetch project info for the link displayed on the public spectator page.
    let project = projects::Entity::find_by_id(row.project_id_fk)
        .one(&state.db)
        .await
        .ok()
        .flatten();
    let (project_id, project_name, project_slug, project_description) = match project {
        Some(p) => (
            Some(p.id.to_string()),
            Some(p.name),
            p.slug,
            Some(p.description),
        ),
        None => (None, None, None, None),
    };
    Json(serde_json::json!({
        "id": row.id,
        "join_code": row.join_code,
        "state": row.status,
        "owner_id": row.owner_id_fk,
        "project_id": project_id,
        "project_name": project_name,
        "project_slug": project_slug,
        "project_description": project_description,
        "created_at": row.created_at,
        "started_at": row.started_at,
        "finished_at": row.finished_at,
    }))
    .into_response()
}

/// One player's cleared run of a campaign part, as the session dashboard
/// shows it: who, and where to read what they did.
#[derive(serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct CampaignClearedBy {
    pub user_id: Uuid,
    pub display_name: String,
    /// The session the part was cleared in — a link the dashboard can offer.
    pub join_code: String,
    pub finished_at: Option<chrono::DateTime<Utc>>,
}

#[derive(serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct CampaignPartRow {
    pub project_id: Uuid,
    pub name: String,
    pub slug: Option<String>,
    pub part_ordinal: i32,
    /// This is the part the session is playing right now.
    pub current: bool,
    /// Players of *this* session who have cleared this part, most recent run
    /// per player. Empty on the current part and on parts still ahead.
    pub cleared_by: Vec<CampaignClearedBy>,
}

#[derive(serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct SessionCampaignResp {
    pub id: Uuid,
    pub name: String,
    pub slug: Option<String>,
    /// Where the session sits in the campaign, 0-based like `part_ordinal`.
    pub current_part_ordinal: i32,
    /// The session's own status, so the card can say "playing now" only while
    /// that is true.
    pub session_status: SessionStatus,
    pub parts: Vec<CampaignPartRow>,
}

/// `GET /api/sessions/by-code/:join_code/campaign`
///
/// The campaign context of a session whose project is a campaign part: the
/// ladder, and which of the earlier parts this session's players have already
/// cleared, with the session each was cleared in.
///
/// A campaign part continues the codebase of the part before it, so "what came
/// before" is the first thing a spectator of part three needs — and the player
/// list on the dashboard is the right place to answer it. `204 No Content`
/// when the session is not part of a campaign: there is nothing to say, and an
/// empty object would put an empty card on every other dashboard.
/// Unauthenticated, like the join-code lookup it accompanies, and rate-limited
/// the same way.
pub async fn get_campaign_by_code(
    conn_info: Option<ConnectInfo<SocketAddr>>,
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(code): Path<String>,
) -> Result<Response, SessionError> {
    let ip = extract_ip(&headers, conn_info.as_ref());
    if !state.rate_limiter.check_and_record(&ip) {
        return Err(SessionError::RateLimited);
    }

    let session = sessions::Entity::find()
        .filter(sessions::Column::JoinCode.eq(code.to_uppercase()))
        .one(&state.db)
        .await?
        .ok_or(SessionError::NotFound)?;

    let project = projects::Entity::find_by_id(session.project_id_fk)
        .one(&state.db)
        .await?
        .ok_or(SessionError::NotFound)?;

    let (Some(campaign_id), Some(current_ordinal)) =
        (project.parent_project_id_fk, project.part_ordinal)
    else {
        return Ok(StatusCode::NO_CONTENT.into_response());
    };
    let Some(campaign) = projects::Entity::find_by_id(campaign_id)
        .one(&state.db)
        .await?
    else {
        return Ok(StatusCode::NO_CONTENT.into_response());
    };

    let mut siblings = projects::Entity::find()
        .filter(projects::Column::ParentProjectIdFk.eq(campaign_id))
        .all(&state.db)
        .await?;
    siblings.sort_by_key(|p| p.part_ordinal.unwrap_or(i32::MAX));

    // The people in this session — the ones whose history is worth showing.
    let members: Vec<(Uuid, String)> = players::Entity::find()
        .filter(players::Column::SessionIdFk.eq(session.id))
        .filter(players::Column::RevokedAt.is_null())
        .all(&state.db)
        .await?
        .into_iter()
        .filter_map(|p| p.user_id_fk.map(|uid| (uid, p.display_name)))
        .collect();
    let mut name_by_user: HashMap<Uuid, String> = HashMap::new();
    for (uid, name) in &members {
        name_by_user.entry(*uid).or_insert_with(|| name.clone());
    }
    let user_ids: Vec<Uuid> = name_by_user.keys().copied().collect();

    // Only the parts *before* this one: a part cleared later is not what the
    // player brought with them into this session.
    let earlier_ids: Vec<Uuid> = siblings
        .iter()
        .filter(|p| p.part_ordinal.unwrap_or(i32::MAX) < current_ordinal)
        .map(|p| p.id)
        .collect();
    let runs =
        arena_core::campaign::completing_runs_for_users(&state.db, &earlier_ids, &user_ids).await?;

    // One entry per (player, part): the most recent clearing run, which is
    // also the snapshot the next part continued from.
    let mut latest: HashMap<(Uuid, Uuid), arena_core::campaign::FinishedRun> = HashMap::new();
    for r in runs {
        latest
            .entry((r.user_id, r.run.project_id))
            .and_modify(|best| {
                if r.run.finished_at >= best.finished_at {
                    *best = r.run;
                }
            })
            .or_insert(r.run);
    }
    let cleared_session_ids: Vec<Uuid> = latest.values().map(|r| r.session_id).collect();
    let code_by_session: HashMap<Uuid, String> = sessions::Entity::find()
        .filter(sessions::Column::Id.is_in(cleared_session_ids))
        .all(&state.db)
        .await?
        .into_iter()
        .map(|s| (s.id, s.join_code))
        .collect();

    // The part being played is answered by *this* session alone: whether the
    // players finished its task list here. Their clearing it in some earlier
    // run is not what a spectator of this session is asking about, and a chip
    // linking away from the page they are on would be a worse answer still.
    let completed_here: Vec<Uuid> = session_scheduler_state::Entity::find()
        .filter(session_scheduler_state::Column::SessionIdFk.eq(session.id))
        .filter(
            session_scheduler_state::Column::State
                .eq(arena_core::session_completion::SCHEDULER_STATE_COMPLETED),
        )
        .all(&state.db)
        .await?
        .into_iter()
        .map(|r| r.player_id_fk)
        .collect();
    let users_done_here: HashSet<Uuid> = players::Entity::find()
        .filter(players::Column::Id.is_in(completed_here))
        .all(&state.db)
        .await?
        .into_iter()
        .filter(|p| p.revoked_at.is_none())
        .filter_map(|p| p.user_id_fk)
        .collect();

    let parts = siblings
        .iter()
        .map(|p| {
            let ordinal = p.part_ordinal.unwrap_or(0);
            if p.id == project.id {
                let mut cleared_by: Vec<CampaignClearedBy> = name_by_user
                    .iter()
                    .filter(|(uid, _)| users_done_here.contains(uid))
                    .map(|(uid, name)| CampaignClearedBy {
                        user_id: *uid,
                        display_name: name.clone(),
                        join_code: session.join_code.clone(),
                        finished_at: session.finished_at,
                    })
                    .collect();
                cleared_by.sort_by(|a, b| a.display_name.cmp(&b.display_name));
                return CampaignPartRow {
                    project_id: p.id,
                    name: p.name.clone(),
                    slug: p.slug.clone(),
                    part_ordinal: ordinal,
                    current: true,
                    cleared_by,
                };
            }
            let mut cleared_by: Vec<CampaignClearedBy> = name_by_user
                .iter()
                .filter_map(|(uid, name)| {
                    let run = latest.get(&(*uid, p.id))?;
                    Some(CampaignClearedBy {
                        user_id: *uid,
                        display_name: name.clone(),
                        join_code: code_by_session.get(&run.session_id).cloned()?,
                        finished_at: run.finished_at,
                    })
                })
                .collect();
            cleared_by.sort_by(|a, b| a.display_name.cmp(&b.display_name));
            CampaignPartRow {
                project_id: p.id,
                name: p.name.clone(),
                slug: p.slug.clone(),
                part_ordinal: ordinal,
                current: false,
                cleared_by,
            }
        })
        .collect();

    Ok(Json(SessionCampaignResp {
        id: campaign.id,
        name: campaign.name,
        slug: campaign.slug,
        current_part_ordinal: current_ordinal,
        session_status: session.status,
        parts,
    })
    .into_response())
}

/// Query for [`get_session_artifact`]: `i` selects the file of a
/// multi-file delivery (default 0).
#[derive(Debug, serde::Deserialize)]
pub struct SessionArtifactQuery {
    #[serde(default)]
    pub i: Option<usize>,
}

/// `GET /api/sessions/:id/artifacts/:probe_id?i=N` — stream a delivered
/// interactive-probe artifact (screenshot, screencast, report) out of the
/// player's pushed repo. Authorization mirrors the activity feed: whoever
/// may view the session sees what its activity log shows. The blob
/// reference comes from the probe row the server itself wrote.
pub async fn get_session_artifact(
    State(state): State<AppState>,
    claims: Option<AccessClaims>,
    Path((id, probe_id)): Path<(Uuid, Uuid)>,
    axum::extract::Query(q): axum::extract::Query<SessionArtifactQuery>,
) -> Result<Response, SessionError> {
    use arena_core::evaluation::ProbeConfig;
    use axum::http::header;

    let session = sessions::Entity::find_by_id(id)
        .one(&state.db)
        .await?
        .ok_or(SessionError::NotFound)?;
    authorize_session_view(&state.db, &session, claims.as_ref()).await?;

    let probe = probes::Entity::find_by_id(probe_id)
        .one(&state.db)
        .await?
        .ok_or(SessionError::NotFound)?;
    if probe.session_id != session.id {
        return Err(SessionError::NotFound);
    }
    let reference = crate::api::players::artifact_reference(&probe, q.i.unwrap_or(0))
        .ok_or(SessionError::NotFound)?;

    // Content type from the probe's own request config, not from the blob.
    let content_type = arena_core::entities::tests::Entity::find_by_id(probe.test_id)
        .one(&state.db)
        .await?
        .and_then(|t| t.probe_config)
        .and_then(|c| ProbeConfig::from_json(&c).ok())
        .and_then(|c| c.artifact.map(|a| a.content_type))
        .unwrap_or_else(|| "application/octet-stream".to_string());

    let repos_base = arena_core::git_store::repos_base_dir().ok_or(SessionError::NotFound)?;
    let repo_dir =
        arena_core::git_store::player_repo_path(&repos_base, session.id, probe.player_id);
    let bytes = crate::api::players::read_artifact_blob(&repo_dir, &reference)
        .await
        .ok_or(SessionError::NotFound)?;

    let mut resp = bytes.into_response();
    resp.headers_mut().insert(
        header::CONTENT_TYPE,
        content_type
            .parse()
            .unwrap_or(header::HeaderValue::from_static("application/octet-stream")),
    );
    // Player-supplied content: never render in the page origin as HTML.
    resp.headers_mut().insert(
        header::CONTENT_SECURITY_POLICY,
        header::HeaderValue::from_static("sandbox"),
    );
    resp.headers_mut().insert(
        header::HeaderName::from_static("x-content-type-options"),
        header::HeaderValue::from_static("nosniff"),
    );
    // The blob reference is content-addressed (sha:path) — safe to cache.
    resp.headers_mut().insert(
        header::CACHE_CONTROL,
        header::HeaderValue::from_static("private, max-age=3600"),
    );
    Ok(resp)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(kind: &str, player: Uuid, task: Uuid, secs: i64) -> activity_event::Model {
        activity_event::Model {
            id: Uuid::new_v4(),
            session_id_fk: Uuid::nil(),
            player_id_fk: player,
            task_id_fk: task,
            event_kind: kind.to_string(),
            task_ordinal: 1,
            task_title: "Task".to_string(),
            player_display_name: "Player".to_string(),
            judge_name: None,
            point_delta: None,
            timestamp: chrono::DateTime::from_timestamp(secs, 0).unwrap(),
            version: 0,
            detail: None,
        }
    }

    fn judge_row(player: Uuid, task: Uuid, judge: &str, secs: i64) -> activity_event::Model {
        activity_event::Model {
            judge_name: Some(judge.to_string()),
            ..row("task_scored", player, task, secs)
        }
    }

    #[test]
    fn dedupe_collapses_repeated_started_and_implemented_per_player_and_task() {
        let player = Uuid::new_v4();
        let other_player = Uuid::new_v4();
        let task = Uuid::new_v4();
        let mut rows = vec![
            row("task_started", player, task, 10),
            row("task_scored", player, task, 20), // "implemented" — first kept
            row("task_started", player, task, 30), // repeat started — dropped
            row("task_started", other_player, task, 40),
            row("task_scored", player, task, 50), // implemented repeat — dropped
            row("task_started", player, task, 60), // repeat started — dropped
        ];
        dedupe_activity_feed(&mut rows);
        let kinds_and_ts: Vec<(&str, i64)> = rows
            .iter()
            .map(|r| (r.event_kind.as_str(), r.timestamp.timestamp()))
            .collect();
        assert_eq!(
            kinds_and_ts,
            vec![
                ("task_started", 10),
                ("task_scored", 20),
                ("task_started", 40),
            ],
        );
    }

    #[test]
    fn dedupe_keeps_distinct_judges_but_collapses_same_judge_reruns() {
        let player = Uuid::new_v4();
        let task = Uuid::new_v4();
        // A judge verdict is a task_scored carrying a judge_name. Different
        // judges on one task are distinct rows and must survive. A re-run of
        // the SAME judge (recovery sweep / manual re-judge) is a stale
        // duplicate — the single judge_result is upserted — so only its LATEST
        // verdict row is kept.
        let mut rows = vec![
            row("task_scored", player, task, 10), // implemented — kept (earliest)
            row("task_scored", player, task, 15), // implemented repeat — dropped
            judge_row(player, task, "anti-cheater", 20), // superseded by ts=30
            judge_row(player, task, "code-cleanliness", 25),
            judge_row(player, task, "anti-cheater", 30), // latest anti-cheater — kept
        ];
        dedupe_activity_feed(&mut rows);
        let kept: Vec<(&str, i64)> = rows
            .iter()
            .map(|r| {
                (
                    r.judge_name.as_deref().unwrap_or("implemented"),
                    r.timestamp.timestamp(),
                )
            })
            .collect();
        assert_eq!(
            kept,
            vec![
                ("implemented", 10),
                ("code-cleanliness", 25),
                ("anti-cheater", 30),
            ],
            "one implemented + one row per distinct judge, anti-cheater at its latest verdict"
        );
    }

    #[test]
    fn dedupe_keeps_same_judge_across_different_tasks() {
        let player = Uuid::new_v4();
        let task_a = Uuid::new_v4();
        let task_b = Uuid::new_v4();
        // One judge legitimately scores every task it is attached to — those
        // are per-task verdicts, not duplicates, and all must survive.
        let mut rows = vec![
            judge_row(player, task_a, "golf-verify", 10),
            judge_row(player, task_b, "golf-verify", 20),
        ];
        dedupe_activity_feed(&mut rows);
        assert_eq!(rows.len(), 2, "one golf-verify verdict per task is kept");
    }
}
