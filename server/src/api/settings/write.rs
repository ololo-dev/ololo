//! Write handlers for admin settings and admin user management.

use crate::api::settings::common::{AdminUser, AdminUserDto, SettingsError, is_valid_bool_value};
use crate::auth::password::hash_password;
use crate::state::AppState;
use crate::validation::username::validate_username;
use arena_core::entities::{app_settings, users};
use arena_core::util::username_gen::generate_username;
use axum::Json;
use axum::extract::{Path, State};
use chrono::Utc;
use rand::SeedableRng;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, Set,
    sea_query::OnConflict,
};
use uuid::Uuid;

/// `PUT /api/admin/settings` — update a setting.
/// Spec: FR-009, FR-010, FR-012, AC-010–AC-012, AC-022.
///
/// Key allowlist (FR-010): email settings, the project-creation gate, and
/// the Arena Points formula parameters. LLM configuration lives in the
/// `llm_providers` admin API, not here.
pub async fn put_settings(
    _admin: AdminUser,
    State(state): State<AppState>,
    Json(mut body): Json<crate::api::settings::common::PutSettingsBody>,
) -> Result<Json<serde_json::Value>, SettingsError> {
    // FR-010: key allowlist.
    const STATIC_ALLOWED_KEYS: &[&str] = &[
        "allow_user_project_creation",
        "llm_model_prices",
        "show_llm_costs_in_session",
        crate::api::settings::SESSION_REPLAY_KEY,
        arena_core::quota::PLANS_ENABLED_KEY,
        arena_core::quota::FREE_JUDGE_RUN_LIMIT_KEY,
        arena_core::quota::PREMIUM_JUDGE_RUN_LIMIT_KEY,
        // Cross-session copy/paste validation (game server reads this):
        // duplication % below which no penalty applies; 100 disables.
        "similarity_threshold_pct",
        "email.provider",
        "email.ses_region",
        "email.access_key_id",
        "email.secret_access_key",
        "email.from_address",
        "email.cloudflare_account_id",
        "email.cloudflare_api_token",
    ];
    if !STATIC_ALLOWED_KEYS.contains(&body.key.as_str()) {
        return Err(SettingsError::UnknownKey);
    }

    // Reject the [redacted] sentinel to prevent overwriting the real key with the display placeholder.
    if crate::api::settings::common::is_secret_setting(&body.key) && body.value == "[redacted]" {
        return Err(SettingsError::SentinelNotAllowed);
    }

    // Key-specific validation.
    if body.key == "email.provider" {
        // Normalise to lowercase so startup selection can compare exactly.
        body.value = body.value.to_ascii_lowercase();
        if !matches!(body.value.as_str(), "ses" | "cloudflare") {
            return Err(SettingsError::InvalidEmailProvider);
        }
    } else if body.key == "allow_user_project_creation" {
        // Contract AC-017: value must be "true" or "false" (case-insensitive).
        // Normalise to lowercase before storage so the gate's exact check always matches.
        if !is_valid_bool_value(&body.value) {
            return Err(SettingsError::InvalidProjectCreationValue);
        }
        body.value = body.value.to_ascii_lowercase();
    } else if body.key == "show_llm_costs_in_session"
        || body.key == crate::api::settings::SESSION_REPLAY_KEY
        || body.key == arena_core::quota::PLANS_ENABLED_KEY
    {
        if !is_valid_bool_value(&body.value) {
            return Err(SettingsError::InvalidProjectCreationValue);
        }
        body.value = body.value.to_ascii_lowercase();
    } else if body.key == "llm_model_prices" {
        // A JSON map model → per-million-token USD prices, all non-negative.
        let parsed: std::collections::HashMap<String, crate::api::analytics::ModelPrice> =
            serde_json::from_str(&body.value).map_err(|_| SettingsError::UnknownKey)?;
        if parsed.values().any(|p| {
            !(p.input >= 0.0 && p.output >= 0.0 && p.cache_read >= 0.0 && p.cache_write >= 0.0)
        }) {
            return Err(SettingsError::UnknownKey);
        }
    } else if body.key == "similarity_threshold_pct" {
        // A percentage: 0–100, where 100 turns the check off.
        let parsed: u32 = body
            .value
            .trim()
            .parse()
            .map_err(|_| SettingsError::InvalidPlanLimitValue)?;
        if parsed > 100 {
            return Err(SettingsError::InvalidPlanLimitValue);
        }
        body.value = parsed.to_string();
    } else if body.key == arena_core::quota::FREE_JUDGE_RUN_LIMIT_KEY
        || body.key == arena_core::quota::PREMIUM_JUDGE_RUN_LIMIT_KEY
    {
        // Monthly judge-run limit for the tier: a non-negative integer.
        let parsed: i64 = body
            .value
            .trim()
            .parse()
            .map_err(|_| SettingsError::InvalidPlanLimitValue)?;
        if parsed < 0 {
            return Err(SettingsError::InvalidPlanLimitValue);
        }
        body.value = parsed.to_string();
    }

    // Encrypt sensitive values before storage.
    let value_to_store = if crate::api::settings::common::is_secret_setting(&body.key) {
        state.settings_encryption.encrypt(&body.value)
    } else {
        body.value.clone()
    };

    // Upsert into app_settings.
    app_settings::Entity::insert(app_settings::ActiveModel {
        key: Set(body.key.clone()),
        value: Set(value_to_store),
    })
    .on_conflict(
        OnConflict::column(app_settings::Column::Key)
            .update_column(app_settings::Column::Value)
            .to_owned(),
    )
    .exec(&state.db)
    .await
    .map_err(|_| SettingsError::Database)?;

    Ok(Json(serde_json::json!({ "ok": true })))
}

