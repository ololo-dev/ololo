//! Probe-permission popup: a modal question shown before an unapproved
//! probe command runs. Rendered above every other overlay — nothing else
//! may obscure the command the player is being asked to approve.

use crate::tui::app::TuiApp;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Clear, Padding, Paragraph, Wrap};

pub(crate) fn render_permission_popup(f: &mut Frame, app: &TuiApp) {
    let Some(prompt) = app.permission_popup.as_ref() else {
        return;
    };

    let key_style = Style::default()
        .fg(Color::Cyan)
        .add_modifier(Modifier::BOLD);
    let cmd_style = Style::default().fg(Color::Yellow);
    let dim = Style::default().fg(Color::DarkGray);
    let selected = Style::default()
        .bg(Color::Cyan)
        .fg(Color::Black)
        .add_modifier(Modifier::BOLD);

    let options: [(char, String); 4] = [
        ('a', "allow once".to_string()),
        (
            'w',
            format!("always allow — saves \"{}\"", prompt.always_rule),
        ),
        (
            's',
            "approve all for the session (nothing saved, asks again next session)".to_string(),
        ),
        ('d', "decline (the probe is reported as failed)".to_string()),
    ];

    let mut lines: Vec<Line> = vec![
        Line::from("The session wants to run this command in your workspace:"),
        Line::default(),
        Line::from(Span::styled(prompt.command.clone(), cmd_style)),
        Line::default(),
    ];
    for (i, (key, label)) in options.iter().enumerate() {
        let is_selected = app.permission_cursor == i;
        let marker = if is_selected { "▶ " } else { "  " };
        let row = format!("{marker}{key}  {label}");
        lines.push(if is_selected {
            Line::from(Span::styled(row, selected))
        } else {
            Line::from(vec![
                Span::raw(marker.to_string()),
                Span::styled(format!("{key}  "), key_style),
                Span::raw(label.clone()),
            ])
        });
    }
    lines.push(Line::default());
    lines.push(Line::from(Span::styled(
        format!(
            "unanswered past the {}s probe deadline = declined",
            prompt.deadline_secs.max(1)
        ),
        dim,
    )));

    let area = popup_rect(f.area());
    f.render_widget(Clear, area);
    let block = Block::default()
        .title(" probe permission ")
        .title_bottom(Line::from(Span::styled(
            " ↑/↓ move · Enter confirm · a/w/s/d shortcuts · Esc decline ",
            dim,
        )))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Yellow))
        .padding(Padding::new(2, 2, 1, 1));
    let inner = block.inner(area);
    f.render_widget(block, area);
    lines.truncate(inner.height as usize);
    f.render_widget(
        Paragraph::new(Text::from(lines)).wrap(Wrap { trim: false }),
        inner,
    );
}

/// Centered popup: ~70% wide, height for the fixed lines plus a wrapped
/// command, clamped to the terminal.
fn popup_rect(area: Rect) -> Rect {
    let w = (area.width * 7 / 10).clamp(30, area.width);
    let h = 15.min(area.height);
    let x = area.width.saturating_sub(w) / 2;
    let y = area.height.saturating_sub(h) / 2;
    Rect::new(x, y, w, h)
}
