//! Unified LLM request telemetry.
//!
//! Every LLM request the platform issues — judge runs, session-memory
//! extraction, task adaptation, project/task AI helpers — lands one row in
//! `llm_requests` via [`record_llm_request`]. Recording is strictly
//! best-effort: a failed insert logs a warning and returns; telemetry must
//! never fail or slow the caller path beyond the insert itself.
//!
//! [`complete_recorded`] mirrors [`ModelConfig::complete`] but routes the
//! call through a [`JudgeRunRecorder`] so real provider token usage is
//! captured, then records the row.

use std::path::PathBuf;
use std::sync::Arc;

use sea_orm::{ActiveValue::Set, DatabaseConnection, EntityTrait};
use uuid::Uuid;

use crate::entities::llm_requests;
use crate::judging::{AgentResponse, JudgeRunRecorder, truncate_chars};
use crate::llm::{LlmError, ModelConfig};

/// `llm_requests.status` values.
pub const LLM_STATUS_OK: &str = "ok";
pub const LLM_STATUS_FAILED: &str = "failed";

/// Size caps keeping a telemetry row bounded.
const ERROR_CAP: usize = 1024;
const RESPONSE_EXCERPT_CAP: usize = 2000;
const USER_EXCERPT_CAP: usize = 1000;
/// Cap on the serialized per-turn trace stored per row (~0.5 MB).
const EVENTS_JSON_CAP: usize = 512 * 1024;

/// Optional attribution for a recorded request. All fields nullable —
/// operations without session/player context leave them `None`.
#[derive(Debug, Clone, Default)]
pub struct LlmContext {
    pub session_id: Option<Uuid>,
    pub player_id: Option<Uuid>,
    pub task_id: Option<Uuid>,
    pub judge_slug: Option<String>,
}

/// One `llm_requests` row about to be inserted.
#[derive(Debug, Clone)]
pub struct LlmRequestRecord {
    pub operation: String,
    pub provider: String,
    /// Display name of the provider row used, when there was one.
    pub provider_name: Option<String>,
    pub model: String,
    /// [`LLM_STATUS_OK`] | [`LLM_STATUS_FAILED`].
    pub status: String,
    /// Truncated to 1KB at insert time.
    pub error: Option<String>,
    pub tokens_input: i64,
    pub tokens_output: i64,
    pub tokens_cache_read: i64,
    pub tokens_cache_write: i64,
    pub duration_ms: i64,
    pub session_id: Option<Uuid>,
    pub player_id: Option<Uuid>,
    pub task_id: Option<Uuid>,
    pub judge_slug: Option<String>,
    /// Bounded JSON context (see [`completion_detail_json`]).
    pub detail_json: Option<String>,
    /// Full per-turn trace (see [`events_json`]).
    pub events_json: Option<String>,
}

impl LlmRequestRecord {
    /// A successful zero-token record for `operation` against `cfg`;
    /// callers layer on failures, durations, tokens, and detail.
    pub fn new(operation: &str, cfg: &ModelConfig, ctx: LlmContext) -> Self {
        Self {
            operation: operation.to_string(),
            provider: cfg.provider.clone(),
            provider_name: cfg.provider_name.clone(),
            model: cfg.model.clone(),
            status: LLM_STATUS_OK.to_string(),
            error: None,
            tokens_input: 0,
            tokens_output: 0,
            tokens_cache_read: 0,
            tokens_cache_write: 0,
            duration_ms: 0,
            session_id: ctx.session_id,
            player_id: ctx.player_id,
            task_id: ctx.task_id,
            judge_slug: ctx.judge_slug,
            detail_json: None,
            events_json: None,
        }
    }

    /// Mark the request failed with `error` (truncated at insert time).
    pub fn failed(mut self, error: &str) -> Self {
        self.status = LLM_STATUS_FAILED.to_string();
        self.error = Some(error.to_string());
        self
    }

    pub fn with_duration_ms(mut self, duration_ms: i64) -> Self {
        self.duration_ms = duration_ms;
        self
    }

    /// Token totals in `(input, output, cache_read, cache_write)` order —
    /// matches `JudgeRunRecorder::token_totals` + `cache_token_totals`.
    pub fn with_tokens(
        mut self,
        input: u64,
        output: u64,
        cache_read: u64,
        cache_write: u64,
    ) -> Self {
        self.tokens_input = input as i64;
        self.tokens_output = output as i64;
        self.tokens_cache_read = cache_read as i64;
        self.tokens_cache_write = cache_write as i64;
        self
    }

    pub fn with_detail_json(mut self, detail_json: String) -> Self {
        self.detail_json = Some(detail_json);
        self
    }

