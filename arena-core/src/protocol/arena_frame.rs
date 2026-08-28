//! Server/observer/dashboard bidirectional frames.

use serde::{Deserialize, Serialize};

use crate::protocol::ids::{PlayerId, TaskId};
use crate::protocol::session::{
    AdaptationRetryingPayload, AdminAdaptedTaskView, AdminTaskEntry, LeaderboardEntry, MemberInfo,
    PlayerMetadataEntry, PlayerRunStatus, PlayerSummary, ProjectSessionSummary,
    SessionSnapshotPayload,
};
use crate::session_status::SessionStatus;

/// Frames sent from a PAT-authenticated client (observer / ololo CLI) to the
/// server after the WebSocket upgrade. The first frame MUST be
/// `PlayerHandshake`.
///
/// `deny_unknown_fields` is enforced at the enum level via the internally-tagged
/// representation: any unknown `type` value or unexpected field causes an error.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ClientMessage {
    /// First frame from a PAT client after WS upgrade.
    /// `kind` MUST NOT be present — all players are PAT-authenticated agents
    /// by definition.
    PlayerHandshake {
        join_code: String,
        display_name: String,
        fingerprint: Option<String>,
        metadata_json: Option<String>,
    },
    /// Probe result from the observer.
    TestResult {
        task_id: TaskId,
        attempt: u32,
        pass: bool,
        duration_ms: u64,
        exit_code: i32,
        stdout_tail: String,
    },
    /// Liveness ping.
    Heartbeat,
}

