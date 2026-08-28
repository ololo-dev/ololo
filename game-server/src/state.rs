use arena_core::entities::sessions;
use arena_core::protocol::{ArenaFrame, ZmqEvent};
use arena_core::session_status::SessionStatus;
use chrono::{DateTime, Utc};
use dashmap::DashMap;
use jsonwebtoken::{DecodingKey, EncodingKey};
use sea_orm::prelude::Expr;
use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter};
use std::sync::{Arc, RwLock};
use tokio::sync::{Semaphore, broadcast, mpsc};
use uuid::Uuid;

use crate::zmq_pub::EventPublisher;

pub struct SessionCacheInner {
    pub session_id: Uuid,
    pub phase: SessionStatus,
    pub version: u64,
    pub participants: Vec<arena_core::protocol::MemberInfo>,
    pub leaderboard: Vec<arena_core::protocol::LeaderboardEntry>,
    pub started_at: Option<DateTime<Utc>>,
}

impl SessionCacheInner {
    pub fn new(session_id: Uuid, phase: SessionStatus, started_at: Option<DateTime<Utc>>) -> Self {
        Self {
            session_id,
            phase,
            version: 0,
            participants: Vec::new(),
            leaderboard: Vec::new(),
            started_at,
        }
    }
}

pub struct SessionEntry {
    pub tx: broadcast::Sender<ArenaFrame>,
    pub cache: Arc<RwLock<SessionCacheInner>>,
    pub cancel: tokio_util::sync::CancellationToken,
}

pub type SessionRegistry = Arc<DashMap<String, SessionEntry>>;
pub type PlayerAgentRegistry =
    Arc<DashMap<Uuid, mpsc::Sender<arena_core::protocol::PlayerAgentFrame>>>;

#[derive(Clone)]
pub struct GameServerState {
    pub db: DatabaseConnection,
    pub server_id: Uuid,
    pub advertise_url: String,
    pub jwt_encoding_key: Arc<EncodingKey>,
    pub jwt_decoding_key: Arc<DecodingKey>,
    pub jwt_signing_secret: Arc<Vec<u8>>,
    pub session_registry: SessionRegistry,
    pub player_agent_registry: PlayerAgentRegistry,
    pub lobby_timer_secs: u64,
    pub event_publisher: Arc<dyn EventPublisher>,
    pub judge_semaphore: Arc<Semaphore>,
    /// Decrypts `llm_providers.api_key_enc` for per-run model resolution.
    pub settings_encryption: Arc<arena_core::settings_encryption::SettingsEncryption>,
}

impl GameServerState {
    /// Resolve the model for `operation` via admin-configured providers
    /// (per-judge override → `llm_op_<operation>` → `llm_default`).
    /// `override_*` carries a per-judge override when present.
    ///
    /// Last-resort fallback when no provider/assignment exists yet (e.g. the
    /// game server boots before the main server ever seeded the LLM
    /// defaults): a static keyless Ollama config on `OLLAMA_URL` (or the
    /// registry default endpoint) with the registry default model.
    pub async fn resolve_llm(
        &self,
        operation: &str,
        over: &arena_core::llm::resolve::LlmOverride<'_>,
    ) -> arena_core::llm::ModelConfig {
        self.resolve_llm_candidates(operation, over)
            .await
            .swap_remove(0)
    }

    /// Every candidate for `operation`, in failover order — a pool assignment
    /// expands to its members (tier by tier), anything else to one entry.
    ///
    /// Never empty: the same last-resort Ollama config [`resolve_llm`]
    /// documents is returned as a single candidate when nothing resolves, so
    /// callers can index the list without a length check.
    pub async fn resolve_llm_candidates(
        &self,
        operation: &str,
        over: &arena_core::llm::resolve::LlmOverride<'_>,
    ) -> Vec<arena_core::llm::ModelConfig> {
        let candidates = arena_core::llm::resolve::resolve_operation_candidates(
            &self.db,
            &self.settings_encryption,
            operation,
            over,
        )
        .await;
        if !candidates.is_empty() {
            return candidates;
        }
        // Loud, because this is a misroute rather than a default: whatever
        // the admin configured did not resolve (no provider rows yet, or
        // the assigned provider was disabled/deleted), so the work silently
        // went to a local Ollama that may not even hold the model. The
        // per-request telemetry records provider/model, so a run that
        // landed here is identifiable after the fact too.
        tracing::warn!(
            operation,
            "no LLM provider resolved; falling back to keyless local Ollama — \
             check the provider assignments in admin settings"
        );
        vec![arena_core::llm::ModelConfig {
            provider_name: None,
            provider: "ollama".to_string(),
            model: "llama3.2".to_string(),
            base_url: std::env::var("OLLAMA_URL").ok().filter(|v| !v.is_empty()),
            api_key: None,
        }]
    }
}

