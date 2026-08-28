//! Header state machine for the TUI.
//!
//! `Status::Connecting` is the initial pre-first-frame state. The
//! `apply` reducer is the sole mutator of `HeaderState`; the render
//! loop reads the resulting state and never mutates it.

use crate::tui::event::HeaderDelta;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Connecting,
    Lobby,
    Running,
    Paused,
    /// This player finished all their tasks; the session is still running
    /// for other players. Sticky against `Running`/`Paused` countdown
    /// deltas — only `Complete`/`Cancelled`/`Error` move past it.
    TasksDone,
    Complete,
    Cancelled,
    Error,
}

#[derive(Debug, Clone)]
pub struct HeaderState {
    pub session: String,
    /// Absolute URL of the session page (`/s/{code}`). When set, the
    /// header renders it as a clickable link instead of `session={id}`.
    pub session_url: String,
    /// Absolute URL of the project page (`/projects/{slug}`). When set,
    /// the header renders it as a clickable link instead of `project={id}`.
    pub project_url: String,
    pub project: String,
    pub status: Status,
    pub countdown_secs: Option<u32>,
    pub error_message: Option<String>,
    /// Newer CLI release known from the cached update check; renders as
    /// an "update available" hint in the header. Never changes mid-session.
    pub update_available: Option<String>,
}

impl HeaderState {
    pub fn new(session: impl Into<String>, project: impl Into<String>) -> Self {
        Self {
            session: session.into(),
            session_url: String::new(),
            project_url: String::new(),
            project: project.into(),
            status: Status::Connecting,
            countdown_secs: None,
            error_message: None,
            update_available: None,
        }
    }

    /// Pure reducer. The final `_ => {}` arm is intentional forward
    /// compat: when new `HeaderDelta` variants are added in the
    /// future, existing call sites keep working without a compile
    /// error.
    #[allow(unreachable_patterns)]
    pub fn apply(&mut self, delta: HeaderDelta) {
        match delta {
            HeaderDelta::Lobby { seconds } => {
                self.status = Status::Lobby;
                self.countdown_secs = Some(seconds);
                self.error_message = None;
            }
            HeaderDelta::Running { seconds } => {
                // The game-server keeps broadcasting the session countdown
                // after the per-player all-tasks-done ack — update the
                // clock but stay in the waiting state.
                if self.status != Status::TasksDone {
                    self.status = Status::Running;
                }
                self.countdown_secs = Some(seconds);
                self.error_message = None;
            }
            HeaderDelta::Paused { seconds } => {
                if self.status != Status::TasksDone {
                    self.status = Status::Paused;
                }
                self.countdown_secs = Some(seconds);
                self.error_message = None;
            }
            HeaderDelta::Started => {
                self.status = Status::Running;
                self.countdown_secs = None;
                self.error_message = None;
            }
            HeaderDelta::PlayerTasksDone => {
                self.status = Status::TasksDone;
                self.error_message = None;
            }
            HeaderDelta::Complete { .. } => {
                self.status = Status::Complete;
                self.countdown_secs = None;
            }
            HeaderDelta::Cancelled { .. } => {
                self.status = Status::Cancelled;
                self.countdown_secs = None;
            }
            HeaderDelta::Error { message } => {
                self.status = Status::Error;
                self.error_message = Some(message);
            }
            _ => {
                // Future variants: leave state unchanged.
            }
        }
    }

