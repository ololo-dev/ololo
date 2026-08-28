#![allow(dead_code)]

//! TuiApp: the application state for the TUI render loop.
//!
//! The full render loop (tokio::select! over crossterm::EventStream,
//! mpsc::Receiver<TuiEvent>, tick, render timer) lands in WP-008's
//! production constructor `TuiApp::new` plus `tui::run`. This file
//! provides the `on_key`, `on_event`, and `on_session_started`
//! entry points that the render loop calls per event.

use crate::tui::event::{ProbeResultInfo, QuitReason, TuiEvent};
use crate::tui::header::HeaderState;
use crate::tui::pty_host::PtyHost;
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::sync::atomic::AtomicU64;
use uuid::Uuid;
use vt100::Parser;

mod keymap;
mod run_loop;
pub(crate) use run_loop::pty_inner_rect;
pub use run_loop::{run, run_headless};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputFocus {
    Tui,
    Pty,
}

/// What the sidebar shows: the classic probe list, or the session retold
/// as a chat transcript (tasks → checks → judge verdicts), mirroring the
/// web player page. F5 toggles.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SidebarView {
    Probes,
    Chat,
}

/// One judge verdict, kept for the chat view. The wire frame carries no
/// task id, so the verdict is pinned to the scheduler's current task at
/// arrival — the task it judged in every realistic ordering.
#[derive(Debug, Clone)]
pub struct JudgeVerdict {
    pub judge_name: String,
    pub point_delta: i32,
    pub feedback: String,
    /// `max_task_ordinal` when the verdict arrived; `None` before any task.
    pub task_ordinal: Option<i32>,
}

/// The player's done-note: the completion flag file's contents, kept for
/// the chat view as their message — the same file the web chat renders as
/// the player speaking. Pinned to the task in play when the flag settled.
#[derive(Debug, Clone)]
pub struct DoneNote {
    /// Worktree-relative flag path (`.ololo/<name>-done.md`).
    pub path: String,
    pub text: String,
    /// `max_task_ordinal` when the flag was published; `None` before any task.
    pub task_ordinal: Option<i32>,
}

/// One message of the chat transcript, in render order (oldest first).
/// Mirrors the web player chat's message kinds: ololo hands out tasks and
/// runs checks, judges ask for evidence and deliver verdicts, the player
/// answers with done-notes.
pub enum ChatMsg<'a> {
    /// A task began: `── TASK #n title ──` with its running points total.
    TaskHeader {
        ordinal: i32,
        title: String,
        points: Option<i64>,
        passed: bool,
    },
    /// ololo's message: the task brief, in full.
    Brief { text: &'a str },
    /// One check, collapsed across re-runs of the same test: the latest
    /// probe speaks, `runs` says how many attempts it took. `question`
    /// carries the quiz question the probe asked, when its command has one.
    Check {
        probe: &'a ProbeResultInfo,
        runs: usize,
        question: Option<String>,
    },
    /// A judge's evidence request, retold from the probe's shell one-liner:
    /// who asks, what to capture, where to save it, and whether it landed.
    Request {
        judge: String,
        instruction: String,
        path: String,
        delivered: bool,
    },
    /// The player's message: their done-note.
    DoneNote(&'a DoneNote),
    /// A judge's word on the task.
    Verdict(&'a JudgeVerdict),
    /// Synthetic marker (member joined, session status, …) — a quiet
    /// system line.
    System { text: String },
}

/// A selectable row in the probes sidebar: a task header or a probe entry.
/// Identified by id (not index) so the cursor survives probe arrival and
/// eviction. Synthetic probes share `Uuid::nil()` — selection resolves to
/// the first match, which is acceptable for markers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NavTarget {
    Task(Uuid),
    Probe(Uuid),
}

/// One task's probes plus its derived display state, in render order.
pub struct TaskGroup<'a> {
    pub task_id: Uuid,
    pub ordinal: i32,
    /// Oldest-first (arrival order); renderers iterate `.rev()`.
    pub probes: Vec<&'a ProbeResultInfo>,
    /// The scheduler advanced past this task (or legacy all-pass heuristic).
    pub passed: bool,
    /// Display fold state: manual override, else folded-iff-passed.
    pub folded: bool,
    /// Sum of server-graded `point_delta` across the task's probes.
    /// `None` until at least one probe is graded.
    pub points: Option<i64>,
}

pub const RENDER_INTERVAL_MS: u64 = 33; // ~30 Hz

/// Lines of scrollback the agent's PTY parser retains for its *primary*
/// screen buffer (see `scrollback_delta`/the `Event::Mouse` handling in
/// `run()`). Alt-screen apps never get scrollback here regardless of this
/// value -- `vt100::Screen` hardcodes the alternate-screen grid to 0, same
/// as a real terminal.
pub const PTY_SCROLLBACK_LINES: usize = 2000;