/// Bump the cached session version, returning the new value (0 if no entry).
pub fn bump_session_version(state: &GameServerState, join_code: &str) -> u64 {
    if let Some(entry) = state.session_registry.get(join_code)
        && let Ok(mut cache) = entry.cache.write()
    {
        cache.version = cache.version.saturating_add(1);
        return cache.version;
    }
    0
}

/// Broadcast `frame` to the session's subscribers (no-op if no entry).
pub fn broadcast_frame(state: &GameServerState, join_code: &str, frame: ArenaFrame) {
    if let Some(entry) = state.session_registry.get(join_code) {
        let _ = entry.tx.send(frame);
    }
}

/// FR-012 remaining arithmetic: `max(0, duration - elapsed + paused)`.
/// Single source of truth shared by session_lifecycle and recovery.
pub fn compute_remaining(
    duration_secs: u64,
    started_at: DateTime<Utc>,
    now: DateTime<Utc>,
    paused_duration_secs: Option<i64>,
) -> i64 {
    // `elapsed` is wall-clock time since started_at, which keeps growing during pause.
    // `paused_duration_secs` is the accumulated pause window; adding it back cancels
    // the pause window out of elapsed so `remaining` stays frozen across pause/resume.
    let elapsed = now.signed_duration_since(started_at).num_seconds();
    (duration_secs as i64 - elapsed + paused_duration_secs.unwrap_or(0)).max(0)
}

/// Transition a session to Finished: bump version, broadcast
/// `SessionComplete`, publish the FR-014 `SessionStatus` event, and update
/// the DB row for any non-terminal status.
pub async fn finish_session(
    state: &GameServerState,
    session_id: Uuid,
    join_code: &str,
    reason: &str,
) {
    let version = bump_session_version(state, join_code);
    if let Some(entry) = state.session_registry.get(join_code)
        && let Ok(mut cache) = entry.cache.write()
    {
        cache.phase = SessionStatus::Finished;
    }
    broadcast_frame(
        state,
        join_code,
        ArenaFrame::SessionComplete {
            reason: reason.to_string(),
            version,
        },
    );
    state
        .event_publisher
        .publish(&ZmqEvent::SessionStatus {
            join_code: join_code.to_string(),
            status: SessionStatus::Finished.to_string(),
            version,
            cancel_reason: None,
            cancelled_by: None,
        })
        .await;

    let now = Utc::now();
    // Conditional UPDATE … WHERE status IN (Running, Paused): the DB is the
    // final word on double-finish races (two last finishers, or a finisher
    // racing the running timer) — exactly one caller flips the row, and only
    // that caller runs the awards block below.
    let update = sessions::Entity::update_many()
        .col_expr(
            sessions::Column::Status,
            Expr::value(SessionStatus::Finished),
        )
        .col_expr(sessions::Column::FinishedAt, Expr::value(Some(now)))
        .filter(sessions::Column::Id.eq(session_id))
        .filter(sessions::Column::Status.is_in([SessionStatus::Running, SessionStatus::Paused]))
        .exec(&state.db)
        .await;
    match update {
        Err(e) => {
            tracing::error!(session_id = %session_id, error = %e, "finish_session: DB update failed");
        }
        Ok(res) if res.rows_affected == 0 => {
            // Session was not Running/Paused (already finished or cancelled
            // elsewhere) — lost the race, nothing to award.
        }
        Ok(_) => {
            // Judges may still be pending no matter how the session ended:
            // time expiry interrupts players mid-task, and even the everyone-
            // finished path races the final task's judge queue — those runs
            // were enqueued at task completion and deliver their point deltas
            // minutes later. Awards written before they settle freeze a stale
            // total into session_awards.game_points, and the project's Top
            // Players board then disagrees with the session report forever
            // (prod session 36N4MS: report 398, award 316). So ALWAYS count
            // the expected-but-unsettled judge runs and defer the awards when
            // any exist; the no-judges case still finds zero pending and
            // stays synchronous (and deterministic for callers/tests).
            // Cross-session copy/paste validation runs before the award
            // decision so its deterministic score adjustment is already a
            // task_results row when totals are aggregated — on both the
            // immediate and the deferred award path. Bounded per player;
            // any failure degrades to "no penalty".
            crate::similarity::run_similarity_checks(state, session_id, join_code).await;

            let pending = arena_core::session_completion::expired_session_pending_judges(
                &state.db, session_id,
            )
            .await
            .unwrap_or_else(|e| {
                tracing::error!(session_id = %session_id, error = %e, "finish_session: pending-judge check failed; awarding immediately");
                0
            });
            if pending == 0 {
                settle_and_publish(state, session_id, join_code, now).await;
            } else {
                tracing::info!(
                    session_id = %session_id,
                    pending,
                    "finish_session: deferring settlement until expected judge runs finish"
                );
                let state = state.clone();
                let join_code = join_code.to_string();
                tokio::spawn(async move {
                    expiry_judges_then_award(
                        &state,
                        session_id,
                        &join_code,
                        std::time::Duration::from_secs(3),
                        std::time::Duration::from_secs(600),
                    )
                    .await;
                });
            }
        }
    }
}

