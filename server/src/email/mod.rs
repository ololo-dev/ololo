//! Email service abstraction and implementations.
//!
//! [`EmailService`] is the DI trait. Production: [`ses::SesEmailService`] or
//! [`cloudflare::CloudflareEmailService`] (selected via the `email.provider`
//! setting). Tests: [`StubEmailService`].

pub mod cloudflare;
pub mod encryption;
pub mod ses;
pub mod token;

use async_trait::async_trait;
use std::sync::Arc;
use tokio::sync::Mutex;

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------
// Template types
// ---------------------------------------------------------------------------

/// A single resolved email template with subject, HTML body, and plain-text body.
#[derive(Debug, Clone)]
pub struct LoadedEmailTemplate {
    pub subject: String,
    pub body_html: String,
    pub body_text: String,
}

/// Every template the server can send, loaded once at startup.
#[derive(Debug, Clone)]
pub struct LoadedEmailTemplates {
    pub verify: LoadedEmailTemplate,
    pub reset_password: LoadedEmailTemplate,
    pub magic_link: LoadedEmailTemplate,
}

impl LoadedEmailTemplates {
    /// Read templates from `dir` at runtime.
    ///
    /// Returns `Err` (with the OS error) if any of the required files
    /// cannot be read, so the caller can log the path and fall back.
    pub fn from_dir(dir: &str) -> Result<Self, std::io::Error> {
        let p = |name: &str| format!("{dir}/{name}");
        Ok(Self {
            verify: LoadedEmailTemplate {
                subject: "Verify your ololo.dev email address".to_string(),
                body_html: std::fs::read_to_string(p("verify.html"))?,
                body_text: std::fs::read_to_string(p("verify.txt"))?,
            },
            reset_password: LoadedEmailTemplate {
                subject: "Reset your ololo.dev password".to_string(),
                body_html: std::fs::read_to_string(p("reset_password.html"))?,
                body_text: std::fs::read_to_string(p("reset_password.txt"))?,
            },
            magic_link: LoadedEmailTemplate {
                subject: "Your ololo.dev sign-in link".to_string(),
                body_html: std::fs::read_to_string(p("magic_link.html"))?,
                body_text: std::fs::read_to_string(p("magic_link.txt"))?,
            },
        })
    }

    /// Compiled-in fallback — used when the templates directory is absent
    /// (e.g. in unit/integration tests) or when `from_dir` fails.
    pub fn builtin() -> Self {
        Self {
            verify: LoadedEmailTemplate {
                subject: "Verify your ololo.dev email address".to_string(),
                body_html: include_str!("../../../email-templates/verify.html").to_string(),
                body_text: include_str!("../../../email-templates/verify.txt").to_string(),
            },
            reset_password: LoadedEmailTemplate {
                subject: "Reset your ololo.dev password".to_string(),
                body_html: include_str!("../../../email-templates/reset_password.html").to_string(),
                body_text: include_str!("../../../email-templates/reset_password.txt").to_string(),
            },
            magic_link: LoadedEmailTemplate {
                subject: "Your ololo.dev sign-in link".to_string(),
                body_html: include_str!("../../../email-templates/magic_link.html").to_string(),
                body_text: include_str!("../../../email-templates/magic_link.txt").to_string(),
            },
        }
    }
}

impl Default for LoadedEmailTemplates {
    fn default() -> Self {
        Self::builtin()
    }
}

/// Error type for email sending operations.
#[derive(Debug, thiserror::Error)]
pub enum EmailError {
    #[error("email send error: {0}")]
    SendFailure(String),
    #[error("email service not configured")]
    NotConfigured,
}

/// A single email sent via the stub — captured for test inspection.
#[derive(Debug, Clone)]
pub struct SentEmail {
    pub to: String,
    pub subject: String,
    pub body_html: String,
    pub body_text: String,
}

/// DI trait for sending transactional email.
///
/// Production implementation: [`ses::SesEmailService`].
/// Test implementation: [`StubEmailService`].
#[async_trait]
pub trait EmailService: Send + Sync {
    async fn send_email(
        &self,
        to: &str,
        subject: &str,
        body_html: &str,
        body_text: &str,
    ) -> Result<(), EmailError>;
}

/// Shared handle to an [`EmailService`] implementation.
pub type EmailServiceHandle = Arc<dyn EmailService + Send + Sync>;

/// Test stub that captures sent messages without making real SES calls.
///
/// Inspect sent messages via [`StubEmailService::sent()`].
#[derive(Default)]
pub struct StubEmailService {
    sent: Mutex<Vec<SentEmail>>,
}

impl StubEmailService {
    pub fn new() -> Self {
        Self::default()
    }

    /// Return a snapshot of all messages sent so far.
    pub async fn sent(&self) -> Vec<SentEmail> {
        self.sent.lock().await.clone()
    }
}

#[async_trait]
impl EmailService for StubEmailService {
    async fn send_email(
        &self,
        to: &str,
        subject: &str,
        body_html: &str,
        body_text: &str,
    ) -> Result<(), EmailError> {
        self.sent.lock().await.push(SentEmail {
            to: to.to_string(),
            subject: subject.to_string(),
            body_html: body_html.to_string(),
            body_text: body_text.to_string(),
        });
        Ok(())
    }
}
