//! Wire types mirrored from `server/src/protocol.rs` (`PlayerAgentFrame` /
//! `PlayerAgentClientFrame`) — kept local to avoid a shared crate dependency.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PlayerAgentFrame {
    TestPush {
        probe_id: Uuid,
        rendered_command: String,
        deadline_secs: i64,
        #[serde(default)]
        task_id: Option<Uuid>,
        #[serde(default)]
        task_ordinal: i32,
        #[serde(default)]
        task_title: String,
        #[serde(default)]
        task_description: String,
        #[serde(default)]
        test_ordinal: i32,
        #[serde(default)]
        test_total: i32,
        #[serde(default)]
        test_label: String,
        #[serde(default)]
        test_description: String,
        #[serde(default)]
        expected_answer: Option<String>,
        #[serde(default)]
        answer_template: String,
        #[serde(default)]
        validation_kind: arena_core::protocol::ValidationKind,
    },
    SessionComplete {
        #[allow(dead_code)]
        session_id: String,
        #[serde(default)]
        reason: Option<String>,
    },
    SessionPaused {
        #[allow(dead_code)]
        session_id: String,
        seconds_remaining: u32,
    },
    SessionCancelled {
        #[allow(dead_code)]
        session_id: String,
    },
    /// Server-graded probe outcome — authoritative pass/fail/no_response.
    ProbeGraded {
        probe_id: Uuid,
        outcome: arena_core::protocol::ProbeOutcome,
        point_delta: i32,
        #[serde(default)]
        expected: Option<String>,
        #[serde(default)]
        actual: Option<String>,
    },
    /// Lobby countdown — forwarded by the game-server during the pre-start phase.
    LobbyCountdown {
        #[allow(dead_code)]
        session_id: String,
        seconds_remaining: u32,
        #[allow(dead_code)]
        version: u64,
    },
    /// Session has transitioned from lobby to running.
    SessionStarted {
        #[allow(dead_code)]
        session_id: String,
        #[allow(dead_code)]
        version: u64,
        #[serde(default)]
        total_tasks: Option<u32>,
    },
    /// Running-phase countdown — forwarded by the game-server each second.
    /// `paused` is true while the session is frozen (server reuses this frame
    /// for both running and paused; see `RunningCountdown` on the wire).
    RunningCountdown {
        #[allow(dead_code)]
        session_id: String,
        seconds_remaining: u32,
        #[allow(dead_code)]
        version: u64,
        #[serde(default)]
        paused: bool,
    },
    /// Leaderboard snapshot — forwarded by the game-server on every score change.
    LeaderboardUpdate {
        entries: Vec<crate::tui::event::LeaderboardEntry>,
        #[serde(default)]
        #[allow(dead_code)]
        version: u64,
    },
    /// Per-player progress tick — forwarded by the game-server.
    PlayerProgressUpdate {
        player_id: Uuid,
        #[serde(default)]
        #[allow(dead_code)]
        current_task_id: Option<Uuid>,
        #[serde(default)]
        attempt: u32,
        status: crate::tui::event::PlayerRunStatus,
    },
    /// Open-ended tasks: the server asks for a snapshot commit+push now.
    /// The git push is the acknowledgment — no reply frame.
    SnapshotRequest {
        task_id: Uuid,
        #[serde(default)]
        task_title: String,
        reason: String,
    },
    /// Session broadcast: a player began working on a task. The CLI narrates
    /// only its own player's starts.
    TaskStarted {
        player_id: Uuid,
        #[serde(default)]
        task_ordinal: i32,
        #[serde(default)]
        task_title: String,
    },
    /// A judge delivered its verdict on this player's task.
    JudgeScored {
        #[serde(default)]
        judge_name: String,
        point_delta: i32,
        #[serde(default)]
        feedback: String,
    },
}

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PlayerAgentClientFrame {
    TestResult {
        probe_id: Uuid,
        stdout: String,
        exit_code: Option<i32>,
        duration_ms: u64,
        /// Set when the probe produced no measurement (declined, or the
        /// command could not be executed) so the server fails it instead of
        /// validating the placeholder in `stdout`. Skipped when absent: an
        /// older server rejects unknown fields, and a probe that ran must keep
        /// working against one.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error: Option<String>,
    },
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResolveResponse {
    #[allow(dead_code)]
    pub game_server_url: String,
    #[allow(dead_code)]
    pub session_id: String,
    pub player_id: String,
}

/// Resolution outcome for `resolve_session`.
///
/// - `Ok((url, Some(player_id)))` on success — the game server WS base URL and viewer player_id
/// - `Ok((url, None))` on 404 fallback (direct connection, no resolve payload)
/// - `Err(Retry(String))` on 503 (no game server assigned yet) — caller should retry resolve
/// - `Err(Fatal(String))` on 401/403/410 or parse error — caller should abort
pub enum ResolveError {
    Retry(String),
    Fatal(String),
}
