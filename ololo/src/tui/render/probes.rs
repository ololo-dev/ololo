//! Sidebar (probe list) renderer.

use crate::tui::app::{InputFocus, NavTarget, TuiApp};
use crate::tui::event::ProbeResultInfo;
use crate::tui::render::common::{truncate, wrap_text};
use crate::tui::render::tokens::render_tokens;
use arena_core::protocol::ProbeOutcome;
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Padding};

pub(crate) fn render_sidebar(f: &mut Frame, area: Rect, app: &TuiApp) {
    // If we have token data, split the sidebar: probes on top, tokens on bottom.
    let has_tokens = app.tokens.as_ref().is_some_and(|v| !v.is_empty());
    let chat_view = app.sidebar_view == crate::tui::app::SidebarView::Chat;

    let (sidebar_area, tokens_area) = if has_tokens {
        // Size the panel to its content (summary + per-session blocks),
        // capped so probes keep at least the top of the sidebar.
        let token_rows = {
            let counts = app.tokens.as_ref().unwrap();
            let empty: Vec<agent_tokens::SessionStats> = Vec::new();
            let stats = app.token_stats.as_ref().unwrap_or(&empty);
            crate::tui::render::tokens::panel_line_count(
                &crate::tui::render::tokens::merge_session_views(counts, stats),
            )
        };
        // 2 rows overhead (border) + content lines, capped at 30
        let tokens_height = (token_rows + 2).min(30);
        let split = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(5), Constraint::Length(tokens_height)])
            .split(area);
        (split[0], split[1])
    } else {
        (area, Rect::new(0, 0, 0, 0))
    };

    // F5: the same pane retold as a chat transcript. Tokens keep their
    // split either way.
    if chat_view {
        crate::tui::render::chat::render_chat(f, sidebar_area, app);
        if has_tokens {
            render_tokens(f, tokens_area, app);
        }
        return;
    }

    let dropped = app.dropped_count.load(std::sync::atomic::Ordering::Relaxed);
    let footer = if dropped > 0 {
        format!(" probes: {} (+{} dropped) ", app.probes.len(), dropped)
    } else {
        format!(" probes: {} ", app.probes.len())
    };
    // Borders (2) + horizontal padding (2) around the text.
    let inner_w = sidebar_area.width.saturating_sub(4).max(1) as usize;
    let inner_h = sidebar_area.height.saturating_sub(2) as usize;

    // Group probes by task (shared with keyboard navigation — see
    // `TuiApp::task_groups`). Folded tasks render as a 1-line summary;
    // unfolded tasks render a header plus one entry per probe (result
    // glyph + full wrapped description + Expected/Actual). Newest first.
    // ponytail: O(n) per render; cache in app if 30Hz redraw shows on flamegraph.
    let groups = app.task_groups();
    let total_tasks = groups.len();
    let passed_tasks = groups.iter().filter(|g| g.passed).count();
    let cursor = app.sidebar_cursor;

    let mut items: Vec<ListItem> = Vec::new();
    let mut used_lines = 0usize;
    for g in &groups {
        if used_lines >= inner_h {
            break;
        }
        let selected = cursor == Some(NavTarget::Task(g.task_id));
        if g.folded {
            used_lines += 1;
            items.push(ListItem::new(Text::from(vec![task_header_line(
                g, inner_w, selected,
            )])));
            continue;
        }

        // Unfolded task: header + probe entries, clipped to remaining rows.
        let mut lines = vec![task_header_line(g, inner_w, selected)];
        for p in g.probes.iter().rev() {
            let probe_selected = cursor == Some(NavTarget::Probe(p.probe_id));
            lines.extend(probe_lines(p, inner_w, probe_selected));
        }
        let sep_len = inner_w.min(16);
        lines.push(Line::from(Span::styled(
            "─".repeat(sep_len),
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::DIM),
        )));
        let remaining = inner_h - used_lines;
        lines.truncate(remaining);
        used_lines += lines.len();
        items.push(ListItem::new(Text::from(lines)));
    }

    // Ungrouped probes (task_id: None — synthetic markers or back-compat
    // with old game-servers): render as standalone entries, newest-first,
    // with a title header when the probe carries one.
    for p in app.ungrouped_probes().iter().rev() {
        if used_lines >= inner_h {
            break;
        }
        let mut lines: Vec<Line> = Vec::new();
        if !p.task_title.is_empty() {
            lines.push(Line::from(Span::styled(
                truncate(&p.task_title, inner_w),
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            )));
        }
        let probe_selected = cursor == Some(NavTarget::Probe(p.probe_id));
        lines.extend(probe_lines(p, inner_w, probe_selected));
        let remaining = inner_h - used_lines;
        lines.truncate(remaining);
        used_lines += lines.len();
        items.push(ListItem::new(Text::from(lines)));
    }

    let probes_n = app.probes.len();
    let mut title = String::from(" probes ");
    // Task progress segment: only when we have a task basis (server-reported
    // total, or probes that group into tasks).
    if app.total_tasks.is_some() || total_tasks > 0 {
        let proj_total = app.total_tasks.map(|n| n as usize).unwrap_or(total_tasks);
        title = format!(" probes | {passed_tasks}/{proj_total} tasks ");
    }
    // Probe count segment: only when there are probes.
    if probes_n > 0 {
        title = format!("{title}| {probes_n} ");
    }
    if let Some(att) = app.progress_attempt {
        let status_str = match app.progress_status {
            Some(crate::tui::event::PlayerRunStatus::AwaitingResult) => "await",
            Some(crate::tui::event::PlayerRunStatus::Backoff) => "backoff",
            Some(crate::tui::event::PlayerRunStatus::Completed) => "done",
            Some(crate::tui::event::PlayerRunStatus::Failed) => "fail",
            None => "—",
        };
        title = format!("{title}| attempt={att} {status_str} ");
    }
    let mut block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .padding(Padding::horizontal(1));
    if dropped > 0 {
        block = block.title_bottom(Line::from(footer));
    } else {
        // Navigation hint: how to reach / drive the panel.
        let hint = if app.input_focus == InputFocus::Tui {
            " ↑↓ move · ⏎ fold/details · F5 chat · F1 help "
        } else {
            " F9: navigate probes · F5 chat · F1 help "
        };
        block = block.title_bottom(Line::from(Span::styled(
            hint,
            Style::default().fg(Color::DarkGray),
        )));
    }
    f.render_widget(List::new(items).block(block), sidebar_area);

    // Render the tokens panel if we have data.
    if has_tokens {
        render_tokens(f, tokens_area, app);
    }
}

