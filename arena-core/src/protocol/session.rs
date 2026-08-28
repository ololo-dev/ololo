//! Dashboard / session / admin snapshot payloads.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::protocol::ids::PlayerId;
use crate::protocol::player::PlayerCompletionStatus;
use crate::session_status::SessionStatus;

/// Lifecycle state of an adapted task as seen by the admin dashboard.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AdaptedTaskStatus {
    Pending,
    Ready,
    Failed,
}

/// One adapted task row in an admin dashboard frame.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdminTaskEntry {
    pub task_id: uuid::Uuid,
    pub task_order: i32,
    pub title: String,
    pub adapted_content: String,
    pub status: AdaptedTaskStatus,
    pub adaptation_attempts: u32,
}

/// Per-player view in admin adapted-task frames.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdminAdaptedTaskView {
    pub player_id: uuid::Uuid,
    pub player_display_name: String,
    pub tasks: Vec<AdminTaskEntry>,
}

/// Lifecycle state surfaced on the dashboard fan-out channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlayerRunStatus {
    AwaitingResult,
    Backoff,
    Completed,
    Failed,
}

/// One row on the dashboard leaderboard.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LeaderboardEntry {
    pub player_id: PlayerId,
    pub display_name: String,
    /// AI coding agent the participant is running (e.g. "opencode", "claude").
    /// `None` for players that have not registered agent metadata.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_display_name: Option<String>,
    #[serde(default)]
    pub total_points: i64,
    pub tests_passed: u32,
    pub total_wall_ms: u64,
}

/// One sample in a session's score history timeseries.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScoreHistorySample {
    pub t: f64,
    pub scores: BTreeMap<PlayerId, i64>,
}

/// One participant in a session, reported on dashboard fan-out.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MemberInfo {
    pub user_id: String,
    /// Player row id for this participant in this session, when known.
    /// The leaderboard is keyed by player id — carrying it here lets the
    /// frontend merge participants with scored entries without guessing
    /// through user-id maps (which only exist for the viewer's own players).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub player_id: Option<uuid::Uuid>,
    pub display_name: String,
    pub joined_at: String,
    pub avatar_url: Option<String>,
    pub fingerprint: Option<String>,
    pub username: Option<String>,
    /// AI coding agent the participant is running, if reported via metadata.
    /// Populated lazily: `None` at join time, set when ololo PATCHes its
    /// environment metadata. Lobby observers receive an updated
    /// `MemberJoined` frame at that point.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_display_name: Option<String>,
    /// Derived completion state of this participant. `None` means "not
    /// computed" — only detail/snapshot paths compute it; list paths and
    /// pre-upgrade servers omit it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completion_status: Option<PlayerCompletionStatus>,
}

/// Full session state sent to a WebSocket client on connect.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionSnapshotPayload {
    pub session_id: uuid::Uuid,
    pub phase: SessionStatus,
    pub version: u64,
    pub participants: Vec<MemberInfo>,
    pub leaderboard: Vec<LeaderboardEntry>,
    pub started_at: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeline: Option<Vec<SessionTimelineEvent>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub activity: Option<Vec<SessionActivityEvent>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub score_history: Option<Vec<ScoreHistorySample>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionTimelineEvent {
    pub kind: TimelineEventKind,
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub player_id: Option<uuid::Uuid>,
    pub task_id: Option<uuid::Uuid>,
    pub summary: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TimelineEventKind {
    SessionStarted,
    TaskStarted,
    TaskScored,
    PlayerJoined,
    SessionComplete,
    SessionCancelled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionActivityEvent {
    pub event_kind: String,
    pub player_id: uuid::Uuid,
    pub player_display_name: String,
    pub task_id: uuid::Uuid,
    pub task_ordinal: i32,
    pub task_title: String,
    pub judge_name: Option<String>,
    pub point_delta: Option<i32>,
    /// Criteria-judge verdicts: `{"overall": f64, "criteria": [{key, score}]}`.
    /// Omitted when absent, so pre-upgrade readers never see the field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<serde_json::Value>,
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub version: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectSessionSummary {
    pub id: uuid::Uuid,
    pub name: String,
    pub status: String,
    pub owner_id: Option<uuid::Uuid>,
    pub project_id: uuid::Uuid,
    pub join_code: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlayerMetadataEntry {
    pub player_id: uuid::Uuid,
    pub display_name: String,
    pub agent_display_name: Option<String>,
    pub fingerprint: Option<String>,
    pub ai_agents: Vec<String>,
    pub build_tools: Vec<String>,
    pub languages: Vec<String>,
    pub test_tools: Vec<String>,
    pub utility_tools: Vec<String>,
    pub probe_duration_ms: Option<i64>,
    pub platform: Option<String>,
    pub joined_at: chrono::DateTime<chrono::Utc>,
}

/// Summary of a player record, sent in dashboard snapshots.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlayerSummary {
    pub player_id: uuid::Uuid,
    /// The user who owns this player; `None` for anonymous/unlinked players.
    pub user_id: Option<uuid::Uuid>,
    pub display_name: String,
    pub fingerprint: Option<String>,
    pub joined_at: chrono::DateTime<chrono::Utc>,
    pub reconnected_at: Option<chrono::DateTime<chrono::Utc>>,
    pub revoked_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// Payload for the `AdaptationRetrying` observer frame.
/// Broadcast when the LLM adaptation loop retries after a failure.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdaptationRetryingPayload {
    pub session_id: uuid::Uuid,
    pub task_id: uuid::Uuid,
    pub player_id: uuid::Uuid,
    pub attempt: u32,
}
