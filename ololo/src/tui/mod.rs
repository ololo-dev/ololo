//! TUI mode for `ololo start` / `ololo join`.
//!
//! The TUI covers `start` and `join` only. `login`, `whoami`, and
//! `profile` remain text-mode (see `ololo/src/ui.rs`).

pub mod agent_picker;
pub mod app;
pub mod event;
pub mod git_diff;
pub mod header;
pub(crate) mod log_redir;
pub mod pty_host;
pub mod render;

#[allow(unused_imports)]
pub use agent_picker::{AgentSource, PickedAgent, PickerArgs, select_agent};
#[allow(unused_imports)]
pub use app::{InputFocus, TuiApp, run, run_headless};
#[allow(unused_imports)]
pub use event::{
    BusSink, EventSink, EventSinkError, HeaderDelta, LogLevel, Origin, ProbeInfo, ProbeResultInfo,
    QuitReason, RecordingSink, StderrSink, TuiEvent,
};
#[allow(unused_imports)]
pub use header::{HeaderState, Status};
#[allow(unused_imports)]
pub use pty_host::{Cell, Color, PtyHost, grid_to_ratatui};
