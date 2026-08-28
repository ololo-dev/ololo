use crate::auth::jwt::AccessClaims;
use crate::state::AppState;
use arena_core::entities::{game_servers, projects, sessions};
use arena_core::join_code;
use arena_core::session_status::SessionStatus;
use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use chrono::Utc;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, DatabaseConnection, DbErr, EntityTrait, QueryFilter, QueryOrder,
    Set,
};
use tokio::sync::broadcast;
use uuid::Uuid;

use super::common::*;

#[tracing::instrument(level = "info", skip_all)]
pub async fn post_create(
    State(state): State<AppState>,
    claims: AccessClaims,
    Json(req): Json<CreateSessionReq>,
) -> Result<Response, SessionError> {
    let user_id = parse_user_id(&claims)?;
    let name = validate_name(&req.name)?;
    let project_id = req.project_id;

    // Validate project: existence, access, archive status (outside transaction).
    let project = projects::Entity::find_by_id(project_id)
        .one(&state.db)
        .await?
        .ok_or(SessionError::ProjectNotFound)?;
    if !project.public && project.owner_user_id_fk != user_id {
        return Err(SessionError::ProjectForbidden);
    }
    if project.archived_at.is_some() {
        return Err(SessionError::ProjectArchived);
    }

    // One live session per user: creating a session leads straight to
    // playing it (`ololo start` auto-joins), so refusing here — before the
    // row exists — beats refusing the join and stranding an orphan lobby.
    ensure_single_active_session(&state.db, user_id, None).await?;

    // Campaign rules: a parent hosts no sessions, and a part waits for its
    // predecessor. Checked after the live-session guard so the more
    // actionable "finish the session you're in" wins when both apply.
    ensure_campaign_unlocked(&state.db, &project, user_id).await?;

    let now = Utc::now();
    let id = Uuid::new_v4();

    const MAX_ATTEMPTS: usize = 3;
    let mut last_err: Option<DbErr> = None;
    for _ in 0..MAX_ATTEMPTS {
        let code = join_code::generate();
        let am = sessions::ActiveModel {
            id: Set(id),
            name: Set(name.clone()),
            created_at: Set(now),
            owner_id_fk: Set(Some(user_id)),
            status: Set(SessionStatus::Lobby),
            join_code: Set(code),
            started_at: Set(None),
            finished_at: Set(None),
            paused_at: Set(None),
            paused_duration_secs: Set(None),
            project_id_fk: Set(project_id),
            game_server_id: Set(None),
            cancel_reason: Set(None),
            cancelled_by: Set(None),
        };
        match am.insert(&state.db).await {
            Ok(model) => {
                let jc = model.join_code.clone();
                let session_id = model.id;

                assign_session_to_game_server(&state.db, session_id).await;

                // Defensive evict: if a stale entry with this join_code somehow
                // lingered from a previous session, remove it before inserting.
                state.session_registry.remove(&jc);

                // Capacity 256 — enough for all dashboard clients per session.
                let (tx, _) = broadcast::channel::<crate::protocol::ArenaFrame>(256);
                state.session_registry.insert(
                    jc.clone(),
                    crate::state::SessionEntry {
                        tx,
                        cache: std::sync::Arc::new(std::sync::RwLock::new(
                            crate::state::SessionCacheInner {
                                session_id,
                                phase: SessionStatus::Lobby,
                                version: 1,
                                participants: vec![],
                                leaderboard: vec![],
                                started_at: None,
                            },
                        )),
                    },
                );

                // Fan out new session (lobby status) to project observers and
                // — for public projects — the landing's live-sessions block.
                let created_frame = crate::protocol::ArenaFrame::ProjectSessionUpdate {
                    session_id: model.id,
                    name: model.name.clone(),
                    status: model.status,
                    // Freshly created: the owner joins as a player later.
                    player_count: 0,
                    project_id,
                    join_code: Some(jc.clone()),
                    created_at: model.created_at,
                    cancel_reason: None,
                    cancelled_by: None,
                };
                fan_project_update(&state.project_registry, project_id, created_frame.clone());
                if project.public {
                    let _ = state.landing_tx.send(created_frame);
                }

                // Spawn lobby countdown timer. The server owns the
                // spectator-facing countdown broadcast + the lobby→running
                // lifecycle transition. Game-server's lobby_timer (spawned on
                // first player-agent connect) is a fallback guarded by the
                // `model.status == Lobby` DB check.
                let lt_state = state.clone();
                let lt_jc = jc.clone();
                let lt_secs = state.lobby_timer_secs;
                tokio::spawn(lobby_timer(lt_state, model.id, lt_jc, project_id, lt_secs));

                return Ok((StatusCode::CREATED, Json(to_summary(model, Some(jc)))).into_response());
            }
            Err(e) => {
                // Retry only on unique constraint violations (join_code collision).
                if is_unique_violation(&e) {
                    last_err = Some(e);
                    continue;
                }
                return Err(SessionError::Db(e));
            }
        }
    }
    // All 3 attempts collided — extremely unlikely but contract-required (FR-JC-003).
    tracing::error!(
        "join_code unique constraint exhausted after {MAX_ATTEMPTS} attempts; last error: {:?}",
        last_err
    );
    Err(SessionError::JoinCodeExhausted)
}

