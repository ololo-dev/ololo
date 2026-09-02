//! Chat-transcript sidebar renderer — the default sidebar pane (F5 flips
//! to the probes forensics view).
//!
//! The session retold as a conversation the way the web player chat does:
//! ololo hands out task briefs and runs checks, judges ask for evidence and
//! deliver verdicts, the player answers with done-notes. The feed is
//! bottom-anchored — the newest message sits at the bottom, ↑ scrolls
//! into history.
//!
//! Nothing here is truncated. A chat that ends its messages in `…` makes the
//! reader open a popup to finish a sentence, which is what the probes pane
//! already is; this view earns its place by being readable on its own. Long
//! text wraps with a hanging indent so the structure survives, and history
//! is reached by scrolling rather than by cutting.

use crate::tui::app::{ChatMsg, SidebarView, StatusLine, TuiApp};
use crate::tui::render::common::wrap_text;
use crate::tui::render::markdown::md_indented;
use crate::tui::render::probes::result_glyph;
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Padding, Paragraph};

/// The rendered transcript: every wrapped line, each bubble's line range,
/// and the visible window — one source of truth for the renderer and the
/// mouse hit-test, so a click lands on the same bubble the eye sees.
struct TranscriptLayout {
    lines: Vec<Line<'static>>,
    /// Per-message `[start, end)` into `lines`, same order as the transcript.
    ranges: Vec<(usize, usize)>,
    /// Visible window `[start, end)` into `lines`.
    start: usize,
    end: usize,
}

fn transcript_layout(app: &TuiApp, inner_w: usize, inner_h: usize) -> TranscriptLayout {
    let msgs = app.chat_transcript();
    let selected = app.chat_cursor.and_then(|c| msgs.len().checked_sub(1 + c));
    let mut lines: Vec<Line> = Vec::new();
    let mut ranges: Vec<(usize, usize)> = Vec::with_capacity(msgs.len());
    for (i, m) in msgs.iter().enumerate() {
        // A blank row between bubbles — without it the transcript reads as
        // one unbroken column of text.
        if i > 0 {
            lines.push(Line::default());
        }
        let start = lines.len();
        lines.extend(msg_lines(m, inner_w, selected == Some(i)));
        ranges.push((start, lines.len()));
    }

    // Bottom-anchored window: `chat_scroll` lines up from the newest.
    // With a selection, the window follows the selected bubble instead:
    // bottom-aligned when it fits, top-aligned when taller than the pane.
    let total = lines.len();
    let scroll = match selected.and_then(|i| ranges.get(i)).copied() {
        Some((bs, be)) => {
            if be - bs >= inner_h {
                total.saturating_sub(bs + inner_h)
            } else {
                total - be
            }
        }
        None => app.chat_scroll,
    }
    .min(total.saturating_sub(inner_h));
    let end = total - scroll;
    let start = end.saturating_sub(inner_h);
    TranscriptLayout {
        lines,
        ranges,
        start,
        end,
    }
}

/// The chat pane's rect for a terminal of `term_cols`×`term_rows` — the
/// same header/body and 55/45 splits `view` performs, via the same layout
/// solver, so mouse hit-tests see the exact geometry.
pub(crate) fn chat_area(app: &TuiApp, term_cols: u16, term_rows: u16) -> Option<Rect> {
    if app.sidebar_view != SidebarView::Chat || (app.has_pty && !app.show_sidebar) {
        return None;
    }
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(0)])
        .split(Rect::new(0, 0, term_cols, term_rows));
    let body = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(55), Constraint::Percentage(45)])
        .split(chunks[1]);
    Some(body[1])
}

/// The compose bar's terminal row, when the chat pane shows one.
pub(crate) fn compose_bar_row(app: &TuiApp, term_cols: u16, term_rows: u16) -> Option<u16> {
    if !app.has_pty {
        return None;
    }
    let area = chat_area(app, term_cols, term_rows)?;
    (area.height > 2).then(|| area.y + area.height - 2)
}

