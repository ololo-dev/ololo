use crate::auth::jwt::AccessClaims;
use crate::state::AppState;
use arena_core::entities::{players, sessions, users};
use arena_core::protocol::ZmqEvent;
use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use chrono::Utc;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, DatabaseConnection, DbErr, EntityTrait, QueryFilter, Set,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

const ROLE_PARTICIPANT: &str = "participant";
const ROLE_OBSERVER: &str = "observer";
const ROLE_MEMBER_LEGACY: &str = "member";
const ROLE_VIEWER_LEGACY: &str = "viewer";

fn canonicalise_role(role: &str) -> Option<&'static str> {
    match role {
        ROLE_PARTICIPANT => Some(ROLE_PARTICIPANT),
        ROLE_OBSERVER => Some(ROLE_OBSERVER),
        ROLE_MEMBER_LEGACY => Some(ROLE_PARTICIPANT),
        ROLE_VIEWER_LEGACY => Some(ROLE_OBSERVER),
        _ => None,
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AddMemberReq {
    user_id: Uuid,
    role: String,
}

#[derive(Debug, Serialize)]
struct MemberSummary {
    session_id: Uuid,
    user_id: Uuid,
    role: String,
    joined_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Serialize)]
struct MemberListResp {
    members: Vec<MemberSummary>,
}

#[derive(Debug)]
pub enum MemberError {
    NotFound,
    Forbidden,
    InvalidRole,
    UnknownUser,
    AlreadyMember,
    CannotRemoveOwner,
    Db(DbErr),
}

impl From<DbErr> for MemberError {
    fn from(e: DbErr) -> Self {
        MemberError::Db(e)
    }
}

crate::api::error::impl_api_error!(MemberError {
    Self::NotFound => (NOT_FOUND, "not_found"),
    Self::Forbidden => (FORBIDDEN, "forbidden"),
    Self::InvalidRole => (UNPROCESSABLE_ENTITY, "invalid_role"),
    Self::UnknownUser => (UNPROCESSABLE_ENTITY, "unknown_user"),
    Self::AlreadyMember => (CONFLICT, "already_member"),
    Self::CannotRemoveOwner => (CONFLICT, "cannot_remove_owner"),
    Self::Db(_) => (INTERNAL_SERVER_ERROR, "database_error"),
});

fn parse_user_id(claims: &AccessClaims) -> Result<Uuid, MemberError> {
    claims.user_id().map_err(|_| MemberError::Forbidden)
}

fn to_summary(m: players::Model) -> MemberSummary {
    MemberSummary {
        session_id: m.session_id_fk,
        user_id: m.user_id_fk.unwrap_or(Uuid::nil()),
        role: "participant".to_string(),
        joined_at: m.joined_at,
    }
}

fn to_summary_with_role(m: players::Model, role: &str) -> MemberSummary {
    MemberSummary {
        session_id: m.session_id_fk,
        user_id: m.user_id_fk.unwrap_or(Uuid::nil()),
        role: role.to_string(),
        joined_at: m.joined_at,
    }
}

async fn load_for_owner(
    db: &DatabaseConnection,
    id: Uuid,
    user_id: Uuid,
) -> Result<sessions::Model, MemberError> {
    let row = sessions::Entity::find_by_id(id)
        .one(db)
        .await?
        .ok_or(MemberError::NotFound)?;
    match row.owner_id_fk {
        Some(owner) if owner == user_id => Ok(row),
        Some(_) => {
            let is_member = players::Entity::find()
                .filter(players::Column::SessionIdFk.eq(row.id))
                .filter(players::Column::UserIdFk.eq(user_id))
                .one(db)
                .await?
                .is_some();
            if is_member {
                Err(MemberError::Forbidden)
            } else {
                Err(MemberError::NotFound)
            }
        }
        None => Err(MemberError::Forbidden),
    }
}

async fn load_visible(
    db: &DatabaseConnection,
    id: Uuid,
    user_id: Uuid,
) -> Result<sessions::Model, MemberError> {
    let row = sessions::Entity::find_by_id(id)
        .one(db)
        .await?
        .ok_or(MemberError::NotFound)?;
    if row.owner_id_fk == Some(user_id) {
        return Ok(row);
    }
    let is_member = players::Entity::find()
        .filter(players::Column::SessionIdFk.eq(row.id))
        .filter(players::Column::UserIdFk.eq(user_id))
        .one(db)
        .await?
        .is_some();
    if is_member {
        Ok(row)
    } else {
        Err(MemberError::NotFound)
    }
}

