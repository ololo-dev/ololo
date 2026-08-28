//! AI judge execution pipeline.
//!
//! `run_judge` drives an agentic tool-calling loop: the judge LLM receives
//! the admin-authored prompt as system preamble, task context + prior
//! scoring as the initial user prompt, and a set of git-read tools it can
//! call to inspect the player's code. The loop terminates on a final JSON
//! verdict `{"rating", "feedback"}`.

pub mod agent_setup;
pub mod appraisal;
pub mod criteria;
pub mod dossier;
pub mod evidence;
pub mod execution;
pub mod programs;
pub mod report;
pub mod session;
pub mod task_commit;
pub mod task_dossier;
pub mod tools;

use std::path::Path;
use std::time::Duration;

use chrono::Utc;
use sea_orm::{
    ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, Set, sea_query::OnConflict,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::entities::judge_results;
use crate::evaluation::VERDICT_KIND_FULL;
use crate::validation::judge_results::{RatingScale, validate_feedback, validate_rating_output};

pub use task_commit::resolve_task_commit;
pub use tools::{ProbeRegistrar, ToolDef, dispatch_tool, register_probe_def, tool_defs};

/// Maximum tool-call round-trips before the loop aborts (FR-007).
pub const MAX_TOOL_CALLS: usize = 20;

/// `judges.kind`: an LLM answers a question about the work.
pub const JUDGE_KIND_LLM: &str = "llm";

/// `judges.kind`: the server re-runs the task's probes; no model is called.
pub const JUDGE_KIND_EXECUTION: &str = "execution";

/// `judges.kind`: an LLM writes the player's session report. It scores
/// nothing — the run path fixes its rating at zero — so it is the one kind
/// whose `rating_scale` is inert. Session-scoped only: a report is about a
/// session, and there is nothing to report on a single task that the task's
/// own judges have not already said.
pub const JUDGE_KIND_REPORT: &str = "report";

/// `judges.scope`: runs once per (player, task) when that task's commit lands.
pub const JUDGE_SCOPE_TASK: &str = "task";

/// `judges.scope`: runs once per (player, session), scoring every task the
/// player reached in a single pass.
pub const JUDGE_SCOPE_SESSION: &str = "session";

/// `judges.evidence_mode`: the judge investigates through git tool calls.
pub const EVIDENCE_MODE_TOOLS: &str = "tools";

/// `judges.evidence_mode`: the judge is handed a server-built evidence pack
/// and answers in a single completion, with no tools offered.
pub const EVIDENCE_MODE_DOSSIER: &str = "dossier";

/// Total wall-clock budget for the agent loop (FR-006).
pub const LOOP_TIMEOUT: Duration = Duration::from_secs(180);

/// Abstraction over the rig-core agent-loop API (Constitution Commandment 5).
///
/// The trait keeps the loop testable via `FakeJudgeLlm` without depending on
/// rig-core directly. The game-server provides the production impl wrapping
/// a rig `Agent` with tool-calling.
#[async_trait::async_trait]
pub trait JudgeLlm: Send + Sync {
    /// Run one agent turn. The LLM either requests a tool call or produces
    /// a final verdict. `prior_tool_result` is `Some` when the previous turn
    /// was a `ToolCall` — it carries the tool's output to feed back.
    async fn run_agent(
        &self,
        system: &str,
        user: &str,
        tools: Vec<ToolDef>,
        prior_tool_result: Option<&str>,
    ) -> Result<AgentResponse, JudgeError>;

    /// Like [`JudgeLlm::run_agent`], with image attachments (participant
    /// screenshot artifacts). The default drops the images and runs
    /// text-only, so non-vision implementations and fakes stay valid; the
    /// rig implementation overrides this to attach them to the user turn.
    async fn run_agent_with_images(
        &self,
        system: &str,
        user: &str,
        tools: Vec<ToolDef>,
        prior_tool_result: Option<&str>,
        _images: &[JudgeImage],
    ) -> Result<AgentResponse, JudgeError> {
        self.run_agent(system, user, tools, prior_tool_result).await
    }
}

/// One screenshot artifact attached to a judge's user turn.
#[derive(Debug, Clone)]
pub struct JudgeImage {
    /// MIME type, e.g. `image/png`.
    pub media_type: String,
    /// Base64-encoded bytes.
    pub base64: String,
    /// Where it came from, for the prompt ("screenshot from probe …").
    pub label: String,
}

/// One LLM turn: either a tool call or a final verdict.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum AgentResponse {
    ToolCall {
        name: String,
        args: serde_json::Value,
    },
    Final {
        text: String,
    },
}

/// One chronological event of a judge run — an LLM completion turn or a
/// tool invocation. Persisted as JSON in `judge_results.run_log` for the
/// admin judges tab. Field names follow the spirit of the OpenTelemetry
/// GenAI semantic conventions rig emits (`gen_ai.prompt`,
/// `gen_ai.completion`, `gen_ai.usage.*`, tool spans).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct JudgeLogEvent {
    /// Epoch ms when the event started.
    pub at_ms: i64,
    /// "llm" | "tool".
    pub kind: String,
    /// Tool name for `kind == "tool"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Tool args JSON (truncated) for `kind == "tool"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub args: Option<String>,
    /// Size of the tool output / LLM final text, in chars.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_chars: Option<u64>,
    pub duration_ms: i64,
    /// Aggregated token usage for `kind == "llm"` (0 = provider did not
    /// report usage).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tokens_input: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tokens_output: Option<u64>,
    /// Provider-cache token usage (gen_ai.usage.cache_read /
    /// cache_creation input tokens).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tokens_cache_read: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tokens_cache_write: Option<u64>,
    /// Requested model id for `kind == "llm"` (gen_ai.request.model).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// What was sent: system instructions + user prompt for `kind == "llm"`
    /// (gen_ai.prompt), truncated.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input: Option<String>,
    /// What came back: the final completion text for `kind == "llm"`
    /// (gen_ai.completion) or the tool result for `kind == "tool"`,
    /// truncated.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
    /// Full turn-by-turn transcript of the agent loop for `kind == "llm"`
    /// (rig `PromptResponse.messages`): every assistant message including
    /// tool calls, and every tool result. Omitted when serialization
    /// exceeds the size cap.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub messages: Option<serde_json::Value>,
    /// Error text when the event failed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Char-truncate `s` to `cap`, appending a marker when cut.
pub fn truncate_chars(s: &str, cap: usize) -> String {
    if s.chars().count() <= cap {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(cap).collect();
        out.push_str("…[truncated]");
        out
    }
}

/// Thread-safe collector for [`JudgeLogEvent`]s, shared between the judge
/// loop, the rig LLM adapter, and its tools (rig runs tools internally, so
/// the loop cannot observe them without this).
#[derive(Default)]
pub struct JudgeRunRecorder {
    events: std::sync::Mutex<Vec<JudgeLogEvent>>,
    seen: std::sync::Mutex<Option<SeenByRun>>,
}

/// What the run was looking at, as opposed to what it did with it.
///
/// Recorded once per run, before the verdict, so a stored run answers "on
/// what basis" and not only "through which turns" — and so a failed run still
/// says what it had in hand when it failed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SeenByRun {
    /// [`judge_fingerprint`] of the definition that ran.
    pub judge_fingerprint: String,
    /// The [`evidence::Evidence`] snapshot, serialized.
    pub evidence: serde_json::Value,
}