/// Per-task metadata kept by the app for snapshot commits and stats windows.
#[derive(Debug, Clone)]
struct TaskRecord {
    id: Uuid,
    ordinal: i32,
    title: String,
    /// When the first probe for this task arrived (epoch ms) — the start of
    /// the task's agent-stats collection window.
    first_seen_ms: i64,
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

pub struct TuiApp {
    pub header: HeaderState,
    pub agent_label: String,
    pub pty: Option<PtyHost>,
    pub pty_parser: Parser,
    pub pty_cols: u16,
    pub pty_rows: u16,
    pub has_pty: bool,
    /// The chosen agent opens its own window, so there is no PTY to show and
    /// the agent pane explains that instead of sitting blank.
    pub agent_is_desktop: bool,
    pub show_sidebar: bool,
    /// Probe list or chat transcript (F5).
    pub sidebar_view: SidebarView,
    /// Chat view: lines scrolled up from the bottom (0 = follow latest).
    pub chat_scroll: usize,
    /// Chat view: the selected bubble, counted from the newest (0 = the
    /// last message). `None` = no selection, the feed follows the latest.
    /// A selected bubble can be sent to the hosted agent (⏎/p) the way
    /// F3 sends the last failed probe.
    pub chat_cursor: Option<usize>,
    /// Judge verdicts in arrival order, for the chat view.
    pub judge_verdicts: Vec<JudgeVerdict>,
    /// The player's done-notes in arrival order, for the chat view.
    pub done_notes: Vec<DoneNote>,
    /// The chat compose line: `Some(text)` while the player is typing a
    /// message to the agent; `None` otherwise.
    pub chat_input: Option<String>,
    pub probes: VecDeque<ProbeResultInfo>,
    pub dropped_count: Arc<AtomicU64>,
    pub should_quit: Option<QuitReason>,
    pub input_focus: InputFocus,
    /// Where focus was before a modal (permission prompt, help, probe popup)
    /// pulled it to Tui. Restored when the modal closes, so an agent prompt
    /// mid-typing never strands the user in the probes pane (they used to
    /// have to notice and press F9 after every popup).
    pub focus_return: Option<InputFocus>,
    pub score: Option<i64>,
    pub rank: Option<usize>,
    pub progress_attempt: Option<u32>,
    pub progress_status: Option<crate::tui::event::PlayerRunStatus>,
    /// Total tasks in the project, set when `SessionStarted` arrives.
    /// `None` until the server sends it.
    pub total_tasks: Option<u32>,
    /// Highest task ordinal seen in any `TestPush` — the scheduler's
    /// current task position. Tasks with a lower ordinal are done
    /// (server advanced past them). `None` until the first probe.
    pub max_task_ordinal: Option<i32>,
    pub viewer_player_id: Option<uuid::Uuid>,
    pub tokens: Option<Vec<agent_tokens::SessionCounts>>,
    /// Behavioural stats (messages, tools, skills) for the same sessions,
    /// joined with `tokens` by (agent, session id) in the tokens panel.
    pub token_stats: Option<Vec<agent_tokens::SessionStats>>,
    pub tokens_dirty: bool,
    pub pty_resize_pending: bool,
    pub diff_stats: crate::tui::git_diff::DiffStats,
    /// Snapshot repo for per-task commits. `None` in text mode or when
    /// snapshot init failed.
    pub snapshot: Option<Arc<std::sync::Mutex<crate::snapshot::SnapshotRepo>>>,
    /// Judge artifact requests we are watching the worktree for. When the
    /// file lands, the app commits `artifact(<probe_id>)` and pushes.
    /// Fingerprint of `.ololo/artifacts/**` in the worktree — when it
    /// changes, the files are committed and pushed (git is the only channel
    /// for content; the availability probe then passes on its own).
    pub artifacts_fingerprint: u64,
    /// The open-ended judge phase: set when the server asks for the final
    /// snapshot, cleared when a new probe arrives or the session ends —
    /// the header shows what the silence is.
    pub judging: bool,
    /// Tasks we've already committed to the snapshot store.
    committed_tasks: HashMap<Uuid, ()>,
    /// The task last pushed to the snapshot repo's current-task marker, so
    /// each probe does not re-take the snapshot lock.
    snapshot_current_task: Option<Uuid>,
    /// Tasks seen so far (id → record), used to look up title/ordinal
    /// when a later probe advances `max_task_ordinal`.
    known_tasks: HashMap<Uuid, TaskRecord>,
    /// Sender for the task-stats reporter task (`crate::task_stats`).
    /// `None` in text mode or when reporting is disabled.
    pub stats_tx: Option<tokio::sync::mpsc::UnboundedSender<crate::task_stats::CompletedTask>>,
    /// Tasks whose stats have already been handed to the reporter.
    reported_stats: HashMap<Uuid, ()>,
    /// Manual fold overrides per task (true = folded). Absent → the task
    /// folds automatically when passed.
    pub fold_overrides: HashMap<Uuid, bool>,
    /// Sidebar selection (↑/↓ in Tui focus). `None` = nothing selected.
    pub sidebar_cursor: Option<NavTarget>,
    /// Probe whose details are shown in the popup overlay. `None` = closed.
    pub probe_popup: Option<Uuid>,
    /// Hotkey-help popup overlay (F1 / `?`).
    pub show_help: bool,
    /// Text queued for pasting into the agent PTY ("p" in the probe
    /// popup). The render loop owns the PTY writer and drains this.
    pub pty_paste_pending: Option<String>,
    /// Probe command awaiting the player's permission — modal over
    /// everything; keys go nowhere else until it is answered.
    pub permission_popup: Option<crate::tui::event::PermissionPrompt>,
    /// Highlighted option in the permission popup: 0 allow once,
    /// 1 always allow, 2 decline. Reset to 0 on every new prompt.
    pub permission_cursor: usize,
}

impl TuiApp {
    /// Test constructor used by the render-layer snapshots.
    #[allow(dead_code)]
    pub fn new_for_test(
        header: HeaderState,
        pty_parser: Parser,
        dropped_count: Arc<AtomicU64>,
        cols: u16,
        rows: u16,
    ) -> Self {
        Self {
            header,
            agent_label: "test-agent".to_string(),
            pty: None,
            pty_parser,
            pty_cols: cols,
            pty_rows: rows,
            has_pty: false,
            agent_is_desktop: false,
            show_sidebar: true,
            sidebar_view: SidebarView::Chat,
            chat_scroll: 0,
            chat_cursor: None,
            judge_verdicts: Vec::new(),
            done_notes: Vec::new(),
            chat_input: None,
            probes: VecDeque::new(),
            dropped_count,
            should_quit: None,
            input_focus: InputFocus::Tui,
            focus_return: None,
            score: None,
            rank: None,
            progress_attempt: None,
            progress_status: None,
            total_tasks: None,
            max_task_ordinal: None,
            viewer_player_id: None,
            tokens: None,
            token_stats: None,
            tokens_dirty: false,
            pty_resize_pending: false,
            diff_stats: Default::default(),
            snapshot: None,
            artifacts_fingerprint: 0,
            judging: false,
            committed_tasks: HashMap::new(),
            snapshot_current_task: None,
            known_tasks: HashMap::new(),
            stats_tx: None,
            reported_stats: HashMap::new(),
            fold_overrides: HashMap::new(),
            sidebar_cursor: None,
            probe_popup: None,
            show_help: false,
            pty_paste_pending: None,
            permission_popup: None,
            permission_cursor: 0,
        }
    }