/// Announce that a finished session's standings are final.
///
/// Split out from the finish transition because it also runs after the
/// post-expiry judge-settle wait and from the recovery sweep: whichever
/// path gets there first, the pages learn the scores stopped moving.
pub async fn settle_and_publish(
    state: &GameServerState,
    session_id: Uuid,
    join_code: &str,
    now: DateTime<Utc>,
) {
    state
        .event_publisher
        .publish(&ZmqEvent::SessionSettled {
            join_code: join_code.to_string(),
            session_id,
            timestamp: now,
        })
        .await;
}

/// Post-expiry awards flow: run judges for tasks interrupted by the time
/// signal, then poll until every expected judge run for the session is
/// terminal (scored or failed), then award Arena Points. `deadline` bounds
/// the wait — a judge run lost before its result row was created (e.g. a
/// restart) must not starve participants of their points forever.
pub async fn expiry_judges_then_award(
    state: &GameServerState,
    session_id: Uuid,
    join_code: &str,
    poll: std::time::Duration,
    deadline: std::time::Duration,
) {
    crate::judge_queue::run_interrupted_task_judges(state, session_id).await;
    // Session-scoped judges run after the interrupted-task judges (so the
    // final snapshot commit has landed) and before the settle poll below,
    // which is what waits for their rows.
    crate::judge_queue::run_session_judges(state, session_id).await;

    let started = tokio::time::Instant::now();
    loop {
        match arena_core::session_completion::expired_session_pending_judges(&state.db, session_id)
            .await
        {
            Ok(0) => break,
            Ok(pending) => {
                if started.elapsed() >= deadline {
                    tracing::warn!(
                        session_id = %session_id,
                        pending,
                        "expiry awards: judge-settle deadline reached; awarding with pending runs"
                    );
                    break;
                }
            }
            Err(e) => {
                tracing::error!(session_id = %session_id, error = %e, "expiry awards: pending-judge check failed; awarding");
                break;
            }
        }
        tokio::time::sleep(poll).await;
    }
    settle_and_publish(state, session_id, join_code, Utc::now()).await;
}

/// Finish the session with `"all_tasks_completed"` iff every eligible
/// (non-revoked) player has completed all of their tasks. Returns whether
/// this call requested the finish. Safe under concurrent invocation: the
/// conditional UPDATE inside [`finish_session`] lets exactly one racer apply
/// the transition.
pub async fn finish_session_if_all_done(
    state: &GameServerState,
    session_id: Uuid,
    join_code: &str,
) -> bool {
    match arena_core::session_completion::all_eligible_players_done(&state.db, session_id).await {
        Ok(true) => {
            finish_session(state, session_id, join_code, "all_tasks_completed").await;
            true
        }
        Ok(false) => false,
        Err(e) => {
            tracing::error!(session_id = %session_id, error = %e, "finish_session_if_all_done: completion check failed");
            false
        }
    }
}

/// One player has exhausted their tasks. Build the per-player
/// `SessionComplete` acknowledgment for THAT player (reason
/// `player_tasks_completed` — the session may still be running for others)
/// and finish the session only when every eligible player is now done (the
/// LAST finisher ends the session). Returns the frame to send to the player
/// plus whether this call finished the session.
pub async fn on_player_tasks_exhausted(
    state: &GameServerState,
    session_id: Uuid,
    join_code: &str,
) -> (arena_core::protocol::PlayerAgentFrame, bool) {
    let frame = arena_core::protocol::PlayerAgentFrame::SessionComplete {
        session_id,
        reason: Some(
            arena_core::protocol::SESSION_COMPLETE_REASON_PLAYER_TASKS_COMPLETED.to_string(),
        ),
    };
    let finished = finish_session_if_all_done(state, session_id, join_code).await;
    (frame, finished)
}
