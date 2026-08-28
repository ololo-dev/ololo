//! Shared settings types, errors, and the Ollama HTTP abstraction.
//!
//! Public surface re-exported by `mod.rs`:
//! - [`OllamaHttp`], [`OllamaHttpHandle`], [`OllamaClientError`], [`ReqwestOllamaHttp`]
//! - [`AdminUser`], [`SettingsError`]
//! - request bodies, [`AdminUserDto`]
//! - [`is_valid_bool_value`]

use crate::auth::middleware::{ACCESS_COOKIE, extract_bearer, extract_cookie};
use crate::state::AppState;
use arena_core::entities::users;
use async_trait::async_trait;
use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use sea_orm::EntityTrait;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Ollama HTTP trait (Commandment 2: DI)
// ---------------------------------------------------------------------------

/// Abstraction over Ollama's `/api/tags` endpoint.
/// Production: [`ReqwestOllamaHttp`]. Tests: stub implementing this trait.
#[async_trait]
pub trait OllamaHttp: Send + Sync {
    async fn list_models(&self) -> Result<Vec<String>, OllamaClientError>;
}

/// Type alias stored in [`AppState`].
pub type OllamaHttpHandle = Arc<dyn OllamaHttp>;

/// Error from the Ollama HTTP client.
#[derive(Debug, thiserror::Error)]
pub enum OllamaClientError {
    #[error("Ollama unreachable")]
    Unreachable,
    #[error("Ollama response error")]
    BadResponse,
}

/// Production [`OllamaHttp`] implementation. Reads `OLLAMA_URL` from the
/// environment (default: `http://localhost:11434`).
pub struct ReqwestOllamaHttp {
    base_url: String,
}

impl ReqwestOllamaHttp {
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
        }
    }

    /// Construct from `OLLAMA_URL` env var, falling back to localhost.
    pub fn from_env() -> Self {
        let base_url =
            std::env::var("OLLAMA_URL").unwrap_or_else(|_| "http://localhost:11434".to_string());
        Self::new(base_url)
    }
}

#[async_trait]
impl OllamaHttp for ReqwestOllamaHttp {
    async fn list_models(&self) -> Result<Vec<String>, OllamaClientError> {
        let url = format!("{}/api/tags", self.base_url.trim_end_matches('/'));
        let resp = reqwest::Client::new()
            .get(&url)
            .timeout(std::time::Duration::from_secs(5))
            .send()
            .await
            .map_err(|_| OllamaClientError::Unreachable)?
            .error_for_status()
            .map_err(|_| OllamaClientError::BadResponse)?;

        #[derive(serde::Deserialize)]
        struct OllamaModel {
            name: String,
        }
        #[derive(serde::Deserialize)]
        struct OllamaTagsResponse {
            models: Vec<OllamaModel>,
        }

        let body: OllamaTagsResponse = resp
            .json()
            .await
            .map_err(|_| OllamaClientError::BadResponse)?;
        Ok(body.models.into_iter().map(|m| m.name).collect())
    }
}

// ---------------------------------------------------------------------------
// AdminUser extractor (FR-008, FR-009)
// ---------------------------------------------------------------------------

/// Axum extractor that verifies the caller is an authenticated admin.
///
/// - 401 if the access cookie is absent, invalid, or expired.
/// - 403 if the authenticated user does not have `is_admin = true`.
pub struct AdminUser {
    pub id: Uuid,
}

/// Errors that can be returned from admin-only endpoints.
#[derive(Debug, thiserror::Error)]
pub enum SettingsError {
    #[error("unauthorized")]
    Unauthorized,
    #[error("forbidden")]
    Forbidden,
    #[error("unknown settings key")]
    UnknownKey,
    #[error("invalid project creation value — must be \"true\" or \"false\"")]
    InvalidProjectCreationValue,
    #[error("database error")]
    Database,
    #[error("email already taken")]
    EmailTaken,
    #[error("user not found")]
    UserNotFound,
    #[error("cannot delete your own account")]
    CannotDeleteSelf,
    #[error("user has related records and cannot be deleted")]
    HasRelatedRecords,
    #[error("sentinel value not allowed")]
    SentinelNotAllowed,
    #[error("username already taken")]
    UsernameTaken,
    #[error("invalid username")]
    InvalidUsername,
    #[error("invalid arena points value — must be a non-negative integer")]
    InvalidArenaPointsValue,
    #[error("invalid email provider — must be \"ses\" or \"cloudflare\"")]
    InvalidEmailProvider,
    #[error("invalid plan — must be \"free\" or \"premium\"")]
    InvalidPlan,
    #[error("invalid plan limit — must be a non-negative integer")]
    InvalidPlanLimitValue,
    #[error("invalid plan price — a short one-line label, 32 characters max")]
    InvalidPlanPriceValue,
}