impl JudgeRunRecorder {
    /// Record the snapshot this run reasoned from. Last write wins; a run
    /// assembles its evidence once.
    pub fn set_seen(&self, seen: SeenByRun) {
        if let Ok(mut slot) = self.seen.lock() {
            *slot = Some(seen);
        }
    }

    /// The snapshot, if one was recorded.
    pub fn seen(&self) -> Option<SeenByRun> {
        self.seen.lock().ok().and_then(|s| s.clone())
    }

    pub fn record(&self, event: JudgeLogEvent) {
        if let Ok(mut ev) = self.events.lock() {
            // Hard cap so a runaway loop cannot balloon the row.
            if ev.len() < 500 {
                ev.push(event);
            }
        }
    }

    /// Chronological snapshot of the recorded events.
    pub fn events(&self) -> Vec<JudgeLogEvent> {
        self.events.lock().map(|e| e.clone()).unwrap_or_default()
    }

    /// Total (input, output) tokens across LLM events.
    pub fn token_totals(&self) -> (u64, u64) {
        self.events().iter().fold((0, 0), |(i, o), e| {
            (
                i + e.tokens_input.unwrap_or(0),
                o + e.tokens_output.unwrap_or(0),
            )
        })
    }

    /// Total (cache_read, cache_write) tokens across LLM events.
    pub fn cache_token_totals(&self) -> (u64, u64) {
        self.events().iter().fold((0, 0), |(r, w), e| {
            (
                r + e.tokens_cache_read.unwrap_or(0),
                w + e.tokens_cache_write.unwrap_or(0),
            )
        })
    }

    /// `run_log` JSON for persistence; `None` when nothing was recorded.
    pub fn run_log_json(&self) -> Option<serde_json::Value> {
        let events = self.events();
        if events.is_empty() {
            return None;
        }
        serde_json::to_value(events).ok()
    }
}

/// Epoch ms helper for log events.
pub fn log_now_ms() -> i64 {
    Utc::now().timestamp_millis()
}

/// Judge execution errors.
#[derive(Debug, thiserror::Error)]
pub enum JudgeError {
    #[error("ai_timeout")]
    AiTimeout,
    #[error("ai_parse_error")]
    AiParseError,
    #[error("ai_rating_out_of_range")]
    AiRatingOutOfRange,
    #[error("feedback_too_long")]
    FeedbackTooLong,
    #[error("too_many_tool_calls")]
    TooManyToolCalls,
    #[error("db error: {0}")]
    Db(#[from] sea_orm::DbErr),
    #[error("llm error: {0}")]
    Llm(String),
    #[error("git read error: {0}")]
    GitReadError(String),
    #[error("player repo not found")]
    PlayerRepoNotFound,
    #[error("player repo empty")]
    PlayerRepoEmpty,
    #[error("execution timed out")]
    ExecTimeout,
    #[error("execution failed: {0}")]
    ExecFailed(String),
    #[error("task not found")]
    TaskNotFound,
    #[error("judge program failed: {0}")]
    DecideFailed(String),
    #[error("verdict rejected by review: {0}")]
    VerdictRejected(String),
    /// The judged player's account is out of monthly judge runs — the run
    /// never started. Not retryable: the quota won't free up mid-session.
    #[error("judge run quota exceeded: {0}")]
    QuotaExceeded(String),
}

impl JudgeError {
    /// Whether a failure message persisted in `judge_results.error` records a
    /// quota denial rather than a provider/system fault. Denials are the
    /// player's own account state and may be shown verbatim; anything else
    /// stays admin-only. Matching is by substring because the column stores
    /// rendered `Display` text; the two patterns cover the `QuotaExceeded`
    /// prefix and the older hand-written "judge-run limit" wordings already
    /// in the database. Both sides of the wire (game-server writes, server
    /// classifies) must go through here — never match these strings inline.
    pub fn message_is_quota_denial(message: &str) -> bool {
        message.contains("judge run quota exceeded") || message.contains("judge-run limit")
    }
}

/// Row snapshots passed to `run_judge` by the caller.
#[derive(Debug, Clone)]
pub struct TaskJudgeRow {
    pub id: Uuid,
    pub task_id: Uuid,
    pub judge_id: Uuid,
    pub rating_scale_override: Option<serde_json::Value>,
    /// Panel weight (`task_judges.weight`). `None` = 1.0.
    pub weight: Option<f64>,
}

#[derive(Debug, Clone)]
pub struct JudgeRow {
    pub slug: String,
    pub name: String,
    pub prompt: String,
    pub rating_scale: serde_json::Value,
    /// `"llm"` (default) or `"execution"` — selects the run path.
    pub kind: String,
    /// `"task"` (default) or `"session"` — selects the trigger.
    pub scope: String,
    /// Optional per-judge model override: `llm_providers` row + model id.
    /// `None` falls back to the `llm_op_judge` assignment, then the default.
    pub llm_provider_id: Option<uuid::Uuid>,
    pub llm_model: Option<String>,
    /// Optional per-judge pool override; composes with the pair above in the
    /// order given by `llm_source_order`.
    pub llm_pool_id: Option<uuid::Uuid>,
    /// `"pool_first"` (default) or `"model_first"` — see
    /// [`crate::llm::resolve::LlmOverride`].
    pub llm_source_order: String,
    /// `"tools"` (default) or `"dossier"` — see [`EVIDENCE_MODE_DOSSIER`].
    pub evidence_mode: String,
    /// The evidence sections this judge declared it needs, as stored in
    /// `judges.evidence_needs`. `None` means it never declared and gets the
    /// whole snapshot.
    pub evidence_needs: Option<String>,
    /// `judges.criteria`: JSON array of open-ended criteria keys this judge
    /// scores. `None` = classic single-rating judge.
    pub criteria: Option<String>,
    /// `judges.max_interactive`: interactive probes this judge may register.
    pub max_interactive: Option<i32>,
    /// `judges.ignore_paths`: JSON array of repo path prefixes this judge's
    /// git tools must not open. `None` = the whole snapshot is in scope.
    pub ignore_paths: Option<String>,
}

/// Fingerprint of the judge definition as it ran: prompt, run path, evidence
/// mode and scale — everything that decides what the verdict could be.
///
/// A judge has no version. `judges/*.md` is re-seeded over the same slug on
/// every boot and pushed independently of releases, so "what did
/// `task-anti-cheat` say" is unanswerable a week later without recording
/// *which* `task-anti-cheat`. Sixteen hex chars distinguish two definitions;
/// they are not a security boundary.
pub fn judge_fingerprint(judge: &JudgeRow) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    for part in [
        judge.prompt.as_str(),
        judge.kind.as_str(),
        judge.evidence_mode.as_str(),
        judge.evidence_needs.as_deref().unwrap_or(""),
        &judge.rating_scale.to_string(),
    ] {
        h.update(part.as_bytes());
        // Separator, so ("ab", "c") cannot hash as ("a", "bc").
        h.update([0u8]);
    }
    hex::encode(h.finalize())[..16].to_string()
}

#[derive(Debug, Clone)]
pub struct TaskRow {
    pub id: Uuid,
    pub title: String,
    pub description: String,
    pub tags: String,
    /// The task's point budget — the base a criteria judge's panel share is
    /// computed from.
    pub point_value: i32,
    /// `tasks.evaluation`: the open-ended contract. `None` = classic task.
    pub evaluation: Option<serde_json::Value>,
}

/// Prior `task_results` row (passed in as context).
#[derive(Debug, Clone)]
pub struct PriorTaskResult {
    pub point_delta: i32,
    pub answer: String,
}

