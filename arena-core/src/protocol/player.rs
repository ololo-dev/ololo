//! Player-facing snapshot / delta / judge frames.

use serde::{Deserialize, Serialize};

use crate::session_status::SessionStatus;

/// Derived completion state of one player within a session, computed by
/// detail/snapshot endpoints. Carried as `Option<...>` on payloads —
/// `None` means "not computed" (list paths skip the computation).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlayerCompletionStatus {
    /// The player still has unfinished tasks.
    InProgress,
    /// All tasks done; judge verdicts still outstanding.
    AwaitingJudges,
    /// All tasks done and all judges have run.
    Completed,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlayerTaskResult {
    pub status: String,
    pub submitted_answer: Option<String>,
    pub correct_answer: Option<String>,
    pub score_delta: i32,
    pub evaluated_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlayerProbeResult {
    pub status: String,
    pub expected: Option<String>,
    pub actual: Option<String>,
}

/// The human display label stored in `tests.prompt`, if it is a real one.
/// Legacy adaptation wrote placeholder strings there ("Structured markdown
/// test N", "Passthrough test N"); those are plumbing, not labels.
pub fn test_display_label(prompt: &str) -> Option<&str> {
    let t = prompt.trim();
    if t.is_empty()
        || t.starts_with("Structured markdown test")
        || t.starts_with("Passthrough test")
    {
        None
    } else {
        Some(t)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlayerProbeEntry {
    pub id: uuid::Uuid,
    pub task_id: uuid::Uuid,
    pub task_title: String,
    pub task_ordinal: i32,
    pub adapted_test_id: uuid::Uuid,
    /// Ordinal of the probe type (tests row) within the task, when known.
    #[serde(default)]
    pub test_ordinal: Option<i32>,
    /// Human label of the test this probe runs — the `## ` heading from the
    /// task's test definition. The UI shows this instead of parsing shell
    /// out of `rendered_command`. Absent on legacy sessions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// The test section's prose from the task markdown — what the check
    /// verifies, in the author's words (or the judge's instruction for
    /// judge-registered probes). Absent on legacy rows.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub test_command: String,
    pub attempt: i32,
    pub rendered_command: String,
    pub fixture_values: Option<String>,
    pub expected_answer: Option<String>,
    pub state: ProbeState,
    pub outcome: Option<String>,
    pub actual: Option<String>,
    pub expected: Option<String>,
    pub exit_code: Option<i32>,
    pub duration_ms: Option<i64>,
    pub dispatched_at: Option<chrono::DateTime<chrono::Utc>>,
    pub deadline_at: Option<chrono::DateTime<chrono::Utc>>,
    pub resolved_at: Option<chrono::DateTime<chrono::Utc>>,
    pub point_delta: i32,
    pub updated_at: chrono::DateTime<chrono::Utc>,
    pub result: Option<PlayerProbeResult>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlayerTaskSummaryEntry {
    pub task_id: uuid::Uuid,
    pub ordinal: i32,
    pub title: String,
    pub content: String,
    pub tags: Vec<String>,
    pub adapted_content: String,
    pub result: Option<PlayerTaskResult>,
    pub scheduler_state: Option<PlayerSchedulerState>,
    /// Sum of all point deltas the player collected on this task
    /// (probe grades + completion bonus + judge verdicts).
    #[serde(default)]
    pub total_points: i64,
    /// Completion-bonus portion of `total_points` (task_results rows with
    /// `is_bonus`), surfaced separately so the UI can label it.
    #[serde(default)]
    pub bonus_points: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlayerSchedulerState {
    pub state: String,
    pub activated_at: Option<chrono::DateTime<chrono::Utc>>,
    pub deadline_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// The player's session report: one narrative written at the end of the
/// session by a `kind: report` judge. It carries no score — it is the debrief,
/// not a verdict — so it is delivered apart from `judge_results` rather than
/// masquerading as one more judge card on whichever task it was attached to.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlayerSessionReport {
    /// Display name of the judge that wrote it, for attribution.
    pub judge_name: String,
    pub judge_slug: String,
    /// The report as the judge wrote it. Prose only when the model failed to
    /// answer with the document; the page renders `document` when present.
    pub markdown: String,
    /// The structured report the page lays out — brief, completed tasks,
    /// friction, per-judge notes, next steps. Absent for prose fallbacks and
    /// for reports written before the document existed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub document: Option<crate::judging::report::SessionReportDoc>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlayerSnapshotPayload {
    pub player_id: uuid::Uuid,
    pub display_name: String,
    /// Avatar URL of the account that owns this player, if set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avatar_url: Option<String>,
    /// AI coding agent the participant is running, if reported via metadata.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_display_name: Option<String>,
    pub score: i64,
    pub rank: usize,
    pub last_seq: u64,
    pub probes: Vec<PlayerProbeEntry>,
    pub tasks: Vec<PlayerTaskSummaryEntry>,
    pub total_tasks: usize,
    pub next_probe_at: Option<chrono::DateTime<chrono::Utc>>,
    pub session_started_at: Option<chrono::DateTime<chrono::Utc>>,
    pub session_ends_at: Option<chrono::DateTime<chrono::Utc>>,
    pub session_status: SessionStatus,
    /// Whether the player's ololo agent socket is connected right now.
    /// `None` on list paths / pre-upgrade servers that do not compute it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_connected: Option<bool>,
    /// Derived completion state of this player. `None` means "not computed"
    /// (list paths and pre-upgrade servers omit it).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completion_status: Option<PlayerCompletionStatus>,
    /// Judge results already persisted for this player, so a page load after
    /// a judge run still shows them (live updates arrive via `JudgeScored`).
    #[serde(default)]
    pub judge_results: Vec<PlayerJudgeScoredPayload>,
    /// Lifecycle state of every judge attached to a revealed task
    /// (pending/running/scored/failed), so the judges tab can show what will
    /// run, what is running, and what failed — not just successful verdicts.
    #[serde(default)]
    pub judge_statuses: Vec<PlayerJudgeStatusPayload>,
    /// The end-of-session debrief, once a report judge has written it.
    /// `None` while the session runs, and for projects with no report judge.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_report: Option<PlayerSessionReport>,
    /// Open-ended evaluation state per revealed open-ended task: contract
    /// criteria with the panel's aggregated scores, the latest TODO report,
    /// and pending interactive artifact requests. Empty for classic
    /// projects; `#[serde(default)]` for pre-upgrade servers.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evaluations: Vec<PlayerTaskEvaluation>,
    /// Cross-session copy/paste adjustment applied at session finish, if
    /// any — a score change is never allowed to be reasonless on the page.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub similarity_adjustment: Option<PlayerSimilarityAdjustment>,
}

/// The copy/paste validation's result, as shown to the player: the note
/// explains it in a sentence, the delta is the cost (0 = checked, clean),
/// and `sources` name whose code in which session was matched.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlayerSimilarityAdjustment {
    pub note: String,
    pub point_delta: i32,
    #[serde(default)]
    pub duplicated_pct: f64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sources: Vec<PlayerSimilaritySource>,
}

/// One prior session whose code the scan matched.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlayerSimilaritySource {
    pub join_code: String,
    pub player: String,
    pub matched_lines: i64,
}

/// One open-ended task's evaluation state on the player page.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlayerTaskEvaluation {
    pub task_id: uuid::Uuid,
    /// The contract's criteria, each with the panel's scores when judged.
    pub criteria: Vec<PlayerCriterionState>,
    /// Latest parsed TODO report (`result_json.todo` of the report probe).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub todo: Option<serde_json::Value>,
    /// Interactive requests still waiting on the participant.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pending_artifacts: Vec<PlayerArtifactRequest>,
    /// When the work window closes, when it is known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deadline_at: Option<chrono::DateTime<chrono::Utc>>,
    /// Measurement history for progress charts: dispatched_at + the probe's
    /// structured measurement, oldest first, config-carrying probes only.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub measurements: Vec<PlayerMeasurementPoint>,
    /// Resolved interactive artifacts, viewable via the artifact endpoint.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub artifacts: Vec<PlayerArtifactRef>,
    /// Image files committed at the repo's HEAD (paths), viewable via the
    /// repo-file endpoint — participants often commit screenshots on their
    /// own initiative, and those belong in the gallery too.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub repo_images: Vec<String>,
}

/// A resolved artifact the page can render inline.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlayerArtifactRef {
    pub probe_id: uuid::Uuid,
    pub content_type: String,
    /// Repo path, as the label.
    pub label: String,
    /// Slug of the judge whose request this artifact answers — the page
    /// shows the capture inside that judge's verdict block.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub judge_slug: Option<String>,
    /// How many files the request delivered (a request carries up to
    /// [`crate::evaluation::MAX_ARTIFACT_FILES`]); the artifact endpoint's
    /// `?i=N` addresses them.
    #[serde(default = "one")]
    pub file_count: u32,
    /// Repo path of every delivered file, in `?i=N` order. `label` is the
    /// first of them; the rest have names of their own, and a gallery that
    /// only knew the count had to caption them "…/receipt-desktop.png (2/2)"
    /// when the file was called `receipt-mobile.png`. Empty for rows written
    /// before the list was carried.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub files: Vec<String>,
}

fn one() -> u32 {
    1
}

/// One criterion of the contract, with every judge's score of it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlayerCriterionState {
    pub key: String,
    pub title: String,
    pub weight: f64,
    /// Scores judges gave this criterion (0.0–10.0), with rationale.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub scores: Vec<PlayerCriterionScore>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlayerCriterionScore {
    pub judge_slug: String,
    /// `null` = the judge could not assess this criterion.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub score: Option<f64>,
    #[serde(default)]
    pub rationale: String,
}