/// The transcript bubble under the terminal cell `(col, row)`, as an index
/// into `chat_transcript()`. `None` on borders, blank separator rows, the
/// compose bar, or outside the pane.
pub(crate) fn bubble_at(
    app: &TuiApp,
    term_cols: u16,
    term_rows: u16,
    col: u16,
    row: u16,
) -> Option<usize> {
    let area = chat_area(app, term_cols, term_rows)?;
    if col < area.x + 1 || col >= area.x + area.width.saturating_sub(1) {
        return None;
    }
    let inner_w = area.width.saturating_sub(4).max(1) as usize;
    let (compose_rows, status_rows) = bottom_rows(app, inner_w);
    let inner_h = area.height.saturating_sub(2 + compose_rows + status_rows) as usize;
    let row_off = row.checked_sub(area.y + 1)? as usize;
    if row_off >= inner_h {
        return None;
    }
    let layout = transcript_layout(app, inner_w, inner_h);
    let line_idx = layout.start + row_off;
    if line_idx >= layout.end {
        return None;
    }
    layout
        .ranges
        .iter()
        .position(|&(s, e)| line_idx >= s && line_idx < e)
}

/// Rows the pane keeps below the transcript: the compose bar (with a
/// hosted agent) and the live status (while the session has something to
/// say about what happens next) — up to `STATUS_MAX_ROWS` of it, so a
/// narrow pane wraps the sentence instead of cutting it.
fn bottom_rows(app: &TuiApp, inner_w: usize) -> (u16, u16) {
    let compose = if app.has_pty { 1 } else { 0 };
    let status = app
        .live_status()
        .map(|st| status_lines(&st, inner_w).len() as u16)
        .unwrap_or(0);
    (compose, status)
}

/// The most rows the status may take from the transcript.
const STATUS_MAX_ROWS: usize = 2;

pub(crate) fn render_chat(f: &mut Frame, area: Rect, app: &TuiApp) {
    // Borders (2) + horizontal padding (2) around the text.
    let inner_w = area.width.saturating_sub(4).max(1) as usize;
    // With a hosted agent the last inner row is the compose bar — the
    // "✉ message" button, or the input line while typing. Above it, the
    // status row says what the session is doing and what comes next.
    let status = app.live_status();
    let (compose_rows, status_rows) = bottom_rows(app, inner_w);
    let inner_h = area.height.saturating_sub(2 + compose_rows + status_rows) as usize;

    let msgs = app.chat_transcript();
    // The selected bubble (chat_cursor counts up from the newest).
    let selected = app.chat_cursor.and_then(|c| msgs.len().checked_sub(1 + c));
    let layout = transcript_layout(app, inner_w, inner_h);
    let scroll = layout.lines.len() - layout.end;
    let visible: Vec<Line> = layout.lines[layout.start..layout.end].to_vec();

    let groups_total = msgs
        .iter()
        .filter(|m| matches!(m, ChatMsg::TaskHeader { .. }))
        .count();
    let passed = msgs
        .iter()
        .filter(|m| matches!(m, ChatMsg::TaskHeader { passed: true, .. }))
        .count();
    let mut title = String::from(" chat ");
    if app.total_tasks.is_some() || groups_total > 0 {
        let proj_total = app.total_tasks.map(|n| n as usize).unwrap_or(groups_total);
        title = format!(" chat | {passed}/{proj_total} tasks ");
    }

    let hint = if app.chat_input.is_some() {
        " ⏎ paste to agent · Esc cancel ".to_string()
    } else if selected.is_some() && app.has_pty {
        " ⏎ send to agent · ↑↓ move · Esc: latest ".to_string()
    } else if selected.is_some() {
        " ↑↓ move · Esc: latest ".to_string()
    } else if scroll > 0 {
        format!(" ↑↓ scroll (−{scroll}) · Esc: latest · F5: probes ")
    } else if app.input_focus == crate::tui::app::InputFocus::Tui {
        " ↑↓ select · F5: probes · F1 help ".to_string()
    } else {
        " click bubble: select, again: send · F5 probes ".to_string()
    };
    let block = Block::default()
        .title(title)
        .title_bottom(Line::from(Span::styled(
            hint,
            Style::default().fg(Color::DarkGray),
        )))
        .borders(Borders::ALL)
        .padding(Padding::horizontal(1));
    let inner = block.inner(area);
    f.render_widget(block, area);
    let transcript = Rect {
        height: inner.height.saturating_sub(compose_rows + status_rows),
        ..inner
    };
    f.render_widget(Paragraph::new(Text::from(visible)), transcript);
    if let Some(st) = status.as_ref()
        && status_rows > 0
        && inner.height >= compose_rows + status_rows
    {
        let rows = Rect {
            y: inner.y + inner.height - status_rows - compose_rows,
            height: status_rows,
            ..inner
        };
        f.render_widget(
            Paragraph::new(Text::from(status_lines(st, rows.width as usize))),
            rows,
        );
    }
    if compose_rows > 0 && inner.height > 0 {
        let bar = Rect {
            y: inner.y + inner.height - 1,
            height: 1,
            ..inner
        };
        f.render_widget(compose_line(app, bar.width as usize), bar);
    }
}