/// The scalar rating of a stored `judge_results.rating` value. Plain judges
/// store a bare number; open-ended judges store `{"overall": n, "criteria":
/// [...]}` — readers that treat the column as `as_f64()` see 0.0 for every
/// open-ended verdict.
pub fn rating_scalar(rating: &serde_json::Value) -> f64 {
    rating
        .as_f64()
        .or_else(|| rating.get("overall").and_then(|v| v.as_f64()))
        .unwrap_or(0.0)
}

/// Prior `judge_results` row (re-run context, FR-013).
#[derive(Debug, Clone)]
pub struct PriorJudgeResult {
    pub rating: f64,
    pub feedback: String,
}

/// This judge's own verdict on an EARLIER task of the same session, for the
/// same player. A project's tasks build on one another, so a judge that
/// cannot see what it already said re-litigates settled ground: it praises a
/// structure it faulted a task ago, or charges twice for one flaw. Carrying
/// its own record forward makes each verdict a continuation rather than a
/// first impression.
#[derive(Debug, Clone)]
pub struct PriorSessionVerdict {
    pub task_ordinal: i32,
    pub task_title: String,
    pub rating: f64,
    pub point_delta: i32,
    /// The reasoning the judge wrote at the time.
    pub feedback: String,
}

/// How much of each earlier verdict's reasoning rides along. Judges write
/// paragraphs; a whole session of them would crowd out the evidence the
/// current task needs.
const PRIOR_VERDICT_FEEDBACK_CAP: usize = 700;

/// At most this many earlier verdicts, newest kept: a long project must not
/// grow the prompt without bound.
const PRIOR_VERDICT_MAX: usize = 6;

/// `run_judge` output.
#[derive(Debug, Clone)]
pub struct JudgeRunOutput {
    pub rating: f64,
    pub point_delta: i32,
    pub feedback: String,
    pub raw_output: String,
    pub model: String,
    pub judge_result_id: Uuid,
    /// Wall-clock duration of the judge run (LLM + tool calls).
    pub duration_ms: i64,
}

/// Parsed LLM verdict JSON. Extra fields ignored (FR-008).
#[derive(Debug, Deserialize)]
pub(crate) struct VerdictJson {
    rating: f64,
    feedback: String,
}

/// Run the judge execution pipeline (FR-001).
///
/// Record a judge run's lifecycle state ("running" or "failed") in
/// `judge_results` without touching any prior verdict fields. Upserts on
/// (task_judge_id, player_id) — the same key `run_judge` uses — so a re-run
/// flips the existing row's status and a successful verdict overwrites it
/// with "scored". Rows inserted here carry a null rating and no feedback;
/// readers must filter on `status` before treating a row as a verdict.
#[allow(clippy::too_many_arguments)]
pub async fn record_judge_run_status(
    db: &DatabaseConnection,
    session_id: Uuid,
    player_id: Uuid,
    task_judge_id: Uuid,
    model: &str,
    provider: &str,
    status: &str,
    error: Option<&str>,
    recorder: Option<&JudgeRunRecorder>,
) -> Result<(), sea_orm::DbErr> {
    let now = Utc::now();
    let (tokens_in, tokens_out) = recorder
        .map(|r| {
            let (i, o) = r.token_totals();
            (Some(i as i64), Some(o as i64))
        })
        .unwrap_or((None, None));
    let (cache_read, cache_write) = recorder
        .map(|r| {
            let (cr, cw) = r.cache_token_totals();
            (Some(cr as i64), Some(cw as i64))
        })
        .unwrap_or((None, None));
    // The bulky event log is NOT stored here — the game-server writes it to
    // its on-disk judge-log store; `run_log` stays for legacy rows only.
    let insert_model = judge_results::ActiveModel {
        id: Set(Uuid::new_v4()),
        session_id_fk: Set(session_id),
        player_id_fk: Set(player_id),
        task_judge_id: Set(task_judge_id),
        rating: Set(serde_json::Value::Null),
        point_delta: Set(0),
        feedback: Set(String::new()),
        model: Set(model.to_string()),
        provider: Set(provider.to_string()),
        raw_output: Set(String::new()),
        duration_ms: Set(None),
        run_log: Set(None),
        tokens_input: Set(tokens_in),
        tokens_output: Set(tokens_out),
        tokens_cache_read: Set(cache_read),
        tokens_cache_write: Set(cache_write),
        status: Set(status.to_string()),
        error: Set(error.map(str::to_string)),
        verdict_kind: Set(None),
        created_at: Set(now),
        updated_at: Set(now),
    };
    judge_results::Entity::insert(insert_model)
        .on_conflict(
            OnConflict::columns([
                judge_results::Column::TaskJudgeId,
                judge_results::Column::PlayerIdFk,
            ])
            .update_columns([
                judge_results::Column::Status,
                judge_results::Column::Error,
                judge_results::Column::Provider,
                judge_results::Column::RunLog,
                judge_results::Column::VerdictKind,
                // A run that did not produce a rating must not leave the
                // previous run's score standing: `compute_player_scores` sums
                // `point_delta` across ALL judge_results rows regardless of
                // status, so a stale delta under a `failed` row keeps
                // penalizing (or rewarding) invisibly. Clearing these makes a
                // re-run that cannot score also undo the score it replaces.
                judge_results::Column::Rating,
                judge_results::Column::PointDelta,
                judge_results::Column::Feedback,
                judge_results::Column::TokensInput,
                judge_results::Column::TokensOutput,
                judge_results::Column::TokensCacheRead,
                judge_results::Column::TokensCacheWrite,
                judge_results::Column::UpdatedAt,
            ])
            .to_owned(),
        )
        .exec_without_returning(db)
        .await?;
    Ok(())
}