    pub fn on_event(&mut self, ev: TuiEvent) {
        match ev {
            TuiEvent::Header(d) => {
                let was_complete = self.header.status == crate::tui::header::Status::Complete;
                let was_tasks_done = self.header.status == crate::tui::header::Status::TasksDone;
                self.header.apply(d);
                // On transition to Complete, commit any tasks not yet committed.
                if !was_complete && self.header.status == crate::tui::header::Status::Complete {
                    self.commit_tasks(true);
                    self.commit_tasks(false);
                    self.report_task_stats(true);
                    self.report_task_stats(false);
                }
                // Per-player all-tasks-done ack: the scheduler moved past
                // this player's LAST task while the session keeps running.
                // Commit every remaining task snapshot (and report stats)
                // now — the judge pipeline waits for the last task's
                // feat(<task_id>) commit and would stall until session end
                // otherwise. Idempotent with the Complete path above.
                if !was_tasks_done && self.header.status == crate::tui::header::Status::TasksDone {
                    self.commit_tasks(true);
                    self.commit_tasks(false);
                    self.report_task_stats(true);
                    self.report_task_stats(false);
                }
            }
            TuiEvent::ProbeArrived(p) => {
                // Track the scheduler's current task position: the highest
                // ordinal seen in any TestPush. Tasks below it are done.
                // Ordinal 0 is a real task (projects number tasks from 0);
                // the ordinal is only trustworthy when task_id is present
                // (old game-servers omit both).
                let prev_max = self.max_task_ordinal;
                if p.task_id.is_some() {
                    self.max_task_ordinal = Some(
                        self.max_task_ordinal
                            .map_or(p.task_ordinal, |m| m.max(p.task_ordinal)),
                    );
                }
                if let Some(tid) = p.task_id {
                    // Preserve first_seen_ms on repeat probes for the same task —
                    // it anchors the stats collection window.
                    self.known_tasks
                        .entry(tid)
                        .and_modify(|t| {
                            t.ordinal = p.task_ordinal;
                            t.title = p.task_title.clone();
                        })
                        .or_insert_with(|| TaskRecord {
                            id: tid,
                            ordinal: p.task_ordinal,
                            title: p.task_title.clone(),
                            first_seen_ms: now_ms(),
                        });
                    // Probes dispatch for the scheduler's active task — tell
                    // the snapshot repo so auxiliary commits (artifacts,
                    // flags, memory) address it in their message.
                    if self.snapshot_current_task != Some(tid) {
                        if let Some(snap) = self.snapshot.clone()
                            && let Ok(guard) = snap.lock()
                        {
                            guard.set_current_task(Some(tid));
                        }
                        self.snapshot_current_task = Some(tid);
                    }
                }
                if let Some(existing) = self.probes.iter_mut().find(|x| x.probe_id == p.probe_id) {
                    existing.command = p.rendered_command.clone();
                    existing.task_id = p.task_id.or(existing.task_id);
                    existing.task_ordinal = if p.task_ordinal != 0 {
                        p.task_ordinal
                    } else {
                        existing.task_ordinal
                    };
                    existing.task_title = p.task_title.clone();
                    existing.task_description = p.task_description.clone();
                    existing.test_ordinal = p.test_ordinal;
                    existing.test_total = p.test_total;
                    existing.test_label = p.test_label.clone();
                    existing.test_description = p.test_description.clone();
                    existing.deadline_secs = Some(p.deadline_secs);
                    existing.expected_answer = p.expected_answer.clone();
                    existing.answer_template = p.answer_template.clone();
                    existing.validation_kind = p.validation_kind;
                } else {
                    self.probes.push_back(ProbeResultInfo {
                        probe_id: p.probe_id,
                        command: p.rendered_command.clone(),
                        stdout: String::new(),
                        exit_code: None,
                        duration_ms: 0,
                        error: None,
                        task_id: p.task_id,
                        task_ordinal: p.task_ordinal,
                        task_title: p.task_title.clone(),
                        task_description: p.task_description.clone(),
                        test_ordinal: p.test_ordinal,
                        test_total: p.test_total,
                        test_label: p.test_label.clone(),
                        test_description: p.test_description.clone(),
                        deadline_secs: Some(p.deadline_secs),
                        expected_answer: p.expected_answer.clone(),
                        answer_template: p.answer_template.clone(),
                        validation_kind: p.validation_kind,
                        outcome: None,
                        point_delta: None,
                        graded_expected: None,
                    });
                }
                // If the scheduler advanced past a task ordinal, commit
                // all tasks with a lower ordinal that we haven't committed,
                // and report their agent stats.
                if self.max_task_ordinal > prev_max {
                    self.commit_tasks(true);
                    self.report_task_stats(true);
                }
            }
            TuiEvent::SnapshotRequested {
                task_id,
                task_title,
                reason,
            } => {
                // Open-ended tasks: the server drives the commit cadence.
                // Track the task first so the stats/final-commit paths know it.
                self.known_tasks
                    .entry(task_id)
                    .or_insert_with(|| TaskRecord {
                        id: task_id,
                        ordinal: self.max_task_ordinal.unwrap_or(0),
                        title: task_title.clone(),
                        first_seen_ms: now_ms(),
                    });
                // A snapshot request names the task in play — keep the repo's
                // current-task marker fresh for auxiliary commit messages.
                if self.snapshot_current_task != Some(task_id) {
                    if let Some(snap) = self.snapshot.clone()
                        && let Ok(guard) = snap.lock()
                    {
                        guard.set_current_task(Some(task_id));
                    }
                    self.snapshot_current_task = Some(task_id);
                }
                if reason != "todo_progress" {
                    self.judging = true;
                }
                let Some(snap) = self.snapshot.clone() else {
                    return;
                };
                if reason == "todo_progress" {
                    if let Ok(guard) = snap.lock()
                        && guard.commit_wip(task_id).is_ok()
                    {
                        let _ = guard.push_to_remote();
                        tracing::info!("wip snapshot pushed: wip({task_id})");
                    }
                } else if !self.committed_tasks.contains_key(&task_id) {
                    let title = self
                        .known_tasks
                        .get(&task_id)
                        .map(|t| t.title.clone())
                        .unwrap_or(task_title);
                    if let Ok(guard) = snap.lock()
                        && guard.commit_task(task_id, &title).is_ok()
                    {
                        self.committed_tasks.insert(task_id, ());
                        let _ = guard.push_to_remote();
                        tracing::info!(
                            "final snapshot pushed ({reason}): feat({task_id}): {title}"
                        );
                    }
                    self.report_task_stats(false);
                }
            }
            TuiEvent::ProbeResult(r) => {
                // A fresh probe means the phase moved on.
                self.judging = false;
                if let Some(existing) = self.probes.iter_mut().find(|x| x.probe_id == r.probe_id) {
                    // Preserve task metadata if the result omits it (defensive — player_ws
                    // now always sends task_title/description, but synthetic paths may not).
                    let mut merged = r;
                    if merged.task_title.is_empty() {
                        merged.task_title = existing.task_title.clone();
                    }
                    if merged.task_description.is_empty() {
                        merged.task_description = existing.task_description.clone();
                    }
                    // Preserve an earlier server grade if the new ProbeResult
                    // arrives after grading (reconnect race).
                    merged.outcome = merged.outcome.or(existing.outcome);
                    merged.point_delta = merged.point_delta.or(existing.point_delta);
                    // Preserve task_id/ordinal if the result omits them.
                    merged.task_id = merged.task_id.or(existing.task_id);
                    if merged.task_ordinal == 0 {
                        merged.task_ordinal = existing.task_ordinal;
                    }
                    if merged.test_total == 0 {
                        merged.test_ordinal = existing.test_ordinal;
                        merged.test_total = existing.test_total;
                    }
                    if merged.test_label.is_empty() {
                        merged.test_label = existing.test_label.clone();
                    }
                    if merged.test_description.is_empty() {
                        merged.test_description = existing.test_description.clone();
                    }
                    merged.graded_expected = merged
                        .graded_expected
                        .take()
                        .or(existing.graded_expected.take());
                    *existing = merged;
                } else {
                    self.probes.push_back(r);
                }
            }
            TuiEvent::ProbeGraded {
                probe_id,
                outcome,
                point_delta,
                expected,
                actual,
            } => {
                if let Some(p) = self.probes.iter_mut().find(|x| x.probe_id == probe_id) {
                    p.outcome = Some(outcome);
                    p.point_delta = Some(point_delta);
                    // ProbeGraded.expected carries the assertEqual-resolved
                    // value (e.g. "42"), not the raw template. Keep it in
                    // graded_expected — expected_answer still holds the
                    // TestPush template for local grading.
                    if expected.is_some() {
                        p.graded_expected = expected;
                    }
                    if let Some(act) = &actual {
                        p.stdout = act.clone();
                    }
                }
            }
            TuiEvent::CountdownDone => {
                self.header.countdown_secs = None;
            }
            TuiEvent::MemberJoined { name } => {
                self.probes.push_back(ProbeResultInfo::member_joined(&name));
            }
            TuiEvent::Log { level: _, msg } => {
                tracing::info!("{}", msg);
            }
            TuiEvent::ShouldQuit(r) => {
                self.should_quit = Some(r);
            }
            TuiEvent::Resized { cols, rows } => {
                if let Some(pty) = self.pty.as_mut() {
                    let _ = pty.resize(rows, cols);
                }
                self.pty_cols = cols;
                self.pty_rows = rows;
            }
            TuiEvent::Tick => {
                self.commit_arrived_artifacts();
                // ponytail: tick-decrement drifts under tick jitter; absolute-timestamp
                // source (TestResultAck.next_probe_at or observer WS) is the upgrade path.
                for p in self.probes.iter_mut() {
                    if p.exit_code.is_none()
                        && p.error.is_none()
                        && let Some(d) = p.deadline_secs
                    {
                        p.deadline_secs = Some(d.saturating_sub(1));
                    }
                }
            }
            TuiEvent::TokensUpdate { counts, stats } => {
                // Window-mode extraction returns a fresh full aggregate per
                // (session, provider, model) for all turns since session start.
                // Replace, not merge — each snapshot is self-contained.
                self.tokens = Some(counts);
                self.token_stats = Some(stats);
                self.tokens_dirty = true;
                self.pty_resize_pending = true;
            }
            TuiEvent::LeaderboardUpdate { entries } => {
                let sorted = Self::rank_entries(entries);
                let viewer = self.viewer_player_id.and_then(|vid| {
                    sorted
                        .iter()
                        .enumerate()
                        .find(|(_, e)| e.player_id == vid)
                        .map(|(i, e)| (i + 1, e.total_points))
                });
                match viewer {
                    Some((rank, score)) => {
                        self.rank = Some(rank);
                        self.score = Some(score);
                    }
                    None => {
                        self.rank = None;
                        self.score = None;
                    }
                }
            }
            TuiEvent::JudgeScored {
                judge_name,
                point_delta,
                feedback,
            } => {
                // Pin to the scheduler's current task; when that is unknown
                // (reconnect raced the TestPush) fall back to the highest
                // ordinal already stored — the task being judged.
                let task_ordinal = self.max_task_ordinal.or_else(|| {
                    self.probes
                        .iter()
                        .filter(|p| p.task_id.is_some())
                        .map(|p| p.task_ordinal)
                        .max()
                });
                self.judge_verdicts.push(JudgeVerdict {
                    judge_name,
                    point_delta,
                    feedback,
                    task_ordinal,
                });
            }
            TuiEvent::CompletionFlagPublished { path, text } => {
                // Pin to the scheduler's current task, same as verdicts.
                let task_ordinal = self.max_task_ordinal.or_else(|| {
                    self.probes
                        .iter()
                        .filter(|p| p.task_id.is_some())
                        .map(|p| p.task_ordinal)
                        .max()
                });
                self.done_notes.push(DoneNote {
                    path,
                    text,
                    task_ordinal,
                });
            }
            TuiEvent::PlayerProgress { attempt, status } => {
                self.progress_attempt = Some(attempt);
                self.progress_status = Some(status);
            }
            TuiEvent::ViewerIdentified(vid) => {
                self.viewer_player_id = Some(vid);
            }
            TuiEvent::TotalTasks(n) => {
                self.total_tasks = Some(n);
            }
            TuiEvent::GitDiffUpdate(stats) => {
                self.diff_stats = stats;
            }
            TuiEvent::PermissionRequest(prompt) => {
                // Modal: pull focus so the answer keys land here, not in
                // the agent PTY — but remember where it was, so answering
                // puts the user right back where they were typing.
                self.stash_focus_for_modal();
                self.permission_popup = Some(prompt);
                self.permission_cursor = 0;
            }
            TuiEvent::PermissionResolved { probe_id } => {
                if self
                    .permission_popup
                    .as_ref()
                    .is_some_and(|p| p.probe_id == probe_id)
                {
                    self.permission_popup = None;
                    self.restore_stashed_focus();
                }
            }
        }
        self.evict_oldest();
    }