/// The compose bar: an idle "✉ message" button, or the input line while
/// the player is typing. Clicking the bar (or pressing `m`) opens it.
fn compose_line(app: &TuiApp, width: usize) -> Paragraph<'static> {
    let line = match app.chat_input.as_ref() {
        Some(text) => Line::from(vec![
            Span::styled(
                " ❯ ".to_string(),
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            ),
            // Long input: keep the tail (where typing happens) in view.
            Span::styled(
                {
                    let budget = width.saturating_sub(5);
                    let chars: Vec<char> = text.chars().collect();
                    let skip = chars.len().saturating_sub(budget);
                    chars[skip..].iter().collect::<String>()
                },
                Style::default().fg(Color::White),
            ),
            Span::styled("▏".to_string(), Style::default().fg(Color::Green)),
        ]),
        None => Line::from(Span::styled(
            format!("{:^width$}", "[ ✉ message the agent — m or click ]"),
            Style::default().fg(Color::DarkGray),
        )),
    };
    Paragraph::new(line)
}

/// Braille spinner frames — the same idiom the agent CLIs use for "working".
const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// The status rows: a spinner (busy) or a still dot, then the sentence
/// wrapped under a hanging indent, with the seconds of a countdown set in
/// bold so the eye finds them. Capped at `STATUS_MAX_ROWS` — a sentence
/// longer than that ends in `…` rather than eating the transcript.
fn status_lines(st: &StatusLine, width: usize) -> Vec<Line<'static>> {
    let glyph = if st.busy {
        let tick = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| (d.as_millis() / 100) as usize)
            .unwrap_or(0);
        SPINNER[tick % SPINNER.len()]
    } else {
        "·"
    };
    let body_w = width.saturating_sub(3).max(1);
    let mut segs = wrap_text(&st.text, body_w);
    if segs.len() > STATUS_MAX_ROWS {
        segs.truncate(STATUS_MAX_ROWS);
        if let Some(last) = segs.last_mut() {
            let mut chars: Vec<char> = last.chars().collect();
            chars.truncate(body_w.saturating_sub(1));
            chars.push('…');
            *last = chars.into_iter().collect();
        }
    }
    let dim = Style::default().fg(Color::Gray);
    let bold = Style::default()
        .fg(Color::White)
        .add_modifier(Modifier::BOLD);
    let last_i = segs.len().saturating_sub(1);
    segs.into_iter()
        .enumerate()
        .map(|(i, seg)| {
            let prefix = if i == 0 {
                format!(" {glyph} ")
            } else {
                "   ".to_string()
            };
            let mut spans = vec![Span::styled(prefix, Style::default().fg(Color::Cyan))];
            // `… in 12s` — the number is the part that changes every second.
            let tail = st.countdown.filter(|s| *s > 0).map(|s| format!("{s}s"));
            match tail {
                Some(tail) if i == last_i && seg.ends_with(&format!("in {tail}")) => {
                    let head = seg.len() - tail.len();
                    spans.push(Span::styled(seg[..head].to_string(), dim));
                    spans.push(Span::styled(tail, bold));
                }
                _ => spans.push(Span::styled(seg, dim)),
            }
            Line::from(spans)
        })
        .collect()
}

/// Wrap `text` when the first line has less room than the rest — the width
/// a leading glyph or a trailing chip leaves behind.
fn wrap_first_rest(text: &str, first_w: usize, rest_w: usize) -> Vec<String> {
    let first_w = first_w.max(1);
    let rest_w = rest_w.max(1);
    if first_w == rest_w {
        return wrap_text(text, rest_w);
    }
    // Wrap at the narrow width, then re-wrap everything after the first line
    // at the wider one so the body is not needlessly ragged.
    let narrow = wrap_text(text, first_w);
    let Some((head, tail)) = narrow.split_first() else {
        return Vec::new();
    };
    let mut out = vec![head.clone()];
    if !tail.is_empty() {
        out.extend(wrap_text(&tail.join(" "), rest_w));
    }
    out
}

