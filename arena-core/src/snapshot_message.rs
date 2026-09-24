//! The commit-message format of ololo snapshots — one grammar for the CLI
//! that writes it and for every reader: the judge tooling that greps for
//! the task-final commit, the history API, the frontend's task attribution,
//! and the health checkpoints that derive a commit's task from the log.
//!
//! # Subject
//!
//! ```text
//! <kind>(<task uuid>): <text>       a commit addressed to a task
//! <kind>: <text>                    no task known yet
//! ololo snapshot: <text>            session start / final
//! ```
//!
//! Kinds and what they mark:
//!
//! | subject                              | meaning                                      |
//! |--------------------------------------|----------------------------------------------|
//! | `start(<id>): <title>`               | task start marker (empty commit)             |
//! | `probe(<id>): #<seq> <title>`        | the tree when probe `seq` was dispatched      |
//! | `wip(<id>): checkpoint`              | server-requested checkpoint of an open task  |
//! | `flag(<id>): <file>`                 | completion flag committed                    |
//! | `artifact(<id>): sync`               | `.ololo/artifacts/**` changed                |
//! | `memory(<id>): sources @ <iso>`      | AGENTS.md / README.md changed                |
//! | `tests(<id>): #<seq> <summary>`      | the test run after probe `seq`: its log      |
//! | `feat(<id>): <title>`                | **task done marker**: the task's final tree  |
//!
//! `feat(<id>)` is load-bearing: `judging::task_commit::resolve_task_commit`
//! greps for it to find the task's final snapshot, so it appears on that
//! one commit only — and never inside a trailer value.
//!
//! # Trailers (format 1)
//!
//! A blank line, then git trailers (`git interpret-trailers` reads them):
//!
//! ```text
//! Ololo-Format: 1
//! Ololo-Session: <session uuid>
//! Ololo-Participant: <player uuid>
//! Ololo-Task: <task uuid>
//! Ololo-Task-Title: <title>
//! Ololo-Probe: <probe uuid>            probe commits
//! Ololo-Probe-Seq: <n>                 probe commits
//! Ololo-Outcome: completed|deadline    feat commits
//! Ololo-Timestamp: <RFC 3339, UTC>
//! ```
//!
//! Messages written before the trailers existed (format 0: the subject
//! alone) parse the same way; the task id then comes from the subject.
//!
//! # Task ranges
//!
//! Every commit between a task's start marker and its `feat` belongs to
//! that task — see [`task_ranges`]. Histories without start markers open a
//! task at its first addressed commit.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use uuid::Uuid;

/// The trailer-block format this module writes.
pub const FORMAT_VERSION: u32 = 1;

pub const TRAILER_FORMAT: &str = "Ololo-Format";
pub const TRAILER_SESSION: &str = "Ololo-Session";
pub const TRAILER_PARTICIPANT: &str = "Ololo-Participant";
pub const TRAILER_TASK: &str = "Ololo-Task";
pub const TRAILER_TASK_TITLE: &str = "Ololo-Task-Title";
pub const TRAILER_PROBE: &str = "Ololo-Probe";
pub const TRAILER_PROBE_SEQ: &str = "Ololo-Probe-Seq";
pub const TRAILER_OUTCOME: &str = "Ololo-Outcome";
pub const TRAILER_TIMESTAMP: &str = "Ololo-Timestamp";

/// `Ololo-Outcome` values a `feat` commit carries.
pub const OUTCOME_COMPLETED: &str = "completed";
pub const OUTCOME_DEADLINE: &str = "deadline";

/// Longest title (or other free-text trailer value) written into a message.
pub const MAX_TITLE_CHARS: usize = 120;

