use crate::auth::AuthError;
use crate::auth::jwt::{AccessClaims, issue_access_token};
use crate::auth::middleware::{ACCESS_COOKIE, REFRESH_COOKIE, extract_cookie};
use crate::auth::password::{hash_password, verify_password};
use crate::auth::refresh::{issue_refresh_token, revoke, verify_and_rotate_any};
use crate::auth::turnstile::verify_turnstile;
use crate::state::AppState;
use arena_core::entities::users;
use arena_core::util::username_gen::generate_username;
use axum::Json;
use axum::extract::{Request, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use chrono::Utc;
use rand::SeedableRng;
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter, Set};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Deserialize)]
pub struct RegisterRequest {
    pub email: String,
    pub password: String,
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub turnstile_token: Option<String>,
}

#[derive(Deserialize)]
pub struct LoginRequest {
    pub email: String,
    pub password: String,
    #[serde(default)]
    pub turnstile_token: Option<String>,
}

#[derive(Serialize)]
pub struct UserDto {
    pub id: Uuid,
    pub email: String,
    pub display_name: String,
}

#[derive(Serialize)]
pub struct TokenResponse {
    pub id: Uuid,
    pub email: String,
    pub display_name: String,
    pub access_token: String,
    /// FR-009 / AC-014: soft gate advisory flag.
    pub email_verified: bool,
}

fn access_cookie(value: &str, max_age: u64) -> String {
    format!(
        "{}={}; HttpOnly; Secure; SameSite=Lax; Path=/; Max-Age={}",
        ACCESS_COOKIE, value, max_age
    )
}

fn refresh_cookie(value: &str, max_age: u64) -> String {
    // Path=/ (not /auth/refresh) so the cookie is also sent on ordinary page
    // navigations: the SvelteKit hooks perform a server-side silent refresh when
    // the short-lived access token has lapsed, and they need the refresh cookie
    // to be present on the page request to do so. HttpOnly + Secure + SameSite=Lax
    // remain the CSRF/theft controls; the refresh endpoint is POST, which
    // SameSite=Lax already shields from cross-site cookie attachment.
    format!(
        "{}={}; HttpOnly; Secure; SameSite=Lax; Path=/; Max-Age={}",
        REFRESH_COOKIE, value, max_age
    )
}

fn expire_cookie(name: &str, path: &str) -> String {
    format!(
        "{}=; HttpOnly; Secure; SameSite=Lax; Path={}; Max-Age=0",
        name, path
    )
}

/// The path the refresh cookie used to be scoped to.
pub(crate) const LEGACY_REFRESH_PATH: &str = "/auth/refresh";

/// Delete the refresh cookie left at the old `/auth/refresh` path.
///
/// Cookies are keyed by (name, domain, path), so widening the scope to `Path=/`
/// did not replace the narrow one: a browser that authenticated on both sides
/// of that change holds two `arena_refresh` cookies and sends both to
/// `/auth/refresh`, most-specific path first. Rotation only ever rewrites the
/// `Path=/` copy, so the legacy one freezes at a value that is revoked the
/// moment it is first rotated — and every later refresh replays it. Expire it
/// wherever a fresh pair is issued, so the duplicate disappears on the next
/// authenticated response.
pub(crate) fn clear_legacy_refresh_cookie(headers: &mut HeaderMap) {
    append_cookie(headers, expire_cookie(REFRESH_COOKIE, LEGACY_REFRESH_PATH));
}

fn append_cookie(headers: &mut HeaderMap, cookie: String) {
    if let Ok(value) = HeaderValue::from_str(&cookie) {
        headers.append(header::SET_COOKIE, value);
    }
}

