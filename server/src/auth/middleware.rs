//! Auth middleware per FR-002, FR-006.
//!
//! - `AccessClaims` extractor: pulls the access JWT from the
//!   `Authorization: Bearer` header first, then falls back to the
//!   `arena_access` cookie. 401 on absence/invalid/expired.
//! - `origin_guard`: middleware that enforces the
//!   `ARENA_FRONTEND_ORIGINS` allow-list on state-changing methods
//!   (POST/PUT/PATCH/DELETE). GET/HEAD/OPTIONS bypass.

use crate::auth::AuthError;
use crate::auth::jwt::{AccessClaims, verify_access_token};
use crate::state::AppState;
use axum::Json;
use axum::extract::{FromRequestParts, State};
use axum::http::{Method, Request, StatusCode, header, request::Parts};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

/// Cookie name carrying the access JWT.
pub const ACCESS_COOKIE: &str = "arena_access";
/// Cookie name carrying the refresh token (`<row_id>.<secret>`).
pub const REFRESH_COOKIE: &str = "arena_refresh";

#[axum::async_trait]
impl FromRequestParts<AppState> for AccessClaims {
    type Rejection = AuthError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        // Prefer `Authorization: Bearer <token>` (localStorage flow) over
        // the HttpOnly cookie (SSR / same-origin flow).
        let token = extract_bearer(parts)
            .or_else(|| extract_cookie(parts, ACCESS_COOKIE))
            .ok_or(AuthError::MissingAccessCookie)?;
        if token.starts_with("ololo_") {
            crate::auth::pat::lookup_pat(&state.db, &token).await
        } else {
            verify_access_token(&state.jwt_decoding_key, &token)
        }
    }
}

/// Extract a Bearer token from the `Authorization` header.
/// Returns `None` when the header is absent or not a Bearer scheme.
pub(crate) fn extract_bearer(parts: &Parts) -> Option<String> {
    let value = parts.headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    value.strip_prefix("Bearer ").map(str::to_owned)
}

/// Read a single cookie value from `Cookie` headers without pulling in
/// `tower-cookies`. Returns the first match (covers both single and
/// folded `Cookie:` headers).
pub(crate) fn extract_cookie(parts: &Parts, name: &str) -> Option<String> {
    extract_cookie_all(parts, name).into_iter().next()
}

/// Every value sent under `name`, in the order the browser sent them.
///
/// A name can legitimately appear more than once: cookies are keyed by
/// (name, domain, path), so a value written at one path and a value written at
/// another are two distinct cookies that are both sent when the request path
/// matches both — most-specific path first (RFC 6265 §5.4). The refresh cookie
/// moved from `Path=/auth/refresh` to `Path=/`, so browsers that authenticated
/// on both sides of that change send two `arena_refresh` values, and reading
/// only the first would always pick the stale one.
pub(crate) fn extract_cookie_all(parts: &Parts, name: &str) -> Vec<String> {
    let prefix = format!("{}=", name);
    let mut out = Vec::new();
    for hv in parts.headers.get_all(header::COOKIE).iter() {
        let Ok(s) = hv.to_str() else { continue };
        for part in s.split(';') {
            if let Some(rest) = part.trim().strip_prefix(&prefix) {
                out.push(rest.to_string());
            }
        }
    }
    out
}