/// What a snapshot commit marks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Kind {
    /// The task's final tree — the done marker.
    Feat,
    Wip,
    Flag,
    Artifact,
    Memory,
    /// The log of the project's test run after a probe, under
    /// `.ololo/probes/`.
    Tests,
    /// Task start marker.
    Start,
    /// The tree at a probe dispatch.
    Probe,
    /// `ololo snapshot: …` — session start / final.
    Session,
    /// An addressed kind this reader does not know (a newer client).
    Other(String),
    /// Not a snapshot message at all.
    Unknown,
}

impl Kind {
    fn from_name(name: &str) -> Kind {
        match name {
            "feat" => Kind::Feat,
            "wip" => Kind::Wip,
            "flag" => Kind::Flag,
            "artifact" => Kind::Artifact,
            "memory" => Kind::Memory,
            "tests" => Kind::Tests,
            "start" => Kind::Start,
            "probe" => Kind::Probe,
            other => Kind::Other(other.to_string()),
        }
    }

    pub fn name(&self) -> &str {
        match self {
            Kind::Feat => "feat",
            Kind::Wip => "wip",
            Kind::Flag => "flag",
            Kind::Artifact => "artifact",
            Kind::Memory => "memory",
            Kind::Tests => "tests",
            Kind::Start => "start",
            Kind::Probe => "probe",
            Kind::Session => "ololo snapshot",
            Kind::Other(name) => name,
            Kind::Unknown => "",
        }
    }
}

/// The structured fields of a message. Every field is optional: a format-0
/// message has none, and a reader must not assume a newer client wrote all.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Trailers {
    pub format: Option<u32>,
    pub session: Option<Uuid>,
    pub participant: Option<Uuid>,
    pub task: Option<Uuid>,
    pub task_title: Option<String>,
    pub probe: Option<Uuid>,
    pub probe_seq: Option<u32>,
    pub outcome: Option<String>,
    pub timestamp: Option<DateTime<Utc>>,
}

impl Trailers {
    fn is_empty(&self) -> bool {
        *self == Trailers::default()
    }

    /// The trailer block as lines, in a fixed order, omitting absent fields.
    fn lines(&self) -> Vec<String> {
        let mut out = Vec::new();
        let mut push = |key: &str, value: Option<String>| {
            if let Some(v) = value {
                out.push(format!("{key}: {}", single_line(&v)));
            }
        };
        push(TRAILER_FORMAT, self.format.map(|f| f.to_string()));
        push(TRAILER_SESSION, self.session.map(|u| u.to_string()));
        push(TRAILER_PARTICIPANT, self.participant.map(|u| u.to_string()));
        push(TRAILER_TASK, self.task.map(|u| u.to_string()));
        push(
            TRAILER_TASK_TITLE,
            self.task_title.as_deref().map(sanitize_title),
        );
        push(TRAILER_PROBE, self.probe.map(|u| u.to_string()));
        push(TRAILER_PROBE_SEQ, self.probe_seq.map(|n| n.to_string()));
        push(TRAILER_OUTCOME, self.outcome.as_deref().map(sanitize_title));
        push(
            TRAILER_TIMESTAMP,
            self.timestamp
                .map(|t| t.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)),
        );
        out
    }
}

/// A parsed snapshot commit message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotMessage {
    pub kind: Kind,
    /// The task id in the subject's `kind(<id>)`, when addressed.
    pub subject_task: Option<Uuid>,
    /// The subject text after `kind(<id>): ` (or the whole first line for
    /// [`Kind::Unknown`]).
    pub subject: String,
    pub trailers: Trailers,
}

impl SnapshotMessage {
    /// Parse any commit message. Never fails: a message that is not a
    /// snapshot message comes back as [`Kind::Unknown`] with its first line.
    pub fn parse(message: &str) -> SnapshotMessage {
        let first_line = message.lines().next().unwrap_or("").trim_end();
        let (kind, subject_task, subject) = parse_subject(first_line);
        let trailers = parse_trailers(message);
        SnapshotMessage {
            kind,
            subject_task,
            subject,
            trailers,
        }
    }