/// `POST /api/admin/users` — create a user (admin only).
pub async fn post_admin_user(
    _admin: AdminUser,
    State(state): State<AppState>,
    Json(body): Json<crate::api::settings::common::CreateAdminUserBody>,
) -> Result<Json<serde_json::Value>, SettingsError> {
    let existing = users::Entity::find()
        .filter(users::Column::Email.eq(body.email.clone()))
        .one(&state.db)
        .await
        .map_err(|_| SettingsError::Database)?;
    if existing.is_some() {
        return Err(SettingsError::EmailTaken);
    }

    let hash = hash_password(&state.argon2, &body.password).map_err(|_| SettingsError::Database)?;
    let now = Utc::now();
    let id = Uuid::new_v4();

    // Resolve username: validate the provided one, or auto-generate.
    let username = if let Some(ref u) = body.username {
        validate_username(u).map_err(|_| SettingsError::InvalidUsername)?;
        let conflict = users::Entity::find()
            .filter(users::Column::Username.eq(u.clone()))
            .one(&state.db)
            .await
            .map_err(|_| SettingsError::Database)?;
        if conflict.is_some() {
            return Err(SettingsError::UsernameTaken);
        }
        u.clone()
    } else {
        let mut rng = rand::rngs::StdRng::from_entropy();
        let mut found = None;
        for _ in 0..10 {
            let candidate = generate_username(&mut rng);
            let exists = users::Entity::find()
                .filter(users::Column::Username.eq(candidate.clone()))
                .one(&state.db)
                .await
                .map_err(|_| SettingsError::Database)?;
            if exists.is_none() {
                found = Some(candidate);
                break;
            }
        }
        found.ok_or(SettingsError::Database)?
    };

    let plan = match body.plan.as_deref() {
        None => arena_core::quota::PLAN_FREE.to_string(),
        Some(p) => validate_plan(p)?,
    };

    users::ActiveModel {
        id: Set(id),
        email: Set(body.email.clone()),
        password_hash: Set(Some(hash)),
        display_name: Set(body.display_name.clone()),
        created_at: Set(now),
        updated_at: Set(now),
        is_admin: Set(body.is_admin),
        avatar_url: Set(None),
        email_verified: Set(false),
        username: Set(Some(username.clone())),
        plan: Set(plan.clone()),
        judge_run_limit: Set(None),
        judge_run_credits: Set(0),
    }
    .insert(&state.db)
    .await
    .map_err(|_| SettingsError::Database)?;

    let judge_run_limit_effective = arena_core::quota::plan_judge_run_limit(&state.db, &plan)
        .await
        .map_err(|_| SettingsError::Database)?;
    let dto = AdminUserDto {
        id,
        email: body.email,
        display_name: body.display_name,
        username: Some(username),
        is_admin: body.is_admin,
        avatar_url: None,
        created_at: now,
        plan,
        judge_run_limit: None,
        judge_run_limit_effective,
        judge_run_credits: 0,
        judge_runs_this_month: 0,
    };
    Ok(Json(serde_json::json!({ "user": dto })))
}

