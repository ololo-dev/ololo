//! Hotkey-help popup (F1 / `?`), rendered above every other overlay.

use crate::tui::app::TuiApp;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Clear, Padding, Paragraph};

/// One `key — description` row, or a section header when `key` is empty.
fn rows(has_pty: bool) -> Vec<(&'static str, &'static str)> {
    let mut rows = vec![
        ("", "Global"),
        ("F1  ?", "toggle this help"),
        ("F2", "show last failed probe"),
    ];
    if has_pty {
        rows.push(("F3", "paste last failed probe to the agent"));
    }
    rows.push(("F4", "show/hide the sidebar"));
    rows.push(("F5", "switch sidebar view: chat ⇄ probes"));
    if has_pty {
        rows.push(("F9", "switch focus: agent ⇄ probes"));
    }
    rows.push(("F10", "quit"));
    rows.extend([
        ("", "Probes panel"),
        ("↑/↓  k/j", "move selection"),
        ("⏎  space", "fold task / open probe details"),
        ("Esc", "clear selection"),
        ("", "Chat view"),
        ("↑/↓  k/j", "select a bubble / back to latest"),
        ("⏎  p", "send the selected bubble to the agent"),
        ("click", "select a bubble · same bubble again: send"),
        ("PgUp/PgDn", "scroll"),
        ("wheel", "scroll the chat"),
        ("m", "message the agent (clicking the ✉ bar works too)"),
        ("Esc", "clear selection, jump to the latest"),
        ("", "Probe details"),
    ]);
    if has_pty {
        rows.push(("p", "paste probe to the agent"));
    }
    rows.push(("Esc", "close popup"));
    if has_pty {
        rows.extend([
            ("", "Agent pane"),
            ("wheel", "scroll agent output"),
            ("", "everything else is typed into the agent"),
        ]);
    }
    rows
}

pub(crate) fn render_help_popup(f: &mut Frame, app: &TuiApp) {
    if !app.show_help {
        return;
    }
    let rows = rows(app.has_pty);

    let key_style = Style::default()
        .fg(Color::Cyan)
        .add_modifier(Modifier::BOLD);
    let section_style = Style::default()
        .fg(Color::Yellow)
        .add_modifier(Modifier::BOLD);
    const KEY_W: usize = 10;
    let mut lines: Vec<Line> = Vec::new(); // reused (truncated) for the widget below
    for (key, desc) in &rows {
        if key.is_empty() {
            if !lines.is_empty() {
                lines.push(Line::default());
            }
            lines.push(Line::from(Span::styled(desc.to_string(), section_style)));
        } else {
            lines.push(Line::from(vec![
                Span::styled(format!("  {key:<KEY_W$}"), key_style),
                Span::raw(desc.to_string()),
            ]));
        }
    }

    let area = help_rect(f.area(), &lines);
    f.render_widget(Clear, area);
    let block = Block::default()
        .title(" hotkeys ")
        .title_bottom(Line::from(Span::styled(
            " Esc: close ",
            Style::default().fg(Color::DarkGray),
        )))
        .borders(Borders::ALL)
        .padding(Padding::new(2, 2, 1, 1));
    let inner = block.inner(area);
    f.render_widget(block, area);
    lines.truncate(inner.height as usize);
    f.render_widget(Paragraph::new(Text::from(lines)), inner);
}

/// Centered popup sized to the help content, clamped to the terminal.
fn help_rect(area: Rect, lines: &[Line]) -> Rect {
    let content_w = lines
        .iter()
        .map(|l| {
            l.spans
                .iter()
                .map(|s| s.content.chars().count())
                .sum::<usize>()
        })
        .max()
        .unwrap_or(0) as u16;
    // Borders (2) + horizontal padding (4) / vertical padding (2).
    let w = (content_w + 6).clamp(20, area.width);
    let h = (lines.len() as u16 + 4).clamp(6, area.height);
    let x = area.width.saturating_sub(w) / 2;
    let y = area.height.saturating_sub(h) / 2;
    Rect::new(x, y, w, h)
}
