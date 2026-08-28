//! Auth module. Spec: FR-001, FR-002, FR-006, FR-007, NFR-001, NFR-002.
//!
//! Submodules:
//! - `password`: Argon2id hash + verify.
//! - `jwt`: HS256 access-token issue + verify with `purpose` claim.
//! - `refresh`: Opaque refresh-token issue + rotate (argon2-hashed at rest).
//! - `middleware`: axum extractor for access cookie + Origin allow-list guard.
//!
//! All public functions return `Result<T, AuthError>`; no `unwrap`/`expect`
//! in production paths (NFR-003).
pub mod jwt;
pub mod middleware;
pub mod password;
pub mod pat;
pub mod refresh;
pub mod turnstile;

use sea_orm::EntityTrait;

/// Whether `user_id` is an administrator. Unknown users are not admins.
pub async fn is_user_admin(
    db: &sea_orm::DatabaseConnection,
    user_id: uuid::Uuid,
) -> Result<bool, sea_orm::DbErr> {
    Ok(arena_core::entities::users::Entity::find_by_id(user_id)
        .one(db)
        .await?
        .map(|u| u.is_admin)
        .unwrap_or(false))
}

/// Unified auth error. Maps to HTTP responses via `IntoResponse`.
#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    #[error("configuration error: {0}")]
    Config(&'static str),
    #[error("invalid credentials")]
    InvalidCredentials,
    #[error("password hashing failed")]
    HashFailed,
    #[error("password hash parse failed")]
    HashParseFailed,
    #[error("token issue failed")]
    TokenIssue,
    #[error("token invalid")]
    TokenInvalid,
    #[error("token expired")]
    TokenExpired,
    #[error("token wrong purpose")]
    TokenWrongPurpose,
    #[error("missing access cookie")]
    MissingAccessCookie,
    #[error("origin not allowed")]
    OriginNotAllowed,
    #[error("user already exists")]
    UserExists,
    #[error("user not found")]
    UserNotFound,
    #[error("refresh token invalid")]
    RefreshInvalid,
    #[error("refresh token revoked or expired")]
    RefreshRevoked,
    #[error("database error")]
    Database,
    #[error("internal error")]
    Internal,
    #[error("ImageKit not configured")]
    ImageKitNotConfigured,
    #[error("username invalid")]
    InvalidUsername,
    #[error("username taken")]
    UsernameTaken,
    #[error("username generation failed")]
    UsernameGenerationFailed,
    #[error("captcha required")]
    CaptchaRequired,
    #[error("captcha invalid")]
    CaptchaInvalid,
    #[error("captcha verification error")]
    CaptchaError,
    #[error("rate limit exceeded")]
    RateLimited,
}

crate::api::error::impl_api_error!(AuthError {
    Self::Config(_) => (INTERNAL_SERVER_ERROR, "config_error"),
    Self::InvalidCredentials => (UNAUTHORIZED, "invalid_credentials"),
    Self::HashFailed | Self::HashParseFailed => (INTERNAL_SERVER_ERROR, "hash_error"),
    Self::TokenIssue => (INTERNAL_SERVER_ERROR, "token_issue_failed"),
    Self::TokenInvalid => (UNAUTHORIZED, "token_invalid"),
    Self::TokenExpired => (UNAUTHORIZED, "token_expired"),
    Self::TokenWrongPurpose => (UNAUTHORIZED, "token_wrong_purpose"),
    Self::MissingAccessCookie => (UNAUTHORIZED, "missing_access_cookie"),
    Self::OriginNotAllowed => (FORBIDDEN, "origin_not_allowed"),
    Self::UserExists => (CONFLICT, "user_exists"),
    Self::UserNotFound => (NOT_FOUND, "user_not_found"),
    Self::RefreshInvalid => (UNAUTHORIZED, "refresh_invalid"),
    Self::RefreshRevoked => (UNAUTHORIZED, "refresh_revoked"),
    Self::Database => (INTERNAL_SERVER_ERROR, "database_error"),
    Self::Internal => (INTERNAL_SERVER_ERROR, "internal_error"),
    Self::ImageKitNotConfigured => (SERVICE_UNAVAILABLE, "imagekit_not_configured"),
    Self::InvalidUsername => (UNPROCESSABLE_ENTITY, "invalid_username"),
    Self::UsernameTaken => (CONFLICT, "username_taken"),
    Self::UsernameGenerationFailed => (SERVICE_UNAVAILABLE, "username_generation_failed"),
    Self::CaptchaRequired => (FORBIDDEN, "captcha_required"),
    Self::CaptchaInvalid => (FORBIDDEN, "captcha_invalid"),
    Self::CaptchaError => (FORBIDDEN, "captcha_error"),
    Self::RateLimited => (TOO_MANY_REQUESTS, "rate_limited"),
});

impl From<sea_orm::DbErr> for AuthError {
    fn from(_: sea_orm::DbErr) -> Self {
        AuthError::Database
    }
}

impl From<crate::auth::turnstile::CaptchaError> for AuthError {
    fn from(e: crate::auth::turnstile::CaptchaError) -> Self {
        match e {
            crate::auth::turnstile::CaptchaError::Required
            | crate::auth::turnstile::CaptchaError::Unconfigured => AuthError::CaptchaRequired,
            crate::auth::turnstile::CaptchaError::Invalid => AuthError::CaptchaInvalid,
            crate::auth::turnstile::CaptchaError::Error => AuthError::CaptchaError,
        }
    }
}