#[tracing::instrument(level = "info", skip_all)]
pub async fn post_register(
    State(state): State<AppState>,
    crate::api::email_auth::common::PeerIp(peer_ip): crate::api::email_auth::common::PeerIp,
    Json(req): Json<RegisterRequest>,
) -> Result<Response, AuthError> {
    if !state
        .auth_rate_limiter
        .check_and_record(&peer_ip.to_string())
    {
        return Err(AuthError::RateLimited);
    }
    verify_turnstile(
        &state.turnstile,
        state.turnstile_verifier.as_ref(),
        req.turnstile_token.as_deref(),
    )
    .await
    .map_err(AuthError::from)?;
    if req.email.is_empty() || req.password.is_empty() {
        return Err(AuthError::InvalidCredentials);
    }
    let existing = users::Entity::find()
        .filter(users::Column::Email.eq(req.email.clone()))
        .one(&state.db)
        .await?;
    if existing.is_some() {
        return Err(AuthError::UserExists);
    }
    let hash = hash_password(&state.argon2, &req.password)?;
    let now = Utc::now();
    let display_name = req
        .display_name
        .unwrap_or_else(|| req.email.split('@').next().unwrap_or("user").to_string());
    let id = Uuid::new_v4();
    // FR-002: first registered user is admin (AC-001); subsequent users are not (AC-002).
    // SQLite serialises all writes; on PostgreSQL the UNIQUE constraint on
    // `email` and the registration race-window are narrow enough that this
    // count is reliable for the first-user scenario.
    let user_count = users::Entity::find().count(&state.db).await?;
    let is_admin = user_count == 0;

    // Generate a unique username (retry up to 10 times; 503 on exhaustion).
    let username = {
        let mut rng = rand::rngs::StdRng::from_entropy();
        let mut found = None;
        for _ in 0..10 {
            let candidate = generate_username(&mut rng);
            let exists = users::Entity::find()
                .filter(users::Column::Username.eq(candidate.clone()))
                .one(&state.db)
                .await?;
            if exists.is_none() {
                found = Some(candidate);
                break;
            }
        }
        found.ok_or(AuthError::UsernameGenerationFailed)?
    };

    let model = users::ActiveModel {
        id: Set(id),
        email: Set(req.email.clone()),
        password_hash: Set(Some(hash)),
        display_name: Set(display_name.clone()),
        created_at: Set(now),
        updated_at: Set(now),
        is_admin: Set(is_admin),
        avatar_url: Set(None),
        email_verified: Set(false),
        username: Set(Some(username)),
        plan: Set(arena_core::quota::PLAN_FREE.to_string()),
        judge_run_limit: Set(None),
        judge_run_credits: Set(0),
    };
    model.insert(&state.db).await?;

    // FR-002: dispatch verification email (fire-and-forget, non-blocking)
    if let Some(email_svc) = state.email_service.clone() {
        let user_email = req.email.clone();
        let db = state.db.clone();
        tokio::spawn(async move {
            use crate::email::token::{generate_token, hash_secret, parse_token};
            use arena_core::entities::auth_tokens;
            use sea_orm::ActiveModelTrait;

            let (token_id, raw_token) = generate_token();
            let (_, secret_bytes) = match parse_token(&raw_token) {
                Ok(v) => v,
                Err(e) => {
                    tracing::error!(handler = "post_register", user_email = %user_email, "token parse error: {e}");
                    return;
                }
            };
            let token_hash = hash_secret(&secret_bytes);
            let now = chrono::Utc::now();
            let expires_at = now + chrono::Duration::hours(24);

            let token_model = auth_tokens::ActiveModel {
                id: sea_orm::Set(token_id),
                user_id: sea_orm::Set(id),
                token_hash: sea_orm::Set(token_hash),
                token_type: sea_orm::Set("email_verification".to_string()),
                expires_at: sea_orm::Set(expires_at),
                created_at: sea_orm::Set(now),
            };
            if let Err(e) = token_model.insert(&db).await {
                tracing::error!(handler = "post_register", user_email = %user_email, "failed to insert verification token: {e}");
                return;
            }

            let base_url = std::env::var("PUBLIC_APP_URL")
                .unwrap_or_else(|_| "http://localhost:5173".to_string());
            let verify_url = format!("{}/auth/verify-email?token={}", base_url, raw_token);

            let tmpl =
                crate::api::email_auth::common::load_template_or_builtin(&db, "verify").await;

            let body_html = tmpl.body_html.replace("{{VERIFY_URL}}", &verify_url);
            let body_text = tmpl.body_text.replace("{{VERIFY_URL}}", &verify_url);

            if let Err(e) = email_svc
                .send_email(&user_email, &tmpl.subject, &body_html, &body_text)
                .await
            {
                tracing::error!(handler = "post_register", user_email = %user_email, "failed to send verification email: {e}");
            }
        });
    }

    Ok((
        StatusCode::CREATED,
        Json(UserDto {
            id,
            email: req.email,
            display_name,
        }),
    )
        .into_response())
}