const SPINNERS: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// The task header row: fold marker + pass mark + title + probe-type
/// progress, e.g. `▾ Trivia (2/4)` or `▸ ✓ Setup`. Selection renders
/// reversed (cursor bar).
fn task_header_line(
    g: &crate::tui::app::TaskGroup<'_>,
    inner_w: usize,
    selected: bool,
) -> Line<'static> {
    let newest = g.probes.last().expect("task group is never empty");
    let marker = if g.folded { "▸" } else { "▾" };
    let mut header = if newest.task_title.is_empty() {
        format!("Task #{}", g.ordinal)
    } else {
        newest.task_title.clone()
    };
    if g.passed {
        header = format!("✓ {header}");
    } else if newest.test_total > 0 {
        header = format!("{header} ({}/{})", newest.test_ordinal, newest.test_total);
    }
    let color = if g.passed { Color::Green } else { Color::Cyan };
    let mut style = Style::default().fg(color).add_modifier(Modifier::BOLD);
    if selected {
        style = style.add_modifier(Modifier::REVERSED);
    }
    // Total graded points for the task, right after the title.
    let points_span = g.points.map(|pts| {
        let mut pts_style = Style::default()
            .fg(match pts.cmp(&0) {
                std::cmp::Ordering::Greater => Color::Green,
                std::cmp::Ordering::Less => Color::Red,
                std::cmp::Ordering::Equal => Color::DarkGray,
            })
            .add_modifier(Modifier::BOLD);
        if selected {
            pts_style = pts_style.add_modifier(Modifier::REVERSED);
        }
        (format!(" {pts:+}"), pts_style)
    });
    let pts_w = points_span
        .as_ref()
        .map(|(s, _)| s.chars().count())
        .unwrap_or(0);
    let mut spans = vec![Span::styled(
        format!(
            "{marker} {}",
            truncate(&header, inner_w.saturating_sub(2 + pts_w))
        ),
        style,
    )];
    if let Some((s, st)) = points_span {
        spans.push(Span::styled(s, st));
    }
    Line::from(spans)
}