/// Width of the speaker gutter every bubble carries (`▌ `).
const GUTTER_W: usize = 2;

/// One chat message as its rendered lines. Every bubble gets a colored
/// left gutter bar keyed to who speaks — cyan for ololo, yellow for the
/// judges, green for the player, dark gray for system lines — so where one
/// message ends and the next begins is visible at a glance. Task headers
/// are dividers, not bubbles, and stay bar-free. A `selected` bubble's
/// first line renders reversed — the cursor a ⏎ would send to the agent.
fn msg_lines(m: &ChatMsg<'_>, inner_w: usize, selected: bool) -> Vec<Line<'static>> {
    let body_w = inner_w.saturating_sub(GUTTER_W).max(1);
    let (speaker, raw) = match m {
        ChatMsg::TaskHeader {
            ordinal,
            title,
            points,
            passed,
        } => {
            return highlight_first(
                task_header_lines(*ordinal, title, *points, *passed, inner_w),
                selected,
            );
        }
        ChatMsg::Brief { text } => (Color::Cyan, brief_lines(text, body_w)),
        ChatMsg::Check {
            probe,
            runs,
            question,
        } => (
            Color::Cyan,
            check_lines(probe, *runs, question.as_deref(), body_w),
        ),
        ChatMsg::Request {
            judge,
            instruction,
            path,
            delivered,
        } => (
            Color::Yellow,
            request_lines(judge, instruction, path, *delivered, body_w),
        ),
        ChatMsg::DoneNote(n) => (Color::Green, done_note_lines(n, body_w)),
        ChatMsg::Verdict(v) => (Color::Yellow, verdict_lines(v, body_w)),
        ChatMsg::System { text } => {
            let style = Style::default().fg(Color::DarkGray);
            let lines = wrap_first_rest(text, body_w.saturating_sub(3), body_w.saturating_sub(3))
                .into_iter()
                .enumerate()
                .map(|(i, seg)| {
                    let prefix = if i == 0 { " · " } else { "   " };
                    Line::from(Span::styled(format!("{prefix}{seg}"), style))
                })
                .collect();
            (Color::DarkGray, lines)
        }
    };
    let bar = Style::default().fg(speaker);
    let lines = raw
        .into_iter()
        .map(|line| {
            let mut spans = vec![Span::styled("▌ ".to_string(), bar)];
            spans.extend(line.spans);
            Line::from(spans)
        })
        .collect();
    highlight_first(lines, selected)
}

/// Reverse a selected bubble's first line — the same cursor language the
/// probes pane speaks.
fn highlight_first(mut lines: Vec<Line<'static>>, selected: bool) -> Vec<Line<'static>> {
    if selected && let Some(first) = lines.first_mut() {
        for span in &mut first.spans {
            span.style = span.style.add_modifier(Modifier::REVERSED);
        }
    }
    lines
}

/// ololo's message: the task brief, whole, indented under the task marker.
/// Briefs are authored as markdown — rendered, not shown with its sigils.
fn brief_lines(text: &str, inner_w: usize) -> Vec<Line<'static>> {
    md_indented(text.trim(), " ", inner_w, Style::default().fg(Color::Gray))
}

/// The artifact-request bubble's own background — the terminal cousin of
/// the web chat's amber request card, so an open ask for evidence stands
/// apart from the regular message flow.
const REQUEST_BG: Color = Color::Rgb(56, 44, 16);

/// Paint a message's lines onto their own background: every span gets the
/// tint and each row is padded to the pane's width, so the block reads as
/// one bubble instead of text-shaped stripes.
fn tint_block(lines: Vec<Line<'static>>, inner_w: usize, bg: Color) -> Vec<Line<'static>> {
    lines
        .into_iter()
        .map(|line| {
            let used: usize = line.spans.iter().map(|s| s.content.chars().count()).sum();
            let mut spans: Vec<Span<'static>> = line
                .spans
                .into_iter()
                .map(|s| {
                    let style = s.style.bg(bg);
                    Span::styled(s.content, style)
                })
                .collect();
            if used < inner_w {
                spans.push(Span::styled(
                    " ".repeat(inner_w - used),
                    Style::default().bg(bg),
                ));
            }
            Line::from(spans)
        })
        .collect()
}

/// ` ⚖ Correctness asks: capture the widget… [waiting]` and the delivery
/// folder underneath — the judge's evidence request in plain words, on its
/// own background block.
fn request_lines(
    judge: &str,
    instruction: &str,
    path: &str,
    delivered: bool,
    inner_w: usize,
) -> Vec<Line<'static>> {
    let name_style = Style::default()
        .fg(Color::Yellow)
        .add_modifier(Modifier::BOLD);
    let (chip, chip_style) = if delivered {
        (" [delivered ✓]", Style::default().fg(Color::Green))
    } else {
        (" [waiting]", Style::default().fg(Color::DarkGray))
    };
    let text_style = Style::default().fg(Color::Gray);

    let head = format!("{judge} asks: ");
    let body_w = inner_w.saturating_sub(3).max(1);
    let segs = wrap_first_rest(
        instruction,
        body_w.saturating_sub(head.chars().count() + chip.chars().count()),
        body_w,
    );
    let mut lines: Vec<Line<'static>> = Vec::new();
    let last = segs.len().saturating_sub(1);
    for (i, seg) in segs.iter().enumerate() {
        let mut spans = if i == 0 {
            vec![
                Span::styled(" ⚖ ".to_string(), Style::default().fg(Color::Yellow)),
                Span::styled(head.clone(), name_style),
                Span::styled(seg.clone(), text_style),
            ]
        } else {
            vec![
                Span::raw("   ".to_string()),
                Span::styled(seg.clone(), text_style),
            ]
        };
        if i == last {
            spans.push(Span::styled(chip.to_string(), chip_style));
        }
        lines.push(Line::from(spans));
    }
    if !path.is_empty() {
        let dim = Style::default().fg(Color::DarkGray);
        for (i, seg) in wrap_text(
            &format!("save into {path} — committed and pushed automatically"),
            inner_w.saturating_sub(5).max(1),
        )
        .into_iter()
        .enumerate()
        {
            let prefix = if i == 0 { "   ↳ " } else { "     " };
            lines.push(Line::from(Span::styled(format!("{prefix}{seg}"), dim)));
        }
    }
    tint_block(lines, inner_w, REQUEST_BG)
}

/// ` ✎ you` and the done-note's own words underneath — the player's message.
fn done_note_lines(n: &crate::tui::app::DoneNote, inner_w: usize) -> Vec<Line<'static>> {
    let mut lines = vec![Line::from(vec![
        Span::styled(
            " ✎ ".to_string(),
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            "you".to_string(),
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("  {}", n.path),
            Style::default().fg(Color::DarkGray),
        ),
    ])];
    // Done-notes are markdown files the player (or their agent) writes.
    lines.extend(md_indented(
        n.text.trim(),
        "   ",
        inner_w,
        Style::default().fg(Color::Gray),
    ));
    lines
}