    /// Attach the recorder's per-turn trace, bounded by [`events_json`].
    pub fn with_events(mut self, recorder: &JudgeRunRecorder) -> Self {
        self.events_json = events_json(recorder);
        self
    }
}

/// Bounded `detail_json` payload for a plain completion: prompt/response
/// sizes plus short excerpts (admin-only surface).
pub fn completion_detail_json(system: &str, user: &str, response: Option<&str>) -> String {
    serde_json::json!({
        "system_chars": system.chars().count(),
        "user_chars": user.chars().count(),
        "response_chars": response.map(|r| r.chars().count()).unwrap_or(0),
        "response_excerpt": response.map(|r| truncate_chars(r, RESPONSE_EXCERPT_CAP)),
        "user_excerpt": truncate_chars(user, USER_EXCERPT_CAP),
    })
    .to_string()
}

/// Serialize a recorder's events for `llm_requests.events_json` — the
/// per-turn trace behind the telemetry drawer (prompt, completion, message
/// transcript, tool args/results, per-turn tokens).
///
/// Two-stage bounding keeps one row from ballooning: the full payload is
/// used when it fits [`EVENTS_JSON_CAP`]; otherwise the (largest) `messages`
/// transcripts are dropped and the slimmer payload retried. A trace that is
/// still too big is dropped entirely rather than stored half-parsed — the
/// row's aggregate columns and `detail_json` still describe the request.
pub fn events_json(recorder: &JudgeRunRecorder) -> Option<String> {
    let mut events = recorder.events();
    if events.is_empty() {
        return None;
    }
    let full = serde_json::to_string(&events).ok()?;
    if full.len() <= EVENTS_JSON_CAP {
        return Some(full);
    }
    for e in &mut events {
        e.messages = None;
    }
    let slim = serde_json::to_string(&events).ok()?;
    (slim.len() <= EVENTS_JSON_CAP).then_some(slim)
}

/// Insert one telemetry row. Best-effort: a DB error logs a warning and
/// returns `()` — telemetry must never fail the caller path.
pub async fn record_llm_request(db: &DatabaseConnection, rec: LlmRequestRecord) {
    let am = llm_requests::ActiveModel {
        id: Set(Uuid::new_v4()),
        operation: Set(rec.operation),
        provider: Set(rec.provider),
        provider_name: Set(rec.provider_name),
        model: Set(rec.model),
        status: Set(rec.status),
        error: Set(rec.error.map(|e| truncate_chars(&e, ERROR_CAP))),
        tokens_input: Set(rec.tokens_input),
        tokens_output: Set(rec.tokens_output),
        tokens_cache_read: Set(rec.tokens_cache_read),
        tokens_cache_write: Set(rec.tokens_cache_write),
        duration_ms: Set(rec.duration_ms),
        session_id: Set(rec.session_id),
        player_id: Set(rec.player_id),
        task_id: Set(rec.task_id),
        judge_slug: Set(rec.judge_slug),
        detail_json: Set(rec.detail_json),
        events_json: Set(rec.events_json),
        created_at: Set(chrono::Utc::now()),
    };
    if let Err(e) = llm_requests::Entity::insert(am).exec(db).await {
        tracing::warn!(error = %e, "llm telemetry: insert failed");
    }
}

