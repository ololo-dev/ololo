//! `AppState` constructors, accessors, and background maintenance tasks.

use crate::adaptation::metrics::AdaptationMetrics;
use crate::adaptation::service::{
    AdaptationRequest, AdaptationServiceHandle, NoopAdaptationService,
};
use crate::api::settings::ReqwestOllamaHttp;
use crate::auth::turnstile::{TurnstileConfig, TurnstileVerifier};
use crate::email::EmailServiceHandle;
use crate::email::encryption::SettingsEncryption;
use crate::llm::{LlmServiceHandle, RigLlmService};
use crate::rate_limiter::{EmailRateLimiter, InMemoryRateLimiter, SlidingWindowLimiter};
use arena_core::protocol::ArenaFrame;
use argon2::Argon2;
use dashmap::DashMap;
use jsonwebtoken::{DecodingKey, EncodingKey};
use sea_orm::DatabaseConnection;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{Semaphore, broadcast, mpsc};

use super::{
    AdminRegistry, AppState, AuthConfig, CliAuthSession, PlayerRegistry, ProjectRegistry,
    SessionRegistry, load_imagekit,
};

/// Parse a positive integer limit from `var`, falling back to `default`.
fn env_limit(var: &str, default: usize) -> usize {
    std::env::var(var)
        .ok()
        .and_then(|v| v.parse().ok())
        .filter(|n: &usize| *n > 0)
        .unwrap_or(default)
}

impl AppState {
    /// Convenience constructor. Spawns the CLI auth cleanup task with an
    /// ephemeral shutdown channel that closes immediately (the task exits
    /// after one tick). Suitable for tests and integration harnesses.
    pub fn new(db: DatabaseConnection, cfg: AuthConfig) -> Self {
        let (_, rx) = broadcast::channel::<()>(1);
        Self::new_with_shutdown(db, cfg, rx)
    }

