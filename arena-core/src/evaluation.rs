//! The open-ended task contract and the extended probe config.
//!
//! A task with a non-NULL `tasks.evaluation` column is *open-ended*: shown in
//! full, worked at the participant's own pace, finished by a designated
//! completion probe (or its deadline), and scored by a judge panel against
//! per-criterion 0.0–10.0 scores. `NULL` means a classic probe-verified task,
//! whose behavior this module never touches.
//!
//! The probe stays the single data-gathering primitive. What this module adds
//! is vocabulary, not machinery: a probe may carry a [`ProbeConfig`] naming
//! its mode (where it runs and what evaluates it), a schedule, opt-in points,
//! and — for `interactive` — the artifact it expects. A probe row without a
//! config is a legacy participant shell probe, byte-for-byte.
//!
//! Everything here is plain serde over JSON. The task DSL writes these as
//! YAML fences; the server converts YAML→JSON at seed time and stores JSON,
//! so this crate needs no YAML dependency.

use serde::{Deserialize, Serialize};

/// `tests.initiator`: declared in task/judge config.
pub const INITIATOR_SYSTEM: &str = "system";
/// `tests.initiator`: registered live by a judge run.
pub const INITIATOR_JUDGE: &str = "judge";

/// `probes.outcome` for a probe whose tool/backend could not run at all.
/// Never a penalty: `point_delta` stays 0 and judges read it as an
/// observation, not a failure of the player's code.
pub const OUTCOME_UNAVAILABLE: &str = "unavailable";

/// `judge_results.verdict_kind` values.
pub const VERDICT_KIND_FULL: &str = "full";
pub const VERDICT_KIND_PARTIAL: &str = "partial";

/// The fixed per-criterion score scale (decided 2026-08-03).
pub const CRITERION_SCORE_MIN: f64 = 0.0;
pub const CRITERION_SCORE_MAX: f64 = 10.0;
pub const CRITERION_SCORE_STEP: f64 = 0.1;

/// Cap on a parsed TODO report stored in `probes.result_json` (§2.5).
pub const TODO_REPORT_MAX_BYTES: usize = 16 * 1024;

// ---------- Evaluation contract (tasks.evaluation) ----------

fn default_kind() -> String {
    "open_ended".to_string()
}
fn default_interactive_per_task() -> u32 {
    2
}
fn default_interactive_per_judge() -> u32 {
    1
}

/// How an open-ended task finishes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompletionSpec {
    /// Section title of the completion probe. When that probe passes, the
    /// task is complete.
    pub probe: String,
    /// Work window in seconds; on expiry the task is force-evaluated on
    /// whatever exists.
    pub deadline_secs: i64,
}

/// One row of the per-criterion score sheet.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CriterionDef {
    pub key: String,
    #[serde(default)]
    pub title: String,
    pub weight: f64,
}

/// Caps on interactive probes (the exception, not the default).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InteractiveLimits {
    #[serde(default = "default_interactive_per_task")]
    pub interactive_probes_per_task: u32,
    #[serde(default = "default_interactive_per_judge")]
    pub interactive_probes_per_judge: u32,
}

impl Default for InteractiveLimits {
    fn default() -> Self {
        Self {
            interactive_probes_per_task: default_interactive_per_task(),
            interactive_probes_per_judge: default_interactive_per_judge(),
        }
    }
}

/// The whole `tasks.evaluation` payload.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvaluationContract {
    #[serde(default = "default_kind")]
    pub kind: String,
    pub completion: CompletionSpec,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub constraints: Option<String>,
    pub criteria: Vec<CriterionDef>,
    #[serde(default)]
    pub limits: InteractiveLimits,
}

impl EvaluationContract {
    /// Parse the `tasks.evaluation` column. `None` column = classic task.
    pub fn from_json(value: &serde_json::Value) -> Result<Self, String> {
        serde_json::from_value(value.clone()).map_err(|e| format!("evaluation contract: {e}"))
    }
}

