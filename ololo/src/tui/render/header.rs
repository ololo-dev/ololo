//! Header (top status) row renderer.

use crate::tui::app::TuiApp;
use crate::tui::header::Status;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

/// The header's right-hand buttons, each a label and the hotkey it stands
/// for. Clicking one does what its key does.
const BUTTONS: [&str; 2] = ["[Help - F1]", "[Exit - F10]"];

/// A clickable header button.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HeaderButton {
    Help,
    Exit,
}

/// Columns the right-aligned buttons take, trailing space included.
fn buttons_width() -> u16 {
    BUTTONS
        .iter()
        .map(|b| Span::raw(*b).width() as u16 + 1)
        .sum()
}

/// The buttons only show when the session link still fits beside them.
const MIN_LEFT_WIDTH: u16 = 20;

/// The button under `column` on a header row `width` columns wide, if any.
/// Mirrors the right-aligned layout in `render_header`.
pub(crate) fn header_button_at(width: u16, column: u16) -> Option<HeaderButton> {
    let total = buttons_width();
    if width < total + MIN_LEFT_WIDTH {
        return None;
    }
    let mut col = width - total;
    for (label, button) in BUTTONS.iter().zip([HeaderButton::Help, HeaderButton::Exit]) {
        let w = Span::raw(*label).width() as u16;
        if column >= col && column < col + w {
            return Some(button);
        }
        col += w + 1;
    }
    None
}

pub(crate) fn render_header(f: &mut Frame, area: Rect, app: &TuiApp) {
    let (area, buttons_area) = if area.width >= buttons_width() + MIN_LEFT_WIDTH {
        let w = buttons_width();
        (
            Rect {
                width: area.width - w,
                ..area
            },
            Some(Rect {
                x: area.x + area.width - w,
                width: w,
                ..area
            }),
        )
    } else {
        (area, None)
    };
    if let Some(buttons_area) = buttons_area {
        let style = Style::default().add_modifier(Modifier::BOLD);
        let mut spans = vec![];
        for label in BUTTONS {
            spans.push(Span::styled(label, style));
            spans.push(Span::raw(" "));
        }
        f.render_widget(Paragraph::new(Line::from(spans)), buttons_area);
    }
    let h = &app.header;
    let (status_emoji, status_label) = match h.status {
        Status::Connecting => ("🔌", "Connecting".to_string()),
        Status::Lobby => match h.countdown_mmss() {
            Some(s) => ("🕒", format!("Lobby {s}")),
            None => ("🕒", "Lobby".to_string()),
        },
        Status::Running if app.judging => match h.countdown_mmss() {
            Some(s) => ("⚖️", format!("Judges reviewing your work {s}")),
            None => ("⚖️", "Judges reviewing your work".to_string()),
        },
        Status::Running => match h.countdown_mmss() {
            Some(s) => ("🏃", format!("Running {s}")),
            None => ("🏃", "Running".to_string()),
        },
        Status::Paused => match h.countdown_mmss() {
            Some(s) => ("⏸️", format!("Paused {s}")),
            None => ("⏸️", "Paused".to_string()),
        },
        Status::TasksDone => match h.countdown_mmss() {
            Some(s) => ("✅", format!("Tasks done — waiting for session finish {s}")),
            None => ("✅", "Tasks done — waiting for session finish".to_string()),
        },
        Status::Complete => ("🏁", "Complete".to_string()),
        Status::Cancelled => ("🚫", "Cancelled".to_string()),
        Status::Error => (
            "❌",
            format!(
                "Error{}",
                h.error_message
                    .as_deref()
                    .map(|m| format!(": {m}"))
                    .unwrap_or_default()
            ),
        ),
    };
    let score_str = match app.score {
        Some(s) => s.to_string(),
        None => "—".to_string(),
    };
    let rank_str = match app.rank {
        Some(r) => format!("#{r}"),
        None => "—".to_string(),
    };
    let link_style = Style::default()
        .fg(Color::Cyan)
        .add_modifier(Modifier::UNDERLINED);
    let mut spans = vec![];
    if h.session_url.is_empty() {
        spans.push(Span::raw(format!(" session={} ", h.session)));
    } else {
        spans.push(Span::raw(" 🔗 "));
        spans.push(Span::styled(h.session_url.clone(), link_style));
        spans.push(Span::raw(" "));
    }
    spans.push(Span::raw(format!(" {status_emoji} {status_label} ")));
    spans.push(Span::raw(format!(" score={score_str} ")));
    spans.push(Span::raw(format!(" rank={rank_str} ")));
    if let Some(v) = &h.update_available {
        spans.push(Span::styled(
            format!(" ⬆ v{v} available — ololo update "),
            Style::default().fg(Color::Yellow),
        ));
    }
    let d = app.diff_stats;
    if d.added != 0 || d.modified != 0 || d.deleted != 0 {
        let green = Style::default()
            .fg(Color::Green)
            .add_modifier(Modifier::BOLD);
        let yellow = Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD);
        let red = Style::default().fg(Color::Red).add_modifier(Modifier::BOLD);
        let mut seg = vec![Span::raw(" "), Span::raw("[")];
        if d.added > 0 {
            seg.push(Span::styled(format!("+{}", d.added), green));
        }
        if d.modified > 0 {
            seg.push(Span::styled(format!("!{}", d.modified), yellow));
        }
        if d.deleted > 0 {
            seg.push(Span::styled(format!("-{}", d.deleted), red));
        }
        seg.push(Span::raw("]"));
        let prefix_w: usize = spans.iter().map(|s| s.width()).sum();
        let seg_w: usize = seg.iter().map(|s| s.width()).sum();
        if (area.width as usize) > prefix_w + seg_w {
            spans.extend(seg);
        }
    }
    let p = Paragraph::new(Line::from(spans));
    f.render_widget(p, area);
}

/// 0-based, half-open `[start, end)` column range of the session link
/// within the header row, or `None` when it isn't rendered. Used to
/// hit-test mouse clicks so only the link (not the whole header) opens the
/// browser. Mirrors the prefix layout in `render_header`.
pub(crate) fn header_link_range(session_url: &str) -> Option<(u16, u16)> {
    if session_url.is_empty() {
        return None;
    }
    let col = Span::raw(" 🔗 ").width() as u16;
    let w = Span::raw(session_url).width() as u16;
    Some((col, col + w))
}