    /// Full constructor for production use. Spawns the CLI auth cleanup task
    /// with the supplied `shutdown` receiver — the task exits when a value is
    /// sent on the corresponding broadcast channel, enabling graceful shutdown.
    pub fn new_with_shutdown(
        db: DatabaseConnection,
        cfg: AuthConfig,
        shutdown: broadcast::Receiver<()>,
    ) -> Self {
        let session_registry: SessionRegistry = Arc::new(DashMap::new());
        let project_registry: ProjectRegistry = Arc::new(DashMap::new());
        let session_project: crate::state::SessionProjectIndex = Arc::new(DashMap::new());
        let admin_registry: AdminRegistry = Arc::new(DashMap::new());
        let player_registry: PlayerRegistry = Arc::new(DashMap::new());
        let settings_encryption = Arc::new(SettingsEncryption::new(&cfg.jwt_signing_key));
        let cli_auth_sessions: Arc<DashMap<String, CliAuthSession>> = Arc::new(DashMap::new());
        let lobby_timer_secs: u64 = std::env::var("ARENA_LOBBY_TIMER_SECS")
            .ok()
            .and_then(|v| v.parse().ok())
            .filter(|n: &u64| *n > 0)
            .unwrap_or(60);
        let shutdown2 = shutdown.resubscribe();
        tokio::spawn(cli_auth_session_ttl(
            Arc::clone(&cli_auth_sessions),
            shutdown,
        ));
        tokio::spawn(cli_tokens_gc(db.clone(), shutdown2));

        // Default adaptation channel: receiver immediately dropped.
        // Sends will return Err(SendError) until a live receiver is wired
        // via AppState::with_adaptation_tx (production startup and integration tests).
        let (adaptation_tx, _adaptation_rx) = mpsc::channel::<AdaptationRequest>(16);

        let ollama_max_concurrent: usize = std::env::var("OLLAMA_MAX_CONCURRENT")
            .ok()
            .and_then(|v| v.parse().ok())
            .filter(|n: &usize| *n > 0)
            .unwrap_or(4);
        let ollama_semaphore = Arc::new(Semaphore::new(ollama_max_concurrent));

        let (zmq_events_tx, _) = broadcast::channel::<arena_core::protocol::ZmqEvent>(256);
        let (landing_tx, _) = broadcast::channel::<arena_core::protocol::ArenaFrame>(256);

        let turnstile = TurnstileConfig::from_env();
        let turnstile_verifier: Arc<dyn TurnstileVerifier> =
            match (&turnstile.secret, &turnstile.enabled) {
                (Some(secret), true) => {
                    Arc::new(crate::auth::turnstile::ReqwestTurnstileVerifier::new(
                        secret.clone(),
                        turnstile.verify_timeout_ms,
                    ))
                }
                _ => Arc::new(crate::auth::turnstile::FakeTurnstileVerifier {
                    outcome: crate::auth::turnstile::VerifyOutcome::Valid,
                }),
            };

        Self {
            db,
            jwt_encoding_key: Arc::new(EncodingKey::from_secret(&cfg.jwt_signing_key)),
            jwt_decoding_key: Arc::new(DecodingKey::from_secret(&cfg.jwt_signing_key)),
            jwt_signing_secret: Arc::new(cfg.jwt_signing_key),
            argon2: Arc::new(Argon2::default()),
            frontend_origins: Arc::new(cfg.frontend_origins),
            access_ttl: cfg.access_ttl,
            refresh_ttl: cfg.refresh_ttl,
            max_players_per_session: cfg.max_agents_per_session,
            rate_limiter: Arc::new(InMemoryRateLimiter::new()),
            ollama_http: Arc::new(ReqwestOllamaHttp::from_env()),
            llm_service: Arc::new(RigLlmService),
            imagekit: load_imagekit(),
            email_service: None,
            email_rate_limiter: Arc::new(EmailRateLimiter::new()),
            auth_rate_limiter: Arc::new(SlidingWindowLimiter::new(
                env_limit("ARENA_AUTH_RATE_LIMIT_PER_MIN", 30),
                Duration::from_secs(60),
            )),
            refresh_rate_limiter: Arc::new(SlidingWindowLimiter::new(
                env_limit("ARENA_REFRESH_RATE_LIMIT_PER_MIN", 120),
                Duration::from_secs(60),
            )),
            settings_encryption,
            cli_auth_sessions,
            session_registry,
            project_registry,
            landing_tx,
            session_project,
            admin_registry,
            player_registry,
            lobby_timer_secs,
            adaptation_service: Arc::new(NoopAdaptationService),
            adaptation_tx,
            adaptation_metrics: Arc::new(AdaptationMetrics::new()),
            ollama_semaphore,
            subscriber_tasks: Arc::new(DashMap::new()),
            zmq_events_tx,
            turnstile,
            turnstile_verifier,
            public_api_url: std::env::var("ARENA_PUBLIC_API_URL")
                .ok()
                .filter(|s| !s.trim().is_empty())
                .map(|s| Arc::new(s.trim().to_string())),
        }
    }

    /// Replace the Ollama HTTP handle. Used by integration tests to inject a
    /// deterministic stub instead of the production `ReqwestOllamaHttp`.
    pub fn with_ollama_http(mut self, http: crate::api::settings::OllamaHttpHandle) -> Self {
        self.ollama_http = http;
        self
    }

    /// Replace the LLM service handle. Used by integration tests to inject a
    /// deterministic `FakeLlmService` instead of the production `RigLlmService`.
    pub fn with_llm_service(mut self, svc: LlmServiceHandle) -> Self {
        self.llm_service = svc;
        self
    }

    /// Replace the adaptation service handle. Used by integration tests to
    /// inject a `FakeAdaptationService` instead of `NoopAdaptationService`.
    pub fn with_adaptation_service(mut self, svc: AdaptationServiceHandle) -> Self {
        self.adaptation_service = svc;
        self
    }

