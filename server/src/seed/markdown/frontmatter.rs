//! Markdown frontmatter shape definitions.
//!
//! These mirror the JSON `Export*` structs but with `#[serde(default)]` on
//! optional fields so a missing frontmatter key is tolerated the same way the
//! JSON path tolerates a missing key. `deny_unknown_fields` is intentionally
//! NOT set here: frontmatter is hand-edited, so a stray key (e.g. a `note:`
//! annotation) should not fatal the whole project.

use serde::Deserialize;

use crate::api::admin_export_import::ExportPoints;
use crate::api::intervals::ExportIntervals;

#[derive(Debug, Deserialize)]
pub(crate) struct ReadmeFrontmatter {
    pub(crate) schema_version: u32,
    pub(crate) project: ReadmeProject,
}

#[derive(Debug, Deserialize, Default)]
pub(crate) struct ReadmeProject {
    pub(crate) name: String,
    // slug is derived from the project directory name, not frontmatter.
    #[serde(default)]
    pub(crate) description: Option<String>,
    #[serde(default)]
    pub(crate) category: Option<String>,
    #[serde(default)]
    pub(crate) tags: Vec<String>,
    #[serde(default)]
    pub(crate) cover_image_url: Option<String>,
    #[serde(default)]
    pub(crate) public: bool,
    #[serde(default)]
    pub(crate) archived_at: Option<String>,
    /// Project-level points defaults. Absent → hard baseline (10/-5/-10/10).
    #[serde(default)]
    pub(crate) points: Option<ReadmePoints>,
    /// Project-level interval defaults. Absent → hard baseline (60/5/5/60).
    #[serde(default)]
    pub(crate) intervals: Option<ReadmeIntervals>,
    /// Default session duration in seconds. Absent → 3600 (one hour).
    #[serde(default)]
    pub(crate) session_duration_secs: Option<i64>,
    /// Session-memory schema: flat map of `key -> scalar default value`,
    /// e.g. `memory: {dev: "npm run dev", port: 1234}`. Absent → none.
    #[serde(default)]
    pub(crate) memory: Option<serde_yaml::Value>,
    /// Whether the public project page lists the task arc up front.
    /// Absent → shown; quiz-shaped projects (Extreme Startup family) set
    /// `show_tasks: false` to keep their question ladder a surprise.
    #[serde(default = "default_show_tasks")]
    pub(crate) show_tasks: bool,
    /// Campaign parts in play order, by project slug. Non-empty marks this
    /// readme as a campaign parent (no `tasks/` directory of its own).
    #[serde(default)]
    pub(crate) parts: Vec<String>,
}

fn default_show_tasks() -> bool {
    true
}

/// Project-level points defaults in markdown frontmatter. All required `i32`;
/// if the `points:` block is omitted entirely, the hard baseline is used.
#[derive(Debug, Deserialize, Clone)]
pub(crate) struct ReadmePoints {
    pub(crate) value: i32,
    #[serde(default = "baseline_fail")]
    pub(crate) fail: i32,
    #[serde(default = "baseline_no_response")]
    pub(crate) no_response: i32,
    #[serde(default = "baseline_completion_bonus")]
    pub(crate) completion_bonus: i32,
}

fn baseline_fail() -> i32 {
    -5
}
fn baseline_no_response() -> i32 {
    -10
}
fn baseline_completion_bonus() -> i32 {
    10
}
fn baseline_value() -> i32 {
    10
}

impl ReadmePoints {
    /// Resolve to the wire `ExportPoints`, filling any omitted field with the
    /// hard baseline. Always returns a fully-populated struct.
    pub(crate) fn resolved(&self) -> ExportPoints {
        ExportPoints {
            value: self.value,
            fail: self.fail,
            no_response: self.no_response,
            completion_bonus: self.completion_bonus,
        }
    }
}

/// Hard-baseline defaults used when the readme omits the `points:` block.
pub(crate) fn baseline_points() -> ExportPoints {
    ExportPoints {
        value: baseline_value(),
        fail: baseline_fail(),
        no_response: baseline_no_response(),
        completion_bonus: baseline_completion_bonus(),
    }
}

