use crate::auth::middleware::ACCESS_COOKIE;
use crate::auth::middleware::extract_cookie;
use crate::state::AppState;
use arena_core::entities::game_servers;
use axum::Json;
use axum::extract::{FromRequestParts, Path, State};
use axum::http::request::Parts;
use chrono::{DateTime, Utc};
use sea_orm::{ActiveModelTrait, EntityTrait, QueryOrder, Set};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A game server is presumed dead when its last heartbeat is older than this.
/// Heartbeats land every ~5s, so 30s tolerates several missed beats before the
/// server is excluded from session assignment and `resolve`.
pub(crate) const STALE_AFTER_SECS: i64 = 30;

/// Heartbeats at or before this instant are stale (server presumed dead).
pub(crate) fn stale_cutoff() -> DateTime<Utc> {
    Utc::now() - chrono::Duration::seconds(STALE_AFTER_SECS)
}

pub struct AdminUser {
    pub id: Uuid,
}

#[derive(Debug, thiserror::Error)]
pub enum GameServerError {
    #[error("unauthorized")]
    Unauthorized,
    #[error("forbidden")]
    Forbidden,
    #[error("game server not found")]
    NotFound,
    #[error("invalid status — must be active, inactive, or draining")]
    InvalidStatus,
    #[error("display name too long — maximum 255 characters")]
    DisplayNameTooLong,
    #[error("server has active sessions")]
    ServerHasActiveSessions { active_sessions: i32 },
    #[error("database error")]
    Database,
}

crate::api::error::impl_api_error!(GameServerError {
    Self::Unauthorized => (UNAUTHORIZED, "unauthorized"),
    Self::Forbidden => (FORBIDDEN, "forbidden"),
    Self::NotFound => (NOT_FOUND, "game_server_not_found"),
    Self::InvalidStatus => (UNPROCESSABLE_ENTITY, "invalid_status"),
    Self::DisplayNameTooLong => (UNPROCESSABLE_ENTITY, "display_name_too_long"),
    Self::ServerHasActiveSessions { active_sessions } => (
        UNPROCESSABLE_ENTITY,
        "server_has_active_sessions",
        "active_sessions": active_sessions,
    ),
    Self::Database => (INTERNAL_SERVER_ERROR, "database_error"),
});

#[axum::async_trait]
impl FromRequestParts<AppState> for AdminUser {
    type Rejection = GameServerError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let token = extract_cookie(parts, ACCESS_COOKIE).ok_or(GameServerError::Unauthorized)?;
        let claims = crate::auth::jwt::verify_access_token(&state.jwt_decoding_key, &token)
            .map_err(|_| GameServerError::Unauthorized)?;

        let user_id = claims
            .user_id()
            .map_err(|_| GameServerError::Unauthorized)?;

        let user = arena_core::entities::users::Entity::find_by_id(user_id)
            .one(&state.db)
            .await
            .map_err(|_| GameServerError::Database)?
            .ok_or(GameServerError::Unauthorized)?;

        if !user.is_admin {
            return Err(GameServerError::Forbidden);
        }

        Ok(AdminUser { id: user.id })
    }
}

const ALLOWED_STATUSES: &[&str] = &["active", "inactive", "draining"];