async fn assign_session_to_game_server(db: &DatabaseConnection, session_id: Uuid) {
    let gs = match game_servers::Entity::find()
        .filter(game_servers::Column::Status.eq("active"))
        .filter(game_servers::Column::LastHeartbeat.gte(crate::api::game_servers::stale_cutoff()))
        .order_by_asc(game_servers::Column::ActiveSessions)
        .one(db)
        .await
    {
        Ok(Some(gs)) => gs,
        _ => {
            tracing::debug!(session_id = %session_id, "no active game server available for assignment");
            return;
        }
    };

    if gs.active_sessions >= gs.capacity {
        tracing::debug!(session_id = %session_id, "all game servers at capacity");
        return;
    }

    match sessions::Entity::find_by_id(session_id).one(db).await {
        Ok(Some(session)) => {
            let mut am: sessions::ActiveModel = session.into();
            am.game_server_id = Set(Some(gs.id));
            if let Err(e) = am.update(db).await {
                tracing::error!(session_id = %session_id, error = %e, "failed to assign game server");
            } else {
                tracing::info!(session_id = %session_id, game_server_id = %gs.id, "assigned session to game server");
            }
        }
        Ok(None) => {
            tracing::warn!(session_id = %session_id, "session not found for game server assignment");
        }
        Err(e) => {
            tracing::error!(session_id = %session_id, error = %e, "DB error in game server assignment");
        }
    }
}

// Broadcast a `ProjectSessionUpdate` frame to any connected project observers.
// ── Server-side countdown timers ───────────────────────────────────────────
// The server owns the spectator-facing countdown broadcast, and the
// lobby→running transition. Both crates run a timer and a DB status check
// decides which one writes.
//
// running→finished is the exception: the game server that owns the session
// writes that one. There the transition is the head of a chain — judges for
// tasks the clock interrupted, session-scoped judges, the judge settle poll,
// the Arena Points award — and none of it lives in this crate, so winning the
// race here would end the session with the chain skipped. This timer defers
// to the owner and only steps in when there is none, or when it has stopped
// heartbeating (see `wait_for_owner_to_finish`).
//
// Pause-aware: re-read DB status each tick; freeze on Paused; recompute on resume.

fn broadcast_frame(
    registry: &crate::state::SessionRegistry,
    join_code: &str,
    frame: crate::protocol::ArenaFrame,
) {
    if let Some(entry) = registry.get(join_code) {
        let _ = entry.tx.send(frame);
    }
}

