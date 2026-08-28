//! Shared application state.
//!
//! Extends the bootstrap `AppState` with auth configuration: JWT keys,
//! Argon2 hasher, frontend origin allow-list, and token TTLs. Constructed
//! once at startup from environment variables; cloned per request via
//! `Arc` internals.
use crate::adaptation::metrics::AdaptationMetrics;
use crate::adaptation::service::{AdaptationRequest, AdaptationServiceHandle};
use crate::api::settings::OllamaHttpHandle;
use crate::auth::AuthError;
use crate::auth::turnstile::{TurnstileConfig, TurnstileVerifier};
use crate::email::EmailServiceHandle;
use crate::email::encryption::SettingsEncryption;
use crate::llm::LlmServiceHandle;
use crate::rate_limiter::RateLimiter;
use arena_core::protocol::{ArenaFrame, LeaderboardEntry, MemberInfo};
use arena_core::session_status::SessionStatus;

use argon2::Argon2;
use dashmap::DashMap;
use jsonwebtoken::{DecodingKey, EncodingKey};
use sea_orm::DatabaseConnection;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{Semaphore, broadcast, mpsc};

/// Cached live session state for snapshot-on-connect.
/// Initialized on session creation; mutated at defined callsites only.
/// Wrapped in `RwLock` inside `SessionEntry`; acquire lock, mutate, release,
/// then send — never hold the lock across a `tx.send()`.
pub struct SessionCacheInner {
    pub session_id: uuid::Uuid,
    pub phase: SessionStatus,
    pub version: u64,
    pub participants: Vec<MemberInfo>,
    pub leaderboard: Vec<LeaderboardEntry>,
    pub started_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// One broadcast channel entry for an active session lobby/competition.
/// Keyed by `join_code` in `AppState::session_registry`.
/// Capacity 256: enough for all connected dashboard clients per session.
pub struct SessionEntry {
    pub tx: broadcast::Sender<ArenaFrame>,
    pub cache: Arc<std::sync::RwLock<SessionCacheInner>>,
}

/// In-memory fan-out registry for session dashboard WebSocket clients.
/// Keyed by `join_code` (String); value is a broadcast sender.
/// Cloneable handle — `Arc<DashMap<...>>` internally.
pub type SessionRegistry = Arc<DashMap<String, SessionEntry>>;

/// In-memory fan-out registry for project-level WebSocket observers.
/// Keyed by `project_id` (Uuid); value is a broadcast sender carrying
/// `ProjectSessionUpdate` frames. Channels are created lazily on first
/// subscriber connect and live for the server lifetime.
/// Capacity 64 per channel — fewer expected subscribers than per-session channels.
pub type ProjectRegistry = Arc<DashMap<uuid::Uuid, tokio::sync::broadcast::Sender<ArenaFrame>>>;

/// Lazy `join_code` → `(session_id, project_id)` index for the ZMQ bridge.
/// See [`AppState::session_project`].
pub type SessionProjectIndex = Arc<DashMap<String, (uuid::Uuid, uuid::Uuid)>>;

/// In-memory fan-out registry for admin WebSocket clients.
/// Keyed by `session_id` (String — TEXT PK); value is a broadcast sender.
/// Created on first admin WS connect for a session; removed on last disconnect
/// or when the session transitions to finished/cancelled.
/// Capacity 64 per channel.
pub type AdminRegistry = Arc<DashMap<uuid::Uuid, tokio::sync::broadcast::Sender<ArenaFrame>>>;

/// In-memory fan-out registry for player WebSocket connections.
/// Keyed by `player_id` (Uuid); value is a `PlayerChannel` wrapping a broadcast
/// sender, a monotonic sequence counter, and a mutex for atomic seq+send.
/// Channels are created on player WS connect; removed on disconnect.
pub type PlayerRegistry = Arc<DashMap<uuid::Uuid, Arc<PlayerChannel>>>;

pub struct PlayerChannel {
    pub sender: broadcast::Sender<crate::protocol::PlayerFrame>,
    pub seq: std::sync::atomic::AtomicU64,
    lock: std::sync::Mutex<()>,
}

impl PlayerChannel {
    pub fn new(sender: broadcast::Sender<crate::protocol::PlayerFrame>) -> Self {
        Self {
            sender,
            seq: std::sync::atomic::AtomicU64::new(0),
            lock: std::sync::Mutex::new(()),
        }
    }

