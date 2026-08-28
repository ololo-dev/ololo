use crate::api::settings::is_project_creation_allowed;
use crate::auth::AuthError;
use crate::auth::jwt::AccessClaims;
use crate::auth::password::{hash_password, verify_password};
use crate::state::AppState;
use arena_core::entities::users;
use arena_core::validation::username::validate_username;
use axum::Json;
use axum::extract::State;
use chrono::Utc;
use hmac::{Hmac, Mac};
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, Set};
use serde::{Deserialize, Serialize};
use sha1::Sha1;
use uuid::Uuid;

#[derive(Serialize)]
pub struct MeDto {
    pub id: Uuid,
    pub email: String,
    pub display_name: String,
    pub is_admin: bool,
    /// Avatar URL (ImageKit delivery URL) or `null` when not set (FR-005).
    pub avatar_url: Option<String>,
    /// Whether the server allows non-admin users to create projects (FR-001).
    /// Sourced from `app_settings.allow_user_project_creation`; absent row → `false`.
    pub allow_project_creation: bool,
    /// FR-009 / AC-014: email verified flag for advisory banner.
    pub email_verified: bool,
    /// Unique username. Empty string during migration window until backfill runs.
    pub username: String,
    /// Account plan: `"free"` or `"premium"`.
    pub plan: String,
    /// Metered judge runs charged to this account this calendar month.
    pub judge_runs_used: i64,
    /// This month's judge-run limit (per-user override or the plan's tier limit).
    pub judge_run_limit: i64,
    /// Purchased pack credits still unspent (consumed after the monthly limit).
    pub judge_run_credits: i64,
    /// Whether account tiers are enforced on this instance. Off → the quota
    /// fields above are informational and no run is ever denied.
    pub plans_enabled: bool,
    /// Whether the session replay bar is offered on this instance. It is an
    /// admin tool either way; this says whether the instance offers it at all.
    pub session_replay_enabled: bool,
}

/// Assemble the `MeDto` for `user` — shared by `get_me` and `patch_me`.
async fn build_me_dto(state: &AppState, user: users::Model) -> Result<MeDto, AuthError> {
    let allow_project_creation = is_project_creation_allowed(&state.db)
        .await
        .unwrap_or(false);
    let quota = arena_core::quota::judge_quota_for_user(&state.db, &user).await?;
    let plans_enabled = arena_core::quota::plans_enabled(&state.db).await?;
    // Absent row means "never configured", which is on — the switch exists to
    // turn the replay off, not to have to turn it on.
    let session_replay_enabled = crate::api::settings::session_replay_enabled(
        arena_core::entities::app_settings::Entity::find_by_id(
            crate::api::settings::SESSION_REPLAY_KEY.to_string(),
        )
        .one(&state.db)
        .await
        .ok()
        .flatten()
        .as_ref()
        .map(|row| row.value.as_str()),
    );
    Ok(MeDto {
        id: user.id,
        email: user.email,
        display_name: user.display_name,
        is_admin: user.is_admin,
        avatar_url: user.avatar_url,
        allow_project_creation,
        email_verified: user.email_verified,
        username: user.username.unwrap_or_default(),
        plan: user.plan,
        judge_runs_used: quota.used,
        judge_run_limit: quota.limit,
        judge_run_credits: quota.credits,
        plans_enabled,
        session_replay_enabled,
    })
}

#[derive(Deserialize)]
pub struct PatchMeRequest {
    pub display_name: Option<String>,
    /// ImageKit delivery URL to persist as the user's avatar (FR-004).
    /// Validated server-side: must be HTTPS and host must match the
    /// configured `IMAGEKIT_URL_ENDPOINT` host (NFR-009).
    pub avatar_url: Option<String>,
    /// New unique username. Validated against username rules; 409 if taken.
    pub username: Option<String>,
}

#[tracing::instrument(level = "info", skip_all)]
pub async fn get_me(
    claims: AccessClaims,
    State(state): State<AppState>,
) -> Result<Json<MeDto>, AuthError> {
    let user_id = claims.user_id().map_err(|_| AuthError::UserNotFound)?;
    let user = users::Entity::find_by_id(user_id)
        .one(&state.db)
        .await?
        .ok_or(AuthError::UserNotFound)?;
    Ok(Json(build_me_dto(&state, user).await?))
}