/// Caller (game-server) resolves `repo_dir`, `task_commit_sha`, and the row
/// snapshots. This function is pure domain logic — no HTTP, no WS.
#[allow(clippy::too_many_arguments)]
pub async fn run_judge(
    db: &DatabaseConnection,
    judge_llm: &dyn JudgeLlm,
    repo_dir: &Path,
    session_id: Uuid,
    player_id: Uuid,
    _task_id: Uuid,
    task_commit_sha: Option<&str>,
    task_stats_json: Option<&str>,
    task_judge_row: &TaskJudgeRow,
    judge_row: &JudgeRow,
    task_row: &TaskRow,
    prior_results: &[PriorTaskResult],
    prior_judge_result: Option<&PriorJudgeResult>,
    // This judge's own verdicts on earlier tasks of the same session, so a
    // panel member judges the arc rather than each task from scratch.
    prior_session_verdicts: &[PriorSessionVerdict],
    model: &str,
    provider: &str,
    recorder: Option<&JudgeRunRecorder>,
    // Already bounded by `gate_task_judge` at the call site, so the policy
    // "a judge cannot take more than the task paid" lives in one place
    // rather than being re-derived per run path.
    scale: &RatingScale,
    // What the judge's own program asked the model to look at, when it ran
    // and returned `ask({focus})`.
    focus: Option<&str>,
    // The `review` program and the snapshot it reads, when the judge carries
    // one. Runs on the verdict before it is persisted.
    review: Option<(&str, &evidence::Evidence)>,
    // Criteria contract, when this judge scores a per-criterion sheet on an
    // open-ended task. `scale` is then the judge's panel-share point scale;
    // the model itself scores each criterion 0.0–10.0.
    criteria_ctx: Option<&criteria::CriteriaContext>,
    // Live probe registration, when the judge may register probes mid-run.
    registrar: Option<&std::sync::Arc<dyn ProbeRegistrar>>,
    // Screenshot artifacts already delivered for this task, attached to the
    // model's user turn (vision judges); empty for text-only runs.
    images: &[JudgeImage],
) -> Result<JudgeRunOutput, JudgeError> {
    let scale = *scale;

    // What this judge is not allowed to look at. Declared per judge: the UX
    // review lives off the artifacts under `.ololo/`, while a judge reading
    // only the player's code should never spend a token on that tree.
    let scope = tools::ToolScope::from_json(judge_row.ignore_paths.as_deref());

    let dossier_mode = judge_row.evidence_mode == EVIDENCE_MODE_DOSSIER;
    // The judge's program, if it carries one, is instructions for the harness
    // and not for the model — it already ran and said `ask`. Leaving it in the
    // system prompt would invite the model to re-enact a decision that is not
    // its to make.
    let (prompt, _) = programs::split_programs(&judge_row.prompt);
    let system = match criteria_ctx {
        None => build_system_prompt_for(&prompt, dossier_mode),
        Some(ctx) => build_criteria_system_prompt(&prompt, dossier_mode, &ctx.keys),
    };
    let mut user = build_user_prompt(
        task_row,
        task_commit_sha,
        task_stats_json.is_some() && !dossier_mode,
        prior_results,
        prior_judge_result,
        prior_session_verdicts,
        &scale,
    );
    // In dossier mode the evidence is gathered up front and no tools are
    // offered, so the model answers in a single completion instead of paying a
    // round-trip per tool call (and re-sending the transcript each turn).
    let mut extra_tools: Vec<ToolDef> = Vec::new();
    if registrar.is_some() {
        extra_tools.push(register_probe_def());
    }
    let tools = if dossier_mode {
        user.push_str(
            &task_dossier::build_task_dossier(
                repo_dir,
                &task_row.id.to_string(),
                task_commit_sha,
                task_stats_json,
                &scope,
            )
            .await,
        );
        // Dossier judges get no git tools, but registration — when wired —
        // is still theirs: it is how a dossier judge asks for what the pack
        // could not carry.
        extra_tools.clone()
    } else {
        let mut t = tool_defs(&scope);
        t.extend(extra_tools.clone());
        t
    };

    if !images.is_empty() {
        user.push_str("\n\n## Attached screenshots\n\n");
        for image in images {
            user.push_str(&format!("- {}\n", image.label));
        }
        user.push_str(
            "The images above are participant-delivered artifacts, attached to \
             this message. Judge what you can actually see in them.\n",
        );
    }

    // The program's focus goes last, after the evidence pack, so it reads as
    // the final instruction rather than as one more paragraph of briefing.
    if let Some(focus) = focus.map(str::trim).filter(|f| !f.is_empty()) {
        user.push_str(&format!(
            "\n\n## What to look at first\n\nThis judge's own program inspected the run before you were asked, \
             and it wants your attention here: {}\n\nThis narrows where to look. It does not tell you what to \
             conclude, and it does not lower the bar for evidence.\n",
            truncate_chars(focus, 1_000)
        ));
    }

    let run_started = std::time::Instant::now();
    let raw_output = tokio::time::timeout(LOOP_TIMEOUT, async {
        let mut prior_tool_result: Option<String> = None;
        let mut verdict_retries = 0;
        let mut round_tools = tools.clone();
        for _round in 0..MAX_TOOL_CALLS {
            let resp = judge_llm
                .run_agent_with_images(
                    &system,
                    &user,
                    round_tools.clone(),
                    prior_tool_result.as_deref(),
                    images,
                )
                .await?;
            match resp {
                AgentResponse::ToolCall { name, args } => {
                    let result = match (&name[..], registrar) {
                        ("register_probe", Some(reg)) => reg.register(&args).await,
                        _ => {
                            dispatch_tool(
                                repo_dir,
                                &name,
                                &args,
                                task_commit_sha,
                                task_stats_json,
                                &scope,
                            )
                            .await
                        }
                    };
                    prior_tool_result = Some(result);
                }
                AgentResponse::Final { text } => {
                    // Models sometimes end on prose instead of the JSON verdict.
                    // Nudge up to twice before giving up with the raw text. The
                    // nudge round runs WITHOUT tools: each run_agent call is
                    // stateless, so with tools available the model restarts its
                    // investigation instead of answering. Its prior analysis is
                    // fed back in the prompt, so it must only convert it to JSON.
                    let parses = match criteria_ctx {
                        None => parse_verdict(&text).is_ok(),
                        Some(ctx) => parse_and_score_criteria(&text, ctx, &scale).is_ok(),
                    };
                    if parses || verdict_retries >= 2 {
                        return Ok(text);
                    }
                    verdict_retries += 1;
                    round_tools = Vec::new();
                    prior_tool_result = Some(format!(
                        "Your analysis so far:\n{text}\n\nBased on this analysis, respond \
                         now with ONLY the JSON verdict object, no other text: \
                         {{\"rating\": <number>, \"feedback\": \"<text>\"}}"
                    ));
                }
            }
        }
        Err(JudgeError::TooManyToolCalls)
    })
    .await
    .map_err(|_| JudgeError::AiTimeout)??;

    // FR-008: parse JSON, strip markdown fences, retain raw regardless.
    // Criteria judges answer with a score sheet; the sheet is kept whole for
    // the rating column while the derived point mapping drives scoring.
    let (mut verdict, criteria_sheet) = match criteria_ctx {
        None => {
            let v = parse_verdict(&raw_output)?;
            (v, None)
        }
        Some(ctx) => {
            let scored = parse_and_score_criteria(&raw_output, ctx, &scale)
                .map_err(|_| JudgeError::AiParseError)?;
            (
                VerdictJson {
                    rating: scored.mapped_rating,
                    feedback: scored.feedback.clone(),
                },
                Some(scored),
            )
        }
    };

    // FR-009/010: validate rating, compute point_delta. Out-of-range pulls
    // to the nearest bound instead of failing the run — a retry re-bills the
    // whole judge for an answer whose only defect was magnitude.
    verdict.rating =
        crate::validation::judge_results::clamp_rating_to_scale(verdict.rating, &scale);
    let point_delta = validate_rating_output(verdict.rating, &scale)
        .map_err(|_| JudgeError::AiRatingOutOfRange)?;

    // FR-011: validate feedback length.
    validate_feedback(&verdict.feedback).map_err(|_| JudgeError::FeedbackTooLong)?;

    // The cut after the model. A verdict is authored content too, and the
    // things that have gone wrong with one — a penalty citing a commit that
    // does not exist, a rating out of proportion to what the task paid —
    // cannot be prevented by asking. The program sees the same snapshot the
    // model was judged against, plus the verdict itself.
    let (rating, feedback, point_delta) = match review {
        None => (verdict.rating, verdict.feedback.clone(), point_delta),
        Some((script, ev)) => {
            let started = std::time::Instant::now();
            let outcome = programs::run_review(script, ev, verdict.rating, &verdict.feedback);
            if let Some(rec) = recorder {
                rec.record(JudgeLogEvent {
                    at_ms: log_now_ms(),
                    kind: "review".to_string(),
                    name: Some("program".to_string()),
                    duration_ms: started.elapsed().as_millis() as i64,
                    output: Some(match &outcome {
                        Ok(o) => format!("{o:?}"),
                        Err(e) => format!("failed: {e}"),
                    }),
                    ..Default::default()
                });
            }
            match outcome? {
                programs::Review::Accept => (verdict.rating, verdict.feedback.clone(), point_delta),
                programs::Review::Reject { reason } => {
                    // Retryable on purpose: the model is re-prompted from
                    // scratch within the run's attempt budget. Persisting a
                    // verdict the judge's own program called unusable would
                    // score somebody on it.
                    return Err(JudgeError::VerdictRejected(reason));
                }
                programs::Review::Revise { rating, feedback } => {
                    // Held to the same bounds the model's answer was: a
                    // revision is a verdict, and the scale is the scale.
                    let rating =
                        crate::validation::judge_results::clamp_rating_to_scale(rating, &scale);
                    let point_delta = validate_rating_output(rating, &scale).map_err(|e| {
                        JudgeError::DecideFailed(format!("revise({rating}) rejected: {e}"))
                    })?;
                    validate_feedback(&feedback)
                        .map_err(|e| JudgeError::DecideFailed(format!("feedback rejected: {e}")))?;
                    (rating, feedback, point_delta)
                }
            }
        }
    };

    let duration_ms = run_started.elapsed().as_millis() as i64;

    // FR-012: native upsert via ON CONFLICT (task_judge_id, player_id)
    // DO UPDATE per Contract — closes the SQLite/Postgres upsert race without
    // a find-then-insert dance. Re-select by the conflict key afterwards to
    // recover the persisted row's id (the value we INSERT is ignored on the
    // update branch — the original row's id survives).
    let now = Utc::now();
    let insert_id = Uuid::new_v4();
    let (tokens_in, tokens_out) = recorder
        .map(|r| {
            let (i, o) = r.token_totals();
            (Some(i as i64), Some(o as i64))
        })
        .unwrap_or((None, None));
    let (cache_read, cache_write) = recorder
        .map(|r| {
            let (cr, cw) = r.cache_token_totals();
            (Some(cr as i64), Some(cw as i64))
        })
        .unwrap_or((None, None));
    // Event log goes to the game-server's on-disk store, not the DB.
    let rating_json = match &criteria_sheet {
        Some(scored) => serde_json::json!({
            "overall": rating,
            "criteria": scored.verdict.criteria,
        }),
        None => serde_json::json!(rating),
    };
    let insert_model: judge_results::ActiveModel = judge_results::ActiveModel {
        id: Set(insert_id),
        session_id_fk: Set(session_id),
        player_id_fk: Set(player_id),
        task_judge_id: Set(task_judge_row.id),
        rating: Set(rating_json),
        point_delta: Set(point_delta),
        feedback: Set(feedback.clone()),
        model: Set(model.to_string()),
        provider: Set(provider.to_string()),
        raw_output: Set(raw_output.clone()),
        duration_ms: Set(Some(duration_ms)),
        run_log: Set(None),
        tokens_input: Set(tokens_in),
        tokens_output: Set(tokens_out),
        tokens_cache_read: Set(cache_read),
        tokens_cache_write: Set(cache_write),
        status: Set("scored".to_string()),
        error: Set(None),
        verdict_kind: Set(Some(VERDICT_KIND_FULL.to_string())),
        created_at: Set(now),
        updated_at: Set(now),
    };

    judge_results::Entity::insert(insert_model)
        .on_conflict(
            OnConflict::columns([
                judge_results::Column::TaskJudgeId,
                judge_results::Column::PlayerIdFk,
            ])
            .update_columns([
                judge_results::Column::Rating,
                judge_results::Column::Feedback,
                judge_results::Column::RawOutput,
                judge_results::Column::PointDelta,
                judge_results::Column::Model,
                judge_results::Column::Provider,
                judge_results::Column::DurationMs,
                judge_results::Column::RunLog,
                judge_results::Column::TokensInput,
                judge_results::Column::TokensOutput,
                judge_results::Column::TokensCacheRead,
                judge_results::Column::TokensCacheWrite,
                judge_results::Column::Status,
                judge_results::Column::Error,
                judge_results::Column::VerdictKind,
                judge_results::Column::UpdatedAt,
            ])
            .to_owned(),
        )
        .exec_without_returning(db)
        .await?;

    // Re-select to recover the persisted row's id (race-free: the row exists
    // after the upsert under both SQLite and Postgres).
    let row = judge_results::Entity::find()
        .filter(judge_results::Column::TaskJudgeId.eq(task_judge_row.id))
        .filter(judge_results::Column::PlayerIdFk.eq(player_id))
        .one(db)
        .await?
        .ok_or_else(|| {
            JudgeError::Db(sea_orm::DbErr::Custom(
                "upsert returned no row (judge_results)".to_string(),
            ))
        })?;
    let judge_result_id = row.id;

    Ok(JudgeRunOutput {
        rating,
        point_delta,
        feedback,
        raw_output,
        model: model.to_string(),
        judge_result_id,
        duration_ms,
    })
}