/// Render one probe as `{result glyph} {description}` (description wrapped
/// in full — never cut) followed by indented `Expected:` / `Actual:` lines
/// when values are known. `selected` reverses the first row (cursor bar).
fn probe_lines(p: &ProbeResultInfo, inner_w: usize, selected: bool) -> Vec<Line<'static>> {
    let (glyph, color) = result_glyph(p);
    let desc = if p.task_title.is_empty() && p.task_description.is_empty() {
        // Synthetic probe (member-joined / pty-input): surface its
        // human message as the description.
        p.stdout.trim().to_string()
    } else if p.task_description.is_empty() {
        "no description".to_string()
    } else {
        p.task_description.clone()
    };

    let mut lines: Vec<Line> = Vec::new();
    // "{glyph} {desc}", wrapped with a hanging indent under the text.
    // Selection reverses only the first row (the cursor bar).
    let plain_desc = Style::default().fg(Color::Gray);
    let mut glyph_style = Style::default().fg(color).add_modifier(Modifier::BOLD);
    let mut first_desc = plain_desc;
    if selected {
        glyph_style = glyph_style.add_modifier(Modifier::REVERSED);
        first_desc = first_desc.add_modifier(Modifier::REVERSED);
    }
    let glyph_w = glyph.chars().count() + 2; // " {glyph} "
    for (i, seg) in wrap_text(&desc, inner_w.saturating_sub(glyph_w).max(1))
        .into_iter()
        .enumerate()
    {
        if i == 0 {
            lines.push(Line::from(vec![
                Span::styled(format!(" {glyph} "), glyph_style),
                Span::styled(seg, first_desc),
            ]));
        } else {
            lines.push(Line::from(vec![
                Span::raw(" ".repeat(glyph_w)),
                Span::styled(seg, plain_desc),
            ]));
        }
    }

    let expected = p
        .graded_expected
        .clone()
        .or_else(|| p.expected_answer.clone());
    if let Some(v) = expected {
        lines.extend(value_lines("Expected", &v, inner_w));
    }
    // Actual is only meaningful once the command produced output.
    if (p.exit_code.is_some() || p.outcome.is_some()) && !p.stdout.trim().is_empty() {
        lines.extend(value_lines("Actual", p.stdout.trim(), inner_w));
    }
    lines
}

/// `    Expected: value` — wrapped, capped at 3 lines with an ellipsis.
fn value_lines(label: &str, value: &str, inner_w: usize) -> Vec<Line<'static>> {
    const INDENT: &str = "    ";
    const MAX_LINES: usize = 3;
    let style = Style::default().fg(Color::DarkGray);
    let text = format!("{label}: {value}");
    let mut wrapped = wrap_text(&text, inner_w.saturating_sub(INDENT.len()).max(1));
    if wrapped.len() > MAX_LINES {
        wrapped.truncate(MAX_LINES);
        if let Some(last) = wrapped.last_mut() {
            last.push('…');
        }
    }
    wrapped
        .into_iter()
        .map(|seg| Line::from(Span::styled(format!("{INDENT}{seg}"), style)))
        .collect()
}

/// Map a probe's lifecycle to a result glyph and color. Shared with the
/// chat view renderer.
pub(crate) fn result_glyph(p: &ProbeResultInfo) -> (String, Color) {
    if p.error.is_some() {
        return ("✗".to_string(), Color::Red);
    }
    if p.exit_code == Some(-1) {
        return ("⏱".to_string(), Color::Red);
    }
    if let Some(outcome) = p.outcome {
        return match outcome {
            ProbeOutcome::Pass => ("✓".to_string(), Color::Green),
            ProbeOutcome::Error => ("✗".to_string(), Color::Red),
            ProbeOutcome::NoResponse => ("◌".to_string(), Color::Yellow),
        };
    }
    if p.exit_code.is_some() {
        // Command finished, awaiting server grade.
        return ("→".to_string(), Color::Gray);
    }
    // Waiting / running: animated spinner, with the deadline countdown.
    let spinner = SPINNERS[(now_ms() / 120) as usize % SPINNERS.len()];
    match p.deadline_secs {
        Some(d) if d > 0 => (format!("{spinner} {d}s"), Color::Yellow),
        _ => (spinner.to_string(), Color::Yellow),
    }
}

fn now_ms() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