crate::api::error::impl_api_error!(SettingsError {
    Self::Unauthorized => (UNAUTHORIZED, "unauthorized"),
    Self::Forbidden => (FORBIDDEN, "forbidden"),
    Self::UnknownKey => (BAD_REQUEST, "unknown_key"),
    Self::InvalidProjectCreationValue => (UNPROCESSABLE_ENTITY, "invalid_project_creation_value"),
    Self::Database => (INTERNAL_SERVER_ERROR, "database_error"),
    Self::EmailTaken => (CONFLICT, "email_taken"),
    Self::UserNotFound => (NOT_FOUND, "user_not_found"),
    Self::CannotDeleteSelf => (FORBIDDEN, "cannot_delete_self"),
    Self::HasRelatedRecords => (CONFLICT, "has_related_records"),
    Self::UsernameTaken => (CONFLICT, "username_taken"),
    Self::InvalidUsername => (UNPROCESSABLE_ENTITY, "invalid_username"),
    Self::SentinelNotAllowed => (BAD_REQUEST, "sentinel_value_not_allowed"),
    Self::InvalidArenaPointsValue => (UNPROCESSABLE_ENTITY, "invalid_arena_points_value"),
    Self::InvalidEmailProvider => (BAD_REQUEST, "invalid_email_provider"),
    Self::InvalidPlan => (UNPROCESSABLE_ENTITY, "invalid_plan"),
    Self::InvalidPlanLimitValue => (UNPROCESSABLE_ENTITY, "invalid_plan_limit_value"),
    Self::InvalidPlanPriceValue => (UNPROCESSABLE_ENTITY, "invalid_plan_price_value"),
});

#[axum::async_trait]
impl FromRequestParts<AppState> for AdminUser {
    type Rejection = SettingsError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        // Same token sources as the AccessClaims extractor: `Authorization:
        // Bearer` (access JWT or `ololo_` PAT) preferred over the HttpOnly
        // access cookie. PAT acceptance is what lets non-browser tooling (e.g.
        // the push-seeds command) drive the admin API — the is_admin check
        // below still gates every caller.
        let token = extract_bearer(parts)
            .or_else(|| extract_cookie(parts, ACCESS_COOKIE))
            .ok_or(SettingsError::Unauthorized)?;
        let claims = if token.starts_with("ololo_") {
            crate::auth::pat::lookup_pat(&state.db, &token).await
        } else {
            crate::auth::jwt::verify_access_token(&state.jwt_decoding_key, &token)
        }
        .map_err(|_| SettingsError::Unauthorized)?;

        let user_id = claims.user_id().map_err(|_| SettingsError::Unauthorized)?;

        let user = users::Entity::find_by_id(user_id)
            .one(&state.db)
            .await
            .map_err(|_| SettingsError::Database)?
            .ok_or(SettingsError::Unauthorized)?;

        if !user.is_admin {
            return Err(SettingsError::Forbidden);
        }

        Ok(AdminUser { id: user.id })
    }
}

// ---------------------------------------------------------------------------
// Shared request/response types
// ---------------------------------------------------------------------------

/// Request body for `PUT /api/admin/settings`.
#[derive(Deserialize)]
pub struct PutSettingsBody {
    pub key: String,
    pub value: String,
}

/// Public user record safe to expose to admins (no password hash).
#[derive(Serialize)]
pub struct AdminUserDto {
    pub id: Uuid,
    pub email: String,
    pub display_name: String,
    pub username: Option<String>,
    pub is_admin: bool,
    pub avatar_url: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    /// Account plan: `"free"` or `"premium"`.
    pub plan: String,
    /// Per-user monthly judge-run limit override; `null` = tier limit applies.
    pub judge_run_limit: Option<i32>,
    /// The limit actually in force (override, else the plan's tier limit).
    pub judge_run_limit_effective: i64,
    /// Purchased pack credits still unspent.
    pub judge_run_credits: i64,
    /// Metered judge runs charged to this account this calendar month.
    pub judge_runs_this_month: i64,
}

/// Request body for `POST /api/admin/users`.
#[derive(Deserialize)]
pub struct CreateAdminUserBody {
    pub email: String,
    pub display_name: String,
    pub username: Option<String>,
    pub password: String,
    pub is_admin: bool,
    /// Account plan; omitted → `"premium"`.
    pub plan: Option<String>,
}

/// Request body for `PATCH /api/admin/users/:id`.
#[derive(Deserialize)]
pub struct UpdateAdminUserBody {
    pub email: Option<String>,
    pub display_name: Option<String>,
    pub username: Option<String>,
    pub is_admin: Option<bool>,
    pub password: Option<String>,
    /// `"free"` or `"premium"`.
    pub plan: Option<String>,
    /// Absent = leave unchanged; `null` = clear the override (tier limit
    /// applies); a number = set the per-user override.
    #[serde(default, deserialize_with = "deserialize_double_option")]
    pub judge_run_limit: Option<Option<i32>>,
    /// Delta added to the purchased-credits balance (a pack sold, or a
    /// negative correction). The result must not go below zero.
    pub grant_judge_run_credits: Option<i64>,
}

/// Distinguishes an absent JSON field (outer `None`) from an explicit
/// `null` (`Some(None)`): serde only calls this when the field is present.
fn deserialize_double_option<'de, T, D>(de: D) -> Result<Option<Option<T>>, D::Error>
where
    T: Deserialize<'de>,
    D: serde::Deserializer<'de>,
{
    Option::<T>::deserialize(de).map(Some)
}

/// Whether an `app_settings` key stores a secret: encrypted at rest,
/// redacted on read, and sentinel-protected on write. Provider api-key
/// settings come from the registry, so new providers are covered
/// automatically; the email-sending secrets (SES secret key, Cloudflare
/// API token) are the non-provider secrets.
pub fn is_secret_setting(key: &str) -> bool {
    key == "email.secret_access_key"
        || key == "email.cloudflare_api_token"
        || arena_core::llm::PROVIDER_REGISTRY
            .iter()
            .any(|s| !s.api_key_setting.is_empty() && s.api_key_setting == key)
}

/// Returns `true` iff `val` is a case-insensitive `"true"` or `"false"`.
/// Used to validate the `allow_user_project_creation` setting.
pub(crate) fn is_valid_bool_value(val: &str) -> bool {
    matches!(val.to_ascii_lowercase().as_str(), "true" | "false")
}