/// Validate a contract against the task's section titles (the completion
/// probe must name a real section) and the session bounds.
pub fn validate_evaluation_contract(
    contract: &EvaluationContract,
    section_titles: &[String],
) -> Result<(), String> {
    if contract.kind != "open_ended" {
        return Err(format!(
            "evaluation.kind must be \"open_ended\", got {:?}",
            contract.kind
        ));
    }
    if contract.completion.deadline_secs <= 0 {
        return Err("evaluation.completion.deadline_secs must be positive".into());
    }
    if contract.completion.probe.trim().is_empty() {
        return Err("evaluation.completion.probe must name a section".into());
    }
    if !section_titles.is_empty()
        && !section_titles
            .iter()
            .any(|t| t.trim() == contract.completion.probe.trim())
    {
        return Err(format!(
            "evaluation.completion.probe {:?} does not match any task section (have: {})",
            contract.completion.probe,
            section_titles
                .iter()
                .map(|t| format!("{t:?}"))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    if contract.criteria.is_empty() {
        return Err("evaluation.criteria must not be empty".into());
    }
    let mut seen = std::collections::BTreeSet::new();
    let mut weight_sum = 0.0;
    for c in &contract.criteria {
        if c.key.trim().is_empty() {
            return Err("evaluation.criteria[].key must not be empty".into());
        }
        if !seen.insert(c.key.trim().to_string()) {
            return Err(format!("duplicate criterion key {:?}", c.key));
        }
        if !c.weight.is_finite() || c.weight <= 0.0 {
            return Err(format!(
                "criterion {:?} weight must be a positive number",
                c.key
            ));
        }
        weight_sum += c.weight;
    }
    if weight_sum <= 0.0 {
        return Err("evaluation.criteria weights must sum to a positive number".into());
    }
    Ok(())
}

// ---------- Probe config (tests.probe_config) ----------

/// Where a probe runs and what evaluates it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProbeMode {
    /// Sandbox run of the section's command against the player's snapshot
    /// (or the working tree, with `executor: participant`), graded by the
    /// existing evaluators.
    Deterministic,
    /// A registered tool adapter (jscpd, linters, …) over the snapshot.
    /// Missing tool ⇒ outcome `unavailable`, never a penalty.
    Analysis,
    /// REMOVED (2026-08-12): rubric probes duplicated the judges as a second
    /// LLM-evaluation path. The variant only remains so stored definitions
    /// and probe history from before the removal keep deserializing —
    /// validation rejects new ones and the executor records `unavailable`.
    Llm,
    /// Requires participant action: an artifact committed under `.ololo/`.
    Interactive,
}

/// Which side executes the probe.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProbeExecutor {
    Server,
    Participant,
}

/// When a system probe fires.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScheduleOn {
    /// Once, when the task becomes current.
    Start,
    /// Repeatedly, every `interval_secs`.
    Interval,
    /// Once, when the task completes (or its deadline forces evaluation).
    Done,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProbeSchedule {
    pub on: Vec<ScheduleOn>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interval_secs: Option<u32>,
}

/// Opt-in per-probe points. Default 0/0: open-ended probes are measurements;
/// judges convert them into score (decided 2026-08-03).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProbePoints {
    #[serde(default)]
    pub pass: i32,
    #[serde(default)]
    pub fail: i32,
}

/// What a structured report probe carries (§2.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReportKind {
    /// Stdout is the plan file; the server parses checkbox lines into
    /// `result_json.todo` for the player page checklist.
    Todo,
}

/// Recommended default artifact caps (decided 2026-08-03).
pub const ARTIFACT_DEFAULT_MAX_BYTES_IMAGE: u64 = 5 * 1024 * 1024;
pub const ARTIFACT_DEFAULT_MAX_BYTES_VIDEO: u64 = 50 * 1024 * 1024;

/// How many files one artifact request may deliver. The arrival sweep
/// records the first this many (path order) and notes what it dropped;
/// `max_bytes` stays a per-file cap.
pub const MAX_ARTIFACT_FILES: usize = 5;

fn default_artifact_max_bytes() -> u64 {
    ARTIFACT_DEFAULT_MAX_BYTES_IMAGE
}

/// Media content type inferred from a delivered artifact's file extension.
///
/// The judge's request declares an EXPECTED type (`video/webm`), but the
/// participant decides the actual format — a screencast often arrives as a
/// `.gif`. Rendering and serving must follow the file, not the declaration,
/// or the page wraps a GIF in `<video>` and shows nothing. `None` for
/// unrecognized extensions — callers fall back to the declared type.
pub fn artifact_content_type_for_path(path: &str) -> Option<&'static str> {
    let lower = path.to_ascii_lowercase();
    let (_, ext) = lower.rsplit_once('.')?;
    Some(match ext {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        "gif" => "image/gif",
        "mp4" => "video/mp4",
        "webm" => "video/webm",
        "mov" => "video/quicktime",
        _ => return None,
    })
}