/// Upsert a scored verdict on the `(task_judge_id, player_id)` key.
///
/// Extracted so the task-scoped loop and the session-scoped runner persist
/// identically — the row shape is what `compute_player_scores` and the settle
/// poll both read, so a second divergent writer would be a correctness hazard.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn persist_scored_verdict(
    db: &DatabaseConnection,
    session_id: Uuid,
    player_id: Uuid,
    task_judge_id: Uuid,
    rating: f64,
    point_delta: i32,
    feedback: &str,
    raw_output: &str,
    model: &str,
    provider: &str,
    duration_ms: i64,
) -> Result<(), JudgeError> {
    persist_scored_verdict_json(
        db,
        session_id,
        player_id,
        task_judge_id,
        serde_json::json!(rating),
        point_delta,
        feedback,
        raw_output,
        model,
        provider,
        duration_ms,
    )
    .await
}

/// [`persist_scored_verdict`] for a judge that answers with a criteria sheet.
///
/// `judge_results.rating` is `Json` precisely so a sheet can live there whole:
/// the scorecard the player reads is the sheet, and a scalar collapsed from it
/// cannot be expanded back. Callers that score a single number pass
/// `json!(rating)` through the wrapper above.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn persist_scored_verdict_json(
    db: &DatabaseConnection,
    session_id: Uuid,
    player_id: Uuid,
    task_judge_id: Uuid,
    rating: serde_json::Value,
    point_delta: i32,
    feedback: &str,
    raw_output: &str,
    model: &str,
    provider: &str,
    duration_ms: i64,
) -> Result<(), JudgeError> {
    let now = Utc::now();
    let insert_model = judge_results::ActiveModel {
        id: Set(Uuid::new_v4()),
        session_id_fk: Set(session_id),
        player_id_fk: Set(player_id),
        task_judge_id: Set(task_judge_id),
        rating: Set(rating),
        point_delta: Set(point_delta),
        feedback: Set(feedback.to_string()),
        model: Set(model.to_string()),
        provider: Set(provider.to_string()),
        raw_output: Set(raw_output.to_string()),
        duration_ms: Set(Some(duration_ms)),
        run_log: Set(None),
        tokens_input: Set(None),
        tokens_output: Set(None),
        tokens_cache_read: Set(None),
        tokens_cache_write: Set(None),
        status: Set("scored".to_string()),
        error: Set(None),
        verdict_kind: Set(Some(VERDICT_KIND_FULL.to_string())),
        created_at: Set(now),
        updated_at: Set(now),
    };
    judge_results::Entity::insert(insert_model)
        .on_conflict(
            OnConflict::columns([
                judge_results::Column::TaskJudgeId,
                judge_results::Column::PlayerIdFk,
            ])
            .update_columns([
                judge_results::Column::Rating,
                judge_results::Column::Feedback,
                judge_results::Column::RawOutput,
                judge_results::Column::PointDelta,
                judge_results::Column::Model,
                judge_results::Column::Provider,
                judge_results::Column::DurationMs,
                judge_results::Column::Status,
                judge_results::Column::Error,
                judge_results::Column::VerdictKind,
                judge_results::Column::UpdatedAt,
            ])
            .to_owned(),
        )
        .exec_without_returning(db)
        .await?;
    Ok(())
}

