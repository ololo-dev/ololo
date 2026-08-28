//! Idle sweep: cancel running sessions nobody is playing.
//!
//! A session whose every ololo client has dropped keeps its timers, probe
//! rows and dashboard alive until the running timer expires — up to the full
//! session duration. The sweep cancels it earlier: once the session has had
//! **no connected agents** for longer than the project's `idle_timeout_secs`
//! (0 disables), it flips Running → Cancelled and broadcasts the same end
//! frames every other termination path uses, so the web dashboard and any
//! late-reconnecting CLI see an explicit cancellation rather than a silently
//! frozen page.
//!
//! "Empty since when" is read from `players.agent_last_seen_at` (see the
//! presence module) — durable, so a game-server restart does not reset the
//! clock. A session no client ever connected to counts from `started_at`.
//!
//! Cancellation deliberately skips Arena Points: an abandoned session is an
//! aborted run, not a finished one, matching how the running timer treats an
//! externally-cancelled session (it exits without awarding).

use arena_core::entities::{players, projects, sessions};
use arena_core::protocol::{ArenaFrame, ZmqEvent};
use arena_core::session_status::SessionStatus;
use chrono::Utc;
use sea_orm::sea_query::Expr;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};

use crate::state::GameServerState;
use crate::state::{broadcast_frame, bump_session_version};

/// How often the sweep looks for abandoned sessions.
const SWEEP_INTERVAL_SECS: u64 = 30;

pub async fn run_idle_sweep(state: GameServerState) {
    loop {
        if let Err(e) = sweep_once(&state).await {
            tracing::error!(error = %e, "idle sweep failed");
        }
        tokio::time::sleep(std::time::Duration::from_secs(SWEEP_INTERVAL_SECS)).await;
    }
}

/// One pass over this server's running sessions. Returns how many it
/// cancelled (the tests' observable).
pub async fn sweep_once(state: &GameServerState) -> Result<usize, sea_orm::DbErr> {
    let running = sessions::Entity::find()
        .filter(sessions::Column::GameServerId.eq(state.server_id))
        .filter(sessions::Column::Status.eq(SessionStatus::Running))
        .all(&state.db)
        .await?;

    let mut cancelled = 0usize;
    for session in running {
        let Some(project) = projects::Entity::find_by_id(session.project_id_fk)
            .one(&state.db)
            .await?
        else {
            continue;
        };
        if project.idle_timeout_secs <= 0 {
            continue; // disabled for this project
        }

        let roster = players::Entity::find()
            .filter(players::Column::SessionIdFk.eq(session.id))
            .filter(players::Column::RevokedAt.is_null())
            .all(&state.db)
            .await?;
        if roster.iter().any(|p| p.agent_connected) {
            continue; // someone is here
        }

        // Empty since the last time any agent was seen; a session nobody ever
        // connected to counts from its start.
        let empty_since = roster
            .iter()
            .filter_map(|p| p.agent_last_seen_at)
            .max()
            .or(session.started_at);
        let Some(empty_since) = empty_since else {
            continue; // no start timestamp — nothing sane to measure from
        };

        let idle_secs = (Utc::now() - empty_since).num_seconds();
        if idle_secs < project.idle_timeout_secs as i64 {
            continue;
        }

        if cancel_idle_session(state, &session, idle_secs).await? {
            cancelled += 1;
        }
    }
    Ok(cancelled)
}

/// Flip one session Running → Cancelled and broadcast the end frames.
///
/// The conditional UPDATE is the race guard (same pattern as
/// `finish_session`): if the running timer or an admin ended the session
/// between our read and this write, zero rows change and we announce nothing.
async fn cancel_idle_session(
    state: &GameServerState,
    session: &sessions::Model,
    idle_secs: i64,
) -> Result<bool, sea_orm::DbErr> {
    let res = sessions::Entity::update_many()
        .col_expr(
            sessions::Column::Status,
            Expr::value(SessionStatus::Cancelled),
        )
        .col_expr(sessions::Column::FinishedAt, Expr::value(Some(Utc::now())))
        .col_expr(
            sessions::Column::CancelReason,
            Expr::value(Some("idle_timeout".to_string())),
        )
        .filter(sessions::Column::Id.eq(session.id))
        .filter(sessions::Column::Status.eq(SessionStatus::Running))
        .exec(&state.db)
        .await?;
    if res.rows_affected == 0 {
        return Ok(false);
    }

    tracing::info!(
        session_id = %session.id,
        join_code = %session.join_code,
        idle_secs,
        "idle sweep: cancelled session with no connected agents"
    );

    if let Some(entry) = state.session_registry.get(&session.join_code)
        && let Ok(mut cache) = entry.cache.write()
    {
        cache.phase = SessionStatus::Cancelled;
    }
    let version = bump_session_version(state, &session.join_code);
    broadcast_frame(
        state,
        &session.join_code,
        ArenaFrame::SessionCancelled {
            session_id: session.id,
            version,
            cancel_reason: Some("idle_timeout".to_string()),
            cancelled_by: None,
        },
    );
    // Back-compat reason frame for clients that only know SessionComplete.
    // Deliberately the plain "cancelled": deployed CLIs match on exactly that
    // string to pick the cancel styling; the idle cause is in our log line.
    broadcast_frame(
        state,
        &session.join_code,
        ArenaFrame::SessionComplete {
            reason: "cancelled".to_string(),
            version,
        },
    );
    state
        .event_publisher
        .publish(&ZmqEvent::SessionStatus {
            join_code: session.join_code.clone(),
            status: SessionStatus::Cancelled.to_string(),
            version,
            cancel_reason: Some("idle_timeout".to_string()),
            cancelled_by: None,
        })
        .await;
    Ok(true)
}