/// Schema of the artifact an interactive probe expects.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArtifactSpec {
    /// Expected content type, e.g. `image/png`, `video/mp4`, `text/plain`.
    pub content_type: String,
    #[serde(default = "default_artifact_max_bytes")]
    pub max_bytes: u64,
    /// Repo path the artifact must land at. Defaults to
    /// `.ololo/artifacts/<probe_id>/` at dispatch time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

/// The whole `tests.probe_config` payload. NULL column = legacy probe.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProbeConfig {
    pub mode: ProbeMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub executor: Option<ProbeExecutor>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schedule: Option<ProbeSchedule>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub points: Option<ProbePoints>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub report: Option<ReportKind>,
    /// Analysis mode: the registered tool adapter name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool: Option<String>,
    /// Legacy `mode: llm` rubric, kept only so stored definitions from
    /// before the removal keep deserializing. Never read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rubric: Option<serde_json::Value>,
    /// Interactive mode: what to ask the participant for.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instruction: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact: Option<ArtifactSpec>,
    /// Interactive mode: how long the participant has, seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deadline_secs: Option<i64>,
    /// Deterministic probe: on pass, store this probe's trimmed stdout into
    /// session memory under this key. Lets a task feed a *deterministic* value
    /// (e.g. the `run:` command a setup probe prints from the player's docs)
    /// straight into `{memory.<key>}` — no LLM extraction, no pushed snapshot,
    /// works identically in TUI and headless play. The probe still grades and
    /// scores like a normal task check.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capture_memory: Option<String>,
}

impl ProbeConfig {
    pub fn from_json(value: &serde_json::Value) -> Result<Self, String> {
        serde_json::from_value(value.clone()).map_err(|e| format!("probe config: {e}"))
    }

    /// Who actually runs this probe.
    ///
    /// Interactive is always the participant. Analysis is always the server:
    /// it is the platform's own tool (jscpd and friends) reading a snapshot,
    /// not the player's build.
    ///
    /// **Deterministic runs on the participant's machine** unless a project
    /// explicitly asks otherwise. Its command is the player's own — `npm
    /// test`, `cargo test`, a build, a curl at a server they are running —
    /// and their machine is where it has a toolchain, their dependencies and
    /// their running process. The server has a bare snapshot in a scratch
    /// directory, so the same command means something different there: a
    /// judge-registered `npm test` once failed server-side with
    /// `Could not find '/tmp/.tmpfaEuCF/test/**/*.test.ts'` while the very
    /// same command passed in the player's workspace, and the verdict was
    /// written off the server's answer.
    pub fn effective_executor(&self) -> ProbeExecutor {
        match self.mode {
            ProbeMode::Interactive => ProbeExecutor::Participant,
            ProbeMode::Analysis | ProbeMode::Llm => ProbeExecutor::Server,
            ProbeMode::Deterministic => self.executor.unwrap_or(ProbeExecutor::Participant),
        }
    }
}

/// Minimum interval between scheduled probe runs.
pub const MIN_SCHEDULE_INTERVAL_SECS: u32 = 5;