/// Build the system prompt: judge's admin-authored prompt + JSON wrapper
/// instructing the final output shape (FR-002, AD-6).
/// The judge markdown plus the harness contract. `dossier` swaps the
/// tool-calling instructions for the single-completion ones — telling a model
/// to "call tools as needed" when none are offered invites it to stall or to
/// emit a tool call as prose.
fn build_system_prompt_for(judge_prompt: &str, dossier: bool) -> String {
    let verdict = "When you have enough information, respond with a final JSON verdict \
         and nothing else:\n\
         ```json\n\
         {\"rating\": <number>, \"feedback\": \"<text>\"}\n\
         ```\n\
         `rating` must be a number on the provided scale. `feedback` is a \
         short textual review (max 10000 chars).";
    if dossier {
        format!(
            "{judge_prompt}\n\n\
             You are an AI judge evaluating a player's code submission.\n\
             All available evidence is already in your briefing, under \
             `=== EVIDENCE PACK ===`: the commit log, this task's commit, the \
             diff of its work window, the prior content of the files that diff \
             touches, and the player's AI-agent statistics. There are no tools \
             to call and no way to fetch more — a section marked unavailable or \
             truncated stays that way, so judge only what you can see.\n\
             {verdict} Answer in one response."
        )
    } else {
        format!(
            "{judge_prompt}\n\n\
             You are an AI judge evaluating a player's code submission.\n\
             You have git-read tools to inspect the player's repository, and a \
             get_task_stats tool with the player's AI-agent implementation \
             statistics for this task (token usage, messages, tool calls, skills).\n\
             Call tools as needed to understand the submission.\n\
             {verdict} Do not call tools after the final verdict."
        )
    }
}

/// Build the user prompt: task context + prior results + prior judge result
/// + task commit (FR-002, FR-013, FR-014/015).
fn build_user_prompt(
    task: &TaskRow,
    task_commit_sha: Option<&str>,
    has_task_stats: bool,
    prior_results: &[PriorTaskResult],
    prior_judge_result: Option<&PriorJudgeResult>,
    prior_session_verdicts: &[PriorSessionVerdict],
    scale: &RatingScale,
) -> String {
    let mut s = String::new();
    s.push_str(&format!("Task ID: {}\n", task.id));
    s.push_str(&format!("Task: {}\n", task.title));
    s.push_str(&format!("Description:\n{}\n", task.description));
    // The contract's constraints are project context the judge instructions
    // deliberately do not carry (judges are reusable across projects) — the
    // task is what binds them, so they ride with the task.
    if let Some(ev) = &task.evaluation
        && let Ok(contract) = crate::evaluation::EvaluationContract::from_json(ev)
        && let Some(c) = contract
            .constraints
            .as_deref()
            .map(str::trim)
            .filter(|c| !c.is_empty())
    {
        s.push_str(&format!("Constraints (from the task's contract):\n{c}\n"));
    }
    if !task.tags.is_empty() {
        s.push_str(&format!("Tags: {}\n", task.tags));
    }
    s.push_str(&format!(
        "Rating scale: min={}, max={}, step={}\n",
        scale.min, scale.max, scale.step
    ));

    s.push_str(&format!(
        "Task commit: {sha}\n",
        sha = task_commit_sha.unwrap_or("HEAD")
    ));
    if task_commit_sha.is_none() {
        s.push_str("No task-specific commit found; using HEAD.\n");
    }

    if has_task_stats {
        s.push_str(
            "Agent implementation statistics for this task are available via the \
             get_task_stats tool.\n",
        );
    } else {
        s.push_str("No agent implementation statistics were reported for this task.\n");
    }

    if !prior_results.is_empty() {
        s.push_str("Prior task results:\n");
        for r in prior_results {
            s.push_str(&format!(
                "  - points: {}, answer: {}\n",
                r.point_delta, r.answer
            ));
        }
    }

    if let Some(p) = prior_judge_result {
        s.push_str(&format!(
            "Prior rating: {}\nPrior feedback: {}\n",
            p.rating, p.feedback
        ));
    }

    s.push_str(&render_prior_session_verdicts(prior_session_verdicts));

    s
}

/// The judge's own earlier verdicts in this session, oldest first — its
/// record, not the player's. Empty input renders nothing at all rather than
/// an empty heading the model would try to interpret.
fn render_prior_session_verdicts(verdicts: &[PriorSessionVerdict]) -> String {
    if verdicts.is_empty() {
        return String::new();
    }
    // Keep the newest N (a long project must not grow the prompt without
    // bound) but still read oldest → newest, the order the work happened in.
    let skipped = verdicts.len().saturating_sub(PRIOR_VERDICT_MAX);
    let mut s = String::from(
        "\nYour own earlier verdicts in this session (same player, same judge). \
         These are your words: build on them, do not repeat a judgement you \
         already made, and do not charge twice for the same flaw. Say so \
         plainly when the player has fixed something you faulted before.\n",
    );
    if skipped > 0 {
        s.push_str(&format!(
            "[{skipped} earlier verdict(s) omitted; the {PRIOR_VERDICT_MAX} most recent follow.]\n"
        ));
    }
    for v in verdicts.iter().skip(skipped) {
        let feedback = v.feedback.trim();
        let feedback = if feedback.len() <= PRIOR_VERDICT_FEEDBACK_CAP {
            feedback.to_string()
        } else {
            let mut cut = PRIOR_VERDICT_FEEDBACK_CAP;
            while cut > 0 && !feedback.is_char_boundary(cut) {
                cut -= 1;
            }
            format!("{}… [truncated]", &feedback[..cut])
        };
        s.push_str(&format!(
            "\n-- Task #{ord} \"{title}\" — you rated {rating} ({delta:+} pts)\n{body}\n",
            ord = v.task_ordinal,
            title = v.task_title,
            rating = v.rating,
            delta = v.point_delta,
            body = if feedback.is_empty() {
                "(you left no reasoning)"
            } else {
                &feedback
            },
        ));
    }
    s
}

/// What a task-scoped judge is allowed to do, given what the task actually
/// paid this player.
#[derive(Debug, Clone, PartialEq)]
pub enum TaskJudgeGate {
    /// Nothing to reverse: write a terminal zero, never reach the judge.
    Skip { reason: String },
    /// Run, bounded by this scale.
    Run(RatingScale),
}