/// `── TASK #0 ✓ Build the weather widget +163 ─` — the task boundary. A
/// title too long for one row wraps under the marker instead of being cut.
fn task_header_lines(
    ordinal: i32,
    title: &str,
    points: Option<i64>,
    passed: bool,
    inner_w: usize,
) -> Vec<Line<'static>> {
    let color = if passed { Color::Green } else { Color::Cyan };
    let head_style = Style::default().fg(color).add_modifier(Modifier::BOLD);
    let pts_style = |p: i64| {
        Style::default()
            .fg(match p.cmp(&0) {
                std::cmp::Ordering::Greater => Color::Green,
                std::cmp::Ordering::Less => Color::Red,
                std::cmp::Ordering::Equal => Color::DarkGray,
            })
            .add_modifier(Modifier::BOLD)
    };
    let mark = if passed { "✓ " } else { "" };
    let pts = points.map(|p| format!(" {p:+}"));
    let pts_w = pts.as_ref().map(|s| s.chars().count()).unwrap_or(0);

    let head = format!("TASK #{ordinal} {mark}{title}");
    // "── " + head + points + " ─"
    if head.chars().count() + pts_w + 5 <= inner_w {
        let mut spans = vec![
            Span::styled("── ".to_string(), head_style),
            Span::styled(head, head_style),
        ];
        if let Some(p) = points {
            spans.push(Span::styled(format!(" {p:+}"), pts_style(p)));
        }
        spans.push(Span::styled(" ─".to_string(), head_style));
        return vec![Line::from(spans)];
    }

    // Too long for one row: the marker line carries the ordinal and the
    // score, the title follows in full underneath.
    let mut first = vec![
        Span::styled("── ".to_string(), head_style),
        Span::styled(format!("TASK #{ordinal} {mark}"), head_style),
    ];
    if let Some(p) = points {
        first.push(Span::styled(format!("{p:+} "), pts_style(p)));
    }
    first.push(Span::styled("─".to_string(), head_style));

    let mut out = vec![Line::from(first)];
    for seg in wrap_text(title, inner_w.saturating_sub(3)) {
        out.push(Line::from(Span::styled(format!("   {seg}"), head_style)));
    }
    out
}

