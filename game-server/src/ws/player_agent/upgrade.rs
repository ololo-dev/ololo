use crate::state::{GameServerState, SessionCacheInner, SessionEntry};
use crate::ws::player_agent::socket::{SocketPlayer, resolve_socket_player};
use arena_core::entities::sessions;
use arena_core::session_status::SessionStatus;
use axum::extract::ws::WebSocketUpgrade;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, Set};
use std::sync::{Arc, RwLock};

/// Optional identity for the connecting agent. New ololo clients pass the
/// `player_id` they learned from the resolve endpoint so the socket drives
/// THEIR player row; old clients omit it (served only when the authenticated
/// user has exactly one live row in the session). Either way it is only a
/// hint — the PAT in `X-API-Key` decides which rows this caller may drive
/// (SEC-H1: the join code alone is shareable knowledge, not an identity).
#[derive(serde::Deserialize)]
pub struct AgentWsQuery {
    pub player_id: Option<uuid::Uuid>,
}

pub async fn ws_player_agent_handler(
    Path(join_code): Path<String>,
    axum::extract::Query(query): axum::extract::Query<AgentWsQuery>,
    State(state): State<GameServerState>,
    headers: HeaderMap,
    ws: Result<WebSocketUpgrade, axum::extract::ws::rejection::WebSocketUpgradeRejection>,
) -> Response {
    let join_code_upper = join_code.to_uppercase();

    tracing::info!(join_code = %join_code_upper, "player_agent: WS upgrade request received");

    // PAT auth first: the ololo CLI has always sent `X-API-Key` on this
    // request (FR-015), the game server just never checked it. Reject before
    // any session lookup or side effect (claiming, timer spawns).
    let pat = match headers.get("X-API-Key").and_then(|v| v.to_str().ok()) {
        Some(v) => v.to_string(),
        None => {
            tracing::warn!(join_code = %join_code_upper, "player_agent: missing X-API-Key header");
            return StatusCode::UNAUTHORIZED.into_response();
        }
    };
    let user_id = match arena_core::auth::lookup_pat_user(&state.db, &pat).await {
        Ok(uid) => uid,
        Err(e) => {
            tracing::warn!(join_code = %join_code_upper, error = %e, "player_agent: PAT rejected");
            return StatusCode::UNAUTHORIZED.into_response();
        }
    };

    let session = match sessions::Entity::find()
        .filter(sessions::Column::JoinCode.eq(&join_code_upper))
        .one(&state.db)
        .await
    {
        Ok(Some(s)) => s,
        Ok(None) => {
            tracing::warn!(join_code = %join_code_upper, "player_agent: session not found");
            return StatusCode::NOT_FOUND.into_response();
        }
        Err(e) => {
            tracing::error!(join_code = %join_code_upper, error = %e, "player_agent: DB error looking up session");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    if session.status == SessionStatus::Finished || session.status == SessionStatus::Cancelled {
        tracing::warn!(join_code = %join_code_upper, session_id = %session.id, status = %session.status, "player_agent: session already ended");
        return StatusCode::GONE.into_response();
    }

    // Ownership check before any side effect: the caller may only drive a
    // player row that belongs to the PAT's user in this session.
    let player_id = match resolve_socket_player(&state.db, session.id, user_id, query.player_id)
        .await
    {
        Ok(SocketPlayer::Resolved(id)) => id,
        Ok(SocketPlayer::NotFound) => {
            tracing::warn!(
                join_code = %join_code_upper,
                session_id = %session.id,
                user_id = %user_id,
                requested_player_id = ?query.player_id,
                "player_agent: no live player row for this user (not a member, revoked, or spoofed player_id)",
            );
            return StatusCode::FORBIDDEN.into_response();
        }
        Ok(SocketPlayer::Ambiguous { live_players }) => {
            tracing::error!(
                join_code = %join_code_upper,
                session_id = %session.id,
                user_id = %user_id,
                live_players,
                "player_agent: client sent no player_id and the user has several live rows; refusing rather than guessing (client needs upgrading)",
            );
            return StatusCode::CONFLICT.into_response();
        }
        Err(e) => {
            tracing::error!(join_code = %join_code_upper, session_id = %session.id, error = %e, "player_agent: DB error resolving player");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };
    tracing::info!(join_code = %join_code_upper, session_id = %session.id, player_id = %player_id, user_id = %user_id, requested = ?query.player_id, "player_agent: resolved player");

    match session.game_server_id {
        Some(sid) if sid == state.server_id => {}
        Some(sid) => {
            tracing::warn!(join_code = %join_code_upper, session_id = %session.id, expected = %state.server_id, actual = %sid, "player_agent: session assigned to different game server");
            return StatusCode::NOT_FOUND.into_response();
        }
        None => {
            tracing::info!(join_code = %join_code_upper, session_id = %session.id, "player_agent: claiming unassigned session");
            let mut am: sessions::ActiveModel = session.clone().into();
            am.game_server_id = Set(Some(state.server_id));
            if let Err(e) = am.update(&state.db).await {
                tracing::error!(session_id = %session.id, error = %e, "player_agent: failed to claim session");
                return StatusCode::INTERNAL_SERVER_ERROR.into_response();
            }
        }
    }

    let session = if session.game_server_id.is_none() {
        sessions::Entity::find_by_id(session.id)
            .one(&state.db)
            .await
            .ok()
            .flatten()
            .unwrap_or(session)
    } else {
        session
    };

    let ws = match ws {
        Ok(ws) => ws,
        Err(rej) => {
            tracing::warn!(join_code = %join_code_upper, "player_agent: WS upgrade rejected: {:?}", rej);
            return rej.into_response();
        }
    };

    let session_id = session.id;
    let join_code_owned = join_code_upper.clone();

    tracing::info!(join_code = %join_code_upper, session_id = %session_id, "player_agent: WS upgrade accepted, upgrading socket");

    // Spawn lobby countdown timer the first time a player agent connects to a lobby-phase session.
    // The session_registry entry acts as a double-spawn guard: or_insert_with is a no-op if the
    // key already exists, but we still call tokio::spawn. A second concurrent spawn is harmless
    // because lobby_timer checks `model.status == "lobby"` in the DB before transitioning.
    if session.status == SessionStatus::Lobby {
        // Spawn lobby_timer only for the FIRST connector (the one that creates
        // the registry entry). A per-connect spawn ends with every instance
        // "adopting" the running transition and running its own countdown —
        // duplicate ticks and a version counter racing ahead per player.
        let already_tracked = state.session_registry.contains_key(&join_code_upper);
        let (entry_tx, _) =
            tokio::sync::broadcast::channel::<arena_core::protocol::ArenaFrame>(256);
        let entry_cache = Arc::new(RwLock::new(SessionCacheInner::new(
            session_id,
            SessionStatus::Lobby,
            None,
        )));
        state
            .session_registry
            .entry(join_code_upper.clone())
            .or_insert_with(|| SessionEntry {
                tx: entry_tx,
                cache: entry_cache,
                cancel: tokio_util::sync::CancellationToken::new(),
            });
        if !already_tracked {
            let timer_state = state.clone();
            let timer_jc = join_code_upper.clone();
            let timer_secs = state.lobby_timer_secs;
            let project_id = session.project_id_fk;
            tokio::spawn(crate::ws::session_lifecycle::lobby_timer(
                timer_state,
                session_id,
                String::new(),
                project_id,
                timer_jc,
                timer_secs,
            ));
            tracing::info!(
                join_code = %join_code_upper,
                session_id = %session_id,
                lobby_secs = timer_secs,
                "player_agent: spawned lobby_timer",
            );
        }
    } else if session.status == SessionStatus::Running {
        // Player connecting to an already-running session (e.g. reconnect or late join).
        // Ensure a registry entry exists so session_rx below can subscribe and
        // RunningCountdown frames reach the client. If no entry existed yet (first
        // connection to this running session on this game-server), also spawn
        // running_timer with the remaining duration. Double-spawn is benign: running_timer
        // guards its DB write with `model.status == "running"`.
        let already_tracked = state.session_registry.contains_key(&join_code_upper);
        let (entry_tx, _) =
            tokio::sync::broadcast::channel::<arena_core::protocol::ArenaFrame>(256);
        let entry_cache = Arc::new(RwLock::new(SessionCacheInner::new(
            session_id,
            SessionStatus::Running,
            session.started_at,
        )));
        state
            .session_registry
            .entry(join_code_upper.clone())
            .or_insert_with(|| SessionEntry {
                tx: entry_tx,
                cache: entry_cache,
                cancel: tokio_util::sync::CancellationToken::new(),
            });
        if !already_tracked {
            // running_timer re-reads the project's duration and recomputes
            // remaining from started_at on every tick — no pre-reduced
            // remaining is ever handed over.
            let timer_state = state.clone();
            let timer_jc = join_code_upper.clone();
            tokio::spawn(crate::ws::session_lifecycle::running_timer(
                timer_state,
                session_id,
                String::new(),
                timer_jc,
            ));
            tracing::info!(
                join_code = %join_code_upper,
                session_id = %session_id,
                "player_agent: spawned running_timer for already-running session",
            );
        }
    }

    ws.on_upgrade(move |socket| {
        crate::ws::player_agent::socket::handle_player_agent_socket(
            socket,
            session_id,
            join_code_owned,
            player_id,
            state,
        )
    })
    .into_response()
}