/// Bound a penalty-only judge by the task's own payout.
///
/// A judge whose scale tops out at zero can only take points away, and what
/// it takes away is a reward the task granted. A task that granted nothing —
/// the clock cut it short, or no probe ever passed — has no reward to
/// reverse, so there is nothing to judge and neither the model nor the
/// sandbox is reached. Where something *was* paid, the floor tightens to
/// exactly that: no judge may leave a player worse off than never having
/// attempted the task.
///
/// This is the task-scoped twin of the session judge's clawback scale, which
/// has bounded that path since it was written. Its absence here is what let
/// session YC4T6K collect five penalties totalling -125 on honest work, and
/// OD5FJA lose -33 on a rung worth 25.
///
/// Rating judges (`max > 0`) are untouched: they add points rather than
/// reverse them, and are not bounded by what the task paid.
pub fn gate_task_judge(
    base: &serde_json::Value,
    override_: &Option<serde_json::Value>,
    earned: i32,
) -> TaskJudgeGate {
    let mut scale = effective_rating_scale(base, override_);
    if scale.max > 0.0 || scale.step <= 0.0 {
        return TaskJudgeGate::Run(scale);
    }

    // The floor has to stay a whole number of steps from zero, or the judge
    // could no longer answer 0 — the "nothing wrong here" verdict, which
    // `validate_rating_output` would then reject as off-scale. Rounding the
    // budget down to a step errs towards taking less than was paid.
    let steps = (earned.max(0) as f64 / scale.step).floor();
    let floor = -(steps * scale.step);
    if floor >= 0.0 {
        return TaskJudgeGate::Skip {
            reason: "This task paid no points, so there is nothing to withdraw.".to_string(),
        };
    }
    if floor > scale.min {
        scale.min = floor;
    }
    TaskJudgeGate::Run(scale)
}

/// Write the terminal zero for a judge the gate never ran, and return its id.
///
/// A row is written rather than nothing at all because the settle poll waits
/// on a terminal `judge_results` row per attached judge; a silent skip would
/// hold the session open until its deadline.
pub async fn persist_gate_skip(
    db: &DatabaseConnection,
    session_id: Uuid,
    player_id: Uuid,
    task_judge_id: Uuid,
    reason: &str,
) -> Result<Uuid, JudgeError> {
    persist_verdict_and_id(
        db,
        session_id,
        player_id,
        task_judge_id,
        0.0,
        0,
        reason,
        "gate:not-applicable",
        "gate",
    )
    .await
}

/// Upsert a verdict reached without a model — a gate skip, or a judge program
/// that scored on its own — and return the row's id.
///
/// The id has to be re-selected: on the update branch of the upsert the value
/// we insert is discarded and the original row's id survives.
#[allow(clippy::too_many_arguments)]
pub async fn persist_verdict_and_id(
    db: &DatabaseConnection,
    session_id: Uuid,
    player_id: Uuid,
    task_judge_id: Uuid,
    rating: f64,
    point_delta: i32,
    feedback: &str,
    model: &str,
    provider: &str,
) -> Result<Uuid, JudgeError> {
    persist_scored_verdict(
        db,
        session_id,
        player_id,
        task_judge_id,
        rating,
        point_delta,
        feedback,
        "",
        model,
        provider,
        0,
    )
    .await?;
    let row = judge_results::Entity::find()
        .filter(judge_results::Column::TaskJudgeId.eq(task_judge_id))
        .filter(judge_results::Column::PlayerIdFk.eq(player_id))
        .one(db)
        .await?
        .ok_or_else(|| {
            JudgeError::Db(sea_orm::DbErr::Custom(
                "upsert returned no row (judge_results)".to_string(),
            ))
        })?;
    Ok(row.id)
}

/// Resolve the effective `RatingScale`: override if present, else the judge's
/// base scale (FR-009).
pub fn effective_rating_scale(
    base: &serde_json::Value,
    override_: &Option<serde_json::Value>,
) -> RatingScale {
    let v = override_.as_ref().unwrap_or(base);
    parse_rating_scale(v).unwrap_or(RatingScale {
        min: 0.0,
        max: 10.0,
        step: 1.0,
    })
}

fn parse_rating_scale(v: &serde_json::Value) -> Option<RatingScale> {
    let obj = v.as_object()?;
    let min = obj.get("min")?.as_f64()?;
    let max = obj.get("max")?.as_f64()?;
    let step = obj.get("step")?.as_f64()?;
    Some(RatingScale { min, max, step })
}

/// A criteria sheet parsed, validated, and mapped onto the point scale.
pub(crate) struct ScoredCriteria {
    pub verdict: criteria::CriteriaVerdict,
    /// Overall mapped onto the judge's effective scale; 0.0 when every
    /// criterion was `null` (nothing assessable — the neutral verdict).
    pub mapped_rating: f64,
    pub feedback: String,
}

/// Parse either verdict shape from a criteria judge and derive its scalar.
/// A legacy `{rating, feedback}` answer is accepted as-is (the model ignored
/// the sheet contract but stayed on scale); a sheet is validated against the
/// declared keys and weight-averaged.
pub(crate) fn parse_and_score_criteria(
    raw: &str,
    ctx: &criteria::CriteriaContext,
    scale: &RatingScale,
) -> Result<ScoredCriteria, String> {
    match criteria::parse_any_verdict(raw)? {
        criteria::AnyVerdict::Legacy { rating, feedback } => Ok(ScoredCriteria {
            verdict: criteria::CriteriaVerdict {
                criteria: Vec::new(),
                feedback: feedback.clone(),
            },
            mapped_rating: rating,
            feedback,
        }),
        criteria::AnyVerdict::Criteria(verdict) => {
            criteria::validate_criteria_verdict(&verdict, &ctx.keys)?;
            let mapped = match criteria::criteria_overall(&verdict, &ctx.weights) {
                Some(overall) => criteria::map_overall_to_scale(overall, scale),
                // Every criterion "cannot assess": the neutral verdict.
                None => 0.0,
            };
            let mut feedback = verdict.feedback.clone();
            if feedback.trim().is_empty() {
                feedback = verdict
                    .criteria
                    .iter()
                    .map(|c| format!("{}: {}", c.key, c.rationale))
                    .collect::<Vec<_>>()
                    .join("; ");
            }
            Ok(ScoredCriteria {
                verdict,
                mapped_rating: mapped,
                feedback,
            })
        }
    }
}

/// System prompt for a criteria judge: the sheet contract replaces the
/// scalar verdict contract.
fn build_criteria_system_prompt(judge_prompt: &str, dossier: bool, keys: &[String]) -> String {
    let keys_list = keys
        .iter()
        .map(|k| format!("\"{k}\""))
        .collect::<Vec<_>>()
        .join(", ");
    let verdict = format!(
        "When you have enough information, respond with a final JSON verdict and nothing else:\n\
         ```json\n\
         {{\"criteria\": [{{\"key\": \"<key>\", \"score\": <0.0-10.0 or null>, \
         \"rationale\": \"<why>\", \"evidence\": [\"probe:<id>\", \"commit:<sha>\", \
         \"file:<path>:<line>\"]}}, ...], \"feedback\": \"<short review>\"}}\n\
         ```\n\
         Score EXACTLY these criteria keys: {keys_list}. Use `null` only when you \
         genuinely cannot assess a criterion, and say why in its rationale. Cite \
         evidence that exists in what you were shown."
    );
    if dossier {
        format!(
            "{judge_prompt}\n\n\
             You are an AI judge evaluating a player's open-ended task submission.\n\
             All available evidence is already in your briefing, under \
             `=== EVIDENCE PACK ===`. There are no tools to call and no way to \
             fetch more — a section marked unavailable or truncated stays that \
             way, so judge only what you can see.\n\
             {verdict} Answer in one response."
        )
    } else {
        format!(
            "{judge_prompt}\n\n\
             You are an AI judge evaluating a player's open-ended task submission.\n\
             You have git-read tools to inspect the player's repository, and a \
             get_task_stats tool with the player's AI-agent implementation \
             statistics for this task.\n\
             Call tools as needed to understand the submission.\n\
             {verdict} Do not call tools after the final verdict."
        )
    }
}

