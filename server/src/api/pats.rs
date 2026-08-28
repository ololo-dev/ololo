use crate::auth::jwt::AccessClaims;
use crate::state::AppState;
use arena_core::entities::cli_tokens;
use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use chrono::{DateTime, Utc};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use serde::Serialize;
use uuid::Uuid;

/// Error type for PAT handlers.
#[derive(Debug, thiserror::Error)]
pub enum PatError {
    #[error("not found")]
    NotFound,
    #[error("database error")]
    Database,
}

impl From<sea_orm::DbErr> for PatError {
    fn from(_: sea_orm::DbErr) -> Self {
        PatError::Database
    }
}

crate::api::error::impl_api_error!(PatError {
    Self::NotFound => (NOT_FOUND, "not_found"),
    Self::Database => (INTERNAL_SERVER_ERROR, "database_error"),
});

/// Envelope for the PAT list response.
#[derive(Debug, Serialize)]
pub struct PatListResponse {
    pats: Vec<PatDto>,
}

/// Public DTO for a PAT. The raw `token_hash` is never included.
#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct PatDto {
    pub id: Uuid,
    /// First 8 lowercase hex characters of the stored `token_hash`.
    pub fingerprint: String,
    pub created_at: DateTime<Utc>,
    pub expires_at: Option<DateTime<Utc>>,
}

/// Compute the fingerprint: first 8 chars of the lowercase hex `token_hash`.
fn fingerprint(token_hash: &str) -> String {
    token_hash.to_lowercase().chars().take(8).collect()
}

/// `GET /api/users/me/pats` — list all PATs for the authenticated user.
#[tracing::instrument(level = "info", skip_all)]
pub async fn get_list(
    claims: AccessClaims,
    State(state): State<AppState>,
) -> Result<Json<PatListResponse>, PatError> {
    let user_id = claims.user_id().map_err(|_| PatError::Database)?;
    let rows = cli_tokens::Entity::find()
        .filter(cli_tokens::Column::UserId.eq(user_id))
        .all(&state.db)
        .await?;
    let pats = rows
        .into_iter()
        .map(|row| PatDto {
            id: row.id,
            fingerprint: fingerprint(&row.token_hash),
            created_at: row.created_at.into(),
            expires_at: Some(row.expires_at.into()),
        })
        .collect();
    Ok(Json(PatListResponse { pats }))
}

/// `DELETE /api/users/me/pats/:id` — revoke a PAT owned by the caller.
///
/// Uses a single atomic DELETE WHERE id = $1 AND user_id = $2.
/// Returns 404 when the token does not exist or is not owned by the caller.
#[tracing::instrument(level = "info", skip_all)]
pub async fn delete_one(
    claims: AccessClaims,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, PatError> {
    let user_id = claims.user_id().map_err(|_| PatError::Database)?;
    let result = cli_tokens::Entity::delete_many()
        .filter(cli_tokens::Column::Id.eq(id))
        .filter(cli_tokens::Column::UserId.eq(user_id))
        .exec(&state.db)
        .await?;
    if result.rows_affected == 0 {
        return Err(PatError::NotFound);
    }
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::response::IntoResponse;

    #[test]
    fn fingerprint_takes_first_8_lowercase_hex_chars() {
        // Normal case: first 8 chars extracted, already lowercase.
        assert_eq!(fingerprint("a3f2c819beef1234567890abcdef"), "a3f2c819");
    }

    #[test]
    fn fingerprint_lowercases_uppercase_input() {
        // token_hash stored as uppercase hex: fingerprint must be lowercase.
        assert_eq!(fingerprint("A3F2C819BEEF1234"), "a3f2c819");
    }

    #[test]
    fn fingerprint_short_hash_returns_all_chars() {
        // Defensive: if hash is shorter than 8, return all available chars.
        assert_eq!(fingerprint("abc"), "abc");
    }

    #[test]
    fn pat_error_not_found_maps_to_404() {
        let resp = PatError::NotFound.into_response();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[test]
    fn pat_error_database_maps_to_500() {
        let resp = PatError::Database.into_response();
        assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[test]
    fn pat_dto_does_not_serialize_token_hash() {
        let dto = PatDto {
            id: Uuid::nil(),
            fingerprint: "a3f2c819".to_string(),
            created_at: DateTime::<Utc>::from_timestamp(0, 0).unwrap(),
            expires_at: None,
        };
        let json = serde_json::to_string(&dto).unwrap();
        assert!(!json.contains("token_hash"));
        assert!(json.contains("fingerprint"));
        assert!(json.contains("a3f2c819"));
    }
}
