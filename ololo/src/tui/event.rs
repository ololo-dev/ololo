#![allow(dead_code)]

//! Event types and sinks for the TUI.
//!
//! This module is the wire that connects the `player_ws` (and
//! `watch_session_ws_bus`) tasks to the TUI's render loop. Every
//! WebSocket frame, probe result, and lifecycle event becomes a
//! `TuiEvent` flowing through an `EventSink` into a bounded mpsc
//! channel that the render loop consumes.

use serde::Deserialize;
use std::sync::atomic::AtomicU64;
use std::sync::{Arc, Mutex};
use thiserror::Error;
use tokio::sync::mpsc;
use uuid::Uuid;

pub const BUS_CAPACITY: usize = 1024;

pub use arena_core::protocol::ValidationKind;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    Network,
    Render,
    Pty,
    Input,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogLevel {
    Step,
    Success,
    Warn,
    Error,
    Hint,
    Waiting,
}

#[derive(Debug, Clone)]
pub enum HeaderDelta {
    Lobby {
        seconds: u32,
    },
    Running {
        seconds: u32,
    },
    Paused {
        seconds: u32,
    },
    Started,
    /// Per-player "all your tasks are done" acknowledgment
    /// (`SessionComplete` with reason `player_tasks_completed`): the
    /// session keeps running for other players; the real end frame
    /// (`Complete` / `Cancelled`) arrives later.
    PlayerTasksDone,
    Complete {
        session_id: String,
    },
    Cancelled {
        session_id: String,
    },
    Error {
        message: String,
    },
}