/// Origin-allow-list middleware. Applies to POST/PUT/PATCH/DELETE only;
/// safe methods bypass. A missing `Origin` is only allowed when the request
/// also lacks `Sec-Fetch-Site` (i.e. is not browser-issued) — see
/// [`request_origin_allowed`].
///
/// `/auth/cli/*` routes are exempt: they are called by the native `ololo`
/// CLI (which sends no `Origin` header) and are protected by their own
/// CSRF-equivalent mechanism (PKCE-style state token + one-time code).
pub async fn origin_guard<B>(State(state): State<AppState>, req: Request<B>, next: Next) -> Response
where
    B: Send + 'static,
    Request<B>: Into<Request<axum::body::Body>>,
{
    let method = req.method().clone();
    let path = req.uri().path().to_owned();
    if !is_state_changing(&method) || path.starts_with("/auth/cli/") {
        let req: Request<axum::body::Body> = req.into();
        return next.run(req).await;
    }
    let origin = req
        .headers()
        .get(header::ORIGIN)
        .and_then(|h| h.to_str().ok());
    let sec_fetch_site = req
        .headers()
        .get("sec-fetch-site")
        .and_then(|h| h.to_str().ok());
    if !request_origin_allowed(origin, sec_fetch_site, &state.frontend_origins) {
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({ "error": "origin_not_allowed" })),
        )
            .into_response();
    }
    let req: Request<axum::body::Body> = req.into();
    next.run(req).await
}

/// CSRF allow/deny decision for a state-changing request.
///
/// - Origin present → must match an allowed frontend origin.
/// - No Origin, but `Sec-Fetch-Site` present → an anomalous *browser* request
///   (browsers always send `Origin` on state-changing requests), so it is
///   trying to dodge the Origin check: reject. This closes the historical
///   fail-open where a missing `Origin` was blanket-allowed.
/// - Neither header → a non-browser client (CLI/PAT, or SvelteKit's SSR
///   server-side fetch which forwards the cookie but sends no browser headers).
///   These are not CSRF-vulnerable, so allow.
fn request_origin_allowed(
    origin: Option<&str>,
    sec_fetch_site: Option<&str>,
    allowed_origins: &[String],
) -> bool {
    match origin {
        Some(o) => allowed_origins.iter().any(|a| a == o),
        None => sec_fetch_site.is_none(),
    }
}

fn is_state_changing(m: &Method) -> bool {
    matches!(
        *m,
        Method::POST | Method::PUT | Method::PATCH | Method::DELETE
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_methods_bypass() {
        assert!(!is_state_changing(&Method::GET));
        assert!(!is_state_changing(&Method::HEAD));
        assert!(!is_state_changing(&Method::OPTIONS));
    }

    #[test]
    fn unsafe_methods_blocked() {
        assert!(is_state_changing(&Method::POST));
        assert!(is_state_changing(&Method::PUT));
        assert!(is_state_changing(&Method::PATCH));
        assert!(is_state_changing(&Method::DELETE));
    }

    #[test]
    fn ololo_prefix_detected_correctly() {
        assert!("ololo_abc123".starts_with("ololo_"));
        assert!(!"Bearer_abc123".starts_with("ololo_"));
        assert!(!"eyJhbGciOi...".starts_with("ololo_"));
    }

    #[test]
    fn non_ololo_prefix_takes_jwt_path() {
        let key = jsonwebtoken::DecodingKey::from_secret(b"test-secret-at-least-32-bytes-long");
        let result = verify_access_token(&key, "not_a_jwt_token");
        assert!(matches!(result, Err(crate::auth::AuthError::TokenInvalid)));
    }

    #[test]
    fn origin_allowed_matches_allowlist() {
        let allowed = vec!["http://localhost:5173".to_string()];
        assert!(request_origin_allowed(
            Some("http://localhost:5173"),
            Some("same-origin"),
            &allowed
        ));
        assert!(!request_origin_allowed(
            Some("http://evil.test"),
            Some("cross-site"),
            &allowed
        ));
    }

    #[test]
    fn origin_absent_depends_on_sec_fetch_site() {
        let allowed = vec!["http://localhost:5173".to_string()];
        // Non-browser client (CLI/PAT, SSR server-side fetch): neither header.
        assert!(request_origin_allowed(None, None, &allowed));
        // Browser request that omitted Origin but sent Sec-Fetch-Site → reject.
        assert!(!request_origin_allowed(None, Some("same-origin"), &allowed));
        assert!(!request_origin_allowed(None, Some("cross-site"), &allowed));
    }
}
