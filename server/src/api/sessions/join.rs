use crate::auth::jwt::AccessClaims;
use crate::state::AppState;
use arena_core::entities::{players, sessions, users};
use arena_core::protocol::ZmqEvent;
use arena_core::session_status::SessionStatus;
use axum::Json;
use axum::extract::{ConnectInfo, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use chrono::Utc;
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter, Set};
use std::net::SocketAddr;
use uuid::Uuid;

use super::common::*;

/// `POST /api/sessions/join`
///
/// Accepts `{ "code": "<join_code>" }` and joins the authenticated caller to
/// the matching session as a `participant`.
///
/// FR-JC-010 — log redaction: `skip_all` suppresses the request body from
/// the tracing span so the `code` field is never captured in logs.
#[tracing::instrument(level = "info", skip_all)]
pub async fn post_join(
    State(state): State<AppState>,
    claims: AccessClaims,
    conn_info: Option<ConnectInfo<SocketAddr>>,
    headers: HeaderMap,
    Json(req): Json<JoinSessionReq>,
) -> Result<Response, SessionError> {
    // FR-JC-011/012: rate-limit by caller IP.
    let ip = extract_ip(&headers, conn_info.as_ref());
    if !state.rate_limiter.check_and_record(&ip) {
        return Err(SessionError::RateLimited);
    }

    // FR-JC-005a: validate and normalise code.
    let code = validate_join_code(&req.code)?;

    let caller_id = parse_user_id(&claims)?;

    // FR-JC-009: look up session by join_code.
    let session = sessions::Entity::find()
        .filter(sessions::Column::JoinCode.eq(code.clone()))
        .one(&state.db)
        .await?
        .ok_or(SessionError::NotFound)?;

    // FR-JC-006: reject terminal sessions.
    if session.status == SessionStatus::Finished || session.status == SessionStatus::Cancelled {
        return Err(SessionError::SessionClosed);
    }

    // Idempotent re-join: if the caller already has a player row (including when
    // they are the session owner and ran `ololo start`), return the existing player_id
    // so the client can proceed to PATCH metadata without creating a duplicate row.
    let existing = players::Entity::find()
        .filter(players::Column::SessionIdFk.eq(session.id))
        .filter(players::Column::UserIdFk.eq(caller_id))
        .one(&state.db)
        .await?;
    if let Some(p) = existing {
        let git_remote_path = match crate::git_store::repos_base_dir() {
            Some(base) => match crate::git_store::provision_player_repo(&base, session.id, p.id) {
                Ok(_) => Some(format!("/git/{}/{}.git", session.id, p.id)),
                Err(e) => {
                    tracing::warn!(
                        session_id = %session.id,
                        player_id = %p.id,
                        "git repo provision (re-join) failed: {e}"
                    );
                    None
                }
            },
            None => None,
        };
        return Ok((
            StatusCode::OK,
            Json(serde_json::json!({
                "session_id": session.id,
                "player_id": p.id.to_string(),
                "git_remote_path": git_remote_path,
            })),
        )
            .into_response());
    }

    // One live session per user. Checked after the idempotent-rejoin branch
    // above, so reconnecting to the session you are already in never trips
    // it — only joining a SECOND live session does.
    ensure_single_active_session(&state.db, caller_id, Some(session.id)).await?;

    // Enforce the same live-player cap the agent-registration path uses —
    // without it an openly shared join code can flood the session (and its
    // judge queue) with an unbounded roster.
    let active = players::Entity::find()
        .filter(players::Column::SessionIdFk.eq(session.id))
        .filter(players::Column::RevokedAt.is_null())
        .count(&state.db)
        .await?;
    if active >= state.max_players_per_session as u64 {
        return Err(SessionError::SessionFull);
    }

    // FR-JC-007: insert participant row.
    let user = users::Entity::find_by_id(caller_id).one(&state.db).await?;
    let display_name = user
        .as_ref()
        .map(|u| u.display_name.clone())
        .unwrap_or_else(|| "Unknown".to_string());

    let player_id = Uuid::new_v4();
    let joined_at = Utc::now();
    let am = players::ActiveModel {
        id: Set(player_id),
        session_id_fk: Set(session.id),
        user_id_fk: Set(Some(caller_id)),
        display_name: Set(display_name.clone()),
        fingerprint: Set(None),
        metadata_json: Set(None),
        joined_at: Set(joined_at),
        reconnected_at: Set(None),
        revoked_at: Set(None),
        agent_connected: Set(false),
        agent_last_seen_at: Set(None),
    };
    am.insert(&state.db).await?;

    let _ = state.zmq_events_tx.send(ZmqEvent::PlayerJoin {
        join_code: session.join_code.clone(),
        player_id,
        display_name: display_name.clone(),
        user_id: Some(caller_id.to_string()),
        joined_at: joined_at.to_rfc3339(),
        avatar_url: user.as_ref().and_then(|u| u.avatar_url.clone()),
        fingerprint: None,
        username: user.as_ref().and_then(|u| u.username.clone()),
        version: 0,
    });

    // Add the joiner to the in-memory session cache and broadcast a
    // `MemberJoined` frame so lobby/dashboard observers see the player
    // immediately — without waiting for the ololo CLI's session-dashboard
    // WS connection (which may not yet be open, or may have dropped).
    let avatar_url = user.as_ref().and_then(|u| u.avatar_url.clone());
    let username = user.as_ref().and_then(|u| u.username.clone());
    let user_id_str = caller_id.to_string();
    if let Some(entry) = state.session_registry.get(&session.join_code) {
        let is_new = {
            let cache = entry.cache.read().unwrap_or_else(|e| e.into_inner());
            !cache.participants.iter().any(|p| p.user_id == user_id_str)
        };
        if is_new {
            let member_version = {
                let mut cache = entry.cache.write().unwrap_or_else(|e| e.into_inner());
                cache.version = cache.version.saturating_add(1);
                cache.participants.push(arena_core::protocol::MemberInfo {
                    user_id: user_id_str.clone(),
                    player_id: Some(player_id),
                    display_name: display_name.clone(),
                    joined_at: joined_at.to_rfc3339(),
                    avatar_url: avatar_url.clone(),
                    fingerprint: None,
                    username: username.clone(),
                    agent_display_name: None,
                    completion_status: None,
                });
                cache.version
            };
            let _ = entry
                .tx
                .send(arena_core::protocol::ArenaFrame::MemberJoined {
                    user_id: user_id_str,
                    player_id: Some(player_id),
                    display_name,
                    joined_at: joined_at.to_rfc3339(),
                    avatar_url,
                    fingerprint: None,
                    username,
                    agent_display_name: None,
                    version: member_version,
                });
        }
    }

    // Provision the per-player bare git repo (best-effort). On success the
    // response carries `git_remote_path` so ololo can build the push URL
    // (`{server_url}{path}`) and push task commits to the remote store.
    let git_remote_path = match crate::git_store::repos_base_dir() {
        Some(base) => match crate::git_store::provision_player_repo(&base, session.id, player_id) {
            Ok(_) => Some(format!("/git/{}/{}.git", session.id, player_id)),
            Err(e) => {
                tracing::warn!(
                    session_id = %session.id,
                    player_id = %player_id,
                    "git repo provision failed: {e}"
                );
                None
            }
        },
        None => None,
    };

    Ok((
        StatusCode::CREATED,
        Json(serde_json::json!({
            "session_id": session.id,
            "player_id": player_id.to_string(),
            "git_remote_path": git_remote_path,
        })),
    )
        .into_response())
}
