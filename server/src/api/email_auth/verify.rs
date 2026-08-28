//! Email verification handlers.

use crate::auth::jwt::AccessClaims;
use crate::email::token::{generate_token, hash_secret, parse_token, verify_token};
use crate::state::AppState;
use arena_core::entities::{auth_tokens, users};
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use chrono::Utc;
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, Set};

use super::common::*;

/// `GET /auth/verify-email` — the link from the verification email.
///
/// Opened by a human in a browser, so it always 302-redirects to the site
/// (never renders JSON); the root layout toasts the outcome from the
/// `emailVerified` query param.
#[tracing::instrument(level = "info", skip_all)]
pub async fn get_verify_email(
    State(state): State<AppState>,
    Query(params): Query<VerifyEmailQuery>,
) -> Response {
    let dest = match verify_email_token(&state, &params.token).await {
        Ok(()) => "/?emailVerified=1",
        Err(e) => {
            tracing::warn!(
                handler = "get_verify_email",
                "email verification failed: {e}"
            );
            "/?emailVerified=0"
        }
    };
    let mut headers = axum::http::HeaderMap::new();
    headers.insert(
        axum::http::header::LOCATION,
        axum::http::HeaderValue::from_static(dest),
    );
    (StatusCode::FOUND, headers).into_response()
}

async fn verify_email_token(state: &AppState, token: &str) -> Result<(), EmailAuthError> {
    // 1. Parse raw token → (token_id, secret_bytes)
    let (token_id, secret_bytes) = parse_token(token)?;

    // 2. Look up token row WHERE id = token_id AND expires_at > now
    let now = Utc::now();
    let token_row = auth_tokens::Entity::find_by_id(token_id)
        .one(&state.db)
        .await?
        .ok_or(EmailAuthError::TokenNotFound)?;

    if token_row.token_type != TOKEN_TYPE_EMAIL_VERIFICATION {
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

    // 4. Delete the token row (single-use)
    auth_tokens::Entity::delete_by_id(token_id)
        .exec(&state.db)
        .await?;

    // 5. Mark user email_verified = true
    let user = users::Entity::find_by_id(user_id)
        .one(&state.db)
        .await?
        .ok_or(EmailAuthError::UserNotFound)?;
    let mut active: users::ActiveModel = user.into();
    active.email_verified = Set(true);
    active.updated_at = Set(Utc::now());
    active.update(&state.db).await?;

    // Deliberately does NOT touch a pending newsletter opt-in. This click
    // means "activate my account"; the sign-up box ships pre-ticked, so
    // reading it as newsletter consent too would subscribe people who never
    // chose to be. The newsletter carries its own confirmation link.
    Ok(())
}

#[tracing::instrument(level = "info", skip_all)]
pub async fn post_resend_verification(
    State(state): State<AppState>,
    claims: AccessClaims,
    PeerIp(ip): PeerIp,
) -> Result<Response, EmailAuthError> {
    // 1. Rate limit check — FR-013, socket IP only (no X-Forwarded-For)
    if !state.email_rate_limiter.check_and_record(&ip.to_string()) {
        return Err(EmailAuthError::RateLimited);
    }

    // 2. Email service must be present
    let email_svc = state
        .email_service
        .clone()
        .ok_or(EmailAuthError::ServiceUnavailable)?;

    // 3. Load user
    let user_id = claims.user_id().map_err(|_| EmailAuthError::UserNotFound)?;
    let user = users::Entity::find_by_id(user_id)
        .one(&state.db)
        .await?
        .ok_or(EmailAuthError::UserNotFound)?;

    // 4. Already verified?
    if user.email_verified {
        return Err(EmailAuthError::AlreadyVerified);
    }

    let user_email = user.email.clone();

    // 5. Delete any existing verification tokens for this user
    auth_tokens::Entity::delete_many()
        .filter(auth_tokens::Column::UserId.eq(user_id))
        .filter(auth_tokens::Column::TokenType.eq(TOKEN_TYPE_EMAIL_VERIFICATION))
        .exec(&state.db)
        .await?;

    // 6. Generate new token
    let (token_id, raw_token) = generate_token();
    let (_, secret_bytes) = parse_token(&raw_token).map_err(EmailAuthError::TokenParse)?;
    let token_hash = hash_secret(&secret_bytes);
    let now = Utc::now();
    let expires_at = now + chrono::Duration::hours(24);

    // 7. Insert token row
    let token_model = auth_tokens::ActiveModel {
        id: Set(token_id),
        user_id: Set(user_id),
        token_hash: Set(token_hash),
        token_type: Set(TOKEN_TYPE_EMAIL_VERIFICATION.to_string()),
        expires_at: Set(expires_at),
        created_at: Set(now),
    };
    token_model.insert(&state.db).await?;

    // 8. Send synchronously — this endpoint is authenticated (no enumeration
    //    concern), and a fire-and-forget send let the UI toast "sent" while
    //    the email silently failed.
    let base_url =
        std::env::var("PUBLIC_APP_URL").unwrap_or_else(|_| "http://localhost:5173".to_string());
    let verify_url = format!("{}/auth/verify-email?token={}", base_url, raw_token);

    let tmpl = load_template_or_builtin(&state.db, "verify").await;
    let body_html = tmpl.body_html.replace("{{VERIFY_URL}}", &verify_url);
    let body_text = tmpl.body_text.replace("{{VERIFY_URL}}", &verify_url);

    email_svc
        .send_email(&user_email, &tmpl.subject, &body_html, &body_text)
        .await
        .map_err(|e| {
            tracing::error!(
                handler = "post_resend_verification",
                user_email = %user_email,
                "failed to send verification email: {e}"
            );
            EmailAuthError::SendFailed
        })?;

    Ok(StatusCode::OK.into_response())
}
