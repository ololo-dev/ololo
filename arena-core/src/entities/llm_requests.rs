//! `llm_requests` entity — unified telemetry for every LLM request the
//! platform issues (judge runs, session-memory extraction, task adaptation,
//! project/task AI helpers).
//!
//! One row per logical request (a judge run aggregates its whole agent loop
//! into a single row). Pure telemetry: intentionally NO foreign keys, so rows
//! outlive the sessions/players/tasks they reference. Written best-effort by
//! `arena_core::llm::telemetry` — an insert failure never fails the caller.
use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "llm_requests")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    /// "judge" | "memory" | "adaptation" | "project_ai" | "task_ai".
    pub operation: String,
    /// Registry id ("ollama" | "openrouter" | "custom" | "execution").
    /// Not unique per configured endpoint — see `provider_name`.
    pub provider: String,
    /// Display name of the `llm_providers` row used, when there was one.
    /// The only thing that tells two `custom` endpoints apart.
    pub provider_name: Option<String>,
    pub model: String,
    /// "ok" | "failed".
    pub status: String,
    /// Error excerpt (truncated to 1KB) for failed requests.
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
    /// Bounded JSON context (prompt/response sizes + excerpts, event
    /// counts). Admin-only surface.
    pub detail_json: Option<String>,
    /// Full per-turn trace: a JSON array of `JudgeLogEvent` (LLM turns with
    /// prompt/completion/message transcript and token usage, tool calls with
    /// args and results). Powers the Langfuse-style telemetry drawer.
    /// `None` when nothing was recorded or the payload exceeded its cap.
    pub events_json: Option<String>,
    pub created_at: ChronoDateTimeUtc,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