/// The expected value the server compared against: the graded resolution
/// first (e.g. `42`), the TestPush answer template as a fallback.
fn expected_of(p: &crate::tui::event::ProbeResultInfo) -> Option<&str> {
    p.graded_expected
        .as_deref()
        .or(p.expected_answer.as_deref())
        .map(str::trim)
        .filter(|e| !e.is_empty())
}

/// Judge-registered checks carry a machine label (`registered: <slug>`) —
/// say who asked instead, the way the web chat does.
fn check_label(label: &str) -> Option<String> {
    let t = label.trim();
    if t.is_empty() {
        return None;
    }
    match t.strip_prefix("registered:") {
        Some(slug) => Some(format!("extra check from the {} judge", slug.trim())),
        None => Some(t.to_string()),
    }
}

/// ` ✓ what the check is about ×3 +25` — one check in its latest state, the
/// way the web chat's check bubble reads: the quiz question or the test's
/// own label, the author's explanation of what it verifies, the answer it
/// got, and on a fail the expected value next to it. Never the shell.
fn check_lines(
    p: &crate::tui::event::ProbeResultInfo,
    runs: usize,
    question: Option<&str>,
    inner_w: usize,
) -> Vec<Line<'static>> {
    let (glyph, color) = result_glyph(p);
    let answer = {
        let a = p.stdout.trim();
        ((p.exit_code.is_some() || p.outcome.is_some()) && !a.is_empty()).then_some(a)
    };
    let failed = p.error.is_some()
        || p.exit_code == Some(-1)
        || matches!(
            p.outcome,
            Some(arena_core::protocol::ProbeOutcome::Error)
                | Some(arena_core::protocol::ProbeOutcome::NoResponse)
        );
    let label = check_label(&p.test_label);
    // The bubble's main text: the question, else the test's own label,
    // else the answer, else status.
    let heading = question.map(str::to_string).or(label);
    let body = match (&heading, answer) {
        (Some(h), _) => h.clone(),
        (None, Some(a)) => a.to_string(),
        (None, None) => match p.outcome {
            Some(arena_core::protocol::ProbeOutcome::Pass) => "check passed".to_string(),
            Some(_) => "check failed".to_string(),
            None if p.error.is_some() => "check errored".to_string(),
            None => "checking…".to_string(),
        },
    };

    let mut suffix = String::new();
    if runs > 1 {
        suffix.push_str(&format!(" ×{runs}"));
    }
    if let Some(d) = p.point_delta
        && d != 0
    {
        suffix.push_str(&format!(" {d:+}"));
    }

    let glyph_w = glyph.chars().count() + 2; // " {glyph} "
    let indent = " ".repeat(glyph_w);
    let body_w = inner_w.saturating_sub(glyph_w).max(1);
    let body_style = Style::default().fg(Color::Gray);
    let dim = Style::default().fg(Color::DarkGray);

    // The run count and points ride the first row; the body takes what is
    // left there and the full width afterwards.
    let segs = wrap_first_rest(&body, body_w.saturating_sub(suffix.chars().count()), body_w);
    let mut lines: Vec<Line<'static>> = Vec::new();
    for (i, seg) in segs.iter().enumerate() {
        if i == 0 {
            lines.push(Line::from(vec![
                Span::styled(
                    format!(" {glyph} "),
                    Style::default().fg(color).add_modifier(Modifier::BOLD),
                ),
                Span::styled(seg.clone(), body_style),
                Span::styled(suffix.clone(), dim),
            ]));
        } else {
            lines.push(Line::from(vec![
                Span::raw(indent.clone()),
                Span::styled(seg.clone(), body_style),
            ]));
        }
    }
    if lines.is_empty() {
        lines.push(Line::from(vec![
            Span::styled(
                format!(" {glyph} "),
                Style::default().fg(color).add_modifier(Modifier::BOLD),
            ),
            Span::styled(suffix.clone(), dim),
        ]));
    }

    // The author's explanation of what this check verifies, right under
    // the heading — the context a bare "check failed" was missing.
    let desc = p.test_description.trim();
    if !desc.is_empty() {
        let desc_w = inner_w.saturating_sub(glyph_w).max(1);
        for seg in wrap_text(desc, desc_w) {
            lines.push(Line::from(Span::styled(format!("{indent}{seg}"), dim)));
        }
    }

    // The answer under the heading (question or label), whole. Machine
    // output is often many lines; the view scrolls rather than clips.
    if heading.is_some()
        && let Some(a) = answer
    {
        let ans_indent = format!("{indent}  ");
        let ans_w = inner_w.saturating_sub(glyph_w + 2).max(1);
        for (i, seg) in wrap_text(a, ans_w).into_iter().enumerate() {
            let prefix = if i == 0 {
                format!("{indent}↳ ")
            } else {
                ans_indent.clone()
            };
            lines.push(Line::from(Span::styled(format!("{prefix}{seg}"), dim)));
        }
    }
    // A failed check that produced nothing still owes the reader its
    // "got" side of the story.
    if failed && heading.is_some() && answer.is_none() {
        lines.push(Line::from(Span::styled(
            format!("{indent}↳ (no answer)"),
            dim,
        )));
    }
    // On a fail, the expected value tells the player (and their agent)
    // what "right" looks like — shown verbatim, even when it is assertion
    // code: it is the contract the check graded against.
    if failed && let Some(exp) = expected_of(p) {
        let exp_w = inner_w.saturating_sub(glyph_w).max(1);
        for (i, seg) in wrap_text(&format!("expected: {exp}"), exp_w)
            .into_iter()
            .enumerate()
        {
            let prefix = if i == 0 {
                indent.clone()
            } else {
                format!("{indent}  ")
            };
            lines.push(Line::from(Span::styled(format!("{prefix}{seg}"), dim)));
        }
    }
    lines
}