pub async fn get_list(
    State(state): State<AppState>,
    claims: AccessClaims,
    Path(session_id): Path<Uuid>,
) -> Result<Response, MemberError> {
    let user_id = parse_user_id(&claims)?;
    let _ = load_visible(&state.db, session_id, user_id).await?;

    let rows = players::Entity::find()
        .filter(players::Column::SessionIdFk.eq(session_id))
        .filter(players::Column::RevokedAt.is_null())
        .all(&state.db)
        .await?;
    let members: Vec<MemberSummary> = rows.into_iter().map(to_summary).collect();
    Ok(Json(MemberListResp { members }).into_response())
}

pub async fn post_add(
    State(state): State<AppState>,
    claims: AccessClaims,
    Path(session_id): Path<Uuid>,
    Json(req): Json<AddMemberReq>,
) -> Result<Response, MemberError> {
    let user_id = parse_user_id(&claims)?;
    let canonical_role = canonicalise_role(req.role.as_str()).ok_or(MemberError::InvalidRole)?;

    let session = load_for_owner(&state.db, session_id, user_id).await?;

    let target = users::Entity::find_by_id(req.user_id)
        .one(&state.db)
        .await?
        .ok_or(MemberError::UnknownUser)?;

    let existing = players::Entity::find()
        .filter(players::Column::SessionIdFk.eq(session.id))
        .filter(players::Column::UserIdFk.eq(req.user_id))
        .filter(players::Column::RevokedAt.is_null())
        .one(&state.db)
        .await?;
    if existing.is_some() {
        return Err(MemberError::AlreadyMember);
    }

    let model = players::ActiveModel {
        id: Set(Uuid::new_v4()),
        session_id_fk: Set(session.id),
        user_id_fk: Set(Some(req.user_id)),
        display_name: Set(target.display_name),
        fingerprint: Set(None),
        metadata_json: Set(None),
        joined_at: Set(Utc::now()),
        reconnected_at: Set(None),
        revoked_at: Set(None),
        agent_connected: Set(false),
        agent_last_seen_at: Set(None),
    }
    .insert(&state.db)
    .await?;
    Ok((
        StatusCode::CREATED,
        Json(to_summary_with_role(model, canonical_role)),
    )
        .into_response())
}

pub async fn delete_one(
    State(state): State<AppState>,
    claims: AccessClaims,
    Path((session_id, target_user_id)): Path<(Uuid, Uuid)>,
) -> Result<Response, MemberError> {
    let user_id = parse_user_id(&claims)?;
    let session = load_for_owner(&state.db, session_id, user_id).await?;

    if session.owner_id_fk == Some(target_user_id) {
        return Err(MemberError::CannotRemoveOwner);
    }

    let owned = players::Entity::find()
        .filter(players::Column::SessionIdFk.eq(session.id))
        .filter(players::Column::UserIdFk.eq(target_user_id))
        .filter(players::Column::RevokedAt.is_null())
        .all(&state.db)
        .await?;

    if owned.is_empty() {
        return Err(MemberError::NotFound);
    }

    let now = Utc::now();
    for row in owned {
        let player_id = row.id;
        let mut am: players::ActiveModel = row.into();
        am.revoked_at = Set(Some(now));
        am.update(&state.db).await?;
        let _ = state.zmq_events_tx.send(ZmqEvent::PlayerLeave {
            join_code: session.join_code.clone(),
            player_id,
            version: 0,
        });
    }

    Ok(StatusCode::NO_CONTENT.into_response())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn role_validation_accepts_contract_and_legacy() {
        assert_eq!(canonicalise_role("participant"), Some("participant"));
        assert_eq!(canonicalise_role("observer"), Some("observer"));
        assert_eq!(canonicalise_role("member"), Some("participant"));
        assert_eq!(canonicalise_role("viewer"), Some("observer"));
        assert_eq!(canonicalise_role("admin"), None);
        assert_eq!(canonicalise_role(""), None);
    }
}