    /// Answer the pending permission prompt and close its popup. Persisting
    /// an always-allow rule is the `player_ws` gate's job when the answer
    /// arrives — the UI layer never touches the filesystem.
    fn respond_permission(&mut self, decision: crate::permissions::Decision) {
        let Some(prompt) = self.permission_popup.take() else {
            return;
        };
        if let Ok(mut guard) = prompt.responder.lock()
            && let Some(tx) = guard.take()
        {
            let _ = tx.send(decision);
        }
        self.restore_stashed_focus();
    }

    /// A modal is about to pull focus to Tui: remember where it was. Only the
    /// first modal in a burst records — the value tracks where the USER had
    /// focus, not where the previous popup left it.
    fn stash_focus_for_modal(&mut self) {
        if self.focus_return.is_none() {
            self.focus_return = Some(self.input_focus);
        }
        self.set_input_focus(InputFocus::Tui);
    }

    /// The modal closed: hand focus back to wherever the user had it.
    fn restore_stashed_focus(&mut self) {
        if let Some(focus) = self.focus_return.take()
            && self.permission_popup.is_none()
            && self.probe_popup.is_none()
            && !self.show_help
        {
            self.set_input_focus(focus);
        }
    }

    fn evict_oldest(&mut self) {
        const CAP: usize = 200;
        while self.probes.len() > CAP {
            self.probes.pop_front();
        }
    }

    /// Sort leaderboard entries: highest points, then most tests passed, then
    /// alphabetical by display name (tie-break for stable ranking).
    fn rank_entries(
        mut entries: Vec<crate::tui::event::LeaderboardEntry>,
    ) -> Vec<crate::tui::event::LeaderboardEntry> {
        entries.sort_by(|a, b| {
            b.total_points
                .cmp(&a.total_points)
                .then_with(|| b.tests_passed.cmp(&a.tests_passed))
                .then_with(|| a.display_name.cmp(&b.display_name))
        });
        entries
    }

    /// Commit tasks to the snapshot store. When `only_below_max` is set, only
    /// tasks whose ordinal is below `max_task_ordinal` (the scheduler has
    /// advanced past them) are committed; otherwise every not-yet-committed
    /// task is committed (used on session complete). Idempotent — skips
    /// already-committed tasks.
    /// Tasks considered done and not yet present in `exclude`. When
    /// `only_below_max` is set, only tasks whose ordinal is below
    /// `max_task_ordinal` (the scheduler has advanced past them) qualify.
    fn done_tasks(&self, only_below_max: bool, exclude: &HashMap<Uuid, ()>) -> Vec<TaskRecord> {
        let mut done: Vec<TaskRecord> = self
            .known_tasks
            .values()
            .filter(|t| !exclude.contains_key(&t.id))
            .filter(|t| {
                !only_below_max
                    || match self.max_task_ordinal {
                        // ordinal 0 is a real task (the project's first), not a
                        // sentinel — known_tasks only holds probes with task_id.
                        Some(m) if m > 0 => t.ordinal < m,
                        _ => false,
                    }
            })
            .cloned()
            .collect();
        done.sort_by_key(|t| t.ordinal);
        done
    }

    /// Hand completed tasks to the stats reporter (`crate::task_stats`).
    /// Requires the viewer player id (arrives via `ViewerIdentified` early in
    /// the session); tasks stay queued until it is known. Idempotent.
    fn report_task_stats(&mut self, only_below_max: bool) {
        let Some(tx) = self.stats_tx.as_ref() else {
            return;
        };
        let Some(player_id) = self.viewer_player_id else {
            return;
        };
        let end_ms = now_ms();
        for t in self.done_tasks(only_below_max, &self.reported_stats) {
            let sent = tx
                .send(crate::task_stats::CompletedTask {
                    task_id: t.id,
                    ordinal: t.ordinal,
                    player_id,
                    window_start_ms: t.first_seen_ms,
                    window_end_ms: end_ms,
                })
                .is_ok();
            if sent {
                self.reported_stats.insert(t.id, ());
            }
        }
    }

    /// Commit and push `.ololo/artifacts/**` whenever its contents change.
    /// No per-request bookkeeping: the judge's availability probe names the
    /// folder, the agent saves files into it, and this sweep ships whatever
    /// appears — the probe then passes on its next run.
    fn commit_arrived_artifacts(&mut self) {
        let Some(snap) = self.snapshot.clone() else {
            return;
        };
        let Ok(guard) = snap.lock() else { return };
        let dir = guard.worktree().join(".ololo").join("artifacts");
        let mut fp: u64 = 0;
        let mut walk = vec![dir.clone()];
        while let Some(d) = walk.pop() {
            let Ok(entries) = std::fs::read_dir(&d) else {
                continue;
            };
            for e in entries.flatten() {
                let p = e.path();
                if p.is_dir() {
                    walk.push(p);
                } else if let Ok(meta) = e.metadata() {
                    use std::hash::{Hash, Hasher};
                    let mut h = std::collections::hash_map::DefaultHasher::new();
                    p.hash(&mut h);
                    meta.len().hash(&mut h);
                    meta.modified()
                        .ok()
                        .and_then(|m| m.duration_since(std::time::UNIX_EPOCH).ok())
                        .map(|d| d.as_secs())
                        .hash(&mut h);
                    fp ^= h.finish();
                }
            }
        }
        if fp == 0 || fp == self.artifacts_fingerprint {
            return;
        }
        if guard.commit_artifacts_sync().is_ok() {
            let _ = guard.push_to_remote();
            self.artifacts_fingerprint = fp;
            tracing::info!("artifacts committed and pushed (.ololo/artifacts sync)");
        }
    }

    fn commit_tasks(&mut self, only_below_max: bool) {
        let Some(snap) = self.snapshot.as_ref() else {
            return;
        };
        let to_commit = self.done_tasks(only_below_max, &self.committed_tasks);
        for t in to_commit {
            if let Ok(guard) = snap.lock()
                && guard.commit_task(t.id, &t.title).is_ok()
            {
                self.committed_tasks.insert(t.id, ());
                let _ = guard.push_to_remote();
                tracing::info!(
                    "task snapshot committed{}: feat({}): {}",
                    if only_below_max { "" } else { " (final)" },
                    t.id,
                    t.title
                );
            }
        }
    }

