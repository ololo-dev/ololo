use axum::async_trait;
use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use axum::http::{HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use sea_orm::EntityTrait;
use serde::Deserialize;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use crate::api::error::error_response;
use crate::email::token::TokenParseError;
use crate::email::{LoadedEmailTemplate, LoadedEmailTemplates};
use arena_core::entities::email_templates;

pub(crate) const TOKEN_TYPE_EMAIL_VERIFICATION: &str = "email_verification";
pub(crate) const TOKEN_TYPE_PASSWORD_RESET: &str = "password_reset";
pub(crate) const TOKEN_TYPE_MAGIC_LINK: &str = "magic_link";

pub struct PeerIp(pub IpAddr);

#[async_trait]
impl<S: Send + Sync> FromRequestParts<S> for PeerIp {
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        // Behind Cloudflare/Traefik the socket peer is a proxy address shared
        // by every visitor — resolve the real client via headers first, or the
        // email rate limits become one site-wide bucket.
        let peer = parts
            .extensions
            .get::<axum::extract::ConnectInfo<SocketAddr>>()
            .map(|ci| ci.0.ip());
        let ip = crate::client_ip::client_ip_addr(&parts.headers, peer)
            .unwrap_or(IpAddr::V4(Ipv4Addr::LOCALHOST));
        Ok(PeerIp(ip))
    }
}

#[derive(Debug, thiserror::Error)]
pub enum EmailAuthError {
    #[error("database error: {0}")]
    Db(#[from] sea_orm::DbErr),
    #[error("token parse error: {0}")]
    TokenParse(#[from] TokenParseError),
    #[error("token not found or expired")]
    TokenNotFound,
    #[error("token verification failed")]
    TokenVerifyFailed,
    #[error("user not found")]
    UserNotFound,
    #[error("email already verified")]
    AlreadyVerified,
    #[error("email service not configured")]
    ServiceUnavailable,
    #[error("rate limit exceeded")]
    RateLimited,
    #[error("invalid password")]
    InvalidPassword,
    #[error("internal error")]
    Internal,
    #[error("email send failed")]
    SendFailed,
    #[error("captcha failed")]
    Captcha(crate::auth::turnstile::CaptchaError),
}

impl IntoResponse for EmailAuthError {
    fn into_response(self) -> Response {
        match self {
            Self::TokenParse(_) => error_response(StatusCode::BAD_REQUEST, "invalid_token_format"),
            Self::TokenNotFound | Self::TokenVerifyFailed => {
                StatusCode::UNAUTHORIZED.into_response()
            }
            Self::AlreadyVerified => error_response(StatusCode::BAD_REQUEST, "already_verified"),
            Self::ServiceUnavailable => StatusCode::SERVICE_UNAVAILABLE.into_response(),
            Self::RateLimited => {
                let mut resp = error_response(StatusCode::TOO_MANY_REQUESTS, "rate_limited");
                resp.headers_mut()
                    .insert("Retry-After", HeaderValue::from_static("900"));
                resp
            }
            Self::Db(_) | Self::UserNotFound => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
            Self::InvalidPassword => error_response(StatusCode::BAD_REQUEST, "invalid_password"),
            Self::Internal => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
            Self::SendFailed => error_response(StatusCode::BAD_GATEWAY, "email_send_failed"),
            Self::Captcha(e) => e.into_response(),
        }
    }
}

#[derive(Deserialize)]
pub struct VerifyEmailQuery {
    pub token: String,
}

#[derive(Deserialize)]
pub struct ForgotPasswordRequest {
    pub email: String,
    #[serde(default)]
    pub turnstile_token: Option<String>,
}

#[derive(Deserialize)]
pub struct ResetPasswordRequest {
    pub token: String,
    pub new_password: String,
    #[serde(default)]
    pub turnstile_token: Option<String>,
}

#[derive(Deserialize)]
pub struct MagicLinkRequest {
    pub email: String,
    #[serde(default)]
    pub turnstile_token: Option<String>,
}

#[derive(Deserialize)]
pub struct MagicLinkVerifyQuery {
    pub token: String,
    pub next: Option<String>,
}

/// Load an email template from the `email_templates` table, falling back to
/// the compiled-in default when the row is missing or the lookup fails.
/// Templates must never be a reason an auth email silently doesn't go out.
pub(crate) async fn load_template_or_builtin(
    db: &sea_orm::DatabaseConnection,
    kind: &str,
) -> LoadedEmailTemplate {
    match email_templates::Entity::find_by_id(kind).one(db).await {
        Ok(Some(t)) => {
            return LoadedEmailTemplate {
                subject: t.subject,
                body_html: t.body_html,
                body_text: t.body_text,
            };
        }
        Ok(None) => {
            tracing::warn!(
                template = kind,
                "email template missing from DB — using builtin"
            );
        }
        Err(e) => {
            tracing::warn!(
                template = kind,
                "DB error loading email template ({e}) — using builtin"
            );
        }
    }
    let builtin = LoadedEmailTemplates::builtin();
    match kind {
        "reset_password" => builtin.reset_password,
        "magic_link" => builtin.magic_link,
        _ => builtin.verify,
    }
}

pub(crate) fn validate_next(next: Option<String>) -> String {
    let Some(raw) = next else {
        return "/".to_string();
    };

    // Try to parse as an absolute URL. Relative paths (/dashboard) and
    // protocol-relative URLs (//evil.com) fail parsing here → redirect to /.
    let Ok(parsed) = url::Url::parse(&raw) else {
        return "/".to_string();
    };

    // Explicit scheme allowlist — rejects javascript:, data:, and any other
    // non-HTTP scheme regardless of whether a host is present (AC-015).
    let scheme = parsed.scheme();
    if scheme != "http" && scheme != "https" {
        return "/".to_string();
    }

    // If the URL has a host, verify it matches our own origin.
    if parsed.host().is_some() {
        let base_url =
            std::env::var("PUBLIC_APP_URL").unwrap_or_else(|_| "http://localhost:5173".to_string());
        let Ok(base) = url::Url::parse(&base_url) else {
            return "/".to_string();
        };
        if parsed.host() != base.host() {
            return "/".to_string();
        }
    }

    // Extract only path + query — never forward scheme, host, or fragment.
    let path = parsed.path().to_string();
    match parsed.query() {
        Some(q) if !q.is_empty() => format!("{}?{}", path, q),
        _ => path,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_next_rejects_non_http_and_external_hosts() {
        assert_eq!(validate_next(None), "/");
        assert_eq!(validate_next(Some("/dashboard".into())), "/");
        assert_eq!(validate_next(Some("//evil.com/x".into())), "/");
        assert_eq!(validate_next(Some("javascript:alert(1)".into())), "/");
        unsafe { std::env::set_var("PUBLIC_APP_URL", "https://arena.example.com") };
        assert_eq!(validate_next(Some("https://evil.com/x".into())), "/");
        assert_eq!(
            validate_next(Some("https://arena.example.com/p?q=1".into())),
            "/p?q=1"
        );
    }
}