    /// The task this commit is addressed to: the trailer when present,
    /// otherwise the subject.
    pub fn task_id(&self) -> Option<Uuid> {
        self.trailers.task.or(self.subject_task)
    }

    pub fn is_task_start(&self) -> bool {
        self.kind == Kind::Start
    }

    /// `feat(<id>)`: the task's final tree.
    pub fn is_task_done(&self) -> bool {
        self.kind == Kind::Feat
    }
}

/// `kind(<id>): text` when `task` is known, `kind: text` otherwise. The
/// text goes through [`sanitize_title`], so a title cannot smuggle a
/// second `feat(` into the subject.
pub fn subject(kind: &Kind, task: Option<Uuid>, text: &str) -> String {
    let text = sanitize_title(text);
    match (kind, task) {
        (Kind::Session, _) | (Kind::Unknown, _) => format!("ololo snapshot: {text}"),
        (kind, Some(id)) => format!("{}({id}): {text}", kind.name()),
        (kind, None) => format!("{}: {text}", kind.name()),
    }
}

/// The full message: subject, blank line, trailer block. The block is left
/// out when every trailer is absent, so a caller who has nothing to say
/// writes the format-0 message.
pub fn format(subject: &str, trailers: &Trailers) -> String {
    let subject = single_line(subject);
    if trailers.is_empty() {
        return subject;
    }
    format!("{subject}\n\n{}", trailers.lines().join("\n"))
}

/// A title as it may appear in a subject or a trailer value: one line,
/// trimmed, capped at [`MAX_TITLE_CHARS`], free of the record/unit
/// separators the history API splits `git log` output on, and never
/// containing `feat(` (which would match the task-final grep).
pub fn sanitize_title(title: &str) -> String {
    let mut out: String = single_line(title).chars().take(MAX_TITLE_CHARS).collect();
    while out.contains("feat(") {
        out = out.replace("feat(", "feat [");
    }
    out.trim().to_string()
}