#[derive(Serialize)]
pub struct GameServerDto {
    pub id: Uuid,
    pub url: String,
    pub zmq_url: Option<String>,
    pub display_name: Option<String>,
    pub capacity: i32,
    pub active_sessions: i32,
    pub status: String,
    pub last_heartbeat: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl From<game_servers::Model> for GameServerDto {
    fn from(m: game_servers::Model) -> Self {
        GameServerDto {
            id: m.id,
            url: m.url,
            zmq_url: m.zmq_url,
            display_name: m.display_name,
            capacity: m.capacity,
            active_sessions: m.active_sessions,
            status: m.status,
            last_heartbeat: m.last_heartbeat,
            created_at: m.created_at,
            updated_at: m.updated_at,
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PatchGameServerBody {
    pub display_name: Option<String>,
    pub status: Option<String>,
    pub clear_display_name: Option<bool>,
}

pub async fn get_list(
    _admin: AdminUser,
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, GameServerError> {
    let rows = game_servers::Entity::find()
        .order_by_asc(game_servers::Column::CreatedAt)
        .all(&state.db)
        .await
        .map_err(|_| GameServerError::Database)?;

    let dtos: Vec<GameServerDto> = rows.into_iter().map(GameServerDto::from).collect();
    Ok(Json(serde_json::json!({ "servers": dtos })))
}

pub async fn patch_one(
    _admin: AdminUser,
    State(state): State<AppState>,
    Path(server_id): Path<Uuid>,
    Json(body): Json<PatchGameServerBody>,
) -> Result<Json<serde_json::Value>, GameServerError> {
    if let Some(ref s) = body.status
        && !ALLOWED_STATUSES.contains(&s.as_str())
    {
        return Err(GameServerError::InvalidStatus);
    }

    let model = game_servers::Entity::find_by_id(server_id)
        .one(&state.db)
        .await
        .map_err(|_| GameServerError::Database)?
        .ok_or(GameServerError::NotFound)?;

    if let Some(ref s) = body.status
        && s == "inactive"
        && model.active_sessions > 0
    {
        return Err(GameServerError::ServerHasActiveSessions {
            active_sessions: model.active_sessions,
        });
    }

    let mut am: game_servers::ActiveModel = model.into();

    if body.clear_display_name == Some(true) {
        am.display_name = Set(None);
    } else if let Some(ref name) = body.display_name {
        if name.len() > 255 {
            return Err(GameServerError::DisplayNameTooLong);
        }
        am.display_name = Set(Some(name.clone()));
    }

    if let Some(ref s) = body.status {
        am.status = Set(s.clone());
    }

    let has_changes =
        body.clear_display_name.is_some() || body.display_name.is_some() || body.status.is_some();
    if has_changes {
        am.updated_at = Set(chrono::Utc::now());
    }

    let updated: game_servers::Model = am
        .update(&state.db)
        .await
        .map_err(|_| GameServerError::Database)?;
    let dto = GameServerDto::from(updated);
    Ok(Json(serde_json::json!({ "server": dto })))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::StatusCode;
    use axum::response::IntoResponse;

    #[test]
    fn game_server_error_not_found_is_404() {
        let resp = GameServerError::NotFound.into_response();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[test]
    fn game_server_error_invalid_status_is_422() {
        let resp = GameServerError::InvalidStatus.into_response();
        assert_eq!(resp.status(), StatusCode::UNPROCESSABLE_ENTITY);
    }

    #[test]
    fn game_server_error_server_has_active_sessions_is_422() {
        let resp = GameServerError::ServerHasActiveSessions { active_sessions: 5 }.into_response();
        assert_eq!(resp.status(), StatusCode::UNPROCESSABLE_ENTITY);
    }

    #[test]
    fn game_server_error_unauthorized_is_401() {
        let resp = GameServerError::Unauthorized.into_response();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[test]
    fn game_server_error_forbidden_is_403() {
        let resp = GameServerError::Forbidden.into_response();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }

    #[test]
    fn allowed_statuses_contains_valid_values() {
        assert!(ALLOWED_STATUSES.contains(&"active"));
        assert!(ALLOWED_STATUSES.contains(&"inactive"));
        assert!(ALLOWED_STATUSES.contains(&"draining"));
        assert!(!ALLOWED_STATUSES.contains(&"retired"));
    }

    #[test]
    fn game_server_error_display_name_too_long_is_422() {
        let resp = GameServerError::DisplayNameTooLong.into_response();
        assert_eq!(resp.status(), StatusCode::UNPROCESSABLE_ENTITY);
    }
}
