use crate::auth::jwt::AccessClaims;
use crate::auth::pat::{generate_pat, hash_pat};
use crate::state::AppState;
use arena_core::entities::cli_tokens;
use arena_core::protocol::CliAuthFrame;
use axum::Json;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Redirect, Response};
use chrono::Utc;
use sea_orm::{ActiveModelTrait, ActiveValue};
use serde::Deserialize;
use serde_json::json;
use uuid::Uuid;

// ──────────────────────────────────────────────
// Request / response types
// ──────────────────────────────────────────────

#[derive(Deserialize)]
pub struct CliTokenParams {
    pub cli_token: String,
}

#[derive(Deserialize)]
pub struct ConfirmRequest {
    pub cli_token: String,
}

// ──────────────────────────────────────────────
// Validation helper
// ──────────────────────────────────────────────

/// Returns true if `s` is exactly 64 lowercase hex characters.
fn is_valid_cli_token(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}

// ──────────────────────────────────────────────
// Handlers
// ──────────────────────────────────────────────

/// `GET /auth/cli/post-auth?cli_token=…`
///
/// Legacy entry point. It must **never** mint a PAT from a bare
/// cookie-authenticated GET: that is an account-takeover CSRF / session-fixation
/// vector — an attacker who holds their own pending `cli_token` can lure a
/// logged-in victim to this URL and capture a PAT for the *victim's* account
/// (SameSite=Lax sends the cookie on top-level navigations, and this route is
/// exempt from the Origin guard). Token minting lives solely in
/// `POST /auth/cli/confirm`, which requires an explicit user action on the
/// same-origin `/cli-auth` page. Here we only redirect to that page.
pub async fn get_cli_post_auth(Query(params): Query<CliTokenParams>) -> Response {
    if !is_valid_cli_token(&params.cli_token) {
        return Redirect::to("/?auth_error=invalid_cli_token").into_response();
    }
    // cli_token is validated as 64 lowercase hex above, so it is URL-safe.
    Redirect::to(&format!("/cli-auth?cli_token={}", params.cli_token)).into_response()
}

/// `POST /auth/cli/confirm`
///
/// JSON endpoint for authenticated browser sessions to confirm a CLI auth
/// request programmatically (e.g. from the settings page).
pub async fn post_cli_confirm(
    State(app): State<AppState>,
    claims: AccessClaims,
    Json(body): Json<ConfirmRequest>,
) -> Response {
    // 1. Parse user_id from claims
    let user_id = match claims.user_id() {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": "invalid_claims"})),
            )
                .into_response();
        }
    };

    // 2. Atomically claim the session
    let (_, session) = match app.cli_auth_sessions.remove(&body.cli_token) {
        Some(entry) => entry,
        None => {
            return (
                StatusCode::GONE,
                Json(json!({"error": "session_not_found"})),
            )
                .into_response();
        }
    };

    // 3. Generate PAT
    let raw_pat = generate_pat();

    // 4. Insert into DB
    let new_token = cli_tokens::ActiveModel {
        id: ActiveValue::Set(Uuid::new_v4()),
        token_hash: ActiveValue::Set(hash_pat(&raw_pat)),
        user_id: ActiveValue::Set(user_id),
        created_at: ActiveValue::Set(Utc::now().fixed_offset()),
        expires_at: ActiveValue::Set((Utc::now() + chrono::Duration::days(365)).fixed_offset()),
    };
    if new_token.insert(&app.db).await.is_err() {
        let _ = session
            .tx
            .send(CliAuthFrame::CliAuthError {
                code: "db_error".to_string(),
                message: "Failed to store token.".to_string(),
            })
            .await;
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": "db_error"})),
        )
            .into_response();
    }

    // 5. Notify waiting CLI
    let _ = session
        .tx
        .send(CliAuthFrame::CliAuthSuccess { token: raw_pat })
        .await;

    // 6. Return success
    (StatusCode::OK, Json(json!({"ok": true}))).into_response()
}

// ──────────────────────────────────────────────
// Tests
// ──────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_cli_token_format_accepts_64_lowercase_hex() {
        let valid = "a".repeat(64);
        assert!(
            valid.len() == 64
                && valid
                    .bytes()
                    .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        );
    }

    #[test]
    fn validate_cli_token_format_rejects_short() {
        let short = "a".repeat(32);
        assert!(short.len() != 64);
    }

    #[test]
    fn validate_cli_token_format_rejects_uppercase() {
        let upper = "A".repeat(64);
        assert!(!is_valid_cli_token(&upper));
    }

    #[test]
    fn validate_cli_token_format_rejects_non_hex() {
        let bad = "g".repeat(64);
        assert!(!is_valid_cli_token(&bad));
    }

    #[test]
    fn validate_cli_token_accepts_valid() {
        let valid = format!("{:0>64}", "abcdef0123456789".repeat(4));
        assert!(is_valid_cli_token(&valid));
    }

    #[test]
    fn dashmap_second_claim_returns_none() {
        use crate::state::CliAuthSession;
        use dashmap::DashMap;
        use std::sync::Arc;

        let sessions: Arc<DashMap<String, CliAuthSession>> = Arc::new(DashMap::new());
        let (tx, _rx) = tokio::sync::mpsc::channel(1);
        sessions.insert(
            "abc123".to_string(),
            CliAuthSession {
                tx,
                created_at: std::time::Instant::now(),
            },
        );
        // First remove succeeds
        assert!(sessions.remove("abc123").is_some());
        // Second remove returns None — atomic first-claimer-wins
        assert!(sessions.remove("abc123").is_none());
    }
}
