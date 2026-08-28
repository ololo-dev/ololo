//! CLI auth, player-agent, and judge-scored frames.

use serde::{Deserialize, Serialize};

/// Wire frames for the CLI WebSocket authentication device-flow.
/// Separate from ArenaFrame — only used on /ws/cli-auth connections.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CliAuthFrame {
    /// Server → CLI: challenge token. The CLI builds the browser auth URL
    /// itself from the server URL it already resolved (--server / env /
    /// stored profile) plus this token, rather than trusting a server-built
    /// URL — otherwise the server has to independently guess its own public
    /// domain, which drifts across deployments (e.g. preview environments).
    CliAuthChallenge { cli_token: String },
    /// Server → CLI: authentication succeeded; carries the opaque PAT.
    CliAuthSuccess { token: String },
    /// Server → CLI: authentication failed or session expired.
    CliAuthError { code: String, message: String },
}

/// `reason` value for [`PlayerAgentFrame::SessionComplete`] acknowledging
/// that a single player has exhausted their tasks while the session keeps
/// running for other players. Old ololo binaries don't know this value and
/// exit as on any session end; newer clients distinguish it by reason.
pub const SESSION_COMPLETE_REASON_PLAYER_TASKS_COMPLETED: &str = "player_tasks_completed";

/// Outcome of a probe as recorded by the server.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProbeOutcome {
    Pass,
    Error,
    NoResponse,
}

/// Server → ololo player-agent frames.
/// Sent over `/ws/player/:join_code` after successful auth.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum PlayerAgentFrame {
    /// Push a new probe to the player agent.
    TestPush {
        probe_id: uuid::Uuid,
        /// Shell command with all fixture variables already substituted.
        rendered_command: String,
        /// Deadline in seconds from server dispatch time.
        deadline_secs: i64,
        /// Task ID the probe belongs to. Used by the player agent UI to
        /// group probes by task. `#[serde(default)]` for back-compat with
        /// older game-servers that omit it.
        #[serde(default)]
        task_id: Option<uuid::Uuid>,
        /// 1-based ordinal of the task within the project. Used by the
        /// player agent UI for ordering. `#[serde(default)]` for back-compat.
        #[serde(default)]
        task_ordinal: i32,
        /// Human-readable task title for the player agent UI.
        #[serde(default)]
        task_title: String,
        /// Human-readable task description / instructions.
        #[serde(default)]
        task_description: String,
        /// 1-based position of this probe's test (probe type) within the
        /// task's ordinal-ordered test list. 0 when unknown (old servers).
        #[serde(default)]
        test_ordinal: i32,
        /// Total number of tests (probe types) in the task. 0 when unknown.
        #[serde(default)]
        test_total: i32,
        /// Human label of this test — the `## ` heading from the task's
        /// test definition (see `test_display_label`). Empty when the
        /// author gave none or the server predates the field.
        #[serde(default)]
        test_label: String,
        /// The author's explanation of what this check verifies — the probe
        /// section's prose from the task markdown (for judge-registered
        /// probes, the judge's instruction). Empty when absent.
        #[serde(default)]
        test_description: String,
        /// Expected answer string (already pre-evaluated when possible).
        /// When `Some`, the agent compares trimmed stdout to this string
        /// to grade the probe locally (mirroring the server's grading).
        /// `None` means grading is template-only (see `answer_template`)
        /// or deferred to the server.
        #[serde(default)]
        expected_answer: Option<String>,
        /// minijinja predicate/value expression used when the expected
        /// answer cannot be pre-evaluated (e.g. `result != ""`,
        /// `"PASS" in result`). Empty string means: use `expected_answer`
        /// equality, or defer to the server if both are empty.
        #[serde(default)]
        answer_template: String,
        /// Which evaluator the server uses for `answer_template`.
        /// `minijinja` — minijinja expression (result-only predicates work
        ///   locally; templates referencing fixture vars defer to the server).
        /// `javascript` — boa-engine JS expression (ololo evaluates
        ///   result-only JS locally via `eval_js_validation_outcome`).
        /// Defaults to `minijinja` for back-compat with old game-servers.
        #[serde(default)]
        validation_kind: ValidationKind,
    },
    /// Session has ended — or, for one specific reason, this player is done
    /// while the session keeps running. `reason` values:
    /// - `"all_tasks_completed"`, `"time_expired"`, `"cancelled"`,
    ///   `"finished"` — the session is over; agent should disconnect.
    /// - [`SESSION_COMPLETE_REASON_PLAYER_TASKS_COMPLETED`] — per-player
    ///   acknowledgment: THIS player exhausted their tasks but the session
    ///   continues until every player finishes. Reuses this variant (instead
    ///   of a new one) because fielded ololo binaries reject unknown variants
    ///   of this internally-tagged enum; old clients treat it as a normal
    ///   session end and exit (graceful degradation).
    ///
    /// `#[serde(default)]` on `reason` for back-compat with older
    /// game-servers that omit it.
    SessionComplete {
        session_id: uuid::Uuid,
        #[serde(default)]
        reason: Option<String>,
    },
    /// Session was paused by an operator. Carries the frozen remaining seconds
    /// so the agent TUI can show a paused countdown. Unambiguous alternative
    /// to the conflated `RunningCountdown { paused: true }`.
    SessionPaused {
        session_id: uuid::Uuid,
        seconds_remaining: u32,
    },
    /// Session was cancelled by an operator. Agent should show cancelled and
    /// disconnect. Unambiguous alternative to `SessionComplete { reason }`.
    SessionCancelled {
        session_id: uuid::Uuid,
        /// `"user"` | `"idle_timeout"`; None from pre-upgrade senders.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cancel_reason: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cancelled_by: Option<String>,
    },
    /// Server-graded probe outcome. Sent after the server evaluates the
    /// player's `TestResult` so the player agent can display the same
    /// pass/fail/no_response the server recorded — without having to
    /// re-grade locally (which is impossible for templates referencing
    /// fixture vars the server never sent in `TestPush`).
    ProbeGraded {
        probe_id: uuid::Uuid,
        /// The outcome the server recorded for this probe.
        outcome: ProbeOutcome,
        /// Score delta applied for this probe (signed: negative on
        /// fail/no_response).
        point_delta: i32,
        /// Expected answer the server compared against (when known).
        /// Mirrors `probes.expected_answer` for display.
        #[serde(default)]
        expected: Option<String>,
        /// Actual stdout the server recorded (trimmed). Mirrors
        /// `probes.output` for display.
        #[serde(default)]
        actual: Option<String>,
    },
    /// Server → player agent: a judge has scored the player's task attempt.
    JudgeScored(JudgeScoredPayload),
    /// Server → player agent: commit and push a snapshot of the working tree
    /// now. Open-ended tasks only (never sent for classic tasks, so fielded
    /// binaries that predate the variant never see it). On
    /// `todo_complete`/`deadline` the agent writes the final
    /// `feat(<task_id>): <title>` commit; on `todo_progress` a
    /// `wip(<task_id>)` checkpoint. The git push is the acknowledgment —
    /// there is no reply frame.
    SnapshotRequest {
        task_id: uuid::Uuid,
        #[serde(default)]
        task_title: String,
        /// See the `SNAPSHOT_REASON_*` constants.
        reason: String,
    },
}