    /// Atomically increments the sequence counter, constructs the frame, and
    /// sends it via the broadcast channel. The mutex ensures no two concurrent
    /// callers produce out-of-order frames.
    ///
    /// SAFETY: mutex guard is dropped before any .await — the entire critical
    /// section is synchronous (broadcast::Sender::send takes &self, not &mut).
    #[allow(clippy::result_large_err)]
    pub fn send_sequenced(
        &self,
        make_frame: impl FnOnce(u64) -> crate::protocol::PlayerFrame,
    ) -> Result<usize, tokio::sync::broadcast::error::SendError<crate::protocol::PlayerFrame>> {
        let _guard = self.lock.lock().unwrap();
        let seq = self.seq.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let frame = make_frame(seq);
        self.sender.send(frame)
    }

    pub fn subscribe(&self) -> broadcast::Receiver<crate::protocol::PlayerFrame> {
        self.sender.subscribe()
    }
}

/// One in-flight WebSocket CLI auth session. Keyed by `cli_token` in `AppState::cli_auth_sessions`.
/// The sender carries at most one terminal frame (success or error) — bounded channel capacity 1.
pub struct CliAuthSession {
    pub tx: tokio::sync::mpsc::Sender<crate::protocol::CliAuthFrame>,
    pub created_at: std::time::Instant,
}

/// ImageKit upload configuration. Loaded from env; `None` disables the
/// avatar-auth endpoint (NFR-004, FR-010).
#[derive(Clone, Debug)]
pub struct ImageKitConfig {
    pub public_key: String,
    /// Private key used server-side only for HMAC-SHA1 signature generation.
    /// NEVER serialised to any response (NFR-001).
    pub private_key: String,
    /// Full upload endpoint URL, e.g. `https://ik.imagekit.io/your_id`.
    pub url_endpoint: String,
}

#[derive(Clone)]
pub struct AppState {
    pub db: DatabaseConnection,
    pub jwt_encoding_key: Arc<EncodingKey>,
    pub jwt_decoding_key: Arc<DecodingKey>,
    /// Raw JWT signing secret. Required by issuers that need to sign with
    /// non-access claim shapes (e.g. observer tokens per FR-016) where
    /// the `EncodingKey` indirection is inconvenient.
    pub jwt_signing_secret: Arc<Vec<u8>>,
    pub argon2: Arc<Argon2<'static>>,
    pub frontend_origins: Arc<Vec<String>>,
    pub access_ttl: Duration,
    pub refresh_ttl: Duration,
    /// Cap on live players per session, enforced on both the agent-register
    /// and join-by-code paths. Loaded from `ARENA_MAX_AGENTS_PER_SESSION`
    /// (default 16).
    pub max_players_per_session: u32,
    /// Rate limiter for the join-code endpoint (FR-JC-011/012/013).
    /// Injected via trait so tests can swap in `NoOpRateLimiter`.
    pub rate_limiter: Arc<dyn RateLimiter + Send + Sync>,
    /// HTTP abstraction for Ollama model listing. Tests inject a stub;
    /// production wires `ReqwestOllamaHttp` (Commandment 2, FR-016).
    pub ollama_http: OllamaHttpHandle,
    /// LLM completion service for AI project actions.
    /// Tests inject a `FakeLlmService`; production wires `RigLlmService`.
    pub llm_service: LlmServiceHandle,
    /// ImageKit upload config (avatar feature). `None` when env vars are
    /// absent; the avatar-auth endpoint returns 503 in that case (NFR-004).
    pub imagekit: Option<ImageKitConfig>,
    /// Email sending service (FR-001). None when SES credentials are absent.
    /// Hard-gated endpoints return 503 when this is None.
    pub email_service: Option<EmailServiceHandle>,
    /// Rate limiter for email-sending endpoints (FR-013).
    /// 5 requests per 15-minute window per source IP.
    pub email_rate_limiter: Arc<dyn RateLimiter + Send + Sync>,
    /// Rate limiter for the credential endpoints (`/auth/login`,
    /// `/auth/register`). `ARENA_AUTH_RATE_LIMIT_PER_MIN` per IP per
    /// 60-second window (default 30 — sized for an event venue behind one
    /// NAT, not a single user).
    pub auth_rate_limiter: Arc<dyn RateLimiter + Send + Sync>,
    /// Rate limiter for the token endpoints (`/auth/refresh`,
    /// `/auth/logout`). `ARENA_REFRESH_RATE_LIMIT_PER_MIN` per IP per
    /// 60-second window (default 120).
    pub refresh_rate_limiter: Arc<dyn RateLimiter + Send + Sync>,
    /// AES-256-GCM encryption for sensitive app_settings values.
    /// Always constructed from JWT_SIGNING_KEY at startup.
    pub settings_encryption: Arc<SettingsEncryption>,
    /// In-flight WebSocket CLI auth sessions, keyed by cli_token (64-char hex).
    /// Each session holds a bounded-1 channel sender for exactly one terminal frame.
    pub cli_auth_sessions: Arc<DashMap<String, CliAuthSession>>,
    /// Fan-out broadcast registry for session dashboard WebSocket clients.
    /// Keyed by join_code; each entry holds a broadcast sender for that session.
    /// Inserted on session creation, evicted on session finish/cancel.
    pub session_registry: SessionRegistry,
    /// Fan-out broadcast registry for project-level WebSocket observers.
    /// Keyed by project_id; channels are created lazily on first WS connect.
    /// No eviction — channels live for the server lifetime (projects are bounded).
    pub project_registry: ProjectRegistry,
    /// Single broadcast channel for the landing page's live-sessions block
    /// (`/ws/landing/observe`, unauthenticated). Carries the same
    /// `ProjectSessionUpdate` / countdown frames as the project observers, but
    /// only for sessions of PUBLIC projects — a private project's join code is
    /// an invite, and broadcasting it here would publish it.
    pub landing_tx: tokio::sync::broadcast::Sender<ArenaFrame>,
    /// Resolves `join_code` → `(session_id, project_id)` for the ZMQ bridge.
    /// ZMQ events carry only a join code while project observers are keyed by
    /// project id, and timer ticks arrive once a second per live session —
    /// looking that up in the DB every tick would be a query per second.
    /// Populated lazily; entries are tiny and bounded by sessions seen.
    pub session_project: SessionProjectIndex,
    /// Fan-out broadcast registry for admin WebSocket clients.
    /// Keyed by session_id (TEXT); created on first admin WS connect.
    /// Evicted on last subscriber disconnect or when session finishes/cancels.
    pub admin_registry: AdminRegistry,
    /// Fan-out broadcast registry for player WebSocket connections.
    /// Keyed by player_id (Uuid); channels are created on player WS connect
    /// and removed on disconnect.
    pub player_registry: PlayerRegistry,
    /// Lobby countdown duration in seconds. Loaded from `ARENA_LOBBY_TIMER_SECS`
    /// (default 60). Controls how long the lobby countdown broadcasts before
    /// transitioning the session to `running`.
    pub lobby_timer_secs: u64,
    /// LLM-backed task and test adaptation service.
    /// Tests inject a `FakeAdaptationService`; production wires `LiveAdaptationService`.
    /// Default: `NoopAdaptationService` (returns errors on all calls).
    pub adaptation_service: AdaptationServiceHandle,
    /// Channel sender for dispatching `AdaptationRequest`s to the background
    /// `AdaptationLoop` task (buffer ≥ 16). In `AppState::new()` the receiver
    /// is dropped; wire a live receiver via `AppState::with_adaptation_tx` in
    /// `main.rs` and integration tests that exercise adaptation.
    pub adaptation_tx: mpsc::Sender<AdaptationRequest>,
    /// Bounded-cardinality counters for adaptation retries/outcomes.
    pub adaptation_metrics: Arc<AdaptationMetrics>,
    /// Limits concurrent Ollama LLM calls.
    /// Capacity read from `OLLAMA_MAX_CONCURRENT` env var (default 4).
    pub ollama_semaphore: Arc<Semaphore>,
    /// ZMQ subscriber tasks, keyed by game_server_id. Each entry is a
    /// JoinHandle for a long-running SUB loop. On crash the supervisor
    /// removes the entry and re-spawns.
    pub subscriber_tasks: Arc<DashMap<uuid::Uuid, tokio::task::JoinHandle<()>>>,
    /// Broadcast channel for ZMQ events received from game servers.
    /// Admin WS clients subscribe to this for the live event stream.
    pub zmq_events_tx: broadcast::Sender<arena_core::protocol::ZmqEvent>,
    /// Cloudflare Turnstile CAPTCHA config + verifier. Verifier is behind
    /// a trait so tests inject `FakeTurnstileVerifier`. When `enabled` is
    /// false, all handlers skip verification.
    pub turnstile: TurnstileConfig,
    pub turnstile_verifier: Arc<dyn TurnstileVerifier>,
    /// Public origin of this API (`ARENA_PUBLIC_API_URL`), for building
    /// absolute URLs handed to external services. Falls back to the first
    /// configured frontend origin, which is the same host in every
    /// deployment we run.
    pub public_api_url: Option<Arc<String>>,
}

impl AppState {
    /// Origin the browser is on — where a finished checkout returns to.
    pub fn public_site_url(&self) -> String {
        self.frontend_origins
            .first()
            .map(|o| o.trim_end_matches('/').to_string())
            .unwrap_or_default()
    }