/// ` ⚖ Creativity +17` and the judge's reasoning in full underneath.
fn verdict_lines(v: &crate::tui::app::JudgeVerdict, inner_w: usize) -> Vec<Line<'static>> {
    let pts_style = Style::default()
        .fg(match v.point_delta.cmp(&0) {
            std::cmp::Ordering::Greater => Color::Green,
            std::cmp::Ordering::Less => Color::Red,
            std::cmp::Ordering::Equal => Color::DarkGray,
        })
        .add_modifier(Modifier::BOLD);
    let name_style = Style::default()
        .fg(Color::Yellow)
        .add_modifier(Modifier::BOLD);
    let delta = format!(" {:+}", v.point_delta);

    // Name first, points pinned to the end of the row it fits on.
    let name_segs = wrap_first_rest(
        &v.judge_name,
        inner_w.saturating_sub(3 + delta.chars().count()),
        inner_w.saturating_sub(3),
    );
    let mut lines: Vec<Line<'static>> = Vec::new();
    let last = name_segs.len().saturating_sub(1);
    for (i, seg) in name_segs.iter().enumerate() {
        let mut spans = vec![
            Span::styled(
                if i == 0 { " ⚖ " } else { "   " }.to_string(),
                Style::default().fg(Color::Yellow),
            ),
            Span::styled(seg.clone(), name_style),
        ];
        if i == last {
            spans.push(Span::styled(delta.clone(), pts_style));
        }
        lines.push(Line::from(spans));
    }

    // The reasoning, whole — a verdict cut mid-sentence is the one thing a
    // player cannot act on. Judges write markdown; render it, sigil-free.
    let feedback = v.feedback.trim();
    if !feedback.is_empty() {
        lines.extend(md_indented(
            feedback,
            "   ",
            inner_w,
            Style::default().fg(Color::DarkGray),
        ));
    }
    lines
}
