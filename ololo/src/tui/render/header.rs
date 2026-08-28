//! Header (top status) row renderer.

use crate::tui::app::TuiApp;
use crate::tui::header::Status;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

pub(crate) fn render_header(f: &mut Frame, area: Rect, app: &TuiApp) {
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
    if h.project_url.is_empty() {
        spans.push(Span::raw(format!(" project={} ", h.project)));
    } else {
        spans.push(Span::raw(" 📦 "));
        spans.push(Span::styled(h.project_url.clone(), link_style));
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

/// 0-based, half-open `[start, end)` column ranges of the session and
/// project links within the header row, or `None` for a link that isn't
/// rendered. Used to hit-test mouse clicks so only the link (not the
/// whole header) opens the browser. Mirrors the prefix layout in
/// `render_header`.
pub(crate) fn header_link_ranges(
    session_url: &str,
    project_url: &str,
) -> (Option<(u16, u16)>, Option<(u16, u16)>) {
    let mut col: u16 = 0;
    let session_range = if session_url.is_empty() {
        None
    } else {
        col += Span::raw(" 🔗 ").width() as u16;
        let w = Span::raw(session_url).width() as u16;
        let r = Some((col, col + w));
        col += w + Span::raw(" ").width() as u16;
        r
    };
    let project_range = if project_url.is_empty() {
        None
    } else {
        col += Span::raw(" 📦 ").width() as u16;
        let w = Span::raw(project_url).width() as u16;
        Some((col, col + w))
    };
    (session_range, project_range)
}