    /// Origin monobank calls back on. Same host as the site unless an
    /// operator splits them with `ARENA_PUBLIC_API_URL`.
    pub fn public_api_url(&self) -> String {
        match self.public_api_url.as_ref() {
            Some(url) => url.trim_end_matches('/').to_string(),
            None => self.public_site_url(),
        }
    }
}

/// Configuration loaded from environment for production startup.
pub struct AuthConfig {
    pub jwt_signing_key: Vec<u8>,
    pub frontend_origins: Vec<String>,
    pub access_ttl: Duration,
    pub refresh_ttl: Duration,
    /// FR-039: max active agents per session (default 16).
    pub max_agents_per_session: u32,
}

impl AuthConfig {
    /// Load auth configuration from environment variables.
    ///
    /// Required: `JWT_SIGNING_KEY`, `ARENA_FRONTEND_ORIGINS`.
    /// Optional: `ARENA_ACCESS_TOKEN_TTL_SECONDS` (default 900),
    /// `ARENA_REFRESH_TOKEN_TTL_DAYS` (default 30).
    pub fn from_env() -> Result<Self, AuthError> {
        let jwt_signing_key = std::env::var("JWT_SIGNING_KEY")
            .map_err(|_| AuthError::Config("JWT_SIGNING_KEY missing"))?
            .into_bytes();
        if jwt_signing_key.len() < 32 {
            return Err(AuthError::Config("JWT_SIGNING_KEY must be >= 32 bytes"));
        }
        let frontend_origins = std::env::var("ARENA_FRONTEND_ORIGINS")
            .map_err(|_| AuthError::Config("ARENA_FRONTEND_ORIGINS missing"))?
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>();
        if frontend_origins.is_empty() {
            return Err(AuthError::Config("ARENA_FRONTEND_ORIGINS empty"));
        }
        let access_ttl = Duration::from_secs(
            std::env::var("ARENA_ACCESS_TOKEN_TTL_SECONDS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(900),
        );
        let refresh_ttl = Duration::from_secs(
            std::env::var("ARENA_REFRESH_TOKEN_TTL_DAYS")
                .ok()
                .and_then(|v| v.parse::<u64>().ok())
                .unwrap_or(30)
                * 86_400,
        );
        Ok(Self {
            jwt_signing_key,
            frontend_origins,
            access_ttl,
            refresh_ttl,
            max_agents_per_session: std::env::var("ARENA_MAX_AGENTS_PER_SESSION")
                .ok()
                .and_then(|v| v.parse().ok())
                .filter(|n: &u32| *n > 0)
                .unwrap_or(16),
        })
    }
}

fn load_imagekit() -> Option<ImageKitConfig> {
    let public_key = std::env::var("IMAGEKIT_PUBLIC_KEY").ok()?;
    let private_key = std::env::var("IMAGEKIT_PRIVATE_KEY").ok()?;
    let url_endpoint = std::env::var("IMAGEKIT_URL_ENDPOINT").ok()?;
    if public_key.is_empty() || private_key.is_empty() || url_endpoint.is_empty() {
        return None;
    }
    Some(ImageKitConfig {
        public_key,
        private_key,
        url_endpoint,
    })
}

pub mod app_state;

#[cfg(test)]
mod tests;