/// `reason` values for [`PlayerAgentFrame::SnapshotRequest`].
pub const SNAPSHOT_REASON_TODO_PROGRESS: &str = "todo_progress";
pub const SNAPSHOT_REASON_TODO_COMPLETE: &str = "todo_complete";
pub const SNAPSHOT_REASON_DEADLINE: &str = "deadline";

/// Payload for the `JudgeScored` frame (both player-agent and observer
/// directions share the same shape).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JudgeScoredPayload {
    pub task_id: uuid::Uuid,
    pub judge_slug: String,
    pub judge_name: String,
    pub rating: f64,
    pub feedback: String,
    pub point_delta: i32,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// Which evaluator the server uses for an `answer_template`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ValidationKind {
    /// minijinja expression (default for back-compat).
    #[default]
    Minijinja,
    /// boa-engine JavaScript expression.
    Javascript,
}

/// Placeholder stdout an agent sends when the player declined the probe.
///
/// Agents older than the `TestResult.error` field signal a decline only this
/// way, so the grader still has to recognise the string itself.
pub const PROBE_DECLINED_STDOUT: &str = "[ololo] probe command declined by player";

/// ololo → server frames.
/// Sent by the player agent after executing a probe.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum PlayerAgentClientFrame {
    /// Result of executing a probe.
    TestResult {
        probe_id: uuid::Uuid,
        /// Raw stdout (trimmed of trailing whitespace by the agent).
        stdout: String,
        /// Process exit code, or `None` if the process could not be started.
        exit_code: Option<i32>,
        /// Wall-clock execution duration in milliseconds.
        duration_ms: u64,
        /// Why the agent produced no real measurement: the player declined the
        /// command, or it could not be executed. `None` on every probe that
        /// actually ran, whatever it printed or exited with.
        ///
        /// The grader must never validate `stdout` when this is set — the
        /// field carries no output, only an explanation, and a placeholder fed
        /// to a predicate like `result.trim().length > 0` scores a pass.
        /// Omitted on the wire when absent, so an older server (which rejects
        /// unknown fields) still parses the frames of a probe that ran.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error: Option<String>,
    },
    /// The player edited a memory source file (`AGENTS.md` / `README.md`)
    /// and the agent has pushed a commit carrying it.
    ///
    /// Memory is otherwise only re-extracted after a task-completion commit,
    /// so a player who rewrote AGENTS.md mid-task kept being probed with
    /// stale values. There is no post-receive hook on the git endpoint, so
    /// the agent has to say when a push landed.
    MemorySourcesPushed,
    /// The player declared an open-ended task done by writing its completion
    /// flag file (`.ololo/<name>-done.md`); the agent has committed the whole
    /// working tree and pushed it.
    ///
    /// Without this the flag sits until the completion probe's next interval
    /// tick. On receipt the server dispatches the completion probe
    /// immediately, so a passing contract hands the build to the judges
    /// within seconds of the flag appearing. Old servers fail to parse the
    /// variant and drop the frame — interval polling still completes the
    /// task.
    CompletionFlagPushed {
        /// Worktree-relative path of the flag file, for logs/telemetry.
        #[serde(default)]
        path: String,
    },
}
