//! F3: the paste menu — everything the session knows that the hosted agent
//! may need, one keystroke away.
//!
//! The menu lists, most urgent first: the judges' open artifact requests,
//! the current task's brief, the check that fails now, every check of the
//! task with its result, the code-health level, the judges' verdicts, and
//! the briefs of earlier tasks. Each row says what it hands over; what the
//! agent already has is marked, and the cursor starts on the first thing it
//! does not have.
//!
//! That short list is the `All` tab. The others hold one kind in full:
//! every task's brief, every verdict, every request (closed ones too),
//! every check of the task, health.

use super::health;
use super::{
    ChatMsg, InputFocus, TuiApp, chat_msg_paste_text, completion_flag_file, parse_artifact_request,
    probe_paste_text_with, probe_question, request_instruction, request_key, time_left,
};
use crate::tui::event::ProbeResultInfo;
use uuid::Uuid;

/// What F3 offers to paste.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PasteKind {
    /// A judge's open artifact request.
    Request,
    /// The current task's brief.
    Brief,
    /// The check that fails now.
    FailedCheck,
    /// Every check of the current task with its latest result.
    Checks,
    /// The code-health level and what it is made of.
    Health,
    /// The judges' verdicts on the latest judged task.
    Verdicts,
    /// The brief of a task already left behind.
    EarlierBrief,
    /// One check of the current task, whatever its result.
    Check,
    /// One judge's verdict.
    Verdict,
}

/// The menu's tabs: everything worth pasting now, or one kind in full.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PasteTab {
    /// The short list: what is open now, most urgent first.
    All,
    /// Every task's brief.
    Tasks,
    /// Every verdict, per task and one by one.
    Judges,
    /// Every artifact request, open or closed.
    Requests,
    /// The current task's checks, together and one by one.
    Probes,
    Health,
}

impl PasteTab {
    pub const ALL: [PasteTab; 6] = [
        PasteTab::All,
        PasteTab::Tasks,
        PasteTab::Judges,
        PasteTab::Requests,
        PasteTab::Probes,
        PasteTab::Health,
    ];

