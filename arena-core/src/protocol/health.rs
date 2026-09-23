//! Code-health checkpoints on the wire.
//!
//! Every probe the client answers also becomes a checkpoint: the client
//! commits the tree, scores it with `ololo-health` and reports; the server
//! re-scores the pushed commit and its number is the one that counts.
//! The shapes here are shared by the ololo → game-server frame
//! (`PlayerAgentClientFrame::HealthReport`), the game-server → server event
//! (`ZmqEvent::HealthUpdated`), the browser frames and both snapshots.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

pub use ololo_health::{HealthResult, Level, Metrics, Thresholds};

use super::PlayerId;

/// What the server hands the client with a probe when health is tracked.
/// The scan configuration itself is not negotiable — it is the shared
/// crate's default on both sides — only the budget is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HealthProbeConfig {
    /// Wall-clock budget for the client's analysis, in seconds.
    pub timeout_secs: u32,
}

/// How one side's analysis attempt ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HealthReportStatus {
    Ok,
    Failed,
    Timeout,
    /// The client dropped this analysis for a newer probe's (one runs at a
    /// time, one waits) — the report still says so, never silently.
    Skipped,
}

/// Where the push of the reported commit stood when the report was sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PushState {
    Pushed,
    /// Still queued or retrying; the server waits for the commit to land.
    Pending,
    Failed,
    /// The remote refused a non-fast-forward push: the client's history and
    /// the server's diverged (a second client, a re-initialised snapshot).
    RejectedNonFastForward,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PushStatus {
    pub state: PushState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// The last commit the client knows to have reached the server.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pushed_commit: Option<String>,
}

/// ololo → game-server: the client's analysis of the tree it committed for
/// a probe. Sent as its own frame, never inside the probe answer, so it
/// can never delay the answer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HealthReportPayload {
    pub probe_id: uuid::Uuid,
    /// The probe's 1-based position among the player's probes (from
    /// `TestPush.probe_seq`; 0 when the server did not number it).
    #[serde(default)]
    pub probe_seq: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<uuid::Uuid>,
    /// Redundant with the socket's identity — carried so the report is
    /// self-describing in logs and stores.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<uuid::Uuid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub player_id: Option<uuid::Uuid>,
    /// The `probe(<task>)` commit the analysis scored; `None` when the
    /// commit itself failed (the error says why).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commit: Option<String>,
    pub status: HealthReportStatus,
    /// The analysis, when `status` is `ok`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<HealthResult>,
    /// Why there is no result.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// The whole attempt on the client (materialize + scan), in ms.
    pub duration_ms: u64,
    /// The client's `ololo_health::JSCPD_CORE_VERSION` — also inside
    /// `result`, but wanted on failures too.
    pub jscpd_version: String,
    #[serde(default)]
    pub health_schema: u32,
    pub push: PushStatus,
}

/// Which commit a checkpoint scores.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HealthCheckpointKind {
    /// The tree at a probe dispatch (`probe(<task>)` commit).
    Probe,
    /// The task's final tree (`feat(<task>)` commit) — the one the bonus is
    /// paid on.
    TaskFinal,
    /// The tree the session started from (the root of the player's line).
    /// Scored for personal projects, where the code predates the session
    /// and each task's health is measured against where it began.
    Baseline,
}

/// Where the server's own verification of a checkpoint stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HealthCheckStatus {
    Pending,
    Ok,
    Failed,
    Timeout,
    /// The reported commit never reached the server's repository.
    CommitMissing,
    /// The server will not verify: the client's history was rewritten, so
    /// the commit is not on the served line.
    Unverified,
}

/// What a reader should know before trusting the numbers.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HealthFlags {
    /// Client and server scores differ beyond the tolerance.
    #[serde(default)]
    pub score_mismatch: bool,
    /// The client ran another jscpd than the server.
    #[serde(default)]
    pub version_mismatch: bool,
    /// The task the git history attributes the commit to is not the task
    /// the probe belonged to.
    #[serde(default)]
    pub task_mismatch: bool,
    /// The checkpoint arrived after its task closed; it never changes
    /// points.
    #[serde(default)]
    pub late: bool,
    /// The client reported a rejected non-fast-forward push around this
    /// commit.
    #[serde(default)]
    pub history_rewritten: bool,
}

