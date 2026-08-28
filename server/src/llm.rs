//! Shared LLM completion service, provider-agnostic.
//!
//! The provider, model, api key, and base URL are resolved into an
//! `arena_core::llm::ModelConfig` via the admin-configured provider
//! assignments (see [`resolve_model_for_operation`]); the service just
//! executes it.

use async_trait::async_trait;
use std::sync::Arc;
use std::time::Duration;

use crate::state::AppState;
use arena_core::llm::telemetry::{self, LlmContext, LlmRequestRecord};
use arena_core::llm::{LlmError as CoreLlmError, ModelConfig};
use arena_core::settings_encryption::SettingsEncryption;
use sea_orm::DatabaseConnection;

#[derive(Debug, thiserror::Error)]
pub enum LlmError {
    #[error("ai_timeout")]
    Timeout,
    #[error("ai_error: {0}")]
    AiError(String),
}

impl From<CoreLlmError> for LlmError {
    fn from(e: CoreLlmError) -> Self {
        LlmError::AiError(e.to_string())
    }
}

/// Multi-provider LLM completion service stored in `AppState`. Callers pass
/// the fully resolved `ModelConfig` per call.
#[async_trait]
pub trait LlmService: Send + Sync {
    async fn complete(
        &self,
        cfg: &ModelConfig,
        system: &str,
        user: &str,
    ) -> Result<String, LlmError>;

    /// Like [`LlmService::complete`], but records one `llm_requests`
    /// telemetry row for the call.
    ///
    /// The default implementation times the delegated `complete` call and
    /// records duration/status/response-chars with zero token counts (the
    /// trait exposes no usage), so test stubs implementing only `complete`
    /// keep working unchanged. The production [`RigLlmService`] overrides it
    /// to route through `arena_core::llm::telemetry::complete_recorded`,
    /// which captures real provider-reported token usage.
    async fn complete_recorded(
        &self,
        db: &DatabaseConnection,
        cfg: &ModelConfig,
        operation: &str,
        ctx: LlmContext,
        system: &str,
        user: &str,
    ) -> Result<String, LlmError> {
        let started = std::time::Instant::now();
        let result = self.complete(cfg, system, user).await;
        let mut rec = LlmRequestRecord::new(operation, cfg, ctx)
            .with_duration_ms(started.elapsed().as_millis() as i64)
            .with_detail_json(telemetry::completion_detail_json(
                system,
                user,
                result.as_deref().ok(),
            ));
        if let Err(e) = &result {
            rec = rec.failed(&e.to_string());
        }
        telemetry::record_llm_request(db, rec).await;
        result
    }
}

/// Cloneable handle for `AppState` injection (mirrors `OllamaHttpHandle` pattern).
pub type LlmServiceHandle = Arc<dyn LlmService + Send + Sync>;

/// Production implementation: delegates to `ModelConfig::complete` (the only
/// provider dispatch lives in `arena_core::llm`).
pub struct RigLlmService;

#[async_trait]
impl LlmService for RigLlmService {
    async fn complete(
        &self,
        cfg: &ModelConfig,
        system: &str,
        user: &str,
    ) -> Result<String, LlmError> {
        cfg.complete(system, user).await.map_err(LlmError::from)
    }

    async fn complete_recorded(
        &self,
        db: &DatabaseConnection,
        cfg: &ModelConfig,
        operation: &str,
        ctx: LlmContext,
        system: &str,
        user: &str,
    ) -> Result<String, LlmError> {
        // The core implementation records the row itself (with real token
        // totals captured through the run recorder).
        telemetry::complete_recorded(cfg, db, operation, ctx, system, user)
            .await
            .map_err(LlmError::from)
    }
}

/// Run a recorded LLM `complete` call with a hard timeout, returning the
/// shared [`LlmError`]. Callers map it into their own error type via `From`.
///
/// Every call lands one `llm_requests` telemetry row: the inner
/// `complete_recorded` records success/failure, and the timeout path (which
/// drops the inner future before it can record) records a failed row here.
pub async fn complete_with_timeout(
    state: &AppState,
    cfg: &ModelConfig,
    operation: &str,
    ctx: LlmContext,
    system: &str,
    user: &str,
    timeout: Duration,
) -> Result<String, LlmError> {
    let inner =
        state
            .llm_service
            .complete_recorded(&state.db, cfg, operation, ctx.clone(), system, user);
    match tokio::time::timeout(timeout, inner).await {
        Ok(result) => result,
        Err(_) => {
            let rec = LlmRequestRecord::new(operation, cfg, ctx)
                .with_duration_ms(timeout.as_millis() as i64)
                .failed("ai_timeout");
            telemetry::record_llm_request(&state.db, rec).await;
            Err(LlmError::Timeout)
        }
    }
}