fn bump_session_version(registry: &crate::state::SessionRegistry, join_code: &str) -> u64 {
    if let Some(entry) = registry.get(join_code)
        && let Ok(mut cache) = entry.cache.write()
    {
        cache.version = cache.version.saturating_add(1);
        return cache.version;
    }
    0
}

/// Recompute remaining running time from DB timestamps.
/// `paused_duration_secs` is added back to cancel the pause window from elapsed.
fn recompute_remaining(
    duration_secs: u64,
    started_at: chrono::DateTime<chrono::Utc>,
    now: chrono::DateTime<chrono::Utc>,
    paused_duration_secs: Option<i64>,
) -> i64 {
    let elapsed = now.signed_duration_since(started_at).num_seconds();
    (duration_secs as i64 - elapsed + paused_duration_secs.unwrap_or(0)).max(0)
}

async fn lobby_timer(
    state: AppState,
    session_id: Uuid,
    join_code: String,
    project_id: Uuid,
    timer_secs: u64,
) {
    // Landing eligibility checked once — a visibility flip mid-lobby only
    // misroutes ticks for the remainder of a ≤`timer_secs` window.
    let landing_public = matches!(
        projects::Entity::find_by_id(project_id).one(&state.db).await,
        Ok(Some(p)) if p.public
    );
    for remaining in (1..=timer_secs).rev() {
        // Check DB status — exit if no longer lobby (e.g. cancelled or game-server won race).
        let model = match sessions::Entity::find_by_id(session_id)
            .one(&state.db)
            .await
        {
            Ok(Some(m)) => m,
            _ => return,
        };
        if model.status != SessionStatus::Lobby {
            return;
        }
        let version = bump_session_version(&state.session_registry, &join_code);
        let frame = crate::protocol::ArenaFrame::LobbyCountdown {
            session_id,
            seconds_remaining: remaining as u32,
            version,
        };
        broadcast_frame(&state.session_registry, &join_code, frame.clone());
        if landing_public {
            let _ = state.landing_tx.send(frame);
        }
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    }

    // Countdown ended — transition lobby → running.
    let now = Utc::now();
    let model = match sessions::Entity::find_by_id(session_id)
        .one(&state.db)
        .await
    {
        Ok(Some(m)) => m,
        _ => return,
    };
    if model.status != SessionStatus::Lobby {
        return; // Lost race or cancelled.
    }
    let mut am: sessions::ActiveModel = model.into();
    am.status = Set(SessionStatus::Running);
    am.started_at = Set(Some(now));
    let updated = match am.update(&state.db).await {
        Ok(u) => u,
        Err(e) => {
            tracing::error!(session_id = %session_id, error = %e, "lobby_timer: failed to set running");
            return;
        }
    };

    // Update cache + broadcast SessionStarted + ProjectSessionUpdate.
    if let Some(entry) = state.session_registry.get(&join_code)
        && let Ok(mut cache) = entry.cache.write()
    {
        cache.phase = SessionStatus::Running;
        cache.started_at = Some(now);
        cache.version = cache.version.saturating_add(1);
    }
    let version = bump_session_version(&state.session_registry, &join_code);
    broadcast_frame(
        &state.session_registry,
        &join_code,
        crate::protocol::ArenaFrame::SessionStarted {
            session_id,
            version,
            total_tasks: None,
        },
    );
    let lifecycle_frame = crate::protocol::ArenaFrame::ProjectSessionUpdate {
        session_id: updated.id,
        name: updated.name.clone(),
        status: SessionStatus::Running,
        player_count: crate::api::sessions::common::count_players(&state.db, updated.id).await,
        project_id,
        join_code: Some(updated.join_code.clone()),
        created_at: updated.created_at,
        cancel_reason: None,
        cancelled_by: None,
    };
    fan_session_update(&state.session_registry, &join_code, lifecycle_frame.clone());
    fan_project_update(&state.project_registry, project_id, lifecycle_frame.clone());
    if landing_public {
        let _ = state.landing_tx.send(lifecycle_frame);
    }

    // Spawn running_timer for the running phase.
    let rt_state = state.clone();
    let rt_jc = join_code.clone();
    tokio::spawn(running_timer(rt_state, session_id, rt_jc, project_id));
}