/// Validate a probe config together with the section's command presence.
pub fn validate_probe_config(cfg: &ProbeConfig, has_command: bool) -> Result<(), String> {
    match cfg.mode {
        ProbeMode::Deterministic => {
            if !has_command {
                return Err("deterministic probe needs a `sh command` block".into());
            }
            if cfg.tool.is_some() || cfg.rubric.is_some() || cfg.artifact.is_some() {
                return Err("deterministic probe takes no tool/rubric/artifact".into());
            }
        }
        ProbeMode::Analysis => {
            if cfg.tool.as_deref().map(str::trim).unwrap_or("").is_empty() {
                return Err("analysis probe needs `tool`".into());
            }
            if cfg.executor == Some(ProbeExecutor::Participant) {
                return Err("analysis probes run server-side only".into());
            }
            if cfg.rubric.is_some() || cfg.artifact.is_some() {
                return Err("analysis probe takes no rubric/artifact".into());
            }
        }
        ProbeMode::Llm => {
            return Err(
                "`mode: llm` probes were removed — attach a judge for LLM evaluation".into(),
            );
        }
        ProbeMode::Interactive => {
            if cfg
                .instruction
                .as_deref()
                .map(str::trim)
                .unwrap_or("")
                .is_empty()
            {
                return Err("interactive probe needs `instruction`".into());
            }
            let Some(artifact) = &cfg.artifact else {
                return Err("interactive probe needs `artifact`".into());
            };
            if artifact.content_type.trim().is_empty() {
                return Err("artifact.content_type must not be empty".into());
            }
            if artifact.max_bytes == 0 {
                return Err("artifact.max_bytes must be positive".into());
            }
            if cfg.executor == Some(ProbeExecutor::Server) {
                return Err("interactive probes are answered by the participant".into());
            }
            if cfg.tool.is_some() || cfg.rubric.is_some() {
                return Err("interactive probe takes no tool/rubric".into());
            }
        }
    }
    if cfg.report.is_some() && cfg.mode != ProbeMode::Deterministic {
        return Err("`report` is only valid on deterministic probes".into());
    }
    if let Some(key) = &cfg.capture_memory {
        if cfg.mode != ProbeMode::Deterministic {
            return Err("`capture_memory` is only valid on deterministic probes".into());
        }
        if key.trim().is_empty() {
            return Err("`capture_memory` must name a memory key".into());
        }
    }
    if let Some(schedule) = &cfg.schedule {
        if schedule.on.is_empty() {
            return Err("schedule.on must not be empty".into());
        }
        if schedule.on.contains(&ScheduleOn::Interval) {
            match schedule.interval_secs {
                Some(secs) if secs >= MIN_SCHEDULE_INTERVAL_SECS => {}
                Some(secs) => {
                    return Err(format!(
                        "schedule.interval_secs must be ≥ {MIN_SCHEDULE_INTERVAL_SECS}, got {secs}"
                    ));
                }
                None => {
                    return Err("schedule.on contains `interval` but interval_secs is unset".into());
                }
            }
        }
    }
    if let Some(points) = &cfg.points {
        // Fail deltas must not reward; pass deltas must not punish. Keeps the
        // opt-in from inverting the meaning of an outcome.
        if points.pass < 0 || points.fail > 0 {
            return Err("points.pass must be ≥ 0 and points.fail ≤ 0".into());
        }
    }
    if let Some(deadline) = cfg.deadline_secs
        && deadline <= 0
    {
        return Err("deadline_secs must be positive".into());
    }
    Ok(())
}

// ---------- Judge-declared probes (judges.probes_config) ----------

/// A probe a judge declares in its front-matter. Materialized as `tests`
/// rows with `initiator='judge'` when the judge is enqueued for a task.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JudgeProbeDef {
    pub name: String,
    pub mode: ProbeMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    /// JS validation expression, same dialect as task sections.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub validation: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rubric: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instruction: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact: Option<ArtifactSpec>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deadline_secs: Option<i64>,
}

impl JudgeProbeDef {
    /// The [`ProbeConfig`] this definition materializes as.
    pub fn to_probe_config(&self) -> ProbeConfig {
        ProbeConfig {
            mode: self.mode,
            executor: None,
            schedule: None,
            points: None,
            report: None,
            tool: self.tool.clone(),
            rubric: self.rubric.clone(),
            instruction: self.instruction.clone(),
            artifact: self.artifact.clone(),
            deadline_secs: self.deadline_secs,
            capture_memory: None,
        }
    }
}

/// Validate a judge's declared probe list (unique names, per-mode fields).
pub fn validate_judge_probes(defs: &[JudgeProbeDef]) -> Result<(), String> {
    let mut seen = std::collections::BTreeSet::new();
    for def in defs {
        if def.name.trim().is_empty() {
            return Err("judge probe name must not be empty".into());
        }
        if !seen.insert(def.name.trim().to_string()) {
            return Err(format!("duplicate judge probe name {:?}", def.name));
        }
        let has_command = def
            .command
            .as_deref()
            .map(str::trim)
            .is_some_and(|c| !c.is_empty());
        validate_probe_config(&def.to_probe_config(), has_command)
            .map_err(|e| format!("judge probe {:?}: {e}", def.name))?;
    }
    Ok(())
}

// ---------- TODO report parsing (§2.5) ----------

/// One parsed plan item.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TodoItem {
    pub text: String,
    pub done: bool,
}

/// The parsed plan, as stored under `result_json.todo`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TodoReport {
    pub checked: u32,
    pub total: u32,
    pub items: Vec<TodoItem>,
}

