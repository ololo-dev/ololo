//! ZMQ fan-out events.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ZmqEvent {
    SessionTimer {
        join_code: String,
        phase: String,
        seconds_remaining: u32,
        version: u64,
    },
    SessionStatus {
        join_code: String,
        status: String,
        version: u64,
        /// Set when `status` is "cancelled": `"user"` | `"idle_timeout"`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cancel_reason: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cancelled_by: Option<String>,
    },
    PlayerJoin {
        join_code: String,
        player_id: uuid::Uuid,
        display_name: String,
        user_id: Option<String>,
        joined_at: String,
        avatar_url: Option<String>,
        fingerprint: Option<String>,
        username: Option<String>,
        version: u64,
    },
    PlayerLeave {
        join_code: String,
        player_id: uuid::Uuid,
        version: u64,
    },
    /// game-server -> server: the player's ololo agent socket connected or
    /// disconnected. Bridged into `PlayerFrame::AgentPresence` for the player
    /// page (the web "Live" indicator) — the session status alone cannot tell
    /// a live client from a dropped one.
    AgentPresence {
        join_code: String,
        player_id: uuid::Uuid,
        connected: bool,
        timestamp: chrono::DateTime<chrono::Utc>,
    },
    ScoreChange {
        join_code: String,
        player_id: uuid::Uuid,
        delta: i64,
        total: i64,
        version: u64,
    },
    /// game-server -> server: a player's scheduler advanced to a new task.
    /// Bridged into `ArenaFrame::TaskStarted` by the server's ZMQ subscriber.
    TaskStarted {
        join_code: String,
        player_id: uuid::Uuid,
        player_display_name: String,
        task_id: uuid::Uuid,
        task_ordinal: i32,
        task_title: String,
        timestamp: chrono::DateTime<chrono::Utc>,
        version: u64,
    },
    /// game-server -> server: a judge run started evaluating a player's task.
    /// Bridged into `PlayerFrame::JudgeStarted` (player page) by the server's
    /// ZMQ subscriber.
    JudgeStarted {
        join_code: String,
        player_id: uuid::Uuid,
        task_id: uuid::Uuid,
        judge_slug: String,
        judge_name: String,
        timestamp: chrono::DateTime<chrono::Utc>,
    },
    /// game-server -> server: a judge run failed before producing a verdict.
    /// Bridged into `PlayerFrame::JudgeFailed` (player page) by the server's
    /// ZMQ subscriber.
    JudgeFailed {
        join_code: String,
        player_id: uuid::Uuid,
        task_id: uuid::Uuid,
        judge_slug: String,
        judge_name: String,
        error: String,
        timestamp: chrono::DateTime<chrono::Utc>,
    },
    /// game-server -> server: a judge run completed and scored a player's task.
    /// Bridged into `ArenaFrame::TaskScored` (observers) and
    /// `PlayerFrame::JudgeScored` (player page) by the server's ZMQ subscriber.
    JudgeScored {
        join_code: String,
        player_id: uuid::Uuid,
        player_display_name: String,
        task_id: uuid::Uuid,
        task_ordinal: i32,
        task_title: String,
        point_delta: i32,
        judge_slug: String,
        judge_name: String,
        rating: f64,
        feedback: String,
        duration_ms: Option<i64>,
        /// Criteria-judge verdicts: the per-criterion sheet summary.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        detail: Option<serde_json::Value>,
        timestamp: chrono::DateTime<chrono::Utc>,
        version: u64,
    },
    /// game-server -> server: Arena Points were awarded at session finish
    /// (global leaderboard, `session_awards` rows). Not bridged to a browser
    /// frame — the leaderboard page reads `GET /api/leaderboard`.
    /// A server-side or interactive probe resolved (open-ended tasks).
    /// One event per run — server probes dispatch and resolve in one step.
    ProbeFinished {
        join_code: String,
        player_id: uuid::Uuid,
        task_id: uuid::Uuid,
        probe_id: uuid::Uuid,
        /// `deterministic` | `analysis` | `llm` | `interactive`.
        mode: String,
        /// `system` | `judge`.
        initiator: String,
        outcome: String,
        timestamp: chrono::DateTime<chrono::Utc>,
    },
    /// A judge registered a probe mid-run.
    ProbeRegistered {
        join_code: String,
        player_id: uuid::Uuid,
        task_id: uuid::Uuid,
        probe_id: uuid::Uuid,
        judge_slug: String,
        mode: String,
        timestamp: chrono::DateTime<chrono::Utc>,
    },
    /// A participant's artifact landed in the pushed repo for an interactive
    /// probe. Bridged to the session activity feed, which renders images and
    /// videos inline and notes other content types.
    ArtifactReceived {
        join_code: String,
        player_id: uuid::Uuid,
        player_display_name: String,
        task_id: uuid::Uuid,
        task_ordinal: i32,
        task_title: String,
        probe_id: uuid::Uuid,
        /// Repo-relative path of the first delivered file.
        path: String,
        size: u64,
        content_type: String,
        within_cap: bool,
        /// Every delivered file (a request carries up to
        /// [`crate::evaluation::MAX_ARTIFACT_FILES`]); the singular fields
        /// above describe `files[0]` for compatibility.
        #[serde(default)]
        files: Vec<super::ArtifactFile>,
        timestamp: chrono::DateTime<chrono::Utc>,
        version: u64,
    },
    /// An interactive probe is waiting on the participant's artifact.
    ArtifactAwaited {
        join_code: String,
        player_id: uuid::Uuid,
        task_id: uuid::Uuid,
        probe_id: uuid::Uuid,
        instruction: String,
        deadline_at: chrono::DateTime<chrono::Utc>,
        timestamp: chrono::DateTime<chrono::Utc>,
    },
    /// Every judge attached to the task reached a terminal verdict for this
    /// player — the task's evaluation is final.
    EvaluationReady {
        join_code: String,
        player_id: uuid::Uuid,
        task_id: uuid::Uuid,
        timestamp: chrono::DateTime<chrono::Utc>,
    },
    /// game-server -> server: the session report for this player has been
    /// written. Carries no text: the report is a document the page reads from
    /// the snapshot, and announcing its body would post the raw JSON into the
    /// chat as one more verdict. Bridged to `PlayerFrame::SessionReportReady`,
    /// which is the player page's cue to re-fetch — it is written after the
    /// session finished, when nothing else is still polling.
    SessionReportReady {
        join_code: String,
        player_id: uuid::Uuid,
        timestamp: chrono::DateTime<chrono::Utc>,
    },
    SessionAwarded {
        join_code: String,
        session_id: uuid::Uuid,
        users_awarded: u32,
        timestamp: chrono::DateTime<chrono::Utc>,
    },
    /// game-server -> server: every judge run the finished session owed has
    /// settled (scored, failed, or the wait deadline passed) and awards have
    /// been decided — the standings are final. Published exactly once per
    /// finished session, after `SessionAwarded` when awards were granted.
    /// Bridged to `ArenaFrame::SessionSettled` for dashboard/player pages so
    /// they can celebrate without polling the report.
    SessionSettled {
        join_code: String,
        session_id: uuid::Uuid,
        timestamp: chrono::DateTime<chrono::Utc>,
    },
}