/// A probe command awaiting the player's permission. The responder is a
/// one-shot channel back to the `player_ws` loop, Arc-wrapped so the event
/// stays `Clone`; whoever answers takes the sender out.
#[derive(Debug, Clone)]
pub struct PermissionPrompt {
    pub probe_id: Uuid,
    /// The rendered shell command the probe wants to run.
    pub command: String,
    /// Rule that "Always allow" persists to `.ololo/settings.json`.
    pub always_rule: String,
    /// Probe deadline — the prompt declines itself when it passes.
    pub deadline_secs: i64,
    pub responder: Arc<Mutex<Option<tokio::sync::oneshot::Sender<crate::permissions::Decision>>>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QuitReason {
    UserRequested,
    SessionComplete,
    AgentExited(i32),
    WsClosed,
    BusClosed,
    TtyLost,
    PickerFailed(String),
}

#[derive(Debug, Clone)]
pub struct ProbeInfo {
    pub probe_id: Uuid,
    pub rendered_command: String,
    pub deadline_secs: i64,
    /// Task ID the probe belongs to. `None` for back-compat with older
    /// game-servers that omit it from `TestPush`.
    pub task_id: Option<Uuid>,
    /// 1-based ordinal of the task within the project. 0 when unknown.
    pub task_ordinal: i32,
    pub task_title: String,
    pub task_description: String,
    /// 1-based position of the probe's test (probe type) within the task.
    /// 0 when unknown (old game-servers).
    pub test_ordinal: i32,
    /// Total tests (probe types) in the task. 0 when unknown.
    pub test_total: i32,
    /// Human label of the test (`## ` heading from the task definition).
    /// Empty when absent or the game-server predates the field.
    pub test_label: String,
    /// The author's explanation of what this check verifies. Empty when
    /// absent.
    pub test_description: String,
    /// Expected answer string pre-evaluated by the server, if any.
    /// Carried through to `ProbeResultInfo` for local grading.
    pub expected_answer: Option<String>,
    /// minijinja predicate expression (result-only) for local grading.
    /// Empty string means: use `expected_answer` equality, or defer.
    pub answer_template: String,
    /// Which evaluator the server uses for `answer_template`.
    pub validation_kind: ValidationKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct LeaderboardEntry {
    pub player_id: Uuid,
    pub display_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_display_name: Option<String>,
    #[serde(default)]
    pub total_points: i64,
    pub tests_passed: u32,
    pub total_wall_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlayerRunStatus {
    AwaitingResult,
    Backoff,
    Completed,
    Failed,
}

#[derive(Debug, Clone, Default)]
pub struct ProbeResultInfo {
    pub probe_id: Uuid,
    pub command: String,
    pub stdout: String,
    pub exit_code: Option<i32>,
    pub duration_ms: u64,
    pub error: Option<String>,
    /// Task ID the probe belongs to. `None` for back-compat with older
    /// game-servers that omit it from `TestPush`.
    pub task_id: Option<Uuid>,
    /// 1-based ordinal of the task within the project. 0 when unknown.
    pub task_ordinal: i32,
    pub task_title: String,
    pub task_description: String,
    /// 1-based position of the probe's test (probe type) within the task.
    /// 0 when unknown (old game-servers).
    pub test_ordinal: i32,
    /// Total tests (probe types) in the task. 0 when unknown.
    pub test_total: i32,
    /// Human label of the test (`## ` heading from the task definition).
    /// Empty when absent or the game-server predates the field.
    pub test_label: String,
    /// The author's explanation of what this check verifies. Empty when
    /// absent.
    pub test_description: String,
    pub deadline_secs: Option<i64>,
    /// Expected answer (pre-evaluated by the server) for local grading.
    /// `None` means: use `answer_template`, or defer to the server.
    pub expected_answer: Option<String>,
    /// minijinja predicate expression for local grading. Empty means
    /// no template; use `expected_answer` equality, or defer.
    pub answer_template: String,
    /// Which evaluator the server uses for `answer_template`.
    pub validation_kind: ValidationKind,
    /// Server-graded outcome, set when a `ProbeGraded` frame arrives
    /// after the local `ProbeResult`. `None` until the server grades.
    pub outcome: Option<arena_core::protocol::ProbeOutcome>,
    /// Score delta the server applied (signed). Mirrors the
    /// `ProbeGraded.point_delta` frame field.
    pub point_delta: Option<i32>,
    /// Resolved expected value from `ProbeGraded.expected` — for display
    /// only, kept separate from `expected_answer` (the local-grading
    /// template). `None` until graded or when the server withholds it.
    pub graded_expected: Option<String>,
}

impl ProbeResultInfo {
    /// Synthetic probe used to surface a teammate joining the session.
    pub fn member_joined(name: &str) -> Self {
        ProbeResultInfo {
            command: "member".to_string(),
            stdout: format!("{name} joined"),
            ..Default::default()
        }
    }

    /// Synthetic probe used to record a forwarded PTY input intent.
    pub fn pty_input(stdout: &str) -> Self {
        ProbeResultInfo {
            command: "input".to_string(),
            stdout: stdout.to_string(),
            ..Default::default()
        }
    }
}

#[derive(Debug, Clone)]
pub enum TuiEvent {
    Log {
        level: LogLevel,
        msg: String,
    },
    Header(HeaderDelta),
    ProbeArrived(ProbeInfo),
    ProbeResult(ProbeResultInfo),
    /// Server-graded outcome for a probe we previously dispatched.
    /// Updates the matching `ProbeResultInfo` stored in the TUI state.
    ProbeGraded {
        probe_id: Uuid,
        outcome: arena_core::protocol::ProbeOutcome,
        point_delta: i32,
        expected: Option<String>,
        actual: Option<String>,
        /// Seconds until the next probe dispatch, when the server says.
        next_probe_in_secs: Option<u32>,
    },
    LeaderboardUpdate {
        entries: Vec<LeaderboardEntry>,
    },
    /// A judge delivered its verdict on this player's work — stored for the
    /// chat view of the sidebar (the log line the socket also emits is for
    /// text mode).
    JudgeScored {
        /// The task the verdict is on; `None` from pre-upgrade servers.
        task_id: Option<Uuid>,
        judge_name: String,
        point_delta: i32,
        feedback: String,
    },
    /// A judge began evaluating the player's task — the chat's status row
    /// names who is reviewing what until the verdict (or failure) lands.
    JudgeStarted {
        task_id: Option<Uuid>,
        judge_name: String,
    },
    /// A judge gave up on the task; drops it from the "reviewing" line.
    JudgeFailed {
        task_id: Option<Uuid>,
        judge_name: String,
    },
    PlayerProgress {
        attempt: u32,
        status: PlayerRunStatus,
    },
    ViewerIdentified(Uuid),
    /// Server asked for a snapshot commit+push (open-ended tasks):
    /// `todo_complete`/`deadline` → the final `feat(<task_id>)` commit,
    /// `todo_progress` → a `wip(<task_id>)` checkpoint.
    SnapshotRequested {
        task_id: Uuid,
        task_title: String,
        reason: String,
    },
    /// The completion-flag watcher committed and pushed a done file — the
    /// player's own words about what they built, shown in the chat view as
    /// their message (the web chat renders the same file as the done-note).
    CompletionFlagPublished {
        /// Worktree-relative path of the flag file.
        path: String,
        /// The file's markdown contents.
        text: String,
    },
    /// A judge asked for an artifact: the participant (or their agent)
    /// writes it under `path`; the app watches and commits it when it lands.
    CountdownDone,
    /// Total number of tasks in the project, delivered once via
    /// `ArenaFrame::SessionStarted` when the session transitions from
    /// lobby to running.
    TotalTasks(u32),
    MemberJoined {
        name: String,
    },
    ShouldQuit(QuitReason),
    Resized {
        cols: u16,
        rows: u16,
    },
    Tick,
    TokensUpdate {
        counts: Vec<agent_tokens::SessionCounts>,
        /// Behavioural stats (messages, tools, skills) joined with `counts`
        /// by (agent, session id) in the tokens panel renderer.
        stats: Vec<agent_tokens::SessionStats>,
    },
    GitDiffUpdate(crate::tui::git_diff::DiffStats),
    /// A probe command needs the player's permission before it runs.
    PermissionRequest(PermissionPrompt),
    /// The pending permission request resolved elsewhere (answered or the
    /// probe deadline passed) — close the popup if it is still open.
    PermissionResolved {
        probe_id: Uuid,
    },
}

#[derive(Debug, Error)]
pub enum EventSinkError {
    #[error("event sink closed")]
    Closed,
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

pub trait EventSink: Send + Sync {
    fn send(&self, origin: Origin, event: TuiEvent) -> Result<(), EventSinkError>;
}

pub struct BusSink {
    tx: mpsc::Sender<TuiEvent>,
    dropped_count: Arc<AtomicU64>,
}

impl BusSink {
    pub fn new() -> (Arc<Self>, mpsc::Receiver<TuiEvent>, Arc<AtomicU64>) {
        let (tx, rx) = mpsc::channel(BUS_CAPACITY);
        let dropped_count = Arc::new(AtomicU64::new(0));
        let sink = Arc::new(Self {
            tx,
            dropped_count: dropped_count.clone(),
        });
        (sink, rx, dropped_count)
    }
}

impl EventSink for BusSink {
    fn send(&self, _origin: Origin, ev: TuiEvent) -> Result<(), EventSinkError> {
        match self.tx.try_send(ev) {
            Ok(()) => Ok(()),
            Err(mpsc::error::TrySendError::Full(_)) => {
                self.dropped_count
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                Ok(())
            }
            Err(mpsc::error::TrySendError::Closed(_)) => Err(EventSinkError::Closed),
        }
    }
}

pub struct RecordingSink {
    pub events: Mutex<Vec<(Origin, TuiEvent)>>,
}

impl RecordingSink {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            events: Mutex::new(Vec::new()),
        })
    }
    pub fn snapshot(&self) -> Vec<(Origin, TuiEvent)> {
        self.events.lock().unwrap().clone()
    }
}

impl EventSink for RecordingSink {
    fn send(&self, origin: Origin, ev: TuiEvent) -> Result<(), EventSinkError> {
        self.events.lock().unwrap().push((origin, ev));
        Ok(())
    }
}

pub struct StderrSink;

impl EventSink for StderrSink {
    fn send(&self, _origin: Origin, _ev: TuiEvent) -> Result<(), EventSinkError> {
        Ok(())
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bus_sink_try_send_succeeds_under_capacity() {
        let (sink, mut rx, _dropped) = BusSink::new();
        sink.send(Origin::Network, TuiEvent::Tick).unwrap();
        let got = rx.try_recv().unwrap();
        assert!(matches!(got, TuiEvent::Tick));
    }

    #[test]
    fn bus_sink_drops_on_full_and_increments_counter() {
        let (sink, _rx, dropped) = BusSink::new();
        // Fill the bus. We don't drain.
        for i in 0..(BUS_CAPACITY * 2) {
            sink.send(
                Origin::Network,
                TuiEvent::Log {
                    level: LogLevel::Hint,
                    msg: format!("{i}"),
                },
            )
            .unwrap();
        }
        assert!(dropped.load(std::sync::atomic::Ordering::Relaxed) > 0);
    }

    #[test]
    fn bus_sink_returns_closed_when_receiver_dropped() {
        let (sink, rx, _dropped) = BusSink::new();
        drop(rx);
        let result = sink.send(Origin::Network, TuiEvent::Tick);
        assert!(matches!(result, Err(EventSinkError::Closed)));
    }

    #[test]
    fn recording_sink_captures_in_order_with_origin() {
        let sink = RecordingSink::new();
        sink.send(Origin::Network, TuiEvent::Tick).unwrap();
        sink.send(Origin::Input, TuiEvent::Tick).unwrap();
        let snap = sink.snapshot();
        assert_eq!(snap.len(), 2);
        assert_eq!(snap[0].0, Origin::Network);
        assert_eq!(snap[1].0, Origin::Input);
    }

    #[test]
    fn origin_enum_is_distinct() {
        assert_ne!(Origin::Network, Origin::Render);
        assert_ne!(Origin::Render, Origin::Pty);
        assert_ne!(Origin::Pty, Origin::Input);
    }

    #[test]
    fn probe_result_info_carries_deadline_secs() {
        let info = ProbeResultInfo {
            probe_id: Uuid::nil(),
            command: "echo hi".to_string(),
            stdout: String::new(),
            exit_code: None,
            duration_ms: 0,
            error: None,
            task_id: None,
            task_ordinal: 0,
            task_title: String::new(),
            task_description: String::new(),
            test_ordinal: 0,
            test_total: 0,
            test_label: String::new(),
            test_description: String::new(),
            deadline_secs: Some(30),
            expected_answer: None,
            answer_template: String::new(),
            validation_kind: ValidationKind::Minijinja,
            outcome: None,
            point_delta: None,
            graded_expected: None,
        };
        assert_eq!(info.deadline_secs, Some(30));
    }

    #[test]
    fn leaderboard_update_event_carries_entries() {
        let entries = vec![LeaderboardEntry {
            player_id: Uuid::nil(),
            display_name: "p1".to_string(),
            agent_display_name: None,
            total_points: 10,
            tests_passed: 1,
            total_wall_ms: 0,
        }];
        let ev = TuiEvent::LeaderboardUpdate {
            entries: entries.clone(),
        };
        match ev {
            TuiEvent::LeaderboardUpdate { entries: e } => {
                assert_eq!(e.len(), 1);
                assert_eq!(e[0].total_points, 10);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn player_progress_event_carries_attempt_and_status() {
        let ev = TuiEvent::PlayerProgress {
            attempt: 2,
            status: PlayerRunStatus::AwaitingResult,
        };
        match ev {
            TuiEvent::PlayerProgress { attempt, status } => {
                assert_eq!(attempt, 2);
                assert_eq!(status, PlayerRunStatus::AwaitingResult);
            }
            _ => panic!("wrong variant"),
        }
    }
}