fn single_line(text: &str) -> String {
    text.chars()
        .map(|c| match c {
            '\n' | '\r' | '\t' | '\u{1e}' | '\u{1f}' => ' ',
            c => c,
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn parse_subject(line: &str) -> (Kind, Option<Uuid>, String) {
    if let Some(text) = line.strip_prefix("ololo snapshot: ") {
        return (Kind::Session, None, text.to_string());
    }
    // `<name>(<uuid>): <text>` or `<name>: <text>`, where name is lowercase
    // ascii letters only (a conventional-commit scope would be `feat(x)`,
    // which fails the uuid check and lands in Unknown).
    let name_end = line.bytes().take_while(u8::is_ascii_lowercase).count();
    if name_end == 0 {
        return (Kind::Unknown, None, line.to_string());
    }
    let (name, rest) = line.split_at(name_end);
    if let Some(rest) = rest.strip_prefix("(") {
        let Some(close) = rest.find("): ") else {
            return (Kind::Unknown, None, line.to_string());
        };
        let Ok(id) = Uuid::parse_str(&rest[..close]) else {
            return (Kind::Unknown, None, line.to_string());
        };
        return (
            Kind::from_name(name),
            Some(id),
            rest[close + 3..].to_string(),
        );
    }
    if let Some(text) = rest.strip_prefix(": ") {
        return (Kind::from_name(name), None, text.to_string());
    }
    (Kind::Unknown, None, line.to_string())
}

/// The last paragraph of the message when every line of it is a
/// `Key: value` trailer; `Ololo-*` keys are read, the rest ignored.
fn parse_trailers(message: &str) -> Trailers {
    let mut trailers = Trailers::default();
    let text = message.trim_end();
    let Some((_, block)) = text.rsplit_once("\n\n") else {
        return trailers;
    };
    let lines: Vec<&str> = block
        .lines()
        .map(str::trim_end)
        .filter(|l| !l.is_empty())
        .collect();
    if lines.is_empty() || !lines.iter().all(|l| is_trailer_line(l)) {
        return trailers;
    }
    for line in lines {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let value = value.trim();
        match key.trim() {
            TRAILER_FORMAT => trailers.format = value.parse().ok(),
            TRAILER_SESSION => trailers.session = Uuid::parse_str(value).ok(),
            TRAILER_PARTICIPANT => trailers.participant = Uuid::parse_str(value).ok(),
            TRAILER_TASK => trailers.task = Uuid::parse_str(value).ok(),
            TRAILER_TASK_TITLE => trailers.task_title = Some(value.to_string()),
            TRAILER_PROBE => trailers.probe = Uuid::parse_str(value).ok(),
            TRAILER_PROBE_SEQ => trailers.probe_seq = value.parse().ok(),
            TRAILER_OUTCOME => trailers.outcome = Some(value.to_string()),
            TRAILER_TIMESTAMP => {
                trailers.timestamp = DateTime::parse_from_rfc3339(value)
                    .ok()
                    .map(|t| t.with_timezone(&Utc));
            }
            _ => {}
        }
    }
    trailers
}

fn is_trailer_line(line: &str) -> bool {
    let Some((key, value)) = line.split_once(':') else {
        return false;
    };
    !key.is_empty()
        && key.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
        && value.starts_with(' ')
}

/// One commit of a first-parent log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogEntry {
    pub sha: String,
    /// The full message (`%B`), or at least the subject.
    pub message: String,
    pub committed_at: Option<DateTime<Utc>>,
}

/// The commits of one task, from its start marker (or first addressed
/// commit) to its `feat` — plus whatever the client addressed to the task
/// after that (a late probe, an artifact sync), which keeps the range's
/// `end` at the `feat` commit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskRange {
    pub task_id: Uuid,
    /// The title from the start marker's trailer, else from any commit of
    /// the range, else the `feat` subject.
    pub title: Option<String>,
    pub start_sha: String,
    pub start_at: Option<DateTime<Utc>>,
    /// The `feat` commit; `None` while the task is still open.
    pub end_sha: Option<String>,
    pub end_at: Option<DateTime<Utc>>,
    /// Every commit attributed to the task, oldest first, markers included.
    pub commits: Vec<String>,
}

impl TaskRange {
    pub fn is_open(&self) -> bool {
        self.end_sha.is_none()
    }
}

/// Derive the task ranges of a first-parent log given **oldest first**.
///
/// Rules, applied commit by commit:
/// - `start(<X>)` opens X (closing whatever was open).
/// - A commit addressed to X while X is the current range joins it; a
///   `feat(<X>)` closes it.
/// - A commit addressed to X while another task (or none) is current opens
///   X implicitly — that is how histories without start markers work.
/// - A commit addressed to X after X's `feat` (the client still points at
///   the task until the next probe) joins the closed range.
/// - A commit with no task (session start/final, an unaddressed flag)
///   joins the current range when one is open, else is unattributed.
///
/// Returns the ranges in order plus the commit → task attribution.
pub fn task_ranges(log_oldest_first: &[LogEntry]) -> TaskRanges {
    let mut ranges: Vec<TaskRange> = Vec::new();
    let mut by_commit: HashMap<String, Uuid> = HashMap::new();
    // Index into `ranges` of the range new commits join; the last range is
    // always the one that may be open.
    let mut current: Option<usize> = None;

    for entry in log_oldest_first {
        let msg = SnapshotMessage::parse(&entry.message);
        let task = msg.task_id();
        let title = msg.trailers.task_title.clone().or_else(|| match msg.kind {
            Kind::Start | Kind::Feat => Some(msg.subject.clone()),
            _ => None,
        });

        let Some(task) = task else {
            if let Some(i) = current {
                ranges[i].commits.push(entry.sha.clone());
                by_commit.insert(entry.sha.clone(), ranges[i].task_id);
            }
            continue;
        };

        // A done marker for a task other than the current one closes that
        // task's still-open range instead of opening a one-commit range:
        // ololo commits the next task's start marker at its first probe and
        // the previous task's `feat` a beat later, from another thread.
        if msg.is_task_done()
            && current.is_none_or(|i| ranges[i].task_id != task)
            && let Some(i) = ranges
                .iter()
                .rposition(|r| r.task_id == task && r.end_sha.is_none())
        {
            let range = &mut ranges[i];
            range.commits.push(entry.sha.clone());
            range.end_sha = Some(entry.sha.clone());
            range.end_at = entry.committed_at;
            if range.title.is_none() {
                range.title = title;
            }
            by_commit.insert(entry.sha.clone(), task);
            continue;
        }

        // A start marker joins the current range when that range is the same
        // task and still open (a reconnect re-seeded from a stale log wrote a
        // second marker); after the task closed, it reopens the task.
        let joins_current = current.is_some_and(|i| {
            ranges[i].task_id == task && (msg.kind != Kind::Start || ranges[i].end_sha.is_none())
        });
        if !joins_current {
            ranges.push(TaskRange {
                task_id: task,
                title: None,
                start_sha: entry.sha.clone(),
                start_at: entry.committed_at,
                end_sha: None,
                end_at: None,
                commits: Vec::new(),
            });
            current = Some(ranges.len() - 1);
        }
        let i = current.expect("a range was just selected");
        let range = &mut ranges[i];
        range.commits.push(entry.sha.clone());
        by_commit.insert(entry.sha.clone(), task);
        if range.title.is_none() {
            range.title = title;
        }
        if msg.is_task_done() && range.end_sha.is_none() {
            range.end_sha = Some(entry.sha.clone());
            range.end_at = entry.committed_at;
        }
    }

    TaskRanges { ranges, by_commit }
}

/// The result of [`task_ranges`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TaskRanges {
    pub ranges: Vec<TaskRange>,
    /// Commit sha → the task it belongs to (unattributed commits absent).
    pub by_commit: HashMap<String, Uuid>,
}