    pub fn title(self) -> &'static str {
        match self {
            PasteTab::All => "All",
            PasteTab::Tasks => "Tasks",
            PasteTab::Judges => "Judges",
            PasteTab::Requests => "Requests",
            PasteTab::Probes => "Probes",
            PasteTab::Health => "Health",
        }
    }

    fn index(self) -> usize {
        Self::ALL.iter().position(|t| *t == self).unwrap_or(0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PasteSource {
    Probe(Uuid),
    /// The brief carried by this probe's task.
    Brief(Uuid),
    /// Text composed when the menu opened.
    Text(String),
}

#[derive(Debug, Clone)]
pub struct PasteItem {
    pub kind: PasteKind,
    pub label: String,
    /// What choosing the row hands the agent, in a line.
    pub hint: String,
    /// Artifact requests: when they close.
    pub deadline: Option<std::time::Instant>,
    pub source: PasteSource,
    /// What the item hands over, as [`TuiApp::pasted`] remembers it.
    pub key: String,
    /// The agent already has it: pasted before, from F3, a chat bubble or
    /// a probe's details.
    pub pasted: bool,
}

/// What a paste hands over, whichever way it went — F3, a chat bubble, a
/// probe's details — so the same thing is recognised as already pasted:
/// a task by its brief, an artifact request by its judge and ask, a check
/// by its probe.
pub(super) fn brief_key(brief: &str) -> String {
    format!("brief\n{}", brief.trim())
}

pub(super) fn request_paste_key(judge: &str, instruction: &str) -> String {
    format!("request\n{judge}\n{}", instruction.trim())
}

pub(super) fn check_key(probe_id: Uuid) -> String {
    format!("check\n{probe_id}")
}

/// A chat bubble's paste key, when it is one of the things F3 offers.
pub(crate) fn chat_msg_key(m: &ChatMsg<'_>) -> Option<String> {
    match m {
        ChatMsg::Brief { text } => Some(brief_key(text)),
        ChatMsg::Check { probe, .. } => Some(check_key(probe.probe_id)),
        ChatMsg::Request {
            judge, instruction, ..
        } => Some(request_paste_key(judge, instruction)),
        _ => None,
    }
}

#[derive(Debug, Clone)]
pub struct PastePicker {
    pub tab: PasteTab,
    /// The rows of `tab`.
    pub items: Vec<PasteItem>,
    pub cursor: usize,
    /// Rows in each tab, in [`PasteTab::ALL`] order, as of opening — a tab
    /// with none is skipped when switching.
    pub counts: [usize; PasteTab::ALL.len()],
}

fn first_line(text: &str) -> String {
    text.lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("")
        .to_string()
}

/// Sorts requests without a deadline after every one that has one.
fn far_future() -> std::time::Instant {
    std::time::Instant::now() + std::time::Duration::from_secs(10 * 365 * 24 * 3600)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum CheckState {
    Pass,
    Fail,
    Waiting,
}

fn check_state(p: &ProbeResultInfo) -> CheckState {
    use arena_core::protocol::ProbeOutcome;
    match p.outcome {
        Some(ProbeOutcome::Pass) => CheckState::Pass,
        Some(ProbeOutcome::Error | ProbeOutcome::NoResponse) => CheckState::Fail,
        None if p.error.is_some() || p.exit_code == Some(-1) => CheckState::Fail,
        None => CheckState::Waiting,
    }
}

fn check_name(p: &ProbeResultInfo) -> String {
    if !p.test_label.is_empty() {
        return p.test_label.clone();
    }
    probe_question(&p.command).unwrap_or_else(|| first_line(&p.command))
}

fn brief_text(p: &ProbeResultInfo) -> String {
    format!(
        "Task #{}: {}\nTask brief:\n{}\n",
        p.task_ordinal, p.task_title, p.task_description
    )
}

impl TuiApp {
    /// F3: open the paste menu. Nothing is sent until the player chooses —
    /// ⏎ takes the row under the cursor, which starts on the most urgent
    /// thing the agent does not have yet.
    pub(super) fn open_paste_menu(&mut self) {
        if !self.has_pty {
            return;
        }
        let items = self.paste_items();
        if items.is_empty() {
            return;
        }
        let cursor = items.iter().position(|i| !i.pasted).unwrap_or(0);
        let counts = PasteTab::ALL.map(|t| self.paste_items_of(t).len());
        self.stash_focus_for_modal();
        self.show_help = false;
        self.paste_picker = Some(PastePicker {
            tab: PasteTab::All,
            items,
            cursor,
            counts,
        });
    }

    /// The rows of one tab. `All` is the short list of [`Self::paste_items`];
    /// every other tab is its kind in full.
    pub fn paste_items_of(&self, tab: PasteTab) -> Vec<PasteItem> {
        match tab {
            PasteTab::All => self.paste_items(),
            PasteTab::Tasks => {
                let mut items: Vec<PasteItem> = self.brief_item().into_iter().collect();
                items.extend(self.earlier_brief_items());
                items
            }
            PasteTab::Judges => self.judge_items(),
            PasteTab::Requests => self.request_items(true),
            PasteTab::Probes => {
                let mut items: Vec<PasteItem> = self.failed_check_item().into_iter().collect();
                items.extend(self.checks_item());
                items.extend(self.check_items());
                items
            }
            PasteTab::Health => self.health_item().into_iter().collect(),
        }
    }

    /// Move `step` tabs right (or left), past the tabs with nothing in them.
    fn switch_paste_tab(&mut self, step: isize) {
        let Some(picker) = self.paste_picker.as_ref() else {
            return;
        };
        let n = PasteTab::ALL.len() as isize;
        let mut at = picker.tab.index() as isize;
        let tab = loop {
            at = (at + step).rem_euclid(n);
            let tab = PasteTab::ALL[at as usize];
            if tab == picker.tab {
                return;
            }
            if picker.counts[at as usize] > 0 {
                break tab;
            }
        };
        let items = self.paste_items_of(tab);
        let cursor = items.iter().position(|i| !i.pasted).unwrap_or(0);
        if let Some(picker) = self.paste_picker.as_mut() {
            picker.tab = tab;
            picker.items = items;
            picker.cursor = cursor;
        }
    }

    fn item(
        &self,
        kind: PasteKind,
        label: String,
        hint: &str,
        source: PasteSource,
        key: String,
    ) -> PasteItem {
        PasteItem {
            kind,
            label,
            hint: hint.to_string(),
            deadline: None,
            source,
            pasted: self.pasted.contains(&key),
            key,
        }
    }

    /// Everything F3 could paste, most urgent first: open artifact requests
    /// (soonest deadline first), the current task's brief, the last failed
    /// check, the task's check results, code health, the judges' verdicts,
    /// then the briefs of earlier tasks (newest first).
    pub fn paste_items(&self) -> Vec<PasteItem> {
        let mut items = self.request_items(false);
        items.extend(self.brief_item());
        items.extend(self.failed_check_item());
        items.extend(self.checks_item());
        items.extend(self.health_item());
        items.extend(self.verdicts_item());
        items.extend(self.earlier_brief_items());
        items
    }

    fn brief_item(&self) -> Option<PasteItem> {
        let p = self.probes.iter().rev().find(|p| {
            Some(p.task_ordinal) == self.max_task_ordinal && !p.task_description.is_empty()
        })?;
        Some(self.item(
            PasteKind::Brief,
            format!("Task #{}: {} — the brief", p.task_ordinal, p.task_title),
            "what the current task asks for, in full",
            PasteSource::Brief(p.probe_id),
            brief_key(&p.task_description),
        ))
    }

    fn failed_check_item(&self) -> Option<PasteItem> {
        let p = self
            .last_failed_probe()
            .filter(|p| parse_artifact_request(&p.command).is_none())?;
        let what = if p.test_label.is_empty() {
            first_line(&p.command)
        } else {
            p.test_label.clone()
        };
        Some(self.item(
            PasteKind::FailedCheck,
            format!("Failed check: {what}"),
            "the check that fails now: its command, what was expected, what came back",
            PasteSource::Probe(p.probe_id),
            check_key(p.probe_id),
        ))
    }

    /// The judges' artifact requests, soonest deadline first. `closed`
    /// adds the delivered and the expired ones, after the open.
    fn request_items(&self, closed: bool) -> Vec<PasteItem> {
        let mut requests: Vec<PasteItem> = Vec::new();
        let mut done: Vec<PasteItem> = Vec::new();
        let mut seen: Vec<String> = Vec::new();
        for p in self.probes.iter().rev() {
            let Some(req) = parse_artifact_request(&p.command) else {
                continue;
            };
            let key = request_key(req.judge, req.instruction);
            if seen.contains(&key) {
                continue;
            }
            seen.push(key);
            let deadline = self.request_deadline(&req);
            let state = if self.request_delivered(&[p], &req.path) {
                Some("delivered")
            } else if time_left(deadline).is_some_and(|l| l.is_zero()) {
                Some("expired")
            } else {
                None
            };
            if state.is_some() && !closed {
                continue;
            }
            let instruction = request_instruction(&p.test_description, req.instruction);
            let ask = first_line(&instruction);
            let (label, hint) = match state {
                Some(state) => (
                    format!("[{state}] {} asked: {ask}", req.judge),
                    "a closed request — what the judge wanted and where it went",
                ),
                None => (
                    format!("{} asks: {ask}", req.judge),
                    "a judge waits for a file: what to capture, where to save it, by when",
                ),
            };
            let mut item = self.item(
                PasteKind::Request,
                label,
                hint,
                PasteSource::Probe(p.probe_id),
                request_paste_key(req.judge, &instruction),
            );
            let list = if state.is_none() {
                item.deadline = deadline;
                &mut requests
            } else {
                &mut done
            };
            list.push(item);
        }
        requests.sort_by_key(|i| i.deadline.unwrap_or_else(far_future));
        requests.extend(done);
        requests
    }

    /// The current task's checks, re-runs collapsed into their latest
    /// result; the judges' requests are not checks.
    fn latest_checks(&self) -> Vec<&ProbeResultInfo> {
        let Some(current) = self.max_task_ordinal else {
            return Vec::new();
        };
        let mut latest: Vec<&ProbeResultInfo> = Vec::new();
        let of_task = self
            .probes
            .iter()
            .filter(|p| p.task_id.is_some() && p.task_ordinal == current);
        for p in of_task {
            if parse_artifact_request(&p.command).is_some() {
                continue;
            }
            let same = |q: &&ProbeResultInfo| {
                p.test_ordinal > 0
                    && q.test_ordinal == p.test_ordinal
                    && q.test_label == p.test_label
            };
            match latest.iter().position(same) {
                Some(i) => latest[i] = p,
                None => latest.push(p),
            }
        }
        latest
    }

    /// The current task's checks one by one, failing first.
    fn check_items(&self) -> Vec<PasteItem> {
        let mut checks = self.latest_checks();
        checks.retain(|p| completion_flag_file(&p.command).is_none());
        checks.sort_by_key(|p| match check_state(p) {
            CheckState::Fail => 0,
            CheckState::Waiting => 1,
            CheckState::Pass => 2,
        });
        checks
            .into_iter()
            .map(|p| {
                let (state, hint) = match check_state(p) {
                    CheckState::Pass => ("pass", "a passing check: its command and what came back"),
                    CheckState::Fail => (
                        "FAIL",
                        "a failing check: its command, what was expected, what came back",
                    ),
                    CheckState::Waiting => ("waiting", "a check still waiting for its result"),
                };
                self.item(
                    PasteKind::Check,
                    format!("{state}: {}", check_name(p)),
                    hint,
                    PasteSource::Probe(p.probe_id),
                    check_key(p.probe_id),
                )
            })
            .collect()
    }

    /// The current task's checks, re-runs collapsed into their latest
    /// result — the whole picture, where the failed-check row is one line
    /// of it.
    fn checks_item(&self) -> Option<PasteItem> {
        let current = self.max_task_ordinal?;
        let latest = self.latest_checks();
        if latest.is_empty() {
            return None;
        }
        let count = |s: CheckState| latest.iter().filter(|p| check_state(p) == s).count();
        let (passed, failed, waiting) = (
            count(CheckState::Pass),
            count(CheckState::Fail),
            count(CheckState::Waiting),
        );
        let mut tally = format!("{passed} passed, {failed} failed");
        if waiting > 0 {
            tally.push_str(&format!(", {waiting} waiting"));
        }
        let newest = latest.last()?;
        let title = if newest.task_title.is_empty() {
            format!("Task #{current}")
        } else {
            format!("Task #{current}: {}", newest.task_title)
        };
        let mut text = format!("Check results — {title} ({tally})\n");
        for p in &latest {
            if let Some(file) = completion_flag_file(&p.command) {
                text.push_str(&match check_state(p) {
                    CheckState::Pass => format!("- done flag {file}: written\n"),
                    _ => format!(
                        "- done flag {file}: not written yet — write it when the task is complete\n"
                    ),
                });
                continue;
            }
            let name = check_name(p);
            match check_state(p) {
                CheckState::Pass => text.push_str(&format!("- pass: {name}\n")),
                CheckState::Waiting => text.push_str(&format!("- waiting: {name}\n")),
                CheckState::Fail => {
                    text.push_str(&format!("- FAIL: {name}\n"));
                    if !p.test_description.is_empty() {
                        text.push_str(&format!("    What it verifies: {}\n", p.test_description));
                    }
                    if !p.command.is_empty() {
                        text.push_str(&format!("    Command: {}\n", p.command));
                    }
                    if let Some(exp) = p.graded_expected.as_ref().or(p.expected_answer.as_ref()) {
                        text.push_str(&format!("    Expected: {exp}\n"));
                    }
                    let stdout = p.stdout.trim();
                    if !stdout.is_empty() {
                        text.push_str(&format!("    Actual: {stdout}\n"));
                    }
                    if let Some(e) = &p.error {
                        text.push_str(&format!("    Error: {e}\n"));
                    }
                }
            }
        }
        let key = format!("checks\n{text}");
        Some(self.item(
            PasteKind::Checks,
            format!("Check results: {tally}"),
            "every check of the current task with its latest result; failures in detail",
            PasteSource::Text(text),
            key,
        ))
    }

    fn health_item(&self) -> Option<PasteItem> {
        let note = self.health.as_ref()?;
        let tests = self.tests.as_ref();
        let view = health::view(note, tests);
        let text = health::paste_text(note, tests);
        let key = format!("health\n{text}");
        Some(self.item(
            PasteKind::Health,
            format!("Code health: {}", health::headline(&view)),
            &health::hint(note, tests),
            PasteSource::Text(text),
            key,
        ))
    }

    /// The verdicts on the latest task the judges have spoken on.
    fn verdicts_item(&self) -> Option<PasteItem> {
        self.verdicts_item_of(self.judge_verdicts.last()?.task_ordinal)
    }

    /// The Judges tab: the verdicts of every judged task together (newest
    /// task first), then each verdict on its own (newest first).
    fn judge_items(&self) -> Vec<PasteItem> {
        let mut tasks: Vec<Option<i32>> = Vec::new();
        for v in self.judge_verdicts.iter().rev() {
            if !tasks.contains(&v.task_ordinal) {
                tasks.push(v.task_ordinal);
            }
        }
        let mut items: Vec<PasteItem> = tasks
            .into_iter()
            .filter_map(|o| self.verdicts_item_of(o))
            .collect();
        for v in self.judge_verdicts.iter().rev() {
            let task = v
                .task_ordinal
                .map_or_else(String::new, |o| format!(" on task #{o}"));
            let text = chat_msg_paste_text(&ChatMsg::Verdict(v));
            let key = format!("verdict\n{text}");
            items.push(self.item(
                PasteKind::Verdict,
                format!("{}{task}: {:+} pts", v.judge_name, v.point_delta),
                &first_line(&v.feedback),
                PasteSource::Text(text),
                key,
            ));
        }
        items
    }

    fn verdicts_item_of(&self, ordinal: Option<i32>) -> Option<PasteItem> {
        let verdicts: Vec<_> = self
            .judge_verdicts
            .iter()
            .filter(|v| v.task_ordinal == ordinal)
            .collect();
        if verdicts.is_empty() {
            return None;
        }
        let points: i64 = verdicts.iter().map(|v| i64::from(v.point_delta)).sum();
        let task = ordinal.map_or_else(|| "the task".to_string(), |o| format!("task #{o}"));
        let mut text = format!(
            "Judges' verdicts on {task} — {} so far, {points:+} pts\n",
            verdicts.len()
        );
        for v in &verdicts {
            text.push('\n');
            text.push_str(&chat_msg_paste_text(&ChatMsg::Verdict(v)));
        }
        let key = format!("verdicts\n{task}\n{}", verdicts.len());
        Some(self.item(
            PasteKind::Verdicts,
            format!(
                "Judges' verdicts on {task}: {} ({points:+} pts)",
                verdicts.len()
            ),
            "what the judges said and what they took points for — feedback for the next task",
            PasteSource::Text(text),
            key,
        ))
    }

    fn earlier_brief_items(&self) -> Vec<PasteItem> {
        let Some(current) = self.max_task_ordinal else {
            return Vec::new();
        };
        self.task_groups()
            .iter()
            .filter(|g| g.ordinal < current)
            .filter_map(|g| {
                let p = g
                    .probes
                    .iter()
                    .rev()
                    .find(|p| !p.task_description.is_empty())?;
                Some(self.item(
                    PasteKind::EarlierBrief,
                    format!("Task #{}: {} — the brief", p.task_ordinal, p.task_title),
                    "an earlier task's brief — to recall what was asked before",
                    PasteSource::Brief(p.probe_id),
                    brief_key(&p.task_description),
                ))
            })
            .collect()
    }

    /// Queue an item's text for the agent PTY and hand focus over.
    fn paste_item(&mut self, item: &PasteItem) {
        let text = match &item.source {
            PasteSource::Probe(id) => self
                .probe_by_id(*id)
                .map(|p| probe_paste_text_with(p, time_left(item.deadline))),
            PasteSource::Brief(id) => self.probe_by_id(*id).map(brief_text),
            PasteSource::Text(text) => Some(text.clone()),
        };
        if let Some(text) = text {
            self.pasted.insert(item.key.clone());
            self.pty_paste_pending = Some(text);
            self.focus_return = None; // explicit hand-over wins
            self.set_input_focus(InputFocus::Pty);
        }
    }

    /// Keys while the F3 menu is open: move, switch tabs (←/→ or Tab),
    /// choose (⏎ or its number), close. F3 again closes it too.
    pub(super) fn picker_key(&mut self, code: crossterm::event::KeyCode) {
        use crossterm::event::KeyCode;
        let Some(picker) = self.paste_picker.as_mut() else {
            return;
        };
        let n = picker.items.len();
        let chosen = match code {
            KeyCode::Esc | KeyCode::Char('q') => {
                self.close_paste_menu();
                return;
            }
            KeyCode::Up | KeyCode::Char('k') => {
                picker.cursor = (picker.cursor + n - 1) % n.max(1);
                return;
            }
            KeyCode::Down | KeyCode::Char('j') => {
                picker.cursor = (picker.cursor + 1) % n.max(1);
                return;
            }
            KeyCode::Right | KeyCode::Char('l') | KeyCode::Tab => {
                self.switch_paste_tab(1);
                return;
            }
            KeyCode::Left | KeyCode::Char('h') | KeyCode::BackTab => {
                self.switch_paste_tab(-1);
                return;
            }
            KeyCode::Home => {
                picker.cursor = 0;
                return;
            }
            KeyCode::End => {
                picker.cursor = n.saturating_sub(1);
                return;
            }
            KeyCode::Enter | KeyCode::Char(' ') | KeyCode::Char('p') if n > 0 => picker.cursor,
            KeyCode::Char(c) if c.is_ascii_digit() && c != '0' => {
                let i = (c as u8 - b'1') as usize;
                if i >= n {
                    return;
                }
                i
            }
            _ => return,
        };
        let item = picker.items[chosen].clone();
        self.paste_picker = None;
        self.paste_item(&item);
    }

    pub(super) fn close_paste_menu(&mut self) {
        self.paste_picker = None;
        self.restore_stashed_focus();
    }
}