/// Project-level interval defaults in markdown frontmatter. All required;
/// if the `intervals:` block is omitted entirely, the hard baseline is used.
#[derive(Debug, Deserialize, Clone)]
pub(crate) struct ReadmeIntervals {
    #[serde(default = "baseline_deadline_secs")]
    pub(crate) deadline_secs: i64,
    #[serde(default = "baseline_min_interval_secs")]
    pub(crate) min_interval_secs: i32,
    #[serde(default = "baseline_interval_increment_secs")]
    pub(crate) interval_increment_secs: i32,
    #[serde(default = "baseline_max_interval_secs")]
    pub(crate) max_interval_secs: i32,
}

fn baseline_deadline_secs() -> i64 {
    60
}
fn baseline_min_interval_secs() -> i32 {
    5
}
fn baseline_interval_increment_secs() -> i32 {
    5
}
fn baseline_max_interval_secs() -> i32 {
    60
}

impl ReadmeIntervals {
    pub(crate) fn resolved(&self) -> ExportIntervals {
        ExportIntervals {
            deadline_secs: self.deadline_secs,
            min_interval_secs: self.min_interval_secs,
            interval_increment_secs: self.interval_increment_secs,
            max_interval_secs: self.max_interval_secs,
        }
    }
}

pub(crate) fn baseline_intervals() -> ExportIntervals {
    ExportIntervals {
        deadline_secs: baseline_deadline_secs(),
        min_interval_secs: baseline_min_interval_secs(),
        interval_increment_secs: baseline_interval_increment_secs(),
        max_interval_secs: baseline_max_interval_secs(),
    }
}

#[derive(Debug, Deserialize)]
pub(crate) struct TaskFrontmatter {
    // ordinal is derived from the leading number in the task filename.
    pub(crate) title: String,
    #[serde(default)]
    pub(crate) content: String,
    #[serde(default)]
    pub(crate) tags: Vec<String>,
    /// Per-task points overrides. Absent fields inherit the project default.
    #[serde(default)]
    pub(crate) points: Option<TaskPointsFm>,
    /// Per-task interval overrides. Absent fields inherit the project default.
    #[serde(default)]
    pub(crate) intervals: Option<TaskIntervalsFm>,
    /// Judges to attach to the task, in order: a bare slug, or
    /// `{slug, weight}` for open-ended panel weighting.
    #[serde(default)]
    pub(crate) judges: Vec<JudgeRefFm>,
    #[serde(default)]
    pub(crate) test_template: TaskTemplateFm,
    /// Open-ended evaluation contract (free-form YAML, bridged to JSON and
    /// validated downstream). Absent = classic task.
    #[serde(default)]
    pub(crate) evaluation: Option<serde_yaml::Value>,
}

/// Frontmatter view of a judge attachment: `- slug` or `- {slug, weight}`.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub(crate) enum JudgeRefFm {
    Slug(String),
    Weighted {
        slug: String,
        #[serde(default)]
        weight: Option<f64>,
    },
}

/// Per-task points overrides in markdown frontmatter. All `Option<i32>`;
/// `None` means "inherit the project default".
#[derive(Debug, Deserialize, Default)]
pub(crate) struct TaskPointsFm {
    #[serde(default)]
    pub(crate) value: Option<i32>,
    #[serde(default)]
    pub(crate) fail: Option<i32>,
    #[serde(default)]
    pub(crate) no_response: Option<i32>,
    #[serde(default)]
    pub(crate) completion_bonus: Option<i32>,
}

/// Per-task interval overrides in markdown frontmatter. All `Option`;
/// `None` means "inherit the project default".
#[derive(Debug, Deserialize, Default)]
pub(crate) struct TaskIntervalsFm {
    #[serde(default)]
    pub(crate) deadline_secs: Option<i64>,
    #[serde(default)]
    pub(crate) min_interval_secs: Option<i32>,
    #[serde(default)]
    pub(crate) interval_increment_secs: Option<i32>,
    #[serde(default)]
    pub(crate) max_interval_secs: Option<i32>,
}