/// Cap on a single rendered item and on the item count, so a hostile plan
/// file cannot bloat the snapshot. Content is untrusted display text; the
/// frontend must escape it.
const TODO_ITEM_MAX_CHARS: usize = 500;
const TODO_MAX_ITEMS: usize = 200;

/// Parse markdown checkbox lines (`- [ ]` / `- [x]` / `* [X]`, any indent)
/// out of a probe's stdout. Lines that are not checkboxes are ignored — the
/// probe just `cat`s the plan file.
pub fn parse_todo_report(stdout: &str) -> TodoReport {
    let mut items = Vec::new();
    let mut budget = TODO_REPORT_MAX_BYTES;
    for line in stdout.lines() {
        if items.len() >= TODO_MAX_ITEMS {
            break;
        }
        let trimmed = line.trim_start();
        let rest = match trimmed
            .strip_prefix("- ")
            .or_else(|| trimmed.strip_prefix("* "))
        {
            Some(rest) => rest.trim_start(),
            None => continue,
        };
        let (done, text) = if let Some(t) = rest.strip_prefix("[ ]") {
            (false, t)
        } else if let Some(t) = rest
            .strip_prefix("[x]")
            .or_else(|| rest.strip_prefix("[X]"))
        {
            (true, t)
        } else {
            continue;
        };
        let text: String = text.trim().chars().take(TODO_ITEM_MAX_CHARS).collect();
        let cost = text.len() + 16;
        if budget < cost {
            break;
        }
        budget -= cost;
        items.push(TodoItem { text, done });
    }
    let checked = items.iter().filter(|i| i.done).count() as u32;
    TodoReport {
        checked,
        total: items.len() as u32,
        items,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn contract(json: serde_json::Value) -> Result<EvaluationContract, String> {
        EvaluationContract::from_json(&json)
    }

    #[test]
    fn contract_round_trips_and_validates() {
        let c = contract(serde_json::json!({
            "kind": "open_ended",
            "completion": { "probe": "Definition of done", "deadline_secs": 3600 },
            "criteria": [
                { "key": "correctness", "title": "Correctness", "weight": 0.8 },
                { "key": "tests", "weight": 0.2 }
            ],
            "limits": { "interactive_probes_per_task": 1 }
        }))
        .unwrap();
        assert_eq!(c.limits.interactive_probes_per_task, 1);
        assert_eq!(c.limits.interactive_probes_per_judge, 1);
        validate_evaluation_contract(&c, &["Definition of done".to_string(), "Other".to_string()])
            .unwrap();
    }

    #[test]
    fn contract_defaults_kind_and_limits() {
        let c = contract(serde_json::json!({
            "completion": { "probe": "Done", "deadline_secs": 60 },
            "criteria": [ { "key": "k", "weight": 1.0 } ]
        }))
        .unwrap();
        assert_eq!(c.kind, "open_ended");
        assert_eq!(c.limits.interactive_probes_per_task, 2);
    }

    #[test]
    fn contract_rejects_bad_shapes() {
        // Unknown fields refused (deny_unknown_fields).
        assert!(
            contract(serde_json::json!({
                "completion": { "probe": "x", "deadline_secs": 1 },
                "criteria": [ { "key": "k", "weight": 1.0 } ],
                "stages": []
            }))
            .is_err()
        );

        let base = contract(serde_json::json!({
            "completion": { "probe": "Done", "deadline_secs": 60 },
            "criteria": [ { "key": "k", "weight": 1.0 } ]
        }))
        .unwrap();

        // Completion probe must name a real section.
        let err = validate_evaluation_contract(&base, &["Other".to_string()]).unwrap_err();
        assert!(err.contains("does not match any task section"), "{err}");

        // Duplicate criterion keys.
        let mut dup = base.clone();
        dup.criteria = vec![
            CriterionDef {
                key: "k".into(),
                title: String::new(),
                weight: 1.0,
            },
            CriterionDef {
                key: "k".into(),
                title: String::new(),
                weight: 1.0,
            },
        ];
        assert!(
            validate_evaluation_contract(&dup, &[])
                .unwrap_err()
                .contains("duplicate")
        );

        // Non-positive weight.
        let mut zero = base.clone();
        zero.criteria[0].weight = 0.0;
        assert!(validate_evaluation_contract(&zero, &[]).is_err());

        // Wrong kind.
        let mut kinded = base;
        kinded.kind = "staged".into();
        assert!(validate_evaluation_contract(&kinded, &[]).is_err());
    }

    fn probe(json: serde_json::Value) -> Result<ProbeConfig, String> {
        ProbeConfig::from_json(&json)
    }

    #[test]
    fn probe_config_modes_validate() {
        let det = probe(serde_json::json!({
            "mode": "deterministic",
            "executor": "participant",
            "report": "todo",
            "schedule": { "on": ["interval"], "interval_secs": 120 }
        }))
        .unwrap();
        validate_probe_config(&det, true).unwrap();
        assert_eq!(det.effective_executor(), ProbeExecutor::Participant);

        // A deterministic probe that names no side runs on the player's
        // machine: the command is theirs (`npm test`, a build, a curl at
        // their own server) and only their workspace can answer it. The
        // server's scratch copy of the snapshot answers a different
        // question, and a judge that read that answer scored the wrong one.
        let unstated = probe(serde_json::json!({
            "mode": "deterministic",
            "schedule": { "on": ["done"] }
        }))
        .unwrap();
        validate_probe_config(&unstated, true).unwrap();
        assert_eq!(unstated.effective_executor(), ProbeExecutor::Participant);

        // A project may still send one to the server on purpose.
        let server_side = probe(serde_json::json!({
            "mode": "deterministic",
            "executor": "server",
            "schedule": { "on": ["done"] }
        }))
        .unwrap();
        assert_eq!(server_side.effective_executor(), ProbeExecutor::Server);

        let analysis = probe(serde_json::json!({
            "mode": "analysis", "tool": "jscpd",
            "schedule": { "on": ["done"] }
        }))
        .unwrap();
        validate_probe_config(&analysis, false).unwrap();
        assert_eq!(analysis.effective_executor(), ProbeExecutor::Server);

        // `mode: llm` still parses (stored legacy rows must not 500) but a
        // new definition is rejected — judges are the only LLM evaluation.
        let llm = probe(serde_json::json!({
            "mode": "llm",
            "rubric": { "inputs": ["BRIEF.md"],
                        "questions": [ { "key": "coverage", "ask": "How much is done?" } ] }
        }))
        .unwrap();
        assert!(
            validate_probe_config(&llm, false)
                .unwrap_err()
                .contains("removed")
        );

        let interactive = probe(serde_json::json!({
            "mode": "interactive",
            "instruction": "Screenshot the main page",
            "artifact": { "content_type": "image/png" },
            "deadline_secs": 900
        }))
        .unwrap();
        validate_probe_config(&interactive, false).unwrap();
        assert_eq!(interactive.effective_executor(), ProbeExecutor::Participant);
        assert_eq!(
            interactive.artifact.as_ref().unwrap().max_bytes,
            ARTIFACT_DEFAULT_MAX_BYTES_IMAGE
        );
    }

    #[test]
    fn probe_config_rejects_mode_mismatches() {
        // Deterministic without a command.
        let det = probe(serde_json::json!({ "mode": "deterministic" })).unwrap();
        assert!(validate_probe_config(&det, false).is_err());

        // Analysis without a tool.
        let analysis = probe(serde_json::json!({ "mode": "analysis" })).unwrap();
        assert!(validate_probe_config(&analysis, false).is_err());

        // Llm without a rubric.
        let llm = probe(serde_json::json!({ "mode": "llm" })).unwrap();
        assert!(validate_probe_config(&llm, false).is_err());

        // Llm with an empty or path-escaping rubric.
        let empty_q = probe(serde_json::json!({
            "mode": "llm", "rubric": { "inputs": [], "questions": [] }
        }))
        .unwrap();
        assert!(validate_probe_config(&empty_q, false).is_err());
        let escape = probe(serde_json::json!({
            "mode": "llm",
            "rubric": { "inputs": ["../secrets"],
                        "questions": [ { "key": "k", "ask": "?" } ] }
        }))
        .unwrap();
        assert!(validate_probe_config(&escape, false).is_err());

        // Interactive without instruction/artifact.
        let interactive = probe(serde_json::json!({ "mode": "interactive" })).unwrap();
        assert!(validate_probe_config(&interactive, false).is_err());

        // Interval schedule without interval_secs.
        let sched = probe(serde_json::json!({
            "mode": "deterministic", "schedule": { "on": ["interval"] }
        }))
        .unwrap();
        assert!(validate_probe_config(&sched, true).is_err());

        // report on a non-deterministic probe.
        let report = serde_json::json!({
            "mode": "analysis", "tool": "jscpd", "report": "todo"
        });
        let report = ProbeConfig::from_json(&report).unwrap();
        assert!(validate_probe_config(&report, false).is_err());

        // Inverted points.
        let points = probe(serde_json::json!({
            "mode": "deterministic", "points": { "pass": -1 }
        }))
        .unwrap();
        assert!(validate_probe_config(&points, true).is_err());

        // capture_memory is fine on a deterministic probe with a command…
        let cap = probe(serde_json::json!({
            "mode": "deterministic", "executor": "participant", "capture_memory": "run"
        }))
        .unwrap();
        assert_eq!(cap.capture_memory.as_deref(), Some("run"));
        assert!(validate_probe_config(&cap, true).is_ok());

        // …but not on a non-deterministic probe, and not empty.
        let cap_analysis = serde_json::json!({
            "mode": "analysis", "tool": "jscpd", "capture_memory": "run"
        });
        let cap_analysis = ProbeConfig::from_json(&cap_analysis).unwrap();
        assert!(validate_probe_config(&cap_analysis, false).is_err());
        let cap_empty = probe(serde_json::json!({
            "mode": "deterministic", "capture_memory": "  "
        }))
        .unwrap();
        assert!(validate_probe_config(&cap_empty, true).is_err());
    }

    #[test]
    fn judge_probes_validate_per_mode() {
        let defs: Vec<JudgeProbeDef> = serde_json::from_value(serde_json::json!([
            { "name": "build", "mode": "deterministic",
              "command": "cargo build", "validation": "result.includes('ok')" },
            { "name": "dup", "mode": "analysis", "tool": "jscpd" }
        ]))
        .unwrap();
        validate_judge_probes(&defs).unwrap();

        let bad: Vec<JudgeProbeDef> = serde_json::from_value(serde_json::json!([
            { "name": "build", "mode": "deterministic" }
        ]))
        .unwrap();
        assert!(validate_judge_probes(&bad).unwrap_err().contains("build"));

        let dup: Vec<JudgeProbeDef> = serde_json::from_value(serde_json::json!([
            { "name": "a", "mode": "analysis", "tool": "jscpd" },
            { "name": "a", "mode": "analysis", "tool": "oxlint" }
        ]))
        .unwrap();
        assert!(
            validate_judge_probes(&dup)
                .unwrap_err()
                .contains("duplicate")
        );
    }

    #[test]
    fn todo_report_parses_checkboxes() {
        let report = parse_todo_report(
            "# Plan\n\
             - [x] Set up routing\n\
             * [X] Model layer\n\
             - [ ] Watering schedule UI\n\
             not a checkbox\n\
             \t- [ ] Indented item\n",
        );
        assert_eq!(report.total, 4);
        assert_eq!(report.checked, 2);
        assert_eq!(
            report.items[0],
            TodoItem {
                text: "Set up routing".into(),
                done: true
            }
        );
        assert_eq!(report.items[3].text, "Indented item");
    }

    #[test]
    fn todo_report_caps_hostile_input() {
        let big_item = format!("- [ ] {}\n", "x".repeat(10_000));
        let report = parse_todo_report(&big_item.repeat(500));
        assert!(report.items.len() <= TODO_MAX_ITEMS);
        assert!(
            report
                .items
                .iter()
                .all(|i| i.text.len() <= TODO_ITEM_MAX_CHARS)
        );
        let encoded = serde_json::to_string(&report).unwrap();
        assert!(
            encoded.len() <= TODO_REPORT_MAX_BYTES * 2,
            "{}",
            encoded.len()
        );
    }

    #[test]
    fn artifact_content_type_follows_the_file_extension() {
        assert_eq!(
            artifact_content_type_for_path(".ololo/artifacts/x/demo.GIF"),
            Some("image/gif")
        );
        assert_eq!(
            artifact_content_type_for_path("shots/rome-desktop.png"),
            Some("image/png")
        );
        assert_eq!(
            artifact_content_type_for_path("clip.webm"),
            Some("video/webm")
        );
        assert_eq!(
            artifact_content_type_for_path("flow.mov"),
            Some("video/quicktime")
        );
        assert_eq!(artifact_content_type_for_path("notes.txt"), None);
        assert_eq!(artifact_content_type_for_path("no-extension"), None);
    }
}
