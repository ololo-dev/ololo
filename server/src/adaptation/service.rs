//! Adaptation service trait and associated types.
//!
//! `AdaptationService` is the single point of adaptation interaction for per-player
//! adapted test generation. One `adapt` call produces `adapted_tests` row(s)
//! for a `(player, session, task)` request.
//!
//! Production code wires `LiveAdaptationService`; tests inject
//! `FakeAdaptationService`.
use async_trait::async_trait;
use std::sync::Arc;

// ---------------------------------------------------------------------------
// Channel request type
// ---------------------------------------------------------------------------

/// Message sent through the adaptation mpsc channel to request that a single
/// task be adapted for a specific agent in a specific session.
#[derive(Debug, Clone)]
pub struct AdaptationRequest {
    pub player_id: uuid::Uuid,
    pub session_id: uuid::Uuid,
    pub task_id: uuid::Uuid,
    /// Schema version of the adaptation payload contract.
    pub schema_version: String,
    /// Shell command template from the task's `TestTemplate`; used by the
    /// passthrough service to create a `tests` row and by the LLM service
    /// as context in the adaptation prompt.
    pub command_template: String,
    /// Player profile metadata (JSON); `serde_json::Value::Null` if unavailable.
    pub player_metadata: serde_json::Value,
    /// Required project/task context fields.
    pub project_name: String,
    pub project_description: String,
    pub task_title: String,
    pub task_description: String,
    pub task_tags: Vec<String>,
    /// minijinja answer template expression from the task's `TestTemplate`.
    /// Empty string when the task has no answer template (e.g. LLM-generated tests
    /// use a concrete `expected_answer` instead).
    pub answer_template: String,
    /// JSON-serialised `Vec<FixtureDef>` from the task's `TestTemplate`.
    /// `"[]"` when the task has no fixture definitions.
    pub fixture_definitions: String,
}

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

/// Errors that can occur during a single `adapt` call.
#[derive(Debug, thiserror::Error)]
pub enum AdaptationError {
    /// The LLM call failed but retries remain — the loop should re-enqueue
    /// this request after a delay.
    #[error("llm_failed: {0}")]
    LlmFailed(String),
    /// Infrastructure failure (database, serialization). Treated as
    /// re-tryable at the loop level; logged as an error.
    #[error("db_error: {0}")]
    DbError(String),
    /// Max adaptation attempts exhausted. The `adapted_tasks` row has
    /// already been set to `status = "failed"` by the service implementation.
    /// The worker's polling loop will detect the failure and surface
    /// `ArenaFrame::Error` to the agent.
    #[error("permanent_failure")]
    PermanentFailure,
    /// The adaptation output would produce an empty `command_template`.
    /// The row is not persisted; the caller should treat this as a
    /// permanent failure for this test.
    #[error("empty_command_template")]
    EmptyCommandTemplate,
}

// ---------------------------------------------------------------------------
// Trait + handle
// ---------------------------------------------------------------------------

/// Cloneable handle for injection into `AppState` and the adaptation loop.
pub type AdaptationServiceHandle = Arc<dyn AdaptationService + Send + Sync>;

/// Adaptation service.
///
/// Each call performs one adaptation attempt for a single `(player_id, session_id,
/// task_id)` triplet. On success it writes at least one `adapted_tests` row.
/// On failure it returns
/// an `AdaptationError`; the caller (loop_task) decides whether to retry.
///
/// Implementations must be `Send + Sync`.
#[async_trait]
pub trait AdaptationService: Send + Sync {
    /// Perform one adaptation attempt for the given triplet.
    ///
    /// Returns `Ok(())` if the adapted task and test rows were written
    /// successfully (including the idempotent "already ready" case).
    async fn adapt(&self, req: AdaptationRequest) -> Result<(), AdaptationError>;
}

// ---------------------------------------------------------------------------
// No-op implementation (default for unconfigured startup)
// ---------------------------------------------------------------------------

/// A no-op `AdaptationService` used as the default in `AppState::new()`.
///
/// Every call returns `PermanentFailure` so workers surface an error
/// immediately rather than retrying indefinitely against a misconfigured
/// server. Replace via `AppState::with_adaptation_service` in integration
/// tests or production startup.
pub struct NoopAdaptationService;

#[async_trait]
impl AdaptationService for NoopAdaptationService {
    async fn adapt(&self, _req: AdaptationRequest) -> Result<(), AdaptationError> {
        Err(AdaptationError::PermanentFailure)
    }
}