/// Frontmatter view of `TestTemplate`. `kind` defaults to `shell` because that
/// is the overwhelmingly common case for hand-authored tasks; the JSON path
/// requires it to be present, but for markdown we default it.
///
/// `Default` is hand-written rather than derived: a task file with no
/// `test_template:` block at all lands here through `#[serde(default)]`, and
/// the derive would give `kind: ""` where serde's field default gives
/// `"shell"` — the two paths must agree.
#[derive(Debug, Deserialize)]
pub(crate) struct TaskTemplateFm {
    #[serde(default = "default_shell_kind")]
    pub(crate) kind: String,
    #[serde(default)]
    pub(crate) placeholders: Vec<PlaceholderFm>,
    #[serde(default)]
    pub(crate) matchers: MatchersFm,
    #[serde(default)]
    pub(crate) backoff: BackoffFm,
    #[serde(default)]
    pub(crate) fixtures: serde_yaml::Value,
    #[serde(default)]
    pub(crate) answer_template: Option<String>,
}

fn default_shell_kind() -> String {
    "shell".to_string()
}

impl Default for TaskTemplateFm {
    fn default() -> Self {
        Self {
            kind: default_shell_kind(),
            placeholders: Vec::new(),
            matchers: MatchersFm::default(),
            backoff: BackoffFm::default(),
            fixtures: serde_yaml::Value::default(),
            answer_template: None,
        }
    }
}

#[derive(Debug, Deserialize)]
pub(crate) struct PlaceholderFm {
    pub(crate) name: String,
    #[serde(default)]
    pub(crate) description: String,
    #[serde(default)]
    pub(crate) required: bool,
    #[serde(default)]
    pub(crate) secret: bool,
}

/// `Default` is hand-written (see `TaskTemplateFm`): a task file with no
/// `test_template:` block must get the same values serde's field defaults
/// give a partial block — a derived all-zeros default fails validation.
#[derive(Debug, Deserialize)]
pub(crate) struct MatchersFm {
    #[serde(default = "default_exit_code")]
    pub(crate) expected_exit_code: i32,
    #[serde(default)]
    pub(crate) stdout_contains: Option<String>,
    #[serde(default)]
    pub(crate) stdout_regex: Option<String>,
    #[serde(default = "default_max_duration_ms")]
    pub(crate) max_duration_ms: u64,
    #[serde(default = "default_param_timeout_ms")]
    pub(crate) param_timeout_ms: u64,
}

fn default_exit_code() -> i32 {
    0
}
fn default_max_duration_ms() -> u64 {
    60_000
}
fn default_param_timeout_ms() -> u64 {
    60_000
}

impl Default for MatchersFm {
    fn default() -> Self {
        Self {
            expected_exit_code: default_exit_code(),
            stdout_contains: None,
            stdout_regex: None,
            max_duration_ms: default_max_duration_ms(),
            param_timeout_ms: default_param_timeout_ms(),
        }
    }
}

/// `Default` is hand-written for the same reason as `MatchersFm`.
#[derive(Debug, Deserialize)]
pub(crate) struct BackoffFm {
    #[serde(default = "default_initial_ms")]
    pub(crate) initial_ms: u64,
    #[serde(default = "default_multiplier")]
    pub(crate) multiplier: f64,
    #[serde(default = "default_max_ms")]
    pub(crate) max_ms: u64,
    #[serde(default = "default_max_attempts")]
    pub(crate) max_attempts: u32,
}

fn default_initial_ms() -> u64 {
    5_000
}
fn default_multiplier() -> f64 {
    2.0
}
fn default_max_ms() -> u64 {
    300_000
}
fn default_max_attempts() -> u32 {
    8
}

impl Default for BackoffFm {
    fn default() -> Self {
        Self {
            initial_ms: default_initial_ms(),
            multiplier: default_multiplier(),
            max_ms: default_max_ms(),
            max_attempts: default_max_attempts(),
        }
    }
}