    /// Replace the adaptation channel sender. Used by `main.rs` and
    /// integration tests to connect the sender to a live `AdaptationLoop`
    /// receiver. The default sender (from `new()`) is backed by a dropped
    /// receiver and will return `Err(SendError)` on every send.
    ///
    /// Also wires the same sender into `self.adaptation_tx` so that
    /// adaptation requests can be dispatched.
    pub fn with_adaptation_tx(mut self, tx: mpsc::Sender<AdaptationRequest>) -> Self {
        self.adaptation_tx = tx;
        self
    }

    /// Set the email service. Called from main.rs after DB init with
    /// credentials loaded from app_settings.
    pub fn with_email_service(mut self, svc: EmailServiceHandle) -> Self {
        self.email_service = Some(svc);
        self
    }

    /// Replace the Turnstile verifier + config. Used by integration tests
    /// to inject a `FakeTurnstileVerifier` with a preset outcome.
    pub fn with_turnstile(
        mut self,
        config: TurnstileConfig,
        verifier: Arc<dyn TurnstileVerifier>,
    ) -> Self {
        self.turnstile = config;
        self.turnstile_verifier = verifier;
        self
    }

    /// Send an admin fan-out frame to all connected admin WS clients for a session.
    ///
    /// Fire-and-forget: errors are logged at `warn` level but not propagated.
    /// No-ops if no admin entry exists for the session (no admin clients connected).
    pub fn broadcast_admin_update(&self, session_id: uuid::Uuid, frame: ArenaFrame) {
        if let Some(entry) = self.admin_registry.get(&session_id)
            && let Err(e) = entry.send(frame)
        {
            tracing::warn!(session_id = %session_id, error = %e, "broadcast_admin_update: send failed");
        }
    }
}
/// Background task: evicts expired CLI auth sessions every 10 seconds.
/// For each expired session, sends a CliAuthError before dropping.
/// Uses `DashMap::retain()` for atomic single-pass eviction (NFR-008).
async fn cli_auth_session_ttl(
    sessions: Arc<DashMap<String, CliAuthSession>>,
    mut shutdown: broadcast::Receiver<()>,
) {
    use arena_core::protocol::CliAuthFrame;
    const TTL: Duration = Duration::from_secs(120);
    let mut interval = tokio::time::interval(Duration::from_secs(10));
    loop {
        tokio::select! {
            _ = interval.tick() => {
                let now = std::time::Instant::now();
                sessions.retain(|_, session| {
                    if now.duration_since(session.created_at) >= TTL {
                        let _ = session.tx.try_send(CliAuthFrame::CliAuthError {
                            code: "session_expired".to_string(),
                            message: "The CLI auth session has expired.".to_string(),
                        });
                        tracing::debug!("cli_auth_session_ttl: evicted expired session");
                        false // remove entry
                    } else {
                        true // keep entry
                    }
                });
            }
            result = shutdown.recv() => {
                let _ = result;
                tracing::debug!("cli_auth_session_ttl: shutdown received, exiting");
                break;
            }
        }
    }
}

/// Background task: deletes expired cli_tokens rows hourly.
async fn cli_tokens_gc(db: sea_orm::DatabaseConnection, mut shutdown: broadcast::Receiver<()>) {
    use arena_core::entities::cli_tokens;
    use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
    let mut interval = tokio::time::interval(Duration::from_secs(3600));
    loop {
        tokio::select! {
            _ = interval.tick() => {
                let now = chrono::Utc::now();
                match cli_tokens::Entity::delete_many()
                    .filter(cli_tokens::Column::ExpiresAt.lt(now))
                    .exec(&db)
                    .await
                {
                    Ok(res) => tracing::debug!("cli_tokens_gc: deleted {} expired rows", res.rows_affected),
                    Err(e) => tracing::warn!("cli_tokens_gc: delete failed: {e}"),
                }
            }
            result = shutdown.recv() => {
                let _ = result;
                tracing::debug!("cli_tokens_gc: shutdown received, exiting");
                break;
            }
        }
    }
}