/// `PATCH /api/admin/users/:id` — update a user (admin only).
pub async fn patch_admin_user(
    _admin: AdminUser,
    State(state): State<AppState>,
    Path(user_id): Path<Uuid>,
    Json(body): Json<crate::api::settings::common::UpdateAdminUserBody>,
) -> Result<Json<serde_json::Value>, SettingsError> {
    let user = users::Entity::find_by_id(user_id)
        .one(&state.db)
        .await
        .map_err(|_| SettingsError::Database)?
        .ok_or(SettingsError::UserNotFound)?;

    let mut active: users::ActiveModel = user.into();

    if let Some(email) = body.email {
        let existing = users::Entity::find()
            .filter(users::Column::Email.eq(email.clone()))
            .filter(users::Column::Id.ne(user_id))
            .one(&state.db)
            .await
            .map_err(|_| SettingsError::Database)?;
        if existing.is_some() {
            return Err(SettingsError::EmailTaken);
        }
        active.email = Set(email);
    }

    if let Some(display_name) = body.display_name {
        active.display_name = Set(display_name);
    }

    if let Some(username) = body.username {
        validate_username(&username).map_err(|_| SettingsError::InvalidUsername)?;
        let existing = users::Entity::find()
            .filter(users::Column::Username.eq(username.clone()))
            .filter(users::Column::Id.ne(user_id))
            .one(&state.db)
            .await
            .map_err(|_| SettingsError::Database)?;
        if existing.is_some() {
            return Err(SettingsError::UsernameTaken);
        }
        active.username = Set(Some(username));
    }

    if let Some(is_admin) = body.is_admin {
        active.is_admin = Set(is_admin);
    }

    if let Some(plan) = body.plan {
        active.plan = Set(validate_plan(&plan)?);
    }

    if let Some(limit) = body.judge_run_limit {
        if limit.is_some_and(|n| n < 0) {
            return Err(SettingsError::InvalidPlanLimitValue);
        }
        active.judge_run_limit = Set(limit);
    }

    if let Some(delta) = body.grant_judge_run_credits
        && delta != 0
    {
        // A grant is a DELTA on the purchased balance (a pack sold, or a
        // negative correction), never clamped below zero silently.
        let current = users::Entity::find_by_id(user_id)
            .one(&state.db)
            .await
            .map_err(|_| SettingsError::Database)?
            .map(|u| u.judge_run_credits)
            .unwrap_or(0);
        let next = current.saturating_add(delta);
        if next < 0 {
            return Err(SettingsError::InvalidPlanLimitValue);
        }
        active.judge_run_credits = Set(next);
    }

    if let Some(password) = body.password
        && !password.is_empty()
    {
        let hash = hash_password(&state.argon2, &password).map_err(|_| SettingsError::Database)?;
        active.password_hash = Set(Some(hash));
    }

    active.updated_at = Set(Utc::now());
    let updated = active
        .update(&state.db)
        .await
        .map_err(|_| SettingsError::Database)?;

    let quota = arena_core::quota::judge_quota_for_user(&state.db, &updated)
        .await
        .map_err(|_| SettingsError::Database)?;
    let dto = AdminUserDto {
        id: updated.id,
        email: updated.email,
        display_name: updated.display_name,
        username: updated.username,
        is_admin: updated.is_admin,
        avatar_url: updated.avatar_url,
        created_at: updated.created_at,
        plan: updated.plan,
        judge_run_limit: updated.judge_run_limit,
        judge_run_limit_effective: quota.limit,
        judge_run_credits: updated.judge_run_credits,
        judge_runs_this_month: quota.used,
    };
    Ok(Json(serde_json::json!({ "user": dto })))
}

/// `DELETE /api/admin/users/:id` — delete a user (admin only).
///
/// Prevents self-deletion. Cascades will handle `refresh_tokens`,
/// `oauth_identities`, and `session_members`. Returns 409 when the user
/// has projects or agents that would violate FK constraints on Postgres.
pub async fn delete_admin_user(
    admin: AdminUser,
    State(state): State<AppState>,
    Path(user_id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, SettingsError> {
    if admin.id == user_id {
        return Err(SettingsError::CannotDeleteSelf);
    }

    // Confirm user exists before attempting delete.
    let _ = users::Entity::find_by_id(user_id)
        .one(&state.db)
        .await
        .map_err(|_| SettingsError::Database)?
        .ok_or(SettingsError::UserNotFound)?;

    users::Entity::delete_by_id(user_id)
        .exec(&state.db)
        .await
        .map_err(|_| SettingsError::HasRelatedRecords)?;

    Ok(Json(serde_json::json!({ "ok": true })))
}

/// Normalises and validates an account plan value.
fn validate_plan(plan: &str) -> Result<String, SettingsError> {
    let plan = plan.to_ascii_lowercase();
    if plan == arena_core::quota::PLAN_FREE || plan == arena_core::quota::PLAN_PREMIUM {
        Ok(plan)
    } else {
        Err(SettingsError::InvalidPlan)
    }
}

/// Returns `true` iff the `allow_user_project_creation` setting is explicitly
/// `"true"`. Absent row and any other value → `false` (fail-closed, FR-001/FR-002).
///
/// Exported so that `api::users` can embed the flag in `GET /api/users/me`
/// without a cross-module DB query.
pub async fn is_project_creation_allowed(db: &DatabaseConnection) -> Result<bool, sea_orm::DbErr> {
    let row = app_settings::Entity::find()
        .filter(app_settings::Column::Key.eq("allow_user_project_creation"))
        .one(db)
        .await?;
    Ok(row.is_some_and(|r| r.value == "true"))
}