/// Single bidirectional frame schema for server <-> observer (and dashboard
/// fan-out). Direction is implicit: each side only emits its half of the
/// variants.
///
/// `deny_unknown_fields` is applied per-struct body rather than at the enum
/// level: serde rejects the enum-level form on internally-tagged enums.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ArenaFrame {
    /// Server -> observer: a probe was dispatched to a player.
    TestPush {
        task_id: TaskId,
        player_id: PlayerId,
        attempt: u32,
        rendered_command: String,
        deadline_secs: i64,
    },
    /// Server -> observer: terminal frame for a session.
    SessionComplete { reason: String, version: u64 },
    /// Server -> dashboard/player page: every judge run the finished session
    /// owed has settled and awards were decided — the standings are final.
    /// Arrives after `SessionComplete`; pages wait for it before celebrating.
    SessionSettled {
        session_id: uuid::Uuid,
        version: u64,
    },
    /// Server -> dashboard: leaderboard snapshot.
    LeaderboardUpdate {
        entries: Vec<LeaderboardEntry>,
        version: u64,
    },
    /// Server -> dashboard: per-player progress tick.
    PlayerProgressUpdate {
        player_id: PlayerId,
        current_task_id: Option<TaskId>,
        attempt: u32,
        status: PlayerRunStatus,
    },
    /// Observer -> server: probe outcome.
    TestResult {
        task_id: TaskId,
        attempt: u32,
        pass: bool,
        duration_ms: u64,
        exit_code: i32,
        stdout_tail: String,
    },
    /// Bidirectional liveness ping.
    Heartbeat,
    /// Server -> dashboard: countdown before a session begins.
    LobbyCountdown {
        session_id: uuid::Uuid,
        seconds_remaining: u32,
        version: u64,
    },
    /// Server -> dashboard: server-driven countdown for the running/paused phase.
    /// `paused` is true while the session is frozen so clients can distinguish a
    /// paused game from an actively running one (both reuse this frame).
    RunningCountdown {
        session_id: uuid::Uuid,
        seconds_remaining: u32,
        version: u64,
        #[serde(default)]
        paused: bool,
    },
    /// Server -> dashboard/agent: session was paused by an operator. Carries
    /// the frozen remaining seconds. Unambiguous signal distinct from
    /// `RunningCountdown`.
    SessionPaused {
        session_id: uuid::Uuid,
        seconds_remaining: u32,
        version: u64,
    },
    /// Server -> dashboard/agent: session was cancelled by an operator.
    SessionCancelled {
        session_id: uuid::Uuid,
        version: u64,
        /// `"user"` | `"idle_timeout"`; None from pre-upgrade senders.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cancel_reason: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cancelled_by: Option<String>,
    },
    /// Server -> dashboard: session has started.
    SessionStarted {
        session_id: uuid::Uuid,
        version: u64,
        /// Total number of tasks in the project. `#[serde(default)]` for
        /// back-compat with older game-servers that omit it.
        #[serde(default)]
        total_tasks: Option<u32>,
    },
    /// Server -> dashboard: snapshot of all current participants sent on WS connect.
    MemberList { members: Vec<MemberInfo> },
    /// Server -> dashboard: a new participant has joined the session.
    /// Also re-broadcast when a participant's metadata is updated (e.g.
    /// ololo PATCHes its AI agent) so lobby observers see the agent label.
    MemberJoined {
        user_id: String,
        /// Player row id for this participant, when known (see `MemberInfo`).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        player_id: Option<uuid::Uuid>,
        display_name: String,
        joined_at: String,
        avatar_url: Option<String>,
        fingerprint: Option<String>,
        username: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        agent_display_name: Option<String>,
        version: u64,
    },
    /// Server -> dashboard: full session state snapshot sent on WebSocket connect.
    SessionSnapshot(SessionSnapshotPayload),
    /// Server -> observer: adaptation permanently failed; the agent cannot
    /// proceed with the current task. Terminal frame for this agent's run.
    Error { message: String },
    /// Server -> dashboard: a participant has disconnected from the session.
    PlayerDisconnected { player_id: uuid::Uuid, version: u64 },
    /// Server -> project dashboard: a session for this project was created or
    /// its status changed (lobby → running → finished | cancelled).
    ProjectSessionUpdate {
        session_id: uuid::Uuid,
        name: String,
        status: SessionStatus,
        project_id: uuid::Uuid,
        join_code: Option<String>,
        created_at: chrono::DateTime<chrono::Utc>,
        /// Players currently in the session. Drives the live cards on the
        /// project page, so it is re-sent on join/leave as well as on status
        /// changes. `#[serde(default)]` keeps frames from a server that
        /// predates the field decodable during a rolling deploy.
        #[serde(default)]
        player_count: u32,
        /// Set when `status` is Cancelled: `"user"` | `"idle_timeout"`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cancel_reason: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cancelled_by: Option<String>,
    },
    /// Server -> admin dashboard: full snapshot of all players and their adapted
    /// tasks for a session. Sent immediately on WS connect.
    AdminAdaptedTasksSnapshot {
        session_id: uuid::Uuid,
        players: Vec<AdminAdaptedTaskView>,
    },
    /// Server -> admin dashboard: a single player's adapted task entry has
    /// changed state. Client merges this into its local snapshot.
    AdminAdaptedTaskUpdated {
        session_id: uuid::Uuid,
        player_id: uuid::Uuid,
        entry: AdminTaskEntry,
    },
    /// Server -> PAT client: player successfully joined the session.
    PlayerJoined {
        player_id: uuid::Uuid,
        display_name: String,
    },
    /// Server -> PAT client: existing player reconnected via matching fingerprint.
    PlayerResumed { player_id: uuid::Uuid },
    /// Server -> browser client: snapshot of all players tied to the connected
    /// user's PATs for this session. Sent immediately after WS connect for
    /// browser clients.
    UserPlayersSnapshot { players: Vec<PlayerSummary> },
    /// Server -> observer: adaptation is being retried for a player-task pair.
    AdaptationRetrying(AdaptationRetryingPayload),
    /// Server -> dashboard: a player has started working on a task.
    TaskStarted {
        player_id: uuid::Uuid,
        player_display_name: String,
        task_id: uuid::Uuid,
        task_ordinal: i32,
        task_title: String,
        timestamp: chrono::DateTime<chrono::Utc>,
        version: u64,
    },
    /// Server -> dashboard: a player's task attempt has been scored (probe pass
    /// or judge evaluation). `judge_name` is the AI judge's display name when
    /// the score came from a judge run; empty string for probe-pass scoring.
    TaskScored {
        player_id: uuid::Uuid,
        player_display_name: String,
        task_id: uuid::Uuid,
        task_ordinal: i32,
        task_title: String,
        point_delta: i32,
        judge_name: String,
        /// Criteria-judge verdicts: the per-criterion sheet summary.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        detail: Option<serde_json::Value>,
        timestamp: chrono::DateTime<chrono::Utc>,
        version: u64,
    },
    /// Server -> dashboard: a participant's artifact (screenshot, screencast,
    /// report, ...) landed for an interactive probe. The activity feed
    /// renders images/videos inline via the session artifact endpoint and
    /// notes other content types.
    ArtifactReceived {
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
        /// Every delivered file (up to
        /// [`crate::evaluation::MAX_ARTIFACT_FILES`]); the singular fields
        /// above describe `files[0]` for compatibility.
        #[serde(default)]
        files: Vec<super::ArtifactFile>,
        timestamp: chrono::DateTime<chrono::Utc>,
        version: u64,
    },
    ProjectSessionsSnapshot {
        project_id: uuid::Uuid,
        sessions: Vec<ProjectSessionSummary>,
    },
    AdminPlayerMetadataSnapshot {
        session_id: uuid::Uuid,
        metadata: Vec<PlayerMetadataEntry>,
    },
    AdminPlayerMetadataUpdated {
        session_id: uuid::Uuid,
        entry: PlayerMetadataEntry,
    },
}
