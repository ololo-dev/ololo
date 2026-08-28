//! Magic-link sign-in handlers.

use crate::auth::jwt::issue_access_token;
use crate::auth::refresh::issue_refresh_token;
use crate::auth::turnstile::verify_turnstile;
use crate::email::token::{generate_token, hash_secret, parse_token, verify_token};
use crate::state::AppState;
use arena_core::entities::{auth_tokens, users};
use axum::Json;
use axum::extract::{Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use chrono::Utc;
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, Set};

use super::common::*;
#[tracing::instrument(level = "info", skip_all)]
pub async fn post_magic_link_request(
    State(state): State<AppState>,
    PeerIp(ip): PeerIp,
    Json(req): Json<MagicLinkRequest>,
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

    // 2. Email service must be present (FR-007)
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
        // 4. Delete any prior magic-link tokens for this user before inserting a
        //    fresh one — prevents token accumulation across repeated requests.
        auth_tokens::Entity::delete_many()
            .filter(auth_tokens::Column::UserId.eq(user.id))
            .filter(auth_tokens::Column::TokenType.eq(TOKEN_TYPE_MAGIC_LINK))
            .exec(&state.db)
            .await?;

        // 5. Generate token (15 minutes TTL for magic links)
        let (token_id, raw_token) = generate_token();
        let (_, secret_bytes) = parse_token(&raw_token).map_err(EmailAuthError::TokenParse)?;
        let token_hash = hash_secret(&secret_bytes);
        let now = Utc::now();
        let expires_at = now + chrono::Duration::minutes(15);

        // 6. Insert token row
        auth_tokens::ActiveModel {
            id: Set(token_id),
            user_id: Set(user.id),
            token_hash: Set(token_hash),
            token_type: Set(TOKEN_TYPE_MAGIC_LINK.to_string()),
            expires_at: Set(expires_at),
            created_at: Set(now),
        }
        .insert(&state.db)
        .await?;

        // 6. Fire-and-forget email dispatch
        let user_email = user.email.clone();
        let db = state.db.clone();
        tokio::spawn(async move {
            let base_url = std::env::var("PUBLIC_APP_URL")
                .unwrap_or_else(|_| "http://localhost:5173".to_string());
            let magic_link_url = format!("{}/auth/magic-link/verify?token={}", base_url, raw_token);

            let tmpl = load_template_or_builtin(&db, "magic_link").await;

            let body_html = tmpl
                .body_html
                .replace("{{MAGIC_LINK_URL}}", &magic_link_url);
            let body_text = tmpl
                .body_text
                .replace("{{MAGIC_LINK_URL}}", &magic_link_url);

            if let Err(e) = email_svc
                .send_email(&user_email, &tmpl.subject, &body_html, &body_text)
                .await
            {
                tracing::error!(
                    handler = "post_magic_link_request",
                    user_email = %user_email,
                    "failed to send magic link email: {e}"
                );
            }
        });
    }

    Ok((
        StatusCode::OK,
        Json(serde_json::json!({
            "message": "If an account with that address exists, a sign-in link has been sent."
        })),
    )
        .into_response())
}

#[tracing::instrument(level = "info", skip_all)]
pub async fn get_magic_link_verify(
    State(state): State<AppState>,
    Query(params): Query<MagicLinkVerifyQuery>,
) -> Result<Response, EmailAuthError> {
    // 1. Parse raw token → (token_id, secret_bytes)
    let (token_id, secret_bytes) = parse_token(&params.token)?;

    // 2. Look up token row WHERE id = token_id AND expires_at > now
    let now = Utc::now();
    let token_row = auth_tokens::Entity::find_by_id(token_id)
        .one(&state.db)
        .await?
        .ok_or(EmailAuthError::TokenNotFound)?;

    if token_row.token_type != TOKEN_TYPE_MAGIC_LINK {
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

    // 4. Delete token row (single-use)
    auth_tokens::Entity::delete_by_id(token_id)
        .exec(&state.db)
        .await?;

    // 5. Load user
    let user = users::Entity::find_by_id(user_id)
        .one(&state.db)
        .await?
        .ok_or(EmailAuthError::UserNotFound)?;

    // 5b. A clicked magic link proves mailbox ownership — mark the email
    //     verified so the "confirm your email" banner clears.
    if !user.email_verified {
        let mut active: users::ActiveModel = user.clone().into();
        active.email_verified = Set(true);
        active.updated_at = Set(Utc::now());
        active.update(&state.db).await?;
    }

    // Same restraint as the verification link: signing in is not an act of
    // newsletter consent, and a pre-ticked box makes silently promoting a
    // pending opt-in here a subscription nobody chose.

    // 6. Issue access + refresh tokens
    // Magic-link login does NOT invalidate existing refresh tokens (FR-008, Grilling Decision 6)
    let access = issue_access_token(
        &state.jwt_encoding_key,
        user.id,
        &user.email,
        state.access_ttl,
    )
    .map_err(|_| EmailAuthError::Internal)?;

    let refresh = issue_refresh_token(&state.db, &state.argon2, user.id, state.refresh_ttl)
        .await
        .map_err(|_| EmailAuthError::Internal)?;

    // 7. Validate `next` param (NFR-007: URL-parser-based open-redirect prevention)
    let redirect_to = validate_next(params.next);

    // 8. Build redirect response with Set-Cookie headers
    let mut headers = HeaderMap::new();

    let access_cookie = format!(
        "arena_access={}; HttpOnly; Secure; SameSite=Lax; Path=/; Max-Age={}",
        access,
        state.access_ttl.as_secs()
    );
    let refresh_cookie = format!(
        // Path=/ so the SvelteKit hooks can silent-refresh on page loads;
        // see the note in api::users::auth::refresh_cookie.
        "arena_refresh={}; HttpOnly; Secure; SameSite=Lax; Path=/; Max-Age={}",
        refresh,
        state.refresh_ttl.as_secs()
    );

    if let Ok(v) = HeaderValue::from_str(&access_cookie) {
        headers.append(header::SET_COOKIE, v);
    }
    if let Ok(v) = HeaderValue::from_str(&refresh_cookie) {
        headers.append(header::SET_COOKIE, v);
    }
    // Drop the pre-Path=/ duplicate; see api::users::auth::clear_legacy_refresh_cookie.
    if let Ok(v) = HeaderValue::from_str(&format!(
        "arena_refresh=; HttpOnly; Secure; SameSite=Lax; Path={}; Max-Age=0",
        crate::api::users::auth::LEGACY_REFRESH_PATH
    )) {
        headers.append(header::SET_COOKIE, v);
    }
    if let Ok(v) = HeaderValue::from_str(&redirect_to) {
        headers.insert(header::LOCATION, v);
    } else {
        headers.insert(header::LOCATION, HeaderValue::from_static("/"));
    }

    Ok((StatusCode::FOUND, headers).into_response())
}
