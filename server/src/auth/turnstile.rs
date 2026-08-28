//! Cloudflare Turnstile CAPTCHA verification.
//!
//! Siteverify HTTP I/O is abstracted behind [`TurnstileVerifier`] so
//! integration tests inject canned responses. Production wiring uses
//! [`ReqwestTurnstileVerifier`] which drives `reqwest` (already a dep).

use async_trait::async_trait;
use std::fmt;

/// Outcome of a Turnstile token verification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerifyOutcome {
    /// Token is valid — proceed with the request.
    Valid,
    /// Token is invalid (bad, expired, or already used) — reject.
    Invalid,
    /// Network error or timeout contacting Cloudflare — apply fail policy.
    Error,
}

/// Cloudflare Turnstile token verifier. Production impl hits the network;
/// test impls return canned outcomes (trait-DI, no wiremock).
#[async_trait]
pub trait TurnstileVerifier: Send + Sync + 'static {
    async fn verify(&self, token: &str) -> VerifyOutcome;
}

/// Configuration for Turnstile. Loaded from env at startup.
#[derive(Clone)]
pub struct TurnstileConfig {
    pub enabled: bool,
    pub sitekey: Option<String>,
    pub secret: Option<String>,
    pub fail_open: bool,
    pub verify_timeout_ms: u64,
}

impl fmt::Debug for TurnstileConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TurnstileConfig")
            .field("enabled", &self.enabled)
            .field("sitekey", &self.sitekey)
            .field("secret", &"<redacted>")
            .field("fail_open", &self.fail_open)
            .field("verify_timeout_ms", &self.verify_timeout_ms)
            .finish()
    }
}

impl TurnstileConfig {
    /// Load from env vars. Defaults: disabled, fail-open, 3s timeout.
    pub fn from_env() -> Self {
        let enabled = std::env::var("TURNSTILE_ENABLED")
            .ok()
            .and_then(|v| v.parse::<bool>().ok())
            .unwrap_or(false);
        let sitekey = std::env::var("TURNSTILE_SITEKEY")
            .ok()
            .filter(|s| !s.is_empty());
        let secret = std::env::var("TURNSTILE_SECRET")
            .ok()
            .filter(|s| !s.is_empty());
        let fail_open = std::env::var("TURNSTILE_FAIL_OPEN")
            .ok()
            .and_then(|v| v.parse::<bool>().ok())
            .unwrap_or(true);
        let verify_timeout_ms = std::env::var("TURNSTILE_VERIFY_TIMEOUT_MS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(3000);

        if enabled && secret.is_none() {
            tracing::warn!(
                "TURNSTILE_ENABLED=true but TURNSTILE_SECRET is unset — \
                 handlers will fail-closed with captcha_required"
            );
        }

        Self {
            enabled,
            sitekey,
            secret,
            fail_open,
            verify_timeout_ms,
        }
    }
}

const SITEVERIFY_URL: &str = "https://challenges.cloudflare.com/turnstile/v0/siteverify";

/// Production verifier using reqwest. POSTs `secret` + `response` form fields
/// to Cloudflare's siteverify endpoint.
pub struct ReqwestTurnstileVerifier {
    client: reqwest::Client,
    secret: String,
}

impl ReqwestTurnstileVerifier {
    pub fn new(secret: String, timeout_ms: u64) -> Self {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_millis(timeout_ms))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        Self { client, secret }
    }
}

#[derive(serde::Deserialize)]
struct SiteverifyResponse {
    success: bool,
}

#[async_trait]
impl TurnstileVerifier for ReqwestTurnstileVerifier {
    async fn verify(&self, token: &str) -> VerifyOutcome {
        let result = self
            .client
            .post(SITEVERIFY_URL)
            .form(&[("secret", self.secret.as_str()), ("response", token)])
            .send()
            .await;
        match result {
            Ok(resp) => match resp.json::<SiteverifyResponse>().await {
                Ok(body) => {
                    if body.success {
                        VerifyOutcome::Valid
                    } else {
                        VerifyOutcome::Invalid
                    }
                }
                Err(_) => VerifyOutcome::Error,
            },
            Err(_) => VerifyOutcome::Error,
        }
    }
}

/// Fake verifier for tests. Returns a preset outcome.
pub struct FakeTurnstileVerifier {
    pub outcome: VerifyOutcome,
}

#[async_trait]
impl TurnstileVerifier for FakeTurnstileVerifier {
    async fn verify(&self, _token: &str) -> VerifyOutcome {
        self.outcome
    }
}