#[tracing::instrument(level = "info", skip_all)]
pub async fn patch_me(
    claims: AccessClaims,
    State(state): State<AppState>,
    Json(req): Json<PatchMeRequest>,
) -> Result<Json<MeDto>, AuthError> {
    let user_id = claims.user_id().map_err(|_| AuthError::UserNotFound)?;
    let user = users::Entity::find_by_id(user_id)
        .one(&state.db)
        .await?
        .ok_or(AuthError::UserNotFound)?;

    let mut active: users::ActiveModel = user.into();

    if let Some(raw) = req.display_name {
        let trimmed = raw.trim().to_string();
        if trimmed.is_empty() || trimmed.len() > 80 {
            return Err(AuthError::InvalidCredentials);
        }
        active.display_name = Set(trimmed);
    }

    if let Some(url_str) = req.avatar_url {
        validate_avatar_url(&url_str, &state)?;
        active.avatar_url = Set(Some(url_str));
    }

    if let Some(raw_username) = req.username {
        let trimmed = raw_username.trim().to_string();
        validate_username(&trimmed).map_err(|_| AuthError::InvalidUsername)?;
        let conflict = users::Entity::find()
            .filter(users::Column::Username.eq(trimmed.clone()))
            .filter(users::Column::Id.ne(user_id))
            .one(&state.db)
            .await?;
        if conflict.is_some() {
            return Err(AuthError::UsernameTaken);
        }
        active.username = Set(Some(trimmed));
    }

    active.updated_at = Set(Utc::now());
    let updated = active.update(&state.db).await?;
    Ok(Json(build_me_dto(&state, updated).await?))
}

fn validate_avatar_url(url_str: &str, state: &AppState) -> Result<(), AuthError> {
    let parsed = url::Url::parse(url_str).map_err(|_| AuthError::InvalidCredentials)?;
    if parsed.scheme() != "https" {
        return Err(AuthError::InvalidCredentials);
    }
    if let Some(ik) = &state.imagekit {
        let endpoint =
            url::Url::parse(&ik.url_endpoint).map_err(|_| AuthError::InvalidCredentials)?;
        let expected_host = endpoint.host_str().unwrap_or("");
        let actual_host = parsed.host_str().unwrap_or("");
        if actual_host != expected_host {
            return Err(AuthError::InvalidCredentials);
        }
    }
    Ok(())
}

#[derive(Deserialize)]
pub struct ChangePasswordRequest {
    pub current_password: String,
    pub new_password: String,
}

#[tracing::instrument(level = "info", skip_all)]
pub async fn post_change_password(
    claims: AccessClaims,
    State(state): State<AppState>,
    Json(req): Json<ChangePasswordRequest>,
) -> Result<Json<serde_json::Value>, AuthError> {
    if req.new_password.len() < 8 {
        return Err(AuthError::InvalidCredentials);
    }
    let user_id = claims.user_id().map_err(|_| AuthError::UserNotFound)?;
    let user = users::Entity::find_by_id(user_id)
        .one(&state.db)
        .await?
        .ok_or(AuthError::UserNotFound)?;

    let stored_hash = user
        .password_hash
        .as_deref()
        .ok_or(AuthError::InvalidCredentials)?;
    if !verify_password(&state.argon2, stored_hash, &req.current_password)? {
        return Err(AuthError::InvalidCredentials);
    }

    let new_hash = hash_password(&state.argon2, &req.new_password)?;
    let mut active: users::ActiveModel = user.into();
    active.password_hash = Set(Some(new_hash));
    active.updated_at = Set(Utc::now());
    active.update(&state.db).await?;

    Ok(Json(serde_json::json!({ "ok": true })))
}

#[derive(Serialize)]
pub struct AvatarAuthResponse {
    pub token: String,
    pub expire: i64,
    pub signature: String,
    pub public_key: String,
    pub url_endpoint: String,
}

#[tracing::instrument(level = "info", skip_all)]
pub async fn get_avatar_auth(
    _claims: AccessClaims,
    State(state): State<AppState>,
) -> Result<Json<AvatarAuthResponse>, AuthError> {
    let ik = state
        .imagekit
        .as_ref()
        .ok_or(AuthError::ImageKitNotConfigured)?;

    let token = Uuid::new_v4().to_string();
    let expire = Utc::now().timestamp() + 60;

    // HMAC-SHA1 over "{token}{expire}" with the private key (FR-002).
    type HmacSha1 = Hmac<Sha1>;
    let mut mac =
        HmacSha1::new_from_slice(ik.private_key.as_bytes()).expect("HMAC accepts any key length");
    mac.update(format!("{token}{expire}").as_bytes());
    let signature = hex::encode(mac.finalize().into_bytes());

    Ok(Json(AvatarAuthResponse {
        token,
        expire,
        signature,
        public_key: ik.public_key.clone(),
        url_endpoint: ik.url_endpoint.clone(),
    }))
}