/// Centered overlay with the full details of the selected probe
/// (`app.probe_popup`). Rendered last so it sits above every pane.
pub(crate) fn render_probe_popup(f: &mut Frame, app: &TuiApp) {
    let Some(p) = app.probe_popup.and_then(|id| app.probe_by_id(id)) else {
        return;
    };
    let area = popup_rect(f.area());
    f.render_widget(Clear, area);
    let hint = if app.has_pty {
        " Esc: close · p: paste to agent "
    } else {
        " Esc: close "
    };
    let block = Block::default()
        .title(" probe details ")
        .title_bottom(Line::from(Span::styled(
            hint,
            Style::default().fg(Color::DarkGray),
        )))
        .borders(Borders::ALL)
        .padding(Padding::new(2, 2, 1, 1));
    let inner = block.inner(area);
    f.render_widget(block, area);
    let w = inner.width.max(1) as usize;

    let (glyph, color) = result_glyph(p);
    let status = match (p.error.as_ref(), p.exit_code, p.outcome) {
        (Some(_), _, _) => "ERROR",
        (_, Some(-1), _) => "TIMEOUT",
        (_, _, Some(ProbeOutcome::Pass)) => "PASS",
        (_, _, Some(ProbeOutcome::Error)) => "FAIL",
        (_, _, Some(ProbeOutcome::NoResponse)) => "NO RESPONSE",
        (_, Some(_), None) => "SENT — awaiting grade",
        _ => "WAITING",
    };

    let label_style = Style::default()
        .fg(Color::Cyan)
        .add_modifier(Modifier::BOLD);
    let dim = Style::default().fg(Color::DarkGray);
    let mut lines: Vec<Line> = Vec::new();
    let field = |lines: &mut Vec<Line>, label: &str, value: &str| {
        for (i, seg) in wrap_text(value, w.saturating_sub(label.len() + 2).max(1))
            .into_iter()
            .enumerate()
        {
            if i == 0 {
                lines.push(Line::from(vec![
                    Span::styled(format!("{label}: "), label_style),
                    Span::raw(seg),
                ]));
            } else {
                lines.push(Line::from(vec![
                    Span::raw(" ".repeat(label.len() + 2)),
                    Span::raw(seg),
                ]));
            }
        }
    };

    // Task context.
    let mut task = if p.task_title.is_empty() {
        format!("Task #{}", p.task_ordinal)
    } else {
        p.task_title.clone()
    };
    if p.test_total > 0 {
        task = format!("{task} — probe {}/{}", p.test_ordinal, p.test_total);
    }
    field(&mut lines, "Task", &task);
    if !p.task_description.is_empty() {
        field(&mut lines, "Description", &p.task_description);
    }
    lines.push(Line::default());

    // Status line: glyph + label + points/duration/exit.
    let mut status_extras = Vec::new();
    if let Some(d) = p.point_delta {
        status_extras.push(format!("{d:+} pts"));
    }
    if p.duration_ms > 0 {
        status_extras.push(format!("{}ms", p.duration_ms));
    }
    if let Some(ec) = p.exit_code {
        status_extras.push(format!("exit {ec}"));
    }
    let extras = if status_extras.is_empty() {
        String::new()
    } else {
        format!("  ({})", status_extras.join(", "))
    };
    lines.push(Line::from(vec![
        Span::styled("Status: ", label_style),
        Span::styled(
            format!("{glyph} {status}{extras}"),
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        ),
    ]));
    lines.push(Line::default());

    field(&mut lines, "Command", &p.command);
    if let Some(exp) = p.graded_expected.as_ref().or(p.expected_answer.as_ref()) {
        field(&mut lines, "Expected", exp);
    } else if !p.answer_template.is_empty() {
        field(&mut lines, "Expected (template)", &p.answer_template);
    }
    let stdout = p.stdout.trim();
    if !stdout.is_empty() {
        field(&mut lines, "Actual", stdout);
    }
    if let Some(e) = &p.error {
        field(&mut lines, "Error", e);
    }
    if let Some(d) = p.deadline_secs {
        lines.push(Line::from(Span::styled(format!("deadline: {d}s"), dim)));
    }
    lines.push(Line::from(Span::styled(
        format!("probe id: {}", p.probe_id),
        dim,
    )));

    lines.truncate(inner.height as usize);
    f.render_widget(ratatui::widgets::Paragraph::new(Text::from(lines)), inner);
}

/// Centered popup `Rect`: ~76% wide, ~70% tall, clamped to the terminal.
fn popup_rect(area: Rect) -> Rect {
    let w = ((area.width as u32 * 76 / 100) as u16).clamp(20, area.width);
    let h = ((area.height as u32 * 70 / 100) as u16).clamp(6, area.height);
    let x = area.width.saturating_sub(w) / 2;
    let y = area.height.saturating_sub(h) / 2;
    Rect::new(x, y, w, h)
}