    /// F9 toggles focus; F10 quits (Tui focus) or forwards the F10
    /// escape sequence (Pty focus); Ctrl-C in Pty focus writes
    /// `\x03` to the agent PTY.
    ///
    /// Tab is deliberately NOT a focus toggle: while Pty is focused, Tab
    /// must reach the embedded agent (autocomplete, panel-cycling, etc.)
    /// via `key_to_pty_bytes` instead of being intercepted here — see
    /// `run()`'s dispatch order, which only calls `on_key` for Tui-focus
    /// keys and F9/F10 (checked before the Pty-forward branch).
    pub fn on_key(
        &mut self,
        code: crossterm::event::KeyCode,
        modifiers: crossterm::event::KeyModifiers,
    ) {
        use crossterm::event::{KeyCode, KeyModifiers};
        // Permission popup is modal above everything: a probe command must
        // not run before the player has answered, and no key (F9 focus
        // flips included) may leak past the question.
        if self.permission_popup.is_some() {
            match code {
                KeyCode::Up | KeyCode::Char('k') | KeyCode::Left => {
                    self.permission_cursor = self.permission_cursor.saturating_sub(1);
                }
                KeyCode::Down | KeyCode::Char('j') | KeyCode::Right => {
                    self.permission_cursor = (self.permission_cursor + 1).min(3);
                }
                KeyCode::Enter | KeyCode::Char(' ') => {
                    let decision = match self.permission_cursor {
                        0 => crate::permissions::Decision::Allow,
                        1 => crate::permissions::Decision::AlwaysAllow,
                        2 => crate::permissions::Decision::AllowAllSession,
                        _ => crate::permissions::Decision::Decline,
                    };
                    self.respond_permission(decision);
                }
                KeyCode::Char('a') | KeyCode::Char('y') => {
                    self.respond_permission(crate::permissions::Decision::Allow)
                }
                KeyCode::Char('w') => {
                    self.respond_permission(crate::permissions::Decision::AlwaysAllow)
                }
                KeyCode::Char('s') => {
                    self.respond_permission(crate::permissions::Decision::AllowAllSession)
                }
                KeyCode::Char('d') | KeyCode::Esc | KeyCode::Char('q') => {
                    self.respond_permission(crate::permissions::Decision::Decline)
                }
                _ => {}
            }
            return;
        }
        // Chat compose line: while open it swallows every key — the text
        // is a message being typed, not commands.
        if self.chat_input.is_some() {
            match code {
                KeyCode::Enter => {
                    let text = self
                        .chat_input
                        .take()
                        .unwrap_or_default()
                        .trim()
                        .to_string();
                    if !text.is_empty() {
                        self.send_chat_message(text);
                    }
                    self.restore_stashed_focus();
                }
                KeyCode::Esc => {
                    self.chat_input = None;
                    self.restore_stashed_focus();
                }
                KeyCode::Backspace => {
                    if let Some(input) = self.chat_input.as_mut() {
                        input.pop();
                    }
                }
                KeyCode::Char(c) if !modifiers.contains(KeyModifiers::CONTROL) => {
                    if let Some(input) = self.chat_input.as_mut() {
                        input.push(c);
                    }
                }
                _ => {}
            }
            return;
        }
        // F10 quits when focus is Tui, forwards when focus is Pty.
        if code == KeyCode::F(10) {
            if self.input_focus == InputFocus::Tui {
                self.should_quit = Some(QuitReason::UserRequested);
            }
            return;
        }
        // F9 toggles focus (both directions). An explicit choice cancels any
        // pending restore a modal left behind.
        if code == KeyCode::F(9) {
            self.focus_return = None;
            self.set_input_focus(match self.input_focus {
                InputFocus::Tui => InputFocus::Pty,
                InputFocus::Pty => InputFocus::Tui,
            });
            return;
        }
        // Global hotkeys (work in both focuses — `run()` routes F1–F4 here
        // before the Pty-forward branch, so the agent never sees them).
        match code {
            KeyCode::F(1) => {
                self.toggle_help();
                return;
            }
            KeyCode::F(2) => {
                self.open_last_failed();
                return;
            }
            KeyCode::F(3) => {
                self.paste_last_failed();
                return;
            }
            KeyCode::F(4) => {
                self.toggle_sidebar();
                return;
            }
            KeyCode::F(5) => {
                self.toggle_sidebar_view();
                return;
            }
            _ => {}
        }
        // Ctrl-C in Pty focus forwards \x03.
        if self.input_focus == InputFocus::Pty
            && code == KeyCode::Char('c')
            && modifiers.contains(KeyModifiers::CONTROL)
        {
            // The TUI sends raw bytes via the PTY master's writer.
            // In the full render loop, the writer is owned by the
            // PtyHost; here we just record the intent.
            self.probes
                .push_back(ProbeResultInfo::pty_input("[ctrl-c forwarded]"));
            return;
        }
        if self.input_focus != InputFocus::Tui {
            return;
        }
        // Help popup swallows keys until closed (checked before the probe
        // popup — it renders on top of it).
        if self.show_help {
            if matches!(
                code,
                KeyCode::Esc | KeyCode::Enter | KeyCode::Char('q') | KeyCode::Char(' ')
            ) {
                self.show_help = false;
                self.restore_stashed_focus();
            }
            return;
        }
        // Probe-details popup swallows keys until closed.
        if let Some(pid) = self.probe_popup {
            match code {
                KeyCode::Esc | KeyCode::Enter | KeyCode::Char('q') | KeyCode::Char(' ') => {
                    self.probe_popup = None;
                    self.restore_stashed_focus();
                }
                // "p" queues the probe details as a paste into the agent
                // PTY (drained by the render loop) and hands focus over so
                // the user can follow up with the agent directly.
                KeyCode::Char('p') if self.has_pty => {
                    if let Some(text) = self.probe_by_id(pid).map(probe_paste_text) {
                        self.pty_paste_pending = Some(text);
                        self.focus_return = None; // explicit hand-over wins
                        self.set_input_focus(InputFocus::Pty);
                    }
                }
                _ => {}
            }
            return;
        }
        // Chat view (Tui focus): ↑/↓ select a bubble (the view follows the
        // selection), ⏎/p sends the selected bubble to the hosted agent,
        // m opens the compose line, PgUp/PgDn scroll freely.
        if self.sidebar_view == SidebarView::Chat {
            match code {
                KeyCode::Up | KeyCode::Char('k') => self.chat_select_by(1),
                KeyCode::Down | KeyCode::Char('j') => self.chat_select_by(-1),
                KeyCode::PageUp => {
                    self.chat_cursor = None;
                    self.chat_scroll_by(10);
                }
                KeyCode::PageDown => {
                    self.chat_cursor = None;
                    self.chat_scroll_by(-10);
                }
                KeyCode::Esc => {
                    self.chat_cursor = None;
                    self.chat_scroll = 0;
                }
                KeyCode::Enter | KeyCode::Char('p') if self.chat_cursor.is_some() => {
                    self.send_selected_bubble();
                }
                KeyCode::Char('m') | KeyCode::Enter => self.open_chat_compose(),
                KeyCode::Char('?') => self.show_help = true,
                _ => {}
            }
            return;
        }
        // Sidebar navigation (Tui focus): move / fold / details.
        match code {
            KeyCode::Up | KeyCode::Char('k') => self.sidebar_move(-1),
            KeyCode::Down | KeyCode::Char('j') => self.sidebar_move(1),
            KeyCode::Enter | KeyCode::Char(' ') => self.sidebar_activate(),
            KeyCode::Esc => self.sidebar_cursor = None,
            KeyCode::Char('?') => self.show_help = true,
            _ => {}
        }
    }

    /// F5: flip the sidebar between the probe list and the chat transcript.
    /// The chat re-follows the newest message on entry.
    fn toggle_sidebar_view(&mut self) {
        self.sidebar_view = match self.sidebar_view {
            SidebarView::Probes => SidebarView::Chat,
            SidebarView::Chat => SidebarView::Probes,
        };
        self.chat_scroll = 0;
        self.chat_cursor = None;
        self.sidebar_cursor = None;
        // The pane must be visible for the toggle to mean anything.
        self.show_sidebar = true;
        // The chat view takes a wider pane than the probe list, so the
        // agent's grid changes size on every flip — not only when the
        // sidebar was hidden.
        self.pty_resize_pending = true;
    }

    /// Open the chat compose line (the "✉ message" button / `m`). Only
    /// meaningful with a hosted agent — the message's destination.
    pub fn open_chat_compose(&mut self) {
        if !self.has_pty || self.chat_input.is_some() {
            return;
        }
        self.stash_focus_for_modal();
        self.chat_input = Some(String::new());
        self.chat_scroll = 0;
    }

    /// Deliver a composed chat message the way F3 delivers a probe: queue
    /// it as a paste into the agent PTY and hand focus over. No auto-Enter
    /// and no chat bubble — the player sees the text land in the agent's
    /// own input, can edit it there, and submits it themselves; the agent
    /// transcript is the record.
    fn send_chat_message(&mut self, text: String) {
        self.pty_paste_pending = Some(text);
        self.focus_return = None; // explicit hand-over wins
        self.set_input_focus(InputFocus::Pty);
        self.chat_scroll = 0;
    }

    /// Move the chat bubble selection: positive = towards older messages
    /// (up), negative = towards newer. Stepping below the newest bubble
    /// drops the selection and the feed re-follows the latest.
    fn chat_select_by(&mut self, delta: isize) {
        let len = self.chat_transcript().len();
        if len == 0 {
            return;
        }
        self.chat_cursor = if delta > 0 {
            Some(
                self.chat_cursor
                    .map_or(0, |c| c.saturating_add(1))
                    .min(len - 1),
            )
        } else {
            match self.chat_cursor {
                Some(0) | None => {
                    self.chat_scroll = 0;
                    None
                }
                Some(c) => Some(c - 1),
            }
        };
    }

    /// A click on transcript bubble `idx` (index into `chat_transcript()`):
    /// first click selects it — and pulls TUI focus so ⏎/↑/↓ work without
    /// an F9 round-trip — a second click on the same bubble sends it to
    /// the agent.
    pub fn chat_click_bubble(&mut self, idx: usize) {
        let len = self.chat_transcript().len();
        let Some(cursor) = len.checked_sub(1 + idx) else {
            return;
        };
        if self.chat_cursor == Some(cursor) {
            self.send_selected_bubble();
        } else {
            self.chat_cursor = Some(cursor);
            self.focus_return = None;
            self.set_input_focus(InputFocus::Tui);
        }
    }