impl ZmqEvent {
    pub fn join_code(&self) -> &str {
        match self {
            ZmqEvent::SessionTimer { join_code, .. } => join_code,
            ZmqEvent::SessionStatus { join_code, .. } => join_code,
            ZmqEvent::PlayerJoin { join_code, .. } => join_code,
            ZmqEvent::PlayerLeave { join_code, .. } => join_code,
            ZmqEvent::ScoreChange { join_code, .. } => join_code,
            ZmqEvent::TaskStarted { join_code, .. } => join_code,
            ZmqEvent::JudgeStarted { join_code, .. } => join_code,
            ZmqEvent::JudgeFailed { join_code, .. } => join_code,
            ZmqEvent::JudgeScored { join_code, .. } => join_code,
            ZmqEvent::AgentPresence { join_code, .. } => join_code,
            ZmqEvent::ProbeFinished { join_code, .. } => join_code,
            ZmqEvent::ProbeRegistered { join_code, .. } => join_code,
            ZmqEvent::ArtifactReceived { join_code, .. } => join_code,
            ZmqEvent::ArtifactAwaited { join_code, .. } => join_code,
            ZmqEvent::EvaluationReady { join_code, .. } => join_code,
            ZmqEvent::SessionReportReady { join_code, .. } => join_code,
            ZmqEvent::SessionAwarded { join_code, .. } => join_code,
            ZmqEvent::SessionSettled { join_code, .. } => join_code,
        }
    }
}