impl TaskRanges {
    pub fn task_of(&self, sha: &str) -> Option<Uuid> {
        self.by_commit.get(sha).copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(n: u8) -> Uuid {
        Uuid::from_bytes([n; 16])
    }

    #[test]
    fn parses_the_format_0_subjects_the_cli_has_always_written() {
        let m = SnapshotMessage::parse(&format!("feat({}): Build the widget", id(1)));
        assert_eq!(m.kind, Kind::Feat);
        assert_eq!(m.subject_task, Some(id(1)));
        assert_eq!(m.subject, "Build the widget");
        assert_eq!(m.trailers, Trailers::default());
        assert_eq!(m.task_id(), Some(id(1)));
        assert!(m.is_task_done());

        let m = SnapshotMessage::parse(&format!("wip({}): checkpoint", id(2)));
        assert_eq!((m.kind.clone(), m.task_id()), (Kind::Wip, Some(id(2))));

        let m = SnapshotMessage::parse("flag: widget-done.md");
        assert_eq!((m.kind.clone(), m.task_id()), (Kind::Flag, None));
        assert_eq!(m.subject, "widget-done.md");

        let m = SnapshotMessage::parse(&format!(
            "memory({}): sources @ 2026-09-22T10:00:00Z",
            id(3)
        ));
        assert_eq!(m.kind, Kind::Memory);

        let m = SnapshotMessage::parse(&format!("tests({}): #4 12 passed, 1 failed", id(3)));
        assert_eq!((m.kind.clone(), m.task_id()), (Kind::Tests, Some(id(3))));
        assert!(!m.is_task_done());

        let m = SnapshotMessage::parse("ololo snapshot: session start @ 2026-09-22T10:00:00Z");
        assert_eq!(m.kind, Kind::Session);
        assert_eq!(m.subject, "session start @ 2026-09-22T10:00:00Z");
        assert_eq!(m.task_id(), None);
    }

    #[test]
    fn foreign_messages_are_unknown_not_errors() {
        for text in [
            "Initial commit",
            "feat(auth): login",
            "",
            "probe(not-a-uuid): x",
            "Feat(x): y",
        ] {
            let m = SnapshotMessage::parse(text);
            assert_eq!(m.kind, Kind::Unknown, "{text:?}");
            assert_eq!(m.task_id(), None);
        }
        // A conventional `feat: x` without a scope has the shape of an
        // unaddressed snapshot kind (like `flag: <file>`) — a task-less
        // commit, harmless to every reader.
        let m = SnapshotMessage::parse("feat: no scope");
        assert_eq!((m.kind.clone(), m.task_id()), (Kind::Feat, None));
        let m = SnapshotMessage::parse(&format!("zap({}): future kind", id(9)));
        assert_eq!(m.kind, Kind::Other("zap".into()));
        assert_eq!(m.task_id(), Some(id(9)));
    }

    fn full_trailers() -> Trailers {
        Trailers {
            format: Some(FORMAT_VERSION),
            session: Some(id(10)),
            participant: Some(id(11)),
            task: Some(id(1)),
            task_title: Some("Build the widget".into()),
            probe: Some(id(12)),
            probe_seq: Some(7),
            outcome: None,
            timestamp: Some("2026-09-22T10:00:00Z".parse().unwrap()),
        }
    }

    #[test]
    fn format_1_messages_round_trip() {
        let trailers = full_trailers();
        let subject = subject(&Kind::Probe, Some(id(1)), "#7 Build the widget");
        assert_eq!(subject, format!("probe({}): #7 Build the widget", id(1)));
        let message = format(&subject, &trailers);
        assert!(message.starts_with(&format!("{subject}\n\nOlolo-Format: 1\n")));
        assert!(message.contains("Ololo-Probe-Seq: 7\n"));
        assert!(message.ends_with("Ololo-Timestamp: 2026-09-22T10:00:00Z"));

        let parsed = SnapshotMessage::parse(&message);
        assert_eq!(parsed.kind, Kind::Probe);
        assert_eq!(parsed.subject, "#7 Build the widget");
        assert_eq!(parsed.trailers, trailers);
        assert_eq!(parsed.task_id(), Some(id(1)));
    }

    #[test]
    fn the_trailer_takes_precedence_over_the_subject_task() {
        let trailers = Trailers {
            task: Some(id(2)),
            ..Default::default()
        };
        let m = SnapshotMessage::parse(&format(
            &subject(&Kind::Wip, Some(id(1)), "checkpoint"),
            &trailers,
        ));
        assert_eq!(m.subject_task, Some(id(1)));
        assert_eq!(m.task_id(), Some(id(2)));
    }

    #[test]
    fn empty_trailers_write_the_format_0_message() {
        let s = subject(&Kind::Feat, Some(id(1)), "Title");
        assert_eq!(format(&s, &Trailers::default()), s);
    }

    #[test]
    fn titles_are_one_line_bounded_and_never_match_the_feat_grep() {
        let messy = format!("Ship\tit\nnow \u{1f}\u{1e} with feat({}) inside", id(1));
        let clean = sanitize_title(&messy);
        assert_eq!(clean, format!("Ship it now with feat [{}) inside", id(1)));
        assert!(!clean.contains("feat("));
        let long = "x".repeat(500);
        assert_eq!(sanitize_title(&long).chars().count(), MAX_TITLE_CHARS);

        let trailers = Trailers {
            task_title: Some(messy.clone()),
            ..Default::default()
        };
        let message = format(&subject(&Kind::Start, Some(id(1)), &messy), &trailers);
        assert_eq!(message.lines().count(), 3);
        assert!(!message.contains('\u{1f}') && !message.contains('\u{1e}'));
        // Only the subject may carry the id in parentheses — and a `start`
        // subject is not `feat(`.
        assert_eq!(message.matches("feat(").count(), 0);
    }

    #[test]
    fn unknown_ololo_trailers_and_foreign_trailers_are_ignored() {
        let message = format!(
            "probe({}): #1 T\n\nOlolo-Format: 1\nOlolo-Future: yes\nSigned-off-by: someone <s@x>\nOlolo-Probe-Seq: 1",
            id(1)
        );
        let m = SnapshotMessage::parse(&message);
        assert_eq!(m.trailers.format, Some(1));
        assert_eq!(m.trailers.probe_seq, Some(1));
    }

    #[test]
    fn a_body_paragraph_that_is_not_trailers_is_not_parsed_as_such() {
        let message = format!("feat({}): T\n\nThis explains: the change\nin prose.", id(1));
        let m = SnapshotMessage::parse(&message);
        assert_eq!(m.trailers, Trailers::default());
    }

    fn entry(sha: &str, message: String, t: i64) -> LogEntry {
        LogEntry {
            sha: sha.into(),
            message,
            committed_at: Some(DateTime::from_timestamp(t, 0).unwrap()),
        }
    }

    #[test]
    fn ranges_from_a_format_0_history_open_at_the_first_addressed_commit() {
        let (a, b) = (id(1), id(2));
        let log = vec![
            entry("s0", "ololo snapshot: session start @ x".into(), 0),
            entry("a1", format!("memory({a}): sources @ x"), 1),
            entry("a2", format!("feat({a}): Task A"), 2),
            entry("b1", format!("wip({b}): checkpoint"), 3),
            entry("b2", format!("artifact({b}): sync"), 4),
            entry("b3", format!("feat({b}): Task B"), 5),
            entry("b4", format!("artifact({b}): sync"), 6),
            entry("s1", "ololo snapshot: final @ x".into(), 7),
        ];
        let r = task_ranges(&log);
        assert_eq!(r.ranges.len(), 2);
        let ra = &r.ranges[0];
        assert_eq!(
            (ra.task_id, ra.start_sha.as_str(), ra.end_sha.as_deref()),
            (a, "a1", Some("a2"))
        );
        assert_eq!(ra.title.as_deref(), Some("Task A"));
        assert_eq!(ra.commits, ["a1", "a2"]);
        let rb = &r.ranges[1];
        assert_eq!(
            (rb.start_sha.as_str(), rb.end_sha.as_deref()),
            ("b1", Some("b3"))
        );
        // The late artifact sync and the final session commit stay with B.
        assert_eq!(rb.commits, ["b1", "b2", "b3", "b4", "s1"]);
        assert_eq!(rb.end_at, log[5].committed_at);
        assert_eq!(r.task_of("s0"), None, "before any task: unattributed");
        assert_eq!(r.task_of("b4"), Some(b));
        assert_eq!(r.task_of("s1"), Some(b));
    }

    #[test]
    fn ranges_from_a_format_1_history_follow_the_markers() {
        let (a, b) = (id(1), id(2));
        let with = |kind: &Kind, task: Uuid, text: &str, seq: Option<u32>| {
            let trailers = Trailers {
                format: Some(1),
                task: Some(task),
                task_title: Some(format!("Title {}", if task == a { "A" } else { "B" })),
                probe_seq: seq,
                ..Default::default()
            };
            format(&subject(kind, Some(task), text), &trailers)
        };
        let log = vec![
            entry("s0", "ololo snapshot: session start @ x".into(), 0),
            entry("a0", with(&Kind::Start, a, "Title A", None), 1),
            entry("a1", with(&Kind::Probe, a, "#1 Title A", Some(1)), 2),
            entry("a2", with(&Kind::Probe, a, "#2 Title A", Some(2)), 3),
            entry("a3", with(&Kind::Feat, a, "Title A", None), 4),
            // A late probe for A, dispatched before the close, lands after.
            entry("a4", with(&Kind::Probe, a, "#3 Title A", Some(3)), 5),
            entry("b0", with(&Kind::Start, b, "Title B", None), 6),
            entry("b1", with(&Kind::Probe, b, "#4 Title B", Some(4)), 7),
            entry("f1", "flag: b-done.md".into(), 8),
        ];
        let r = task_ranges(&log);
        assert_eq!(r.ranges.len(), 2);
        let ra = &r.ranges[0];
        assert_eq!(
            (ra.start_sha.as_str(), ra.end_sha.as_deref()),
            ("a0", Some("a3"))
        );
        assert_eq!(ra.title.as_deref(), Some("Title A"));
        assert_eq!(ra.commits, ["a0", "a1", "a2", "a3", "a4"]);
        let rb = &r.ranges[1];
        assert!(rb.is_open());
        assert_eq!(rb.start_sha, "b0");
        assert_eq!(rb.commits, ["b0", "b1", "f1"]);
        assert_eq!(r.task_of("f1"), Some(b));
        assert_eq!(r.task_of("a4"), Some(a));
    }

    #[test]
    fn a_repeated_start_marker_joins_the_open_range_and_reopens_a_closed_one() {
        let a = id(1);
        let log = vec![
            entry("a0", subject(&Kind::Start, Some(a), "A"), 0),
            entry("a1", subject(&Kind::Probe, Some(a), "#1 A"), 1),
            // A reconnect wrote the marker again while the task was open.
            entry("a2", subject(&Kind::Start, Some(a), "A"), 2),
            entry("a3", subject(&Kind::Feat, Some(a), "A"), 3),
            // The task revisited after it closed is a new range.
            entry("a4", subject(&Kind::Start, Some(a), "A"), 4),
            entry("a5", subject(&Kind::Probe, Some(a), "#2 A"), 5),
        ];
        let r = task_ranges(&log);
        assert_eq!(r.ranges.len(), 2);
        assert_eq!(r.ranges[0].commits, ["a0", "a1", "a2", "a3"]);
        assert_eq!(r.ranges[0].end_sha.as_deref(), Some("a3"));
        assert!(r.ranges[1].is_open());
        assert_eq!(r.ranges[1].commits, ["a4", "a5"]);
    }

    #[test]
    fn a_done_marker_landing_after_the_next_start_closes_its_own_range() {
        // ololo writes start(B) at B's first probe and feat(A) a beat later,
        // from the TUI thread — the history reads start(A) … start(B)
        // probe(B) feat(A) probe(B). A's range closes at feat(A); B's range
        // is one range, still open.
        let (a, b) = (id(1), id(2));
        let log = vec![
            entry("a0", subject(&Kind::Start, Some(a), "A"), 0),
            entry("a1", subject(&Kind::Probe, Some(a), "#1 A"), 1),
            entry("b0", subject(&Kind::Start, Some(b), "B"), 2),
            entry("b1", subject(&Kind::Probe, Some(b), "#2 B"), 3),
            entry("a2", subject(&Kind::Feat, Some(a), "A"), 4),
            entry("b2", subject(&Kind::Probe, Some(b), "#3 B"), 5),
            entry("b3", subject(&Kind::Feat, Some(b), "B"), 6),
        ];
        let r = task_ranges(&log);
        assert_eq!(r.ranges.len(), 2);
        let ra = &r.ranges[0];
        assert_eq!(
            (ra.start_sha.as_str(), ra.end_sha.as_deref()),
            ("a0", Some("a2"))
        );
        assert_eq!(ra.commits, ["a0", "a1", "a2"]);
        let rb = &r.ranges[1];
        assert_eq!(
            (rb.start_sha.as_str(), rb.end_sha.as_deref()),
            ("b0", Some("b3"))
        );
        assert_eq!(rb.commits, ["b0", "b1", "b2", "b3"]);
        assert_eq!(r.task_of("a2"), Some(a));
        assert_eq!(r.task_of("b2"), Some(b));
    }
}