/// Plain completion with telemetry: mirrors [`ModelConfig::complete`] but
/// builds the client with a [`JudgeRunRecorder`] so provider-reported token
/// usage is captured, times the call, and records one `llm_requests` row
/// (ok or failed) before returning the result.
pub async fn complete_recorded(
    cfg: &ModelConfig,
    db: &DatabaseConnection,
    operation: &str,
    ctx: LlmContext,
    system: &str,
    user: &str,
) -> Result<String, LlmError> {
    let recorder = Arc::new(JudgeRunRecorder::default());
    let started = std::time::Instant::now();
    let result: Result<String, LlmError> = async {
        let judge = cfg.build_judge_recorded(PathBuf::new(), None, None, Some(recorder.clone()))?;
        match judge.run_agent(system, user, vec![], None).await? {
            AgentResponse::Final { text } => Ok(text),
            AgentResponse::ToolCall { .. } => Err(LlmError::ClientBuild(
                "unexpected tool call in plain completion".into(),
            )),
        }
    }
    .await;
    let duration_ms = started.elapsed().as_millis() as i64;

    let (tokens_in, tokens_out) = recorder.token_totals();
    let (cache_read, cache_write) = recorder.cache_token_totals();
    let mut rec = LlmRequestRecord::new(operation, cfg, ctx)
        .with_duration_ms(duration_ms)
        .with_tokens(tokens_in, tokens_out, cache_read, cache_write)
        .with_detail_json(completion_detail_json(system, user, result.as_deref().ok()))
        .with_events(&recorder);
    if let Err(e) = &result {
        rec = rec.failed(&e.to_string());
    }
    record_llm_request(db, rec).await;

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detail_json_bounds_long_response_and_user_excerpts() {
        let response = "r".repeat(50_000);
        let user = "u".repeat(20_000);
        let detail = completion_detail_json("sys", &user, Some(&response));
        let v: serde_json::Value = serde_json::from_str(&detail).expect("valid json");

        assert_eq!(v["system_chars"], 3);
        assert_eq!(v["user_chars"], 20_000);
        assert_eq!(v["response_chars"], 50_000);

        let excerpt = v["response_excerpt"].as_str().expect("excerpt");
        assert!(excerpt.starts_with("rrr"));
        // 2000-char excerpt plus the truncation marker, nothing more.
        assert_eq!(
            excerpt.chars().count(),
            RESPONSE_EXCERPT_CAP + "…[truncated]".chars().count()
        );

        let user_excerpt = v["user_excerpt"].as_str().expect("user excerpt");
        assert_eq!(
            user_excerpt.chars().count(),
            USER_EXCERPT_CAP + "…[truncated]".chars().count()
        );

        // The whole payload stays bounded.
        assert!(
            detail.chars().count() < 3_500,
            "detail stays small: {}",
            detail.len()
        );
    }

    #[test]
    fn events_json_drops_transcripts_then_the_whole_trace_when_oversized() {
        use crate::judging::JudgeLogEvent;

        // Fits verbatim.
        let rec = JudgeRunRecorder::default();
        rec.record(JudgeLogEvent {
            at_ms: 1,
            kind: "llm".to_string(),
            input: Some("prompt".to_string()),
            output: Some("answer".to_string()),
            messages: Some(serde_json::json!([{"role": "assistant"}])),
            ..Default::default()
        });
        let json = events_json(&rec).expect("small trace is stored");
        assert!(json.contains("\"messages\""), "{json}");

        // Oversized transcripts are dropped, the rest of the trace kept.
        let rec = JudgeRunRecorder::default();
        rec.record(JudgeLogEvent {
            at_ms: 1,
            kind: "llm".to_string(),
            input: Some("prompt".to_string()),
            messages: Some(serde_json::json!([{"blob": "m".repeat(EVENTS_JSON_CAP + 10)}])),
            ..Default::default()
        });
        let json = events_json(&rec).expect("slimmed trace is stored");
        assert!(!json.contains("\"messages\""), "transcript dropped: {json}");
        assert!(json.contains("prompt"), "rest of the event kept: {json}");

        // Beyond the cap even without transcripts: nothing is stored.
        let rec = JudgeRunRecorder::default();
        rec.record(JudgeLogEvent {
            at_ms: 1,
            kind: "llm".to_string(),
            input: Some("i".repeat(EVENTS_JSON_CAP + 10)),
            ..Default::default()
        });
        assert!(events_json(&rec).is_none(), "oversized trace is dropped");

        // Nothing recorded -> no trace.
        assert!(events_json(&JudgeRunRecorder::default()).is_none());
    }

    #[test]
    fn detail_json_short_response_is_verbatim_and_failed_has_null_excerpt() {
        let detail = completion_detail_json("s", "hello", Some("world"));
        let v: serde_json::Value = serde_json::from_str(&detail).unwrap();
        assert_eq!(v["response_excerpt"], "world");
        assert_eq!(v["user_excerpt"], "hello");

        let failed = completion_detail_json("s", "hello", None);
        let v: serde_json::Value = serde_json::from_str(&failed).unwrap();
        assert!(v["response_excerpt"].is_null());
        assert_eq!(v["response_chars"], 0);
    }

    #[test]
    fn record_builder_sets_failure_and_tokens() {
        let cfg = ModelConfig {
            provider_name: None,
            provider: "ollama".into(),
            model: "m".into(),
            base_url: None,
            api_key: None,
        };
        let rec = LlmRequestRecord::new("memory", &cfg, LlmContext::default())
            .with_tokens(1, 2, 3, 4)
            .with_duration_ms(42)
            .failed("boom");
        assert_eq!(rec.status, LLM_STATUS_FAILED);
        assert_eq!(rec.error.as_deref(), Some("boom"));
        assert_eq!(
            (
                rec.tokens_input,
                rec.tokens_output,
                rec.tokens_cache_read,
                rec.tokens_cache_write
            ),
            (1, 2, 3, 4)
        );
        assert_eq!(rec.duration_ms, 42);
        assert_eq!(rec.provider, "ollama");
        assert_eq!(rec.model, "m");
    }
}
