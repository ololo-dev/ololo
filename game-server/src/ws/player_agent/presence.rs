//! Agent presence: the durable record of "is this player's ololo client
//! actually connected", written on connect/disconnect and read by the web
//! player page for its Live indicator.
//!
//! Persisted in `players.agent_connected` / `agent_last_seen_at` rather than
//! only broadcast, for two consumers the in-memory registry cannot serve:
//! the `server` process builds page snapshots from the DB and has no view of
//! this process's sockets, and the idle sweep needs "empty since when" to
//! survive restarts. A ZMQ event fans the change out for live page updates.

use arena_core::entities::players;
use arena_core::protocol::ZmqEvent;
use chrono::Utc;
use sea_orm::{ActiveModelTrait, DatabaseConnection, EntityTrait, Set};
use uuid::Uuid;

use crate::state::GameServerState;

/// Write `agent_connected` for the player and announce the change.
pub async fn set_agent_presence(
    state: &GameServerState,
    join_code: &str,
    player_id: Uuid,
    connected: bool,
) {
    let now = Utc::now();
    let row = match players::Entity::find_by_id(player_id).one(&state.db).await {
        Ok(Some(row)) => row,
        Ok(None) => return,
        Err(e) => {
            tracing::warn!(player_id = %player_id, error = %e, "presence: player lookup failed");
            return;
        }
    };
    let mut am: players::ActiveModel = row.into();
    am.agent_connected = Set(connected);
    am.agent_last_seen_at = Set(Some(now));
    if let Err(e) = am.update(&state.db).await {
        tracing::warn!(player_id = %player_id, error = %e, "presence: update failed");
        return;
    }
    state
        .event_publisher
        .publish(&ZmqEvent::AgentPresence {
            join_code: join_code.to_string(),
            player_id,
            connected,
            timestamp: now,
        })
        .await;
}

/// Mark every player of this game server's sessions as disconnected.
///
/// Run at startup: after a restart no agent socket survives, but the flags in
/// the DB still say whatever the previous process last wrote. Without this, a
/// crash while players were connected leaves them "Live" forever — and blinds
/// the idle sweep, which skips sessions that appear to have connected agents.
pub async fn reset_agent_presence_on_startup(
    db: &DatabaseConnection,
    server_id: Uuid,
) -> Result<u64, sea_orm::DbErr> {
    use arena_core::entities::sessions;
    use sea_orm::{ColumnTrait, QueryFilter, sea_query::Expr};

    let session_ids: Vec<Uuid> = sessions::Entity::find()
        .filter(sessions::Column::GameServerId.eq(server_id))
        .all(db)
        .await?
        .into_iter()
        .map(|s| s.id)
        .collect();
    if session_ids.is_empty() {
        return Ok(0);
    }
    // Also stamp `agent_last_seen_at`: these agents WERE connected until the
    // process died, and the idle sweep measures "empty since" from this
    // column. Left stale, a restart instantly exceeds the idle timeout and
    // cancels a live session before its CLI can reconnect (session Y3W66Z
    // was cancelled ~10s after a deploy this way). Stamping "now" restarts
    // the idle clock, giving reconnection the full timeout window.
    let res = players::Entity::update_many()
        .col_expr(players::Column::AgentConnected, Expr::value(false))
        .col_expr(
            players::Column::AgentLastSeenAt,
            Expr::value(Some(Utc::now())),
        )
        .filter(players::Column::SessionIdFk.is_in(session_ids))
        .filter(players::Column::AgentConnected.eq(true))
        .exec(db)
        .await?;
    Ok(res.rows_affected)
}