/// Strip markdown code fences and parse the verdict JSON (FR-008).
pub(crate) fn parse_verdict(raw: &str) -> Result<VerdictJson, JudgeError> {
    let stripped = strip_code_fences(raw.trim());
    if let Ok(v) = serde_json::from_str::<VerdictJson>(stripped) {
        return Ok(v);
    }
    // Models sometimes prefix the verdict with leaked reasoning (e.g.
    // "…</think>{\"rating\": …}"). Fall back to the outermost {…} slice.
    if let (Some(start), Some(end)) = (stripped.find('{'), stripped.rfind('}'))
        && start < end
        && let Ok(v) = serde_json::from_str::<VerdictJson>(&stripped[start..=end])
    {
        return Ok(v);
    }
    tracing::warn!(raw_output = %raw, "judge verdict parse failed");
    Err(JudgeError::AiParseError)
}

/// Remove ```json ... ``` or ``` ... ``` fences.
fn strip_code_fences(s: &str) -> &str {
    let s = s.trim();
    if let Some(after_open) = s.strip_prefix("```") {
        // skip optional language tag on first line
        let after_open = after_open
            .split_once('\n')
            .map(|(_, rest)| rest)
            .unwrap_or(after_open);
        if let Some(end) = after_open.rfind("```") {
            return after_open[..end].trim();
        }
        return after_open.trim();
    }
    s
}

#[cfg(test)]
mod prompt_tests {
    use super::*;

    fn task_row(evaluation: Option<serde_json::Value>) -> TaskRow {
        TaskRow {
            id: Uuid::nil(),
            title: "Build the thing".into(),
            description: "A brief.".into(),
            tags: String::new(),
            point_value: 100,
            evaluation,
        }
    }

    fn scale() -> RatingScale {
        RatingScale {
            min: 0.0,
            max: 10.0,
            step: 0.1,
        }
    }

    fn verdict(ordinal: i32, title: &str, feedback: &str) -> PriorSessionVerdict {
        PriorSessionVerdict {
            task_ordinal: ordinal,
            task_title: title.to_string(),
            rating: 8.0,
            point_delta: 17,
            feedback: feedback.to_string(),
        }
    }

    #[test]
    fn a_judge_with_no_history_gets_no_history_section() {
        let s = build_user_prompt(&task_row(None), None, false, &[], None, &[], &scale());
        assert!(
            !s.contains("earlier verdicts"),
            "an empty heading is something the model would try to interpret: {s}"
        );
    }

    #[test]
    fn earlier_verdicts_carry_task_rating_points_and_reasoning() {
        let verdicts = [
            verdict(
                0,
                "Build the widget",
                "Clean layering, weak error handling.",
            ),
            verdict(1, "Add the forecast", "The error handling is fixed."),
        ];
        let s = build_user_prompt(&task_row(None), None, false, &[], None, &verdicts, &scale());

        assert!(
            s.contains("Your own earlier verdicts in this session"),
            "{s}"
        );
        assert!(s.contains(r#"Task #0 "Build the widget""#), "{s}");
        assert!(s.contains(r#"Task #1 "Add the forecast""#), "{s}");
        assert!(s.contains("you rated 8 (+17 pts)"), "{s}");
        assert!(s.contains("Clean layering, weak error handling."), "{s}");
        assert!(s.contains("The error handling is fixed."), "{s}");
        // The instruction is the point: continuity, not a second helping of
        // the same penalty.
        assert!(s.contains("do not charge twice for the same flaw"), "{s}");

        // Oldest first — the order the work happened in.
        let first = s.find("Task #0").expect("task 0 present");
        let second = s.find("Task #1").expect("task 1 present");
        assert!(first < second, "verdicts must read oldest → newest");
    }

    #[test]
    fn a_negative_verdict_keeps_its_sign() {
        let mut v = verdict(0, "Ship it", "Copied wholesale.");
        v.point_delta = -25;
        v.rating = 1.5;
        let s = build_user_prompt(&task_row(None), None, false, &[], None, &[v], &scale());
        assert!(s.contains("you rated 1.5 (-25 pts)"), "{s}");
    }

    #[test]
    fn long_reasoning_is_capped_on_a_char_boundary() {
        // Judges write paragraphs; a whole session of them would crowd out
        // the evidence for the task actually being judged.
        let long = "é".repeat(PRIOR_VERDICT_FEEDBACK_CAP);
        let s = build_user_prompt(
            &task_row(None),
            None,
            false,
            &[],
            None,
            &[verdict(0, "Long one", &long)],
            &scale(),
        );
        assert!(s.contains("[truncated]"), "the cut is announced: {s}");
        assert!(s.len() < PRIOR_VERDICT_FEEDBACK_CAP + 1_500);
    }

    #[test]
    fn a_long_session_keeps_the_most_recent_verdicts_and_says_so() {
        let verdicts: Vec<PriorSessionVerdict> = (0..PRIOR_VERDICT_MAX as i32 + 2)
            .map(|i| verdict(i, &format!("Task {i}"), "reasoning"))
            .collect();
        let s = build_user_prompt(&task_row(None), None, false, &[], None, &verdicts, &scale());
        assert!(
            s.contains("2 earlier verdict(s) omitted"),
            "the model must know its record is partial: {s}"
        );
        assert!(
            !s.contains("Task #0 "),
            "the oldest verdicts are dropped: {s}"
        );
        assert!(
            s.contains(&format!("Task #{}", PRIOR_VERDICT_MAX as i32 + 1)),
            "{s}"
        );
    }

    #[test]
    fn a_verdict_without_reasoning_says_so_rather_than_going_blank() {
        let s = build_user_prompt(
            &task_row(None),
            None,
            false,
            &[],
            None,
            &[verdict(0, "Quiet one", "   ")],
            &scale(),
        );
        assert!(s.contains("(you left no reasoning)"), "{s}");
    }

    #[test]
    fn contract_constraints_ride_with_the_task_prompt() {
        let ev = serde_json::json!({
            "kind": "open_ended",
            "completion": {"probe": "Definition of done", "deadline_secs": 900},
            "constraints": "The pinned dataset is the only data source.",
            "criteria": [{"key": "product", "title": "Product", "weight": 1.0}],
        });
        let s = build_user_prompt(&task_row(Some(ev)), None, false, &[], None, &[], &scale());
        assert!(
            s.contains("Constraints (from the task's contract):\nThe pinned dataset is the only data source."),
            "constraints must reach the judge prompt: {s}"
        );
    }

    #[test]
    fn absent_or_blank_constraints_add_nothing() {
        let s = build_user_prompt(&task_row(None), None, false, &[], None, &[], &scale());
        assert!(!s.contains("Constraints"), "classic task: {s}");

        let ev = serde_json::json!({
            "kind": "open_ended",
            "completion": {"probe": "Done", "deadline_secs": 900},
            "constraints": "   ",
            "criteria": [{"key": "product", "title": "Product", "weight": 1.0}],
        });
        let s = build_user_prompt(&task_row(Some(ev)), None, false, &[], None, &[], &scale());
        assert!(!s.contains("Constraints"), "blank constraints: {s}");
    }

    #[test]
    fn rating_scalar_reads_bare_numbers_and_criteria_sheets() {
        assert_eq!(rating_scalar(&serde_json::json!(7.5)), 7.5);
        assert_eq!(
            rating_scalar(
                &serde_json::json!({"overall": 26.0, "criteria": [{"key": "ux", "score": 8.0}]})
            ),
            26.0
        );
        assert_eq!(rating_scalar(&serde_json::Value::Null), 0.0);
        assert_eq!(rating_scalar(&serde_json::json!({"criteria": []})), 0.0);
    }
}