    /// Send the selected bubble to the hosted agent, the way F3 sends the
    /// last failed probe: its plain-text retelling is queued as a paste
    /// into the agent PTY and focus hands over, so the player can add
    /// their own words and submit.
    fn send_selected_bubble(&mut self) {
        if !self.has_pty {
            return;
        }
        let Some(cur) = self.chat_cursor else {
            return;
        };
        let text = {
            let msgs = self.chat_transcript();
            msgs.len()
                .checked_sub(1 + cur)
                .and_then(|idx| msgs.get(idx).map(chat_msg_paste_text))
        };
        if let Some(text) = text {
            self.pty_paste_pending = Some(text);
            self.focus_return = None; // explicit hand-over wins
            self.set_input_focus(InputFocus::Pty);
        }
    }

    /// Scroll the chat by `delta` lines (positive = further into history).
    /// Loosely capped here; the renderer clamps to the real line count.
    pub fn chat_scroll_by(&mut self, delta: isize) {
        // Nothing in the chat is truncated, so a single message can be
        // dozens of wrapped rows (a TAP dump, a judge's paragraphs). Bound
        // the cursor by the transcript's actual text volume — a per-message
        // guess would strand the reader below the top of a long session.
        // The renderer clamps to the real line count; this only keeps ↑ from
        // running away past the beginning.
        let chars: usize = self
            .chat_transcript()
            .iter()
            .map(|m| match m {
                ChatMsg::TaskHeader { title, .. } => title.len() + 24,
                ChatMsg::Brief { text } => text.len() + 8,
                ChatMsg::Check {
                    probe, question, ..
                } => question.as_deref().map(str::len).unwrap_or(0) + probe.stdout.len() + 24,
                ChatMsg::Request {
                    instruction, path, ..
                } => instruction.len() + path.len() + 32,
                ChatMsg::DoneNote(n) => n.text.len() + n.path.len() + 24,
                ChatMsg::Verdict(v) => v.judge_name.len() + v.feedback.len() + 24,
                ChatMsg::System { text } => text.len() + 8,
            })
            .sum();
        // ~16 columns of text per wrapped row is a deliberate under-estimate:
        // erring wide keeps the top reachable on a narrow pane.
        let cap = chars / 16 + 8;
        self.chat_scroll = self.chat_scroll.saturating_add_signed(delta).min(cap);
    }

    /// The most recent probe that failed: local error, timeout, or a
    /// server grade of Error/NoResponse. A later re-grade to Pass takes
    /// the probe out of the running.
    pub fn last_failed_probe(&self) -> Option<&ProbeResultInfo> {
        use arena_core::protocol::ProbeOutcome;
        self.probes.iter().rev().find(|p| {
            p.error.is_some()
                || p.exit_code == Some(-1)
                || matches!(
                    p.outcome,
                    Some(ProbeOutcome::Error | ProbeOutcome::NoResponse)
                )
        })
    }

    /// F1 / `?`: toggle the hotkey-help overlay. Opening from Pty focus
    /// pulls focus back to Tui so Esc/q close the popup instead of
    /// reaching the agent.
    fn toggle_help(&mut self) {
        if self.show_help {
            self.show_help = false;
            self.restore_stashed_focus();
        } else {
            self.stash_focus_for_modal();
            self.show_help = true;
        }
    }

    /// F2: open the probe-details popup on the last failed probe (no-op
    /// when nothing has failed). Pulls focus to Tui so the popup keys work.
    fn open_last_failed(&mut self) {
        if let Some(pid) = self.last_failed_probe().map(|p| p.probe_id) {
            self.stash_focus_for_modal();
            self.show_help = false;
            self.probe_popup = Some(pid);
        }
    }

    /// F3: paste the last failed probe straight into the agent PTY and
    /// hand focus over — same payload as "p" in the probe popup.
    fn paste_last_failed(&mut self) {
        if !self.has_pty {
            return;
        }
        if let Some(text) = self.last_failed_probe().map(probe_paste_text) {
            self.pty_paste_pending = Some(text);
            self.set_input_focus(InputFocus::Pty);
        }
    }

    /// F4: show/hide the probes sidebar. The PTY inner rect depends on it,
    /// so schedule a resize for the render loop.
    fn toggle_sidebar(&mut self) {
        self.show_sidebar = !self.show_sidebar;
        self.pty_resize_pending = true;
    }

    /// Group probes by task in sidebar render order (newest task first),
    /// with derived passed/folded state. Probes without a `task_id` are
    /// excluded — see [`TuiApp::ungrouped_probes`]. Single source of truth
    /// for the sidebar renderer and keyboard navigation.
    pub fn task_groups(&self) -> Vec<TaskGroup<'_>> {
        let mut by_task: std::collections::BTreeMap<(i32, Uuid), Vec<&ProbeResultInfo>> =
            std::collections::BTreeMap::new();
        for p in self.probes.iter() {
            if let Some(tid) = p.task_id {
                by_task.entry((p.task_ordinal, tid)).or_default().push(p);
            }
        }
        by_task
            .into_iter()
            .rev() // newest (highest ordinal) first
            .map(|((ordinal, task_id), probes)| {
                // A task is passed when the scheduler advanced past it (a
                // higher ordinal was seen). Ordinal 0 is a real task.
                // Legacy fallback (no ordinals tracked): all probes Pass.
                let passed = match self.max_task_ordinal {
                    Some(m) => ordinal < m,
                    None => probes.iter().all(|p| {
                        matches!(p.outcome, Some(arena_core::protocol::ProbeOutcome::Pass))
                    }),
                };
                let folded = self.fold_overrides.get(&task_id).copied().unwrap_or(passed);
                let graded: Vec<i64> = probes
                    .iter()
                    .filter_map(|p| p.point_delta.map(|d| d as i64))
                    .collect();
                let points = (!graded.is_empty()).then(|| graded.iter().sum());
                TaskGroup {
                    task_id,
                    ordinal,
                    probes,
                    passed,
                    folded,
                    points,
                }
            })
            .collect()
    }

    /// Probes without a `task_id` (synthetic markers, old-server
    /// back-compat), oldest-first; renderers iterate `.rev()`.
    pub fn ungrouped_probes(&self) -> Vec<&ProbeResultInfo> {
        self.probes.iter().filter(|p| p.task_id.is_none()).collect()
    }

    /// The session retold as a chat transcript, oldest first — the chat
    /// sidebar view renders this directly (mirrors the web player chat).
    /// Tasks in ordinal order; inside a task: the brief (ololo's message),
    /// checks in arrival order with re-runs of the same test collapsed into
    /// their latest state, judge evidence requests retold in plain words,
    /// the player's done-note, then the judge verdicts pinned to that task.
    /// Synthetic markers (member joined), orphan messages, and the session's
    /// status line close the feed.
    pub fn chat_transcript(&self) -> Vec<ChatMsg<'_>> {
        let mut out: Vec<ChatMsg<'_>> = Vec::new();
        let mut groups = self.task_groups();
        groups.reverse(); // task_groups is newest-first; a chat reads down