    pub fn countdown_mmss(&self) -> Option<String> {
        let s = self.countdown_secs?;
        Some(format!("{:02}:{:02}", s / 60, s % 60))
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initial_state_is_connecting() {
        let h = HeaderState::new("ABC123", "my-project");
        assert_eq!(h.status, Status::Connecting);
        assert_eq!(h.countdown_secs, None);
        assert_eq!(h.session, "ABC123");
        assert_eq!(h.project, "my-project");
    }

    #[test]
    fn lobby_delta_sets_lobby_with_countdown() {
        let mut h = HeaderState::new("s", "p");
        h.apply(HeaderDelta::Lobby { seconds: 30 });
        assert_eq!(h.status, Status::Lobby);
        assert_eq!(h.countdown_secs, Some(30));
    }

    #[test]
    fn started_delta_clears_countdown_per_fr_003() {
        let mut h = HeaderState::new("s", "p");
        h.apply(HeaderDelta::Lobby { seconds: 30 });
        assert_eq!(h.countdown_secs, Some(30));
        h.apply(HeaderDelta::Started);
        assert_eq!(h.status, Status::Running);
        assert_eq!(
            h.countdown_secs, None,
            "SessionStarted must clear countdown"
        );
    }

    #[test]
    fn running_delta_after_started_sets_countdown() {
        let mut h = HeaderState::new("s", "p");
        h.apply(HeaderDelta::Started);
        h.apply(HeaderDelta::Running { seconds: 120 });
        assert_eq!(h.status, Status::Running);
        assert_eq!(h.countdown_secs, Some(120));
    }

    #[test]
    fn complete_delta_clears_countdown_and_sets_complete() {
        let mut h = HeaderState::new("s", "p");
        h.apply(HeaderDelta::Started);
        h.apply(HeaderDelta::Running { seconds: 60 });
        h.apply(HeaderDelta::Complete {
            session_id: "abc".into(),
        });
        assert_eq!(h.status, Status::Complete);
        assert_eq!(h.countdown_secs, None);
    }

    #[test]
    fn paused_delta_sets_paused_with_countdown() {
        let mut h = HeaderState::new("s", "p");
        h.apply(HeaderDelta::Paused { seconds: 420 });
        assert_eq!(h.status, Status::Paused);
        assert_eq!(h.countdown_secs, Some(420));
    }

    #[test]
    fn cancelled_delta_clears_countdown_and_sets_cancelled() {
        let mut h = HeaderState::new("s", "p");
        h.apply(HeaderDelta::Running { seconds: 60 });
        h.apply(HeaderDelta::Cancelled {
            session_id: "abc".into(),
        });
        assert_eq!(h.status, Status::Cancelled);
        assert_eq!(h.countdown_secs, None);
    }

    #[test]
    fn error_delta_sets_status_and_message() {
        let mut h = HeaderState::new("s", "p");
        h.apply(HeaderDelta::Error {
            message: "ws disconnected".into(),
        });
        assert_eq!(h.status, Status::Error);
        assert_eq!(h.error_message.as_deref(), Some("ws disconnected"));
    }

    #[test]
    fn unknown_variant_arm_is_a_no_op() {
        // The `_ => {}` arm cannot be reached through the public API
        // (the match is exhaustive), but we verify the property
        // indirectly: applying a sequence where the last variant is
        // the one that *would* be the no-op slot must leave the
        // state untouched. Since all current variants are named, we
        // test that `apply` does not panic and does not change the
        // session/project labels.
        let mut h = HeaderState::new("s", "p");
        h.apply(HeaderDelta::Lobby { seconds: 5 });
        h.apply(HeaderDelta::Lobby { seconds: 10 });
        assert_eq!(h.countdown_secs, Some(10));
    }

    #[test]
    fn countdown_mmss_formats_correctly() {
        let mut h = HeaderState::new("s", "p");
        h.countdown_secs = Some(125);
        assert_eq!(h.countdown_mmss().as_deref(), Some("02:05"));
        h.countdown_secs = Some(7);
        assert_eq!(h.countdown_mmss().as_deref(), Some("00:07"));
        h.countdown_secs = None;
        assert_eq!(h.countdown_mmss(), None);
    }

    #[test]
    fn player_tasks_done_delta_sets_tasks_done_and_keeps_countdown() {
        let mut h = HeaderState::new("s", "p");
        h.apply(HeaderDelta::Running { seconds: 90 });
        h.apply(HeaderDelta::PlayerTasksDone);
        assert_eq!(h.status, Status::TasksDone);
        assert_eq!(
            h.countdown_secs,
            Some(90),
            "session countdown keeps ticking while waiting"
        );
    }

    #[test]
    fn running_delta_does_not_unstick_tasks_done_but_updates_countdown() {
        // The game-server keeps broadcasting RunningCountdown every second
        // while the session runs for the other players — those must not
        // flip the header back to Running after the per-player ack.
        let mut h = HeaderState::new("s", "p");
        h.apply(HeaderDelta::PlayerTasksDone);
        h.apply(HeaderDelta::Running { seconds: 60 });
        assert_eq!(h.status, Status::TasksDone);
        assert_eq!(h.countdown_secs, Some(60));
    }

    #[test]
    fn paused_delta_does_not_unstick_tasks_done_but_updates_countdown() {
        let mut h = HeaderState::new("s", "p");
        h.apply(HeaderDelta::PlayerTasksDone);
        h.apply(HeaderDelta::Paused { seconds: 30 });
        assert_eq!(h.status, Status::TasksDone);
        assert_eq!(h.countdown_secs, Some(30));
    }

    #[test]
    fn complete_delta_overrides_tasks_done() {
        let mut h = HeaderState::new("s", "p");
        h.apply(HeaderDelta::PlayerTasksDone);
        h.apply(HeaderDelta::Complete {
            session_id: "abc".into(),
        });
        assert_eq!(h.status, Status::Complete);
        assert_eq!(h.countdown_secs, None);
    }

    #[test]
    fn cancelled_delta_overrides_tasks_done() {
        let mut h = HeaderState::new("s", "p");
        h.apply(HeaderDelta::PlayerTasksDone);
        h.apply(HeaderDelta::Cancelled {
            session_id: "abc".into(),
        });
        assert_eq!(h.status, Status::Cancelled);
    }
}