/// One side's numbers for a checkpoint.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HealthSide {
    pub status: HealthReportStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub score: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grade: Option<char>,
    pub level: Level,
    pub jscpd_version: String,
    pub duration_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metrics: Option<Metrics>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// A checkpoint as the browser sees it — one point on the chart.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HealthCheckpointView {
    pub id: uuid::Uuid,
    pub kind: HealthCheckpointKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub probe_id: Option<uuid::Uuid>,
    #[serde(default)]
    pub probe_seq: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<uuid::Uuid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_title: Option<String>,
    pub commit: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    /// Seconds since the session started — the chart's x. `None` when the
    /// session start is unknown.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub t: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client: Option<HealthSide>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server: Option<HealthSide>,
    pub server_status: HealthCheckStatus,
    #[serde(default)]
    pub flags: HealthFlags,
    /// The score to show: the server's once verified, the client's until
    /// then, none when neither scored.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub score: Option<f64>,
    /// The colour of `score`.
    pub level: Level,
}

/// A task's span in a player's history, derived from the commit messages
/// (`arena_core::snapshot_message::task_ranges`), for the chart's task
/// separators.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskRangeView {
    pub task_id: uuid::Uuid,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ordinal: Option<i32>,
    pub start_commit: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_commit: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_at: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_at: Option<chrono::DateTime<chrono::Utc>>,
    /// `start_at` / `end_at` as seconds since the session started.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_t: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_t: Option<f64>,
}

/// One player's health history.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlayerHealthPayload {
    /// Oldest first.
    #[serde(default)]
    pub checkpoints: Vec<HealthCheckpointView>,
    #[serde(default)]
    pub task_ranges: Vec<TaskRangeView>,
}

/// Every participant's health history, as carried in the session snapshot
/// and by the history endpoint.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionHealthPayload {
    pub thresholds: Thresholds,
    #[serde(default)]
    pub players: BTreeMap<PlayerId, PlayerHealthPayload>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_report_round_trips_and_omits_what_is_absent() {
        let report = HealthReportPayload {
            probe_id: uuid::Uuid::new_v4(),
            probe_seq: 3,
            task_id: Some(uuid::Uuid::new_v4()),
            session_id: None,
            player_id: None,
            commit: Some("abc123".into()),
            status: HealthReportStatus::Timeout,
            result: None,
            error: Some("analysis exceeded 30s".into()),
            duration_ms: 30_001,
            jscpd_version: ololo_health::JSCPD_CORE_VERSION.into(),
            health_schema: ololo_health::HEALTH_SCHEMA,
            push: PushStatus {
                state: PushState::Pending,
                error: None,
                pushed_commit: None,
            },
        };
        let json = serde_json::to_string(&report).unwrap();
        assert!(!json.contains("session_id"));
        assert!(!json.contains("\"result\""));
        assert!(json.contains("\"status\":\"timeout\""));
        assert!(json.contains("\"state\":\"pending\""));
        let back: HealthReportPayload = serde_json::from_str(&json).unwrap();
        assert_eq!(back, report);
    }

    #[test]
    fn a_checkpoint_view_round_trips() {
        let view = HealthCheckpointView {
            id: uuid::Uuid::new_v4(),
            kind: HealthCheckpointKind::TaskFinal,
            probe_id: None,
            probe_seq: 0,
            task_id: Some(uuid::Uuid::new_v4()),
            task_title: Some("Build the widget".into()),
            commit: "deadbeef".into(),
            created_at: chrono::Utc::now(),
            t: Some(12.5),
            client: None,
            server: Some(HealthSide {
                status: HealthReportStatus::Ok,
                score: Some(74.3),
                grade: Some('B'),
                level: Level::Green,
                jscpd_version: "0.1.16".into(),
                duration_ms: 180,
                metrics: Some(Metrics::default()),
                error: None,
            }),
            server_status: HealthCheckStatus::Ok,
            flags: HealthFlags {
                late: true,
                ..Default::default()
            },
            score: Some(74.3),
            level: Level::Green,
        };
        let json = serde_json::to_string(&view).unwrap();
        assert!(json.contains("\"kind\":\"task_final\""));
        assert!(json.contains("\"grade\":\"B\""));
        let back: HealthCheckpointView = serde_json::from_str(&json).unwrap();
        assert_eq!(back, view);
    }

    #[test]
    fn a_pre_upgrade_reader_style_payload_parses_with_defaults() {
        // Minimal JSON: every `#[serde(default)]` field absent.
        let json = r#"{"thresholds":{"green_min":70.0,"amber_min":55.0}}"#;
        let payload: SessionHealthPayload = serde_json::from_str(json).unwrap();
        assert!(payload.players.is_empty());
        assert_eq!(payload.thresholds, Thresholds::default());
    }

    #[test]
    fn unknown_fields_are_rejected() {
        let json = r#"{"timeout_secs":30,"extra":1}"#;
        assert!(serde_json::from_str::<HealthProbeConfig>(json).is_err());
    }
}