/// Resolve the model for one LLM-backed operation ("adaptation",
/// "project_ai", "memory", "judge") through the admin-configured provider
/// assignments (`llm_op_<op>` → `llm_default`, see
/// `arena_core::llm::resolve`).
///
/// When nothing is configured, returns a keyless Ollama config with an
/// **empty model** — the existing "not configured" sentinel consumers
/// already check before issuing a call.
///
/// Only the first candidate: a pool assignment degrades to its top member
/// here. Callers that can retry should use
/// [`resolve_candidates_for_operation`] with [`complete_with_failover`].
pub async fn resolve_model_for_operation(
    db: &DatabaseConnection,
    encryption: &SettingsEncryption,
    operation: &str,
) -> ModelConfig {
    arena_core::llm::resolve::resolve_operation_model(
        db,
        encryption,
        operation,
        &arena_core::llm::resolve::LlmOverride::none(),
    )
    .await
    .unwrap_or_else(|| ModelConfig {
        provider_name: None,
        provider: "ollama".into(),
        model: String::new(),
        base_url: None,
        api_key: None,
    })
}

/// Resolve the ordered candidate list for one operation. Empty means
/// nothing is configured; a single-model assignment yields one entry and a
/// pool yields its members tier by tier.
pub async fn resolve_candidates_for_operation(
    db: &DatabaseConnection,
    encryption: &SettingsEncryption,
    operation: &str,
) -> Vec<ModelConfig> {
    arena_core::llm::resolve::resolve_operation_candidates(
        db,
        encryption,
        operation,
        &arena_core::llm::resolve::LlmOverride::none(),
    )
    .await
}

/// Run [`complete_with_timeout`] against each candidate in order, returning
/// the first success.
///
/// Any error — timeout, rate limit, provider outage — moves on to the next
/// candidate, because from here the failures are indistinguishable and the
/// point of a pool is to route around whichever one happened. The last
/// error is returned once every candidate has been tried, so a fully-down
/// pool still surfaces a real provider error rather than a generic one.
///
/// Each attempt records its own `llm_requests` row (via
/// `complete_with_timeout`), so telemetry shows the failed candidates as
/// well as the one that answered.
pub async fn complete_with_failover(
    state: &AppState,
    candidates: &[ModelConfig],
    operation: &str,
    ctx: LlmContext,
    system: &str,
    user: &str,
    timeout: Duration,
) -> Result<String, LlmError> {
    let mut last_error: Option<LlmError> = None;
    for (idx, cfg) in candidates.iter().enumerate() {
        match complete_with_timeout(state, cfg, operation, ctx.clone(), system, user, timeout).await
        {
            Ok(out) => {
                if idx > 0 {
                    tracing::info!(
                        operation,
                        provider = %cfg.provider,
                        model = %cfg.model,
                        failed_candidates = idx,
                        "llm failover: candidate answered after earlier ones failed"
                    );
                }
                return Ok(out);
            }
            Err(e) => {
                tracing::warn!(
                    operation,
                    provider = %cfg.provider,
                    model = %cfg.model,
                    candidate = idx + 1,
                    of = candidates.len(),
                    error = %e,
                    "llm failover: candidate failed"
                );
                last_error = Some(e);
            }
        }
    }
    Err(last_error.unwrap_or_else(|| LlmError::AiError("no_model_configured".into())))
}

/// Telemetry retention: delete `llm_requests` rows older than 30 days.
/// Called once at server boot; a single portable DELETE (plain column
/// comparison, no backend-specific SQL). Errors are logged, never fatal.
pub async fn purge_old_llm_telemetry(db: &DatabaseConnection) {
    use arena_core::entities::llm_requests;
    use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};

    let cutoff = chrono::Utc::now() - chrono::Duration::days(30);
    match llm_requests::Entity::delete_many()
        .filter(llm_requests::Column::CreatedAt.lt(cutoff))
        .exec(db)
        .await
    {
        Ok(res) => {
            tracing::info!(
                deleted = res.rows_affected,
                "llm telemetry: retention purge (>30 days) complete"
            );
        }
        Err(e) => {
            tracing::warn!(error = %e, "llm telemetry: retention purge failed");
        }
    }
}