/// How long to let the owning game server finish its own expired session
/// before this process does it instead. Its timer ticks every second, so this
/// is generous; the point is only that a wedged owner cannot strand a session
/// in `running` forever.
const OWNER_FINISH_GRACE: std::time::Duration = std::time::Duration::from_secs(60);

/// Whether the expired session's ending is somebody else's to write.
///
/// True when the session is owned by a live game server and that server
/// finished it within `grace`. False when the session has no owner, its owner
/// has stopped heartbeating, or the grace ran out — in each of those the
/// caller must finish the session itself, because a session stuck in
/// `running` is worse than an ending that skipped the judge chain (the game
/// server's recovery sweeps re-drive missed judge runs and unawarded
/// sessions).
async fn wait_for_owner_to_finish(
    state: &AppState,
    session_id: Uuid,
    game_server_id: Option<Uuid>,
    grace: std::time::Duration,
) -> bool {
    let Some(owner_id) = game_server_id else {
        return false;
    };
    let owner_alive = match game_servers::Entity::find_by_id(owner_id)
        .one(&state.db)
        .await
    {
        Ok(Some(gs)) => {
            gs.status == "active" && gs.last_heartbeat > crate::api::game_servers::stale_cutoff()
        }
        // No row, or the lookup failed: assume nobody is coming.
        _ => false,
    };
    if !owner_alive {
        return false;
    }

    let deadline = tokio::time::Instant::now() + grace;
    loop {
        match sessions::Entity::find_by_id(session_id)
            .one(&state.db)
            .await
        {
            Ok(Some(m)) if m.status != SessionStatus::Running => return true,
            Ok(_) => {}
            Err(e) => {
                tracing::warn!(session_id = %session_id, error = %e, "running_timer: session poll failed while waiting for the owning game server");
                return false;
            }
        }
        if tokio::time::Instant::now() >= deadline {
            tracing::warn!(
                session_id = %session_id,
                game_server_id = %owner_id,
                grace_secs = grace.as_secs(),
                "running_timer: owning game server did not finish its expired session; finishing here — its judge and award chain did not run"
            );
            return false;
        }
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    }
}