/// Error codes returned by the Turnstile gate. Each maps to HTTP 403
/// with a JSON body `{"error": "<code>"}`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptchaError {
    /// Token missing or empty when verification is enabled.
    Required,
    /// Token present but Cloudflare rejected it.
    Invalid,
    /// Siteverify returned an error and fail-open is disabled.
    Error,
    /// Verifier is enabled but the secret is unconfigured.
    Unconfigured,
}

impl CaptchaError {
    pub fn code(&self) -> &'static str {
        match self {
            CaptchaError::Required => "captcha_required",
            CaptchaError::Invalid => "captcha_invalid",
            CaptchaError::Error => "captcha_error",
            CaptchaError::Unconfigured => "captcha_required",
        }
    }

    pub fn http_status(&self) -> axum::http::StatusCode {
        axum::http::StatusCode::FORBIDDEN
    }
}

impl axum::response::IntoResponse for CaptchaError {
    fn into_response(self) -> axum::response::Response {
        (
            self.http_status(),
            axum::Json(serde_json::json!({ "error": self.code() })),
        )
            .into_response()
    }
}

/// Run the Turnstile gate against a token. Returns `Ok(())` to proceed,
/// `Err(CaptchaError)` to reject with 403.
///
/// Logic:
/// - `!enabled` → Ok (skip).
/// - `enabled && secret.is_none()` → Err(Unconfigured) + WARN.
/// - `enabled && token.is_none()` → Err(Required) (fail-closed).
/// - `enabled && token.is_some()` → call verifier:
///   - Valid → Ok.
///   - Invalid → Err(Invalid).
///   - Error → if fail_open → Ok + WARN; else → Err(Error).
pub async fn verify_turnstile(
    config: &TurnstileConfig,
    verifier: &dyn TurnstileVerifier,
    token: Option<&str>,
) -> Result<(), CaptchaError> {
    if !config.enabled {
        return Ok(());
    }
    if config.secret.is_none() {
        tracing::warn!("turnstile enabled but secret unset — failing closed");
        return Err(CaptchaError::Unconfigured);
    }
    let token = match token.filter(|t| !t.is_empty()) {
        Some(t) => t,
        None => return Err(CaptchaError::Required),
    };
    match verifier.verify(token).await {
        VerifyOutcome::Valid => Ok(()),
        VerifyOutcome::Invalid => Err(CaptchaError::Invalid),
        VerifyOutcome::Error => {
            if config.fail_open {
                tracing::warn!("turnstile siteverify error — fail-open, proceeding");
                Ok(())
            } else {
                Err(CaptchaError::Error)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    // Serialize env-var-touching tests — they race in parallel.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn fake_verifier_returns_preset_valid() {
        let fake = FakeTurnstileVerifier {
            outcome: VerifyOutcome::Valid,
        };
        let rt = tokio::runtime::Runtime::new().unwrap();
        assert_eq!(rt.block_on(fake.verify("any")), VerifyOutcome::Valid);
    }

    #[test]
    fn fake_verifier_returns_preset_invalid() {
        let fake = FakeTurnstileVerifier {
            outcome: VerifyOutcome::Invalid,
        };
        let rt = tokio::runtime::Runtime::new().unwrap();
        assert_eq!(rt.block_on(fake.verify("any")), VerifyOutcome::Invalid);
    }

    #[test]
    fn fake_verifier_returns_preset_error() {
        let fake = FakeTurnstileVerifier {
            outcome: VerifyOutcome::Error,
        };
        let rt = tokio::runtime::Runtime::new().unwrap();
        assert_eq!(rt.block_on(fake.verify("any")), VerifyOutcome::Error);
    }

    #[test]
    fn config_from_env_defaults_disabled() {
        let _guard = ENV_LOCK.lock().unwrap();
        // safety: serialized by ENV_LOCK
        unsafe {
            std::env::remove_var("TURNSTILE_ENABLED");
            std::env::remove_var("TURNSTILE_SITEKEY");
            std::env::remove_var("TURNSTILE_SECRET");
            std::env::remove_var("TURNSTILE_FAIL_OPEN");
            std::env::remove_var("TURNSTILE_VERIFY_TIMEOUT_MS");
        }
        let cfg = TurnstileConfig::from_env();
        assert!(!cfg.enabled);
        assert!(cfg.sitekey.is_none());
        assert!(cfg.secret.is_none());
        assert!(cfg.fail_open);
        assert_eq!(cfg.verify_timeout_ms, 3000);
    }

    #[test]
    fn config_from_env_enabled() {
        let _guard = ENV_LOCK.lock().unwrap();
        // safety: serialized by ENV_LOCK
        unsafe {
            std::env::set_var("TURNSTILE_ENABLED", "true");
            std::env::set_var("TURNSTILE_SITEKEY", "1x00000000000000000000AA");
            std::env::set_var("TURNSTILE_SECRET", "1x0000000000000000000000000000000AA");
            std::env::set_var("TURNSTILE_FAIL_OPEN", "false");
            std::env::set_var("TURNSTILE_VERIFY_TIMEOUT_MS", "5000");
        }
        let cfg = TurnstileConfig::from_env();
        assert!(cfg.enabled);
        assert_eq!(cfg.sitekey.as_deref(), Some("1x00000000000000000000AA"));
        assert_eq!(
            cfg.secret.as_deref(),
            Some("1x0000000000000000000000000000000AA")
        );
        assert!(!cfg.fail_open);
        assert_eq!(cfg.verify_timeout_ms, 5000);
        // cleanup
        // safety: serialized by ENV_LOCK
        unsafe {
            std::env::remove_var("TURNSTILE_ENABLED");
            std::env::remove_var("TURNSTILE_SITEKEY");
            std::env::remove_var("TURNSTILE_SECRET");
            std::env::remove_var("TURNSTILE_FAIL_OPEN");
            std::env::remove_var("TURNSTILE_VERIFY_TIMEOUT_MS");
        }
    }

    #[test]
    fn config_debug_redacts_secret() {
        let cfg = TurnstileConfig {
            enabled: true,
            sitekey: Some("xxx".to_string()),
            secret: Some("super-secret-value".to_string()),
            fail_open: true,
            verify_timeout_ms: 3000,
        };
        let s = format!("{cfg:?}");
        assert!(s.contains("<redacted>"));
        assert!(!s.contains("super-secret-value"));
    }

    fn cfg(enabled: bool, secret: Option<&str>, fail_open: bool) -> TurnstileConfig {
        TurnstileConfig {
            enabled,
            sitekey: Some("test".to_string()),
            secret: secret.map(|s| s.to_string()),
            fail_open,
            verify_timeout_ms: 3000,
        }
    }

    #[tokio::test]
    async fn verify_disabled_skips() {
        let c = cfg(false, Some("s"), true);
        let fake = FakeTurnstileVerifier {
            outcome: VerifyOutcome::Invalid,
        };
        assert_eq!(verify_turnstile(&c, &fake, None).await, Ok(()));
    }

    #[tokio::test]
    async fn verify_enabled_no_secret_fails_closed() {
        let c = cfg(true, None, true);
        let fake = FakeTurnstileVerifier {
            outcome: VerifyOutcome::Valid,
        };
        assert_eq!(
            verify_turnstile(&c, &fake, Some("tok")).await,
            Err(CaptchaError::Unconfigured)
        );
    }

    #[tokio::test]
    async fn verify_enabled_none_token_rejected() {
        let c = cfg(true, Some("s"), true);
        let fake = FakeTurnstileVerifier {
            outcome: VerifyOutcome::Valid,
        };
        assert_eq!(
            verify_turnstile(&c, &fake, None).await,
            Err(CaptchaError::Required)
        );
    }

    #[tokio::test]
    async fn verify_enabled_empty_token_rejected() {
        let c = cfg(true, Some("s"), true);
        let fake = FakeTurnstileVerifier {
            outcome: VerifyOutcome::Valid,
        };
        assert_eq!(
            verify_turnstile(&c, &fake, Some("")).await,
            Err(CaptchaError::Required)
        );
    }

    #[tokio::test]
    async fn verify_enabled_valid_token_passes() {
        let c = cfg(true, Some("s"), true);
        let fake = FakeTurnstileVerifier {
            outcome: VerifyOutcome::Valid,
        };
        assert_eq!(verify_turnstile(&c, &fake, Some("tok")).await, Ok(()));
    }

    #[tokio::test]
    async fn verify_enabled_invalid_token_rejected() {
        let c = cfg(true, Some("s"), true);
        let fake = FakeTurnstileVerifier {
            outcome: VerifyOutcome::Invalid,
        };
        assert_eq!(
            verify_turnstile(&c, &fake, Some("tok")).await,
            Err(CaptchaError::Invalid)
        );
    }

    #[tokio::test]
    async fn verify_error_fail_open_proceeds() {
        let c = cfg(true, Some("s"), true);
        let fake = FakeTurnstileVerifier {
            outcome: VerifyOutcome::Error,
        };
        assert_eq!(verify_turnstile(&c, &fake, Some("tok")).await, Ok(()));
    }

    #[tokio::test]
    async fn verify_error_fail_closed_rejected() {
        let c = cfg(true, Some("s"), false);
        let fake = FakeTurnstileVerifier {
            outcome: VerifyOutcome::Error,
        };
        assert_eq!(
            verify_turnstile(&c, &fake, Some("tok")).await,
            Err(CaptchaError::Error)
        );
    }
}