#[tracing::instrument(level = "info", skip_all)]
pub async fn post_login(
    State(state): State<AppState>,
    crate::api::email_auth::common::PeerIp(peer_ip): crate::api::email_auth::common::PeerIp,
    Json(req): Json<LoginRequest>,
) -> Result<Response, AuthError> {
    if !state
        .auth_rate_limiter
        .check_and_record(&peer_ip.to_string())
    {
        return Err(AuthError::RateLimited);
    }
    verify_turnstile(
        &state.turnstile,
        state.turnstile_verifier.as_ref(),
        req.turnstile_token.as_deref(),
    )
    .await
    .map_err(AuthError::from)?;
    let user = users::Entity::find()
        .filter(users::Column::Email.eq(req.email.clone()))
        .one(&state.db)
        .await?
        .ok_or(AuthError::InvalidCredentials)?;
    let stored_hash = user
        .password_hash
        .as_deref()
        .ok_or(AuthError::InvalidCredentials)?;
    if !verify_password(&state.argon2, stored_hash, &req.password)? {
        return Err(AuthError::InvalidCredentials);
    }
    let access = issue_access_token(
        &state.jwt_encoding_key,
        user.id,
        &user.email,
        state.access_ttl,
    )?;
    let refresh = issue_refresh_token(&state.db, &state.argon2, user.id, state.refresh_ttl).await?;
    let mut headers = HeaderMap::new();
    append_cookie(
        &mut headers,
        access_cookie(&access, state.access_ttl.as_secs()),
    );
    append_cookie(
        &mut headers,
        refresh_cookie(&refresh, state.refresh_ttl.as_secs()),
    );
    clear_legacy_refresh_cookie(&mut headers);
    Ok((
        StatusCode::OK,
        headers,
        Json(TokenResponse {
            id: user.id,
            email: user.email,
            display_name: user.display_name,
            access_token: access,
            email_verified: user.email_verified,
        }),
    )
        .into_response())
}

#[tracing::instrument(level = "info", skip_all)]
pub async fn post_refresh(
    State(state): State<AppState>,
    crate::api::email_auth::common::PeerIp(peer_ip): crate::api::email_auth::common::PeerIp,
    req: Request,
) -> Result<Response, AuthError> {
    if !state
        .refresh_rate_limiter
        .check_and_record(&peer_ip.to_string())
    {
        return Err(AuthError::RateLimited);
    }
    let (parts, _) = req.into_parts();
    // Every value, not just the first: a browser may still be sending the
    // legacy `Path=/auth/refresh` copy ahead of the live one.
    let cookies = crate::auth::middleware::extract_cookie_all(&parts, REFRESH_COOKIE);
    if cookies.is_empty() {
        return Err(AuthError::RefreshInvalid);
    }
    let candidates: Vec<&str> = cookies.iter().map(String::as_str).collect();
    let (user_id, new_refresh) =
        verify_and_rotate_any(&state.db, &state.argon2, &candidates, state.refresh_ttl).await?;
    let user = users::Entity::find_by_id(user_id)
        .one(&state.db)
        .await?
        .ok_or(AuthError::UserNotFound)?;
    let access = issue_access_token(
        &state.jwt_encoding_key,
        user.id,
        &user.email,
        state.access_ttl,
    )?;
    let mut headers = HeaderMap::new();
    append_cookie(
        &mut headers,
        access_cookie(&access, state.access_ttl.as_secs()),
    );
    append_cookie(
        &mut headers,
        refresh_cookie(&new_refresh, state.refresh_ttl.as_secs()),
    );
    clear_legacy_refresh_cookie(&mut headers);
    Ok((
        StatusCode::OK,
        headers,
        Json(serde_json::json!({ "ok": true, "access_token": access })),
    )
        .into_response())
}

#[tracing::instrument(level = "info", skip_all)]
pub async fn post_logout(
    State(state): State<AppState>,
    _claims: AccessClaims,
    crate::api::email_auth::common::PeerIp(peer_ip): crate::api::email_auth::common::PeerIp,
    req: Request,
) -> Result<Response, AuthError> {
    if !state
        .refresh_rate_limiter
        .check_and_record(&peer_ip.to_string())
    {
        return Err(AuthError::RateLimited);
    }
    let (parts, _) = req.into_parts();
    if let Some(cookie) = extract_cookie(&parts, REFRESH_COOKIE) {
        revoke(&state.db, &cookie).await?;
    }
    let mut headers = HeaderMap::new();
    append_cookie(&mut headers, expire_cookie(ACCESS_COOKIE, "/"));
    // Refresh cookie is now scoped to Path=/ (see refresh_cookie); clear it
    // there, and at the legacy path too so a stale duplicate cannot survive a
    // logout and resurface on the next session.
    append_cookie(&mut headers, expire_cookie(REFRESH_COOKIE, "/"));
    clear_legacy_refresh_cookie(&mut headers);
    Ok((
        StatusCode::OK,
        headers,
        Json(serde_json::json!({ "ok": true })),
    )
        .into_response())
}