/// An interactive probe waiting on the participant, for the banner.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlayerArtifactRequest {
    pub probe_id: uuid::Uuid,
    pub instruction: String,
    pub path: String,
    pub deadline_at: chrono::DateTime<chrono::Utc>,
}

/// One probe measurement for the progress time series.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlayerMeasurementPoint {
    pub test_ordinal: i32,
    /// Section name (tests.prompt) so the chart can label series.
    pub label: String,
    pub at: chrono::DateTime<chrono::Utc>,
    pub outcome: Option<String>,
    pub result_json: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlayerDeltaPayload {
    pub seq: u64,
    pub task_id: uuid::Uuid,
    pub result: Option<PlayerTaskResult>,
    pub scheduler_state: Option<PlayerSchedulerState>,
    pub score: i64,
    pub rank: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProbeState {
    Dispatched,
    Resolved,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProbeUpdatedPayload {
    pub seq: u64,
    pub probe_id: uuid::Uuid,
    pub task_id: uuid::Uuid,
    pub task_title: String,
    pub task_ordinal: i32,
    pub adapted_test_id: uuid::Uuid,
    /// Ordinal of the probe type (tests row) within the task, when known.
    #[serde(default)]
    pub test_ordinal: Option<i32>,
    /// Human label of the test this probe runs — the `## ` heading from the
    /// task's test definition. The UI shows this instead of parsing shell
    /// out of `rendered_command`. Absent on legacy sessions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// The test section's prose — mirrors `PlayerProbeEntry::description`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub test_command: String,
    pub attempt: i32,
    pub rendered_command: String,
    pub fixture_values: Option<String>,
    pub expected_answer: Option<String>,
    pub state: ProbeState,
    pub outcome: Option<String>,
    pub actual: Option<String>,
    pub expected: Option<String>,
    pub exit_code: Option<i32>,
    pub duration_ms: Option<i64>,
    pub dispatched_at: Option<chrono::DateTime<chrono::Utc>>,
    pub deadline_at: Option<chrono::DateTime<chrono::Utc>>,
    pub resolved_at: Option<chrono::DateTime<chrono::Utc>>,
    pub point_delta: i32,
    pub score: i64,
    pub rank: usize,
    pub updated_at: chrono::DateTime<chrono::Utc>,
    pub next_probe_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlayerSessionCompletePayload {
    pub seq: u64,
    pub player_id: uuid::Uuid,
    pub score: i64,
    pub rank: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskRevealedPayload {
    pub seq: u64,
    pub task: PlayerTaskSummaryEntry,
    pub total_tasks: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScoreRankUpdatedPayload {
    pub seq: u64,
    pub score: i64,
    pub rank: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlayerJudgeScoredPayload {
    pub task_id: uuid::Uuid,
    pub judge_slug: String,
    pub judge_name: String,
    pub rating: f64,
    pub feedback: String,
    pub point_delta: i32,
    pub created_at: chrono::DateTime<chrono::Utc>,
    /// Wall-clock duration of the judge run, when recorded.
    #[serde(default)]
    pub duration_ms: Option<i64>,
}

/// Lifecycle state of one judge attachment for one task, as shown on the
/// player page's judges tab. `status` is "pending" (attached, not yet run),
/// "running", "scored", or "failed"; `error` is set only for failed runs and
/// carries a GENERIC public message — the full provider/system error lives
/// in `judge_results.error`, reachable only via the admin details endpoint
/// (keyed by `judge_result_id`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlayerJudgeStatusPayload {
    pub task_id: uuid::Uuid,
    pub judge_slug: String,
    pub judge_name: String,
    pub status: String,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub updated_at: Option<chrono::DateTime<chrono::Utc>>,
    /// `judge_results.id` backing this status, when a run exists. Lets the
    /// admin UI fetch full run details (model, provider, tokens, log).
    #[serde(default)]
    pub judge_result_id: Option<uuid::Uuid>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum PlayerFrame {
    PlayerSnapshot(PlayerSnapshotPayload),
    PlayerTaskDelta(PlayerDeltaPayload),
    ProbeUpdated(ProbeUpdatedPayload),
    TaskRevealed(TaskRevealedPayload),
    ScoreRankUpdated(ScoreRankUpdatedPayload),
    SessionComplete(PlayerSessionCompletePayload),
    SessionStatusChange {
        status: SessionStatus,
        /// Set when `status` is Cancelled: `"user"` | `"idle_timeout"`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cancel_reason: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cancelled_by: Option<String>,
    },
    PlayerError {
        seq: u64,
        message: String,
    },
    /// Server → browser client: the player's ololo agent socket connected or
    /// disconnected on the game server. Drives the page's Live indicator —
    /// session status alone cannot tell a live client from a dropped one.
    AgentPresence {
        connected: bool,
        timestamp: chrono::DateTime<chrono::Utc>,
    },
    /// Server → browser client: a judge has scored the player's task attempt.
    JudgeScored(PlayerJudgeScoredPayload),
    /// Server → browser client: a judge run started for the player's task.
    JudgeStarted(PlayerJudgeStatusPayload),
    /// Server → browser client: a judge run failed before producing a verdict.
    JudgeFailed(PlayerJudgeStatusPayload),
    /// A judge is waiting for a participant artifact (open-ended tasks) —
    /// the player page shows the banner with the countdown.
    ArtifactAwaited {
        task_id: uuid::Uuid,
        probe_id: uuid::Uuid,
        instruction: String,
        deadline_at: chrono::DateTime<chrono::Utc>,
    },
    /// Every judge of the task is terminal — the evaluation is final.
    /// The session report has been written for this player. The document
    /// itself comes from the snapshot the page re-fetches — this frame only
    /// says it is there now.
    SessionReportReady,
    EvaluationReady {
        task_id: uuid::Uuid,
    },
}

/// Internal type shared by WS and REST serialization paths.
/// Assembled from DB queries; converted to PlayerSnapshotPayload for wire.
#[derive(Debug, Clone)]
pub struct PlayerSnapshotData {
    pub player_id: uuid::Uuid,
    pub display_name: String,
    pub avatar_url: Option<String>,
    pub agent_display_name: Option<String>,
    pub score: i64,
    pub rank: usize,
    pub last_seq: u64,
    pub probes: Vec<PlayerProbeEntry>,
    pub tasks: Vec<PlayerTaskSummaryEntry>,
    pub total_tasks: usize,
    pub next_probe_at: Option<chrono::DateTime<chrono::Utc>>,
    pub session_started_at: Option<chrono::DateTime<chrono::Utc>>,
    pub session_ends_at: Option<chrono::DateTime<chrono::Utc>>,
    pub session_status: SessionStatus,
    pub agent_connected: Option<bool>,
    pub completion_status: Option<PlayerCompletionStatus>,
    pub judge_results: Vec<PlayerJudgeScoredPayload>,
    pub judge_statuses: Vec<PlayerJudgeStatusPayload>,
    pub session_report: Option<PlayerSessionReport>,
    pub evaluations: Vec<PlayerTaskEvaluation>,
    pub similarity_adjustment: Option<PlayerSimilarityAdjustment>,
}

impl From<PlayerSnapshotData> for PlayerSnapshotPayload {
    fn from(data: PlayerSnapshotData) -> Self {
        PlayerSnapshotPayload {
            player_id: data.player_id,
            display_name: data.display_name,
            avatar_url: data.avatar_url,
            agent_display_name: data.agent_display_name,
            score: data.score,
            rank: data.rank,
            last_seq: data.last_seq,
            probes: data.probes,
            tasks: data.tasks,
            total_tasks: data.total_tasks,
            next_probe_at: data.next_probe_at,
            session_started_at: data.session_started_at,
            session_ends_at: data.session_ends_at,
            session_status: data.session_status,
            agent_connected: data.agent_connected,
            completion_status: data.completion_status,
            similarity_adjustment: data.similarity_adjustment,
            judge_results: data.judge_results,
            judge_statuses: data.judge_statuses,
            session_report: data.session_report,
            evaluations: data.evaluations,
        }
    }
}