        let current_ordinal = groups.last().map(|g| g.ordinal);
        let mut placed_verdicts = 0usize;
        let mut placed_notes = 0usize;
        // Notes published before any task was known (`task_ordinal: None` —
        // the agent was still booting) came chronologically first: they
        // open the feed, above TASK #0, instead of trailing it forever.
        for n in &self.done_notes {
            if n.task_ordinal.is_none() {
                out.push(ChatMsg::DoneNote(n));
                placed_notes += 1;
            }
        }
        for g in &groups {
            let newest = g.probes.last().expect("task group is never empty");
            let title = if newest.task_title.is_empty() {
                format!("Task #{}", g.ordinal)
            } else {
                newest.task_title.clone()
            };
            out.push(ChatMsg::TaskHeader {
                ordinal: g.ordinal,
                title,
                points: g.points,
                passed: g.passed,
            });
            // ololo hands out the task: the brief, whole. Every probe of the
            // task carries the same text — say it once, up front, and let
            // the checks below speak only for themselves.
            if !newest.task_description.is_empty() {
                out.push(ChatMsg::Brief {
                    text: &newest.task_description,
                });
            }

            let has_note = self
                .done_notes
                .iter()
                .any(|n| n.task_ordinal == Some(g.ordinal));

            // Collapse re-runs: probes sharing a known test ordinal are the
            // same check re-polled — the newest attempt speaks, `runs`
            // carries the history. Ordinal 0 (old servers) stays per-probe.
            let mut order: Vec<(i32, Uuid)> = Vec::new();
            let mut by_test: HashMap<(i32, Uuid), Vec<&ProbeResultInfo>> = HashMap::new();
            for p in &g.probes {
                let key = if p.test_ordinal > 0 {
                    (p.test_ordinal, Uuid::nil())
                } else {
                    (0, p.probe_id)
                };
                let slot = by_test.entry(key).or_default();
                if slot.is_empty() {
                    order.push(key);
                }
                slot.push(p);
            }
            for key in order {
                let runs = &by_test[&key];
                let latest = *runs.last().expect("run group is never empty");
                // A judge asking for a capture is the judge's message, not a
                // check — say who asks, what for, and whether it landed.
                if let Some(req) = parse_artifact_request(&latest.command) {
                    out.push(ChatMsg::Request {
                        judge: req.judge.to_string(),
                        instruction: req.instruction.to_string(),
                        path: req.path.clone(),
                        delivered: matches!(
                            latest.outcome,
                            Some(arena_core::protocol::ProbeOutcome::Pass)
                        ),
                    });
                    continue;
                }
                // The completion contract polls for the done flag; a "check"
                // failing until the player declares done is the system
                // waiting, not a conversation event. Guidance for the task
                // in play; a settled flag speaks through the done-note.
                if let Some(file) = completion_flag_file(&latest.command) {
                    let flag_passed = matches!(
                        latest.outcome,
                        Some(arena_core::protocol::ProbeOutcome::Pass)
                    );
                    if flag_passed {
                        if !has_note {
                            out.push(ChatMsg::System {
                                text: "task delivered — handing the build to the judges"
                                    .to_string(),
                            });
                        }
                    } else if Some(g.ordinal) == current_ordinal && !g.passed {
                        out.push(ChatMsg::System {
                            text: format!("to finish: write {file} when the build is ready"),
                        });
                    }
                    continue;
                }
                out.push(ChatMsg::Check {
                    probe: latest,
                    runs: runs.len(),
                    question: probe_question(&latest.command),
                });
            }

            for n in &self.done_notes {
                if n.task_ordinal == Some(g.ordinal) {
                    out.push(ChatMsg::DoneNote(n));
                    placed_notes += 1;
                }
            }
            for v in &self.judge_verdicts {
                if v.task_ordinal == Some(g.ordinal) {
                    out.push(ChatMsg::Verdict(v));
                    placed_verdicts += 1;
                }
            }
        }

        // Messages that matched no task group (no probes seen yet, or no
        // ordinal at arrival) still deserve a line — at the end, where the
        // newest traffic lands.
        let grouped: std::collections::HashSet<i32> = groups.iter().map(|g| g.ordinal).collect();
        if placed_notes < self.done_notes.len() {
            for n in &self.done_notes {
                // `None` ordinals already opened the feed above.
                let orphan = matches!(n.task_ordinal, Some(ord) if !grouped.contains(&ord));
                if orphan {
                    out.push(ChatMsg::DoneNote(n));
                }
            }
        }
        if placed_verdicts < self.judge_verdicts.len() {
            for v in &self.judge_verdicts {
                let orphan = match v.task_ordinal {
                    Some(ord) => !grouped.contains(&ord),
                    None => true,
                };
                if orphan {
                    out.push(ChatMsg::Verdict(v));
                }
            }
        }

        // Synthetic markers: teammates joining and old-server probes.
        // Forwarded-input markers are machine traffic, not conversation.
        for p in self.ungrouped_probes() {
            if p.command == "member" {
                out.push(ChatMsg::System {
                    text: p.stdout.trim().to_string(),
                });
            } else if p.command != "input" {
                out.push(ChatMsg::Check {
                    probe: p,
                    runs: 1,
                    question: None,
                });
            }
        }

        // The session's status is the transcript's last word — the same
        // narration the web chat's banner carries.
        use crate::tui::header::Status;
        match self.header.status {
            Status::Complete => out.push(ChatMsg::System {
                text: "session complete — final results are on the session page".to_string(),
            }),
            Status::Cancelled => out.push(ChatMsg::System {
                text: "session cancelled — no more probes will run".to_string(),
            }),
            Status::TasksDone => out.push(ChatMsg::System {
                text: "all your tasks are done — the session ends when every player finishes"
                    .to_string(),
            }),
            _ if self.judging => out.push(ChatMsg::System {
                text: "the judges are reviewing your delivery…".to_string(),
            }),
            _ => {}
        }
        out
    }

    /// The selectable sidebar rows in display order: each task header,
    /// then (when unfolded) its probes newest-first, then ungrouped
    /// probes newest-first.
    fn sidebar_nav(&self) -> Vec<NavTarget> {
        let mut nav: Vec<NavTarget> = Vec::new();
        for g in self.task_groups() {
            nav.push(NavTarget::Task(g.task_id));
            if !g.folded {
                nav.extend(g.probes.iter().rev().map(|p| NavTarget::Probe(p.probe_id)));
            }
        }
        nav.extend(
            self.ungrouped_probes()
                .iter()
                .rev()
                .map(|p| NavTarget::Probe(p.probe_id)),
        );
        nav
    }

    /// Move the sidebar cursor by `delta` rows, clamped. Selects the first
    /// row when nothing is selected (or the selection vanished).
    fn sidebar_move(&mut self, delta: isize) {
        let nav = self.sidebar_nav();
        if nav.is_empty() {
            self.sidebar_cursor = None;
            return;
        }
        let next = match self
            .sidebar_cursor
            .and_then(|cur| nav.iter().position(|t| *t == cur))
        {
            Some(i) => i.saturating_add_signed(delta).min(nav.len() - 1),
            None => 0,
        };
        self.sidebar_cursor = Some(nav[next]);
    }

    /// Enter/Space on the selection: toggle a task's fold, or open the
    /// probe-details popup.
    fn sidebar_activate(&mut self) {
        match self.sidebar_cursor {
            Some(NavTarget::Task(tid)) => {
                let folded_now = self
                    .task_groups()
                    .iter()
                    .find(|g| g.task_id == tid)
                    .map(|g| g.folded)
                    .unwrap_or(false);
                self.fold_overrides.insert(tid, !folded_now);
            }
            Some(NavTarget::Probe(pid)) => self.probe_popup = Some(pid),
            None => {}
        }
    }

    /// Look up a probe by id (first match; synthetic probes share nil).
    pub fn probe_by_id(&self, id: Uuid) -> Option<&ProbeResultInfo> {
        self.probes.iter().find(|p| p.probe_id == id)
    }

    /// Switch input focus. Leaving Tui focus drops the sidebar selection
    /// and closes the probe popup — the probes pane is no longer being
    /// driven, so no stale cursor bar should linger.
    pub fn set_input_focus(&mut self, focus: InputFocus) {
        if focus != InputFocus::Tui {
            self.sidebar_cursor = None;
            self.probe_popup = None;
            self.show_help = false;
            self.chat_input = None;
        }
        self.input_focus = focus;
    }
}

/// A judge's request for a capture, parsed back out of the probe command the
/// game server built for it.
///
/// The request travels as a comment header on a shell one-liner, which is the
/// right shape for the thing that runs it and the wrong shape for the person
/// reading it: what the agent needs is the instruction and the folder, not a
/// `test -n "$(ls -A …)"`.
struct ArtifactRequest<'a> {
    judge: &'a str,
    instruction: &'a str,
    /// Repo-relative folder the files belong in; empty when unparseable.
    path: String,
}

fn parse_artifact_request(command: &str) -> Option<ArtifactRequest<'_>> {
    let header = command
        .lines()
        .next()?
        .strip_prefix("# ARTIFACT REQUEST from ")?;
    let (judge, instruction) = header.split_once(": ")?;
    // "# Save the file(s) (up to 5) under <path>; the ololo CLI commits …"
    let path = command
        .lines()
        .find_map(|l| l.split_once(" under ").map(|(_, rest)| rest))
        .and_then(|rest| rest.split(';').next())
        .unwrap_or("")
        .trim()
        .to_string();
    Some(ArtifactRequest {
        judge,
        instruction: instruction.trim(),
        path,
    })
}

/// The question a quiz probe asked, parsed from its command — a
/// `--data-urlencode "q=…"` for the web contract, a `-q "…"` flag for the
/// CLI contract, or a bare `?q=…`/`&q=…` URL parameter. Mirrors the web
/// chat's extraction, so the chat quotes the question instead of the shell.
fn probe_question(command: &str) -> Option<String> {
    let q = quoted_q(command)
        .or_else(|| flag_q(command))
        .or_else(|| bare_q(command))?;
    // A leading `qid:` prefix is plumbing, not question.
    let q = match q.split_once(':') {
        Some((head, rest))
            if head.len() >= 4 && head.chars().all(|c| c.is_ascii_alphanumeric()) =>
        {
            rest.trim().to_string()
        }
        _ => q,
    };
    (!q.is_empty()).then_some(q)
}