async fn running_timer(state: AppState, session_id: Uuid, join_code: String, project_id: Uuid) {
    // None until the first Running tick computes it; the Paused freeze falls
    // back to the project's full duration before that.
    let mut remaining: Option<i64> = None;

    loop {
        // Per-tick poll fetches the project row alongside the session so an
        // edit to `default_session_duration_secs` takes effect within one tick.
        let (model, project) = match sessions::Entity::find_by_id(session_id)
            .find_also_related(projects::Entity)
            .one(&state.db)
            .await
        {
            Ok(Some((m, p))) => (m, p),
            _ => return,
        };
        let (duration_secs, landing_public) = match project {
            Some(p) => (p.default_session_duration_secs.max(0) as u64, p.public),
            None => return, // Project row gone — nothing to count down from.
        };
        let now = Utc::now();
        match model.status {
            SessionStatus::Finished | SessionStatus::Cancelled => return,
            SessionStatus::Paused => {
                // Freeze: broadcast frozen seconds, sleep, re-check.
                let frozen = remaining.unwrap_or(duration_secs as i64);
                let version = bump_session_version(&state.session_registry, &join_code);
                let frame = crate::protocol::ArenaFrame::RunningCountdown {
                    session_id,
                    seconds_remaining: frozen.max(0) as u32,
                    version,
                    paused: true,
                };
                broadcast_frame(&state.session_registry, &join_code, frame.clone());
                if landing_public {
                    let _ = state.landing_tx.send(frame);
                }
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                continue;
            }
            SessionStatus::Running => {
                // Recompute from DB timestamps to correct pause drift.
                let secs = match model.started_at {
                    Some(started_at) => recompute_remaining(
                        duration_secs,
                        started_at,
                        now,
                        model.paused_duration_secs,
                    ),
                    None => duration_secs as i64,
                };
                remaining = Some(secs);
                if secs <= 0 {
                    break; // Transition to finished below.
                }
                let version = bump_session_version(&state.session_registry, &join_code);
                let frame = crate::protocol::ArenaFrame::RunningCountdown {
                    session_id,
                    seconds_remaining: secs as u32,
                    version,
                    paused: false,
                };
                broadcast_frame(&state.session_registry, &join_code, frame.clone());
                if landing_public {
                    let _ = state.landing_tx.send(frame);
                }
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            }
            _ => return, // Lobby in running_timer — unexpected, exit.
        }
    }

    // Countdown ended — transition running → finished.
    let model = match sessions::Entity::find_by_id(session_id)
        .one(&state.db)
        .await
    {
        Ok(Some(m)) => m,
        _ => return,
    };
    if model.status != SessionStatus::Running {
        return;
    }

    // The transition belongs to the game server that owns this session. On
    // that side it is not a status write but the head of a chain: judges for
    // tasks the clock interrupted, session-scoped judges, the settle poll,
    // and the Arena Points award. Writing the row here wins the race and
    // silently skips all of it — the owner's timer then finds the session
    // already finished and exits. Session 2NVOG2 is what that looks like: the
    // last rung's judge sat pending until a recovery sweep re-drove it five
    // minutes later.
    if wait_for_owner_to_finish(&state, session_id, model.game_server_id, OWNER_FINISH_GRACE).await
    {
        // The ending itself reached the dashboard clients over ZMQ. The
        // registry entries are still this timer's to drop — it is the one
        // that would have dropped them had it written the row.
        state.session_registry.remove(&join_code);
        state.admin_registry.remove(&session_id);
        return;
    }
    // Re-read: the grace above may have ended just as the owner finished, and
    // a second write would move `finished_at` and re-broadcast the ending.
    let model = match sessions::Entity::find_by_id(session_id)
        .one(&state.db)
        .await
    {
        Ok(Some(m)) if m.status == SessionStatus::Running => m,
        _ => return,
    };

    let mut am: sessions::ActiveModel = model.into();
    am.status = Set(SessionStatus::Finished);
    am.finished_at = Set(Some(Utc::now()));
    let updated = match am.update(&state.db).await {
        Ok(u) => u,
        Err(e) => {
            tracing::error!(session_id = %session_id, error = %e, "running_timer: failed to set finished");
            return;
        }
    };

    // Update cache + broadcast SessionComplete + ProjectSessionUpdate.
    if let Some(entry) = state.session_registry.get(&join_code)
        && let Ok(mut cache) = entry.cache.write()
    {
        cache.phase = SessionStatus::Finished;
        cache.version = cache.version.saturating_add(1);
    }
    let version = bump_session_version(&state.session_registry, &join_code);
    broadcast_frame(
        &state.session_registry,
        &join_code,
        crate::protocol::ArenaFrame::SessionComplete {
            reason: "time_expired".to_string(),
            version,
        },
    );
    let lifecycle_frame = crate::protocol::ArenaFrame::ProjectSessionUpdate {
        session_id: updated.id,
        name: updated.name.clone(),
        status: SessionStatus::Finished,
        player_count: crate::api::sessions::common::count_players(&state.db, updated.id).await,
        project_id,
        join_code: Some(updated.join_code.clone()),
        created_at: updated.created_at,
        cancel_reason: None,
        cancelled_by: None,
    };
    fan_session_update(&state.session_registry, &join_code, lifecycle_frame.clone());
    fan_project_update(&state.project_registry, project_id, lifecycle_frame.clone());
    fan_landing_update(&state, project_id, lifecycle_frame).await;
    state.session_registry.remove(&join_code);
    state.admin_registry.remove(&session_id);
}

#[cfg(test)]
mod tests;
