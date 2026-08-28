//! Password reset handlers.

use crate::auth::password::hash_password;
use crate::auth::turnstile::verify_turnstile;
use crate::email::token::{generate_token, hash_secret, parse_token, verify_token};
use crate::state::AppState;
use arena_core::entities::{auth_tokens, refresh_tokens, users};
use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use chrono::Utc;
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, Set};

use super::common::*;
#[tracing::instrument(level = "info", skip_all)]
pub async fn post_forgot_password(
    State(state): State<AppState>,
    PeerIp(ip): PeerIp,
    Json(req): Json<ForgotPasswordRequest>,
) -> Result<Response, EmailAuthError> {
    // 1. Rate limit — FR-013
    if !state.email_rate_limiter.check_and_record(&ip.to_string()) {
        return Err(EmailAuthError::RateLimited);
    }

    // 1b. Turnstile verification — after rate limit, before user lookup
    verify_turnstile(
        &state.turnstile,
        state.turnstile_verifier.as_ref(),
        req.turnstile_token.as_deref(),
    )
    .await
    .map_err(EmailAuthError::Captcha)?;

    // 2. Email service must be present (FR-005)
    let email_svc = state
        .email_service
        .clone()
        .ok_or(EmailAuthError::ServiceUnavailable)?;

    // 3. Look up user by email — enumeration prevention: always 200 on user-not-found
    let user = users::Entity::find()
        .filter(users::Column::Email.eq(req.email.clone()))
        .one(&state.db)
        .await?;

    if let Some(user) = user {
        // 4. Delete any existing password reset tokens for this user
        auth_tokens::Entity::delete_many()
            .filter(auth_tokens::Column::UserId.eq(user.id))
            .filter(auth_tokens::Column::TokenType.eq(TOKEN_TYPE_PASSWORD_RESET))
            .exec(&state.db)
            .await?;

        // 5. Generate new token
        let (token_id, raw_token) = generate_token();
        let (_, secret_bytes) = parse_token(&raw_token).map_err(EmailAuthError::TokenParse)?;
        let token_hash = hash_secret(&secret_bytes);
        let now = Utc::now();
        let expires_at = now + chrono::Duration::hours(1); // 1 hour for password reset

        // 6. Insert token row synchronously (so we can report DB errors)
        auth_tokens::ActiveModel {
            id: Set(token_id),
            user_id: Set(user.id),
            token_hash: Set(token_hash),
            token_type: Set(TOKEN_TYPE_PASSWORD_RESET.to_string()),
            expires_at: Set(expires_at),
            created_at: Set(now),
        }
        .insert(&state.db)
        .await?;

        // 7. Fire-and-forget email dispatch
        let user_email = user.email.clone();
        let db = state.db.clone();
        tokio::spawn(async move {
            let base_url = std::env::var("PUBLIC_APP_URL")
                .unwrap_or_else(|_| "http://localhost:5173".to_string());
            let reset_url = format!("{}/reset-password?token={}", base_url, raw_token);

            let tmpl = load_template_or_builtin(&db, "reset_password").await;

            let body_html = tmpl.body_html.replace("{{RESET_URL}}", &reset_url);
            let body_text = tmpl.body_text.replace("{{RESET_URL}}", &reset_url);

            if let Err(e) = email_svc
                .send_email(&user_email, &tmpl.subject, &body_html, &body_text)
                .await
            {
                tracing::error!(
                    handler = "post_forgot_password",
                    user_email = %user_email,
                    "failed to send password reset email: {e}"
                );
            }
        });
    }

    Ok((
        StatusCode::OK,
        Json(serde_json::json!({
            "message": "If an account with that address exists, a reset email has been sent."
        })),
    )
        .into_response())
}

#[tracing::instrument(level = "info", skip_all)]
pub async fn post_reset_password(
    State(state): State<AppState>,
    PeerIp(ip): PeerIp,
    Json(req): Json<ResetPasswordRequest>,
) -> Result<Response, EmailAuthError> {
    // 0. Rate limit — guards argon2 compute + password write (AC-005b)
    if !state.email_rate_limiter.check_and_record(&ip.to_string()) {
        return Err(EmailAuthError::RateLimited);
    }

    // 0b. Turnstile verification — after rate limit, before token processing
    verify_turnstile(
        &state.turnstile,
        state.turnstile_verifier.as_ref(),
        req.turnstile_token.as_deref(),
    )
    .await
    .map_err(EmailAuthError::Captcha)?;

    // 1. Parse raw token → (token_id, secret_bytes)
    let (token_id, secret_bytes) = parse_token(&req.token)?;

    // 2. Look up token row WHERE id = token_id AND expires_at > now
    let now = Utc::now();
    let token_row = auth_tokens::Entity::find_by_id(token_id)
        .one(&state.db)
        .await?
        .ok_or(EmailAuthError::TokenNotFound)?;

    if token_row.token_type != TOKEN_TYPE_PASSWORD_RESET {
        return Err(EmailAuthError::TokenNotFound);
    }

    if token_row.expires_at <= now {
        return Err(EmailAuthError::TokenNotFound);
    }

    // 3. Constant-time verification
    if !verify_token(&secret_bytes, &token_row.token_hash) {
        return Err(EmailAuthError::TokenVerifyFailed);
    }

    let user_id = token_row.user_id;

    // 4. Validate new_password — must be non-empty
    if req.new_password.is_empty() {
        return Err(EmailAuthError::InvalidPassword);
    }

    // 5. Hash new password
    let new_hash =
        hash_password(&state.argon2, &req.new_password).map_err(|_| EmailAuthError::Internal)?;

    // 6. Update users.password_hash
    let user = users::Entity::find_by_id(user_id)
        .one(&state.db)
        .await?
        .ok_or(EmailAuthError::UserNotFound)?;
    let mut active: users::ActiveModel = user.into();
    active.password_hash = Set(Some(new_hash));
    active.updated_at = Set(Utc::now());
    active.update(&state.db).await?;

    // 7. Delete ALL password_reset_tokens for this user (invalidate outstanding tokens)
    auth_tokens::Entity::delete_many()
        .filter(auth_tokens::Column::UserId.eq(user_id))
        .filter(auth_tokens::Column::TokenType.eq(TOKEN_TYPE_PASSWORD_RESET))
        .exec(&state.db)
        .await?;

    // 8. Delete ALL refresh_tokens for this user (terminate all active sessions — AC-005)
    refresh_tokens::Entity::delete_many()
        .filter(refresh_tokens::Column::UserIdFk.eq(user_id))
        .exec(&state.db)
        .await?;

    Ok(StatusCode::OK.into_response())
}