/// `"q=…"` / `'q=…'` — the quoted question of a `--data-urlencode` payload.
fn quoted_q(command: &str) -> Option<String> {
    for (i, _) in command.match_indices("q=") {
        let Some(quote) = command[..i].chars().next_back() else {
            continue;
        };
        if quote != '"' && quote != '\'' {
            continue;
        }
        let rest = &command[i + 2..];
        if let Some(end) = rest.find(quote) {
            let v = rest[..end].trim();
            if !v.is_empty() {
                return Some(v.to_string());
            }
        }
    }
    None
}

/// ` -q "…"` / ` -q '…'` — the CLI quiz contract's question flag.
fn flag_q(command: &str) -> Option<String> {
    let i = command.find(" -q ")?;
    let rest = command[i + 4..].trim_start();
    let quote = rest.chars().next()?;
    if quote != '"' && quote != '\'' {
        return None;
    }
    let rest = &rest[1..];
    let end = rest.find(quote)?;
    let v = rest[..end].trim();
    (!v.is_empty()).then(|| v.to_string())
}

/// `?q=…` / `&q=…` — a bare URL parameter, up to the next delimiter.
fn bare_q(command: &str) -> Option<String> {
    for pat in ["?q=", "&q="] {
        let Some(i) = command.find(pat) else { continue };
        let rest = &command[i + pat.len()..];
        let end = rest
            .find(|c: char| c == '"' || c == '\'' || c == '&' || c.is_whitespace())
            .unwrap_or(rest.len());
        let v = &rest[..end];
        if !v.is_empty() {
            return Some(v.to_string());
        }
    }
    None
}

/// The completion contract's done file, as its polling probe names it —
/// `.ololo/<name>done<…>.md` somewhere in the command. Same shape the CLI's
/// flag watcher matches, so guidance and delivery talk about one file.
fn completion_flag_file(command: &str) -> Option<String> {
    for (i, _) in command.match_indices(".ololo/") {
        let rest = &command[i..];
        let end = rest
            .find(|c: char| !(c.is_ascii_alphanumeric() || "_./-".contains(c)))
            .unwrap_or(rest.len());
        let candidate = &rest[..end];
        if candidate.ends_with(".md") && candidate.contains("done") {
            return Some(candidate.to_string());
        }
    }
    None
}

/// Plain-text rendition of a probe for pasting into the agent PTY —
/// everything the agent needs to react to a failing (or passing) probe,
/// including the task's own brief: the check is meaningless without knowing
/// what the task asked for, and the agent's scrollback may be long past it.
/// Only an artifact request drops it — there the instruction IS the content,
/// and the brief buried it (session TJQJPJ).
/// One chat bubble as text for the agent — the same retelling the bubble
/// renders, without the chrome. Checks reuse the probe paste (command,
/// expected/actual and all); the rest speak in their own words.
pub fn chat_msg_paste_text(m: &ChatMsg<'_>) -> String {
    match m {
        ChatMsg::TaskHeader {
            ordinal,
            title,
            points,
            passed,
        } => {
            let mut out = format!("Task #{ordinal}: {title}");
            if let Some(p) = points {
                out.push_str(&format!(" ({p:+} pts)"));
            }
            if *passed {
                out.push_str(" — passed");
            }
            out.push('\n');
            out
        }
        ChatMsg::Brief { text } => format!("Task brief:\n{text}\n"),
        ChatMsg::Check { probe, .. } => probe_paste_text(probe),
        ChatMsg::Request {
            judge,
            instruction,
            path,
            delivered,
        } => {
            let mut out =
                format!("Artifact request from {judge}\nWhat to capture: {instruction}\n");
            if !path.is_empty() {
                out.push_str(&format!(
                    "Where: save up to 5 files into {path} — the ololo CLI commits and pushes them, do NOT run git.\n"
                ));
            }
            out.push_str(if *delivered {
                "Status: delivered.\n"
            } else {
                "Status: nothing delivered yet — the judge is waiting for this file.\n"
            });
            out
        }
        ChatMsg::DoneNote(n) => format!("My done-note ({}):\n{}\n", n.path, n.text),
        ChatMsg::Verdict(v) => format!(
            "Judge verdict — {} ({:+} pts):\n{}\n",
            v.judge_name, v.point_delta, v.feedback
        ),
        ChatMsg::System { text } => format!("{text}\n"),
    }
}

pub fn probe_paste_text(p: &ProbeResultInfo) -> String {
    use arena_core::protocol::ProbeOutcome;
    let mut task = if p.task_title.is_empty() {
        format!("Task #{}", p.task_ordinal)
    } else {
        p.task_title.clone()
    };
    if p.test_total > 0 {
        task = format!("{task} (probe {}/{})", p.test_ordinal, p.test_total);
    }
    let status = match (p.error.as_ref(), p.exit_code, p.outcome) {
        (Some(_), _, _) => "error",
        (_, Some(-1), _) => "timeout",
        (_, _, Some(ProbeOutcome::Pass)) => "pass",
        (_, _, Some(ProbeOutcome::Error)) => "fail",
        (_, _, Some(ProbeOutcome::NoResponse)) => "no response",
        (_, Some(_), None) => "sent, awaiting grade",
        _ => "waiting",
    };
    // A judge asking for a capture is not a failing check, and reads as one
    // only because it travels as a probe. Say what it is, what to produce and
    // where — and none of the shell that polls for it.
    if let Some(req) = parse_artifact_request(&p.command) {
        let mut out = format!("Artifact request from {} — {task}\n", req.judge);
        if let Some(secs) = p.deadline_secs.filter(|s| *s > 0) {
            out.push_str(&format!(
                "Deliver within {} min.\n",
                secs.div_euclid(60).max(1)
            ));
        }
        out.push_str(&format!("What to capture: {}\n", req.instruction));
        if !req.path.is_empty() {
            out.push_str(&format!(
                "Where: save up to 5 files into {} — the ololo CLI commits and pushes them, do NOT run git.\n",
                req.path
            ));
        }
        out.push_str(match p.outcome {
            Some(ProbeOutcome::Pass) => "Status: delivered.\n",
            _ => "Status: nothing delivered yet — the judge is waiting for this file.\n",
        });
        return out;
    }

    let mut out = format!("Probe result — {task}\nStatus: {status}");
    if let Some(d) = p.point_delta {
        out.push_str(&format!(" ({d:+} pts)"));
    }
    out.push('\n');
    if !p.task_description.is_empty() {
        out.push_str(&format!("Task description: {}\n", p.task_description));
    }
    if !p.test_label.is_empty() {
        out.push_str(&format!("Check: {}\n", p.test_label));
    }
    if !p.test_description.is_empty() {
        out.push_str(&format!("What it verifies: {}\n", p.test_description));
    }
    if !p.command.is_empty() {
        out.push_str(&format!("Command: {}\n", p.command));
    }
    if let Some(exp) = p.graded_expected.as_ref().or(p.expected_answer.as_ref()) {
        out.push_str(&format!("Expected: {exp}\n"));
    } else if !p.answer_template.is_empty() {
        out.push_str(&format!("Expected (template): {}\n", p.answer_template));
    }
    let stdout = p.stdout.trim();
    if !stdout.is_empty() {
        out.push_str(&format!("Actual: {stdout}\n"));
    }
    if let Some(e) = &p.error {
        out.push_str(&format!("Error: {e}\n"));
    }
    out
}

impl TuiApp {
    /// Re-validate the agent at SessionStarted. On miss, set
    /// QuitReason::PickerFailed.
    pub fn on_session_started(&mut self, agent_command: &str) {
        let picked = crate::tui::agent_picker::PickedAgent {
            command: agent_command.to_string(),
            argv: vec![],
            source: crate::tui::agent_picker::AgentSource::Other,
        };
        if let Err(e) = crate::tui::agent_picker::revalidate(&picked) {
            self.header.apply(crate::tui::event::HeaderDelta::Error {
                message: format!("{e}"),
            });
            self.should_quit = Some(QuitReason::PickerFailed(agent_command.to_string()));
        }
    }
}

#[cfg(test)]
mod tests;
