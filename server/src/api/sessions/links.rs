//! Linked sessions — `GET /api/sessions/by-code/:join_code/links`,
//! `POST /api/sessions/:id/links`, `DELETE /api/sessions/:id/links`.
//!
//! A host runs a few sessions as one event and wants them read together:
//! each session's page names the others, the board flips between them, and
//! once every one is over there is one standings for the lot. The link
//! itself lives in `arena_core::session_links`; this module is the door.

use crate::auth::jwt::AccessClaims;
use crate::state::AppState;
use arena_core::entities::{projects, sessions};
use arena_core::session_links::{self, CombinedEntry};
use arena_core::session_status::SessionStatus;
use axum::Json;
use axum::extract::{ConnectInfo, Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::net::SocketAddr;
use uuid::Uuid;

use super::common::*;

/// One session of a linked group, as the pages name it.
#[derive(Debug, Clone, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LinkedSessionRow {
    pub id: Uuid,
    pub join_code: String,
    pub name: String,
    pub status: String,
    pub project_id: Uuid,
    pub project_name: Option<String>,
    pub project_slug: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub started_at: Option<chrono::DateTime<chrono::Utc>>,
    pub finished_at: Option<chrono::DateTime<chrono::Utc>>,
    pub player_count: u32,
    /// The session the request was about.
    pub current: bool,
}

/// `GET /api/sessions/by-code/:join_code/links` — the group this session is
/// linked into: the sessions, in the order they were linked, and the
/// combined standings across them. Public, like the by-code lookup;
/// 204 when the session is linked to nothing.
#[derive(Debug, Clone, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SessionLinksResp {
    pub group_id: Uuid,
    pub sessions: Vec<LinkedSessionRow>,
    /// Every session in the group has ended (finished or cancelled) — the
    /// moment the combined standings are the final word.
    pub all_finished: bool,
    /// The combined standings so far — points added up by account across
    /// the sessions. Pages show it as final once `all_finished`.
    pub leaderboard: Vec<CombinedEntry>,
}

/// What `POST /api/sessions/:id/links` takes: the join code of the session
/// to link with this one.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LinkSessionReq {
    pub join_code: String,
}

pub async fn get_links_by_code(
    conn_info: Option<ConnectInfo<SocketAddr>>,
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(code): Path<String>,
) -> Result<Response, SessionError> {
    let session = find_public_by_code(&state, &headers, conn_info.as_ref(), &code).await?;
    links_response(&state.db, &session).await
}

/// `POST /api/sessions/:id/links` — link this session with the one the join
/// code names. The caller must host both (or be an admin): a link is a
/// claim on the other session's standings too. Both run the same project —
/// a group is one brief in several rooms. Answers with the group.
pub async fn post_link(
    State(state): State<AppState>,
    claims: AccessClaims,
    Path(id): Path<Uuid>,
    Json(req): Json<LinkSessionReq>,
) -> Result<Response, SessionError> {
    let user_id = parse_user_id(&claims)?;
    let is_admin = crate::auth::is_user_admin(&state.db, user_id).await?;
    let session = load_hosted(&state.db, id, user_id, is_admin).await?;
    let code = validate_join_code(&req.join_code)?;
    let other = sessions::Entity::find()
        .filter(sessions::Column::JoinCode.eq(code))
        .one(&state.db)
        .await?
        .ok_or(SessionError::NotFound)?;
    if other.id == session.id {
        return Err(SessionError::LinkToSelf);
    }
    if !is_admin && other.owner_id_fk != Some(user_id) {
        return Err(SessionError::Forbidden);
    }
    if other.project_id_fk != session.project_id_fk {
        return Err(SessionError::LinkAcrossProjects);
    }
    session_links::link(&state.db, session.id, other.id).await?;
    links_response(&state.db, &session).await
}

/// `DELETE /api/sessions/:id/links` — take this session out of its group.
/// Host or admin. 204 either way, linked or not.
pub async fn delete_link(
    State(state): State<AppState>,
    claims: AccessClaims,
    Path(id): Path<Uuid>,
) -> Result<Response, SessionError> {
    let user_id = parse_user_id(&claims)?;
    let is_admin = crate::auth::is_user_admin(&state.db, user_id).await?;
    let session = load_hosted(&state.db, id, user_id, is_admin).await?;
    session_links::unlink(&state.db, session.id).await?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

async fn load_hosted(
    db: &DatabaseConnection,
    id: Uuid,
    user_id: Uuid,
    is_admin: bool,
) -> Result<sessions::Model, SessionError> {
    if is_admin {
        load_any(db, id).await
    } else {
        load_for_owner(db, id, user_id).await
    }
}

/// The group as the pages read it, or 204 when the session is in none.
async fn links_response(
    db: &DatabaseConnection,
    session: &sessions::Model,
) -> Result<Response, SessionError> {
    let Some(group_id) = session_links::group_of(db, session.id).await? else {
        return Ok(StatusCode::NO_CONTENT.into_response());
    };
    let ids = session_links::sessions_in_group(db, group_id).await?;
    let mut rows = sessions::Entity::find()
        .filter(sessions::Column::Id.is_in(ids.iter().copied()))
        .all(db)
        .await?;
    // Keep the order the sessions were linked in.
    let pos: HashMap<Uuid, usize> = ids.iter().enumerate().map(|(i, id)| (*id, i)).collect();
    rows.sort_by_key(|s| pos.get(&s.id).copied().unwrap_or(usize::MAX));

    let project_ids: Vec<Uuid> = rows.iter().map(|s| s.project_id_fk).collect();
    let project_by_id: HashMap<Uuid, projects::Model> = projects::Entity::find()
        .filter(projects::Column::Id.is_in(project_ids))
        .all(db)
        .await?
        .into_iter()
        .map(|p| (p.id, p))
        .collect();
    let session_ids: Vec<Uuid> = rows.iter().map(|s| s.id).collect();
    let counts = arena_core::scoring::player_counts(db, &session_ids).await?;

    let all_finished = rows
        .iter()
        .all(|s| matches!(s.status, SessionStatus::Finished | SessionStatus::Cancelled));
    let leaderboard = session_links::combined_leaderboard(db, &session_ids).await?;

    let sessions = rows
        .into_iter()
        .map(|s| {
            let project = project_by_id.get(&s.project_id_fk);
            LinkedSessionRow {
                current: s.id == session.id,
                player_count: counts.get(&s.id).copied().unwrap_or(0),
                project_name: project.map(|p| p.name.clone()),
                project_slug: project.and_then(|p| p.slug.clone()),
                id: s.id,
                join_code: s.join_code,
                name: s.name,
                status: s.status.to_string(),
                project_id: s.project_id_fk,
                created_at: s.created_at,
                started_at: s.started_at,
                finished_at: s.finished_at,
            }
        })
        .collect();

    Ok(Json(SessionLinksResp {
        group_id,
        sessions,
        all_finished,
        leaderboard,
    })
    .into_response())
}
