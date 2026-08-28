//! Main-pane renderer (PTY grid or placeholder).

use crate::tui::app::{InputFocus, TuiApp};
use crate::tui::header::Status;
use crate::tui::pty_host::{Color as PtyColor, grid_to_ratatui};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};

pub(crate) fn render_main(f: &mut Frame, area: Rect, app: &TuiApp) {
    if app.has_pty {
        // PTY mode: render the vt100 grid inside a box.
        let scrollback = app.pty_parser.screen().scrollback();
        let title = if scrollback > 0 {
            format!(" {} [scrollback -{scrollback}] ", app.agent_label)
        } else {
            format!(" {} ", app.agent_label)
        };
        let inner = Block::default()
            .title(title)
            .borders(Borders::ALL)
            .border_style(if app.input_focus == InputFocus::Pty {
                Style::default().fg(Color::Cyan)
            } else {
                Style::default()
            });
        let inner_area = inner.inner(area);
        f.render_widget(inner, area);

        let grid = grid_to_ratatui(&app.pty_parser, inner_area);
        let lines: Vec<Line> = grid
            .iter()
            .map(|row| {
                let spans: Vec<Span> = row
                    .iter()
                    .map(|cell| {
                        let fg = match cell.fg {
                            PtyColor::Default => Color::Reset,
                            PtyColor::Rgb(r, g, b) => Color::Rgb(r, g, b),
                            PtyColor::Indexed(i) => Color::Indexed(i),
                        };
                        let bg = match cell.bg {
                            PtyColor::Default => Color::Reset,
                            PtyColor::Rgb(r, g, b) => Color::Rgb(r, g, b),
                            PtyColor::Indexed(i) => Color::Indexed(i),
                        };
                        let mut style = Style::default().fg(fg).bg(bg);
                        if cell.bold {
                            style = style.add_modifier(Modifier::BOLD);
                        }
                        if cell.underline {
                            style = style.add_modifier(Modifier::UNDERLINED);
                        }
                        if cell.reverse {
                            style = style.add_modifier(Modifier::REVERSED);
                        }
                        Span::styled(cell.ch.to_string(), style)
                    })
                    .collect();
                Line::from(spans)
            })
            .collect();
        f.render_widget(Paragraph::new(Text::from(lines)), inner_area);
    } else if app.agent_is_desktop {
        render_desktop_agent(f, area, app);
    } else {
        // No PTY: placeholder.
        let body = match app.header.status {
            Status::Running => String::new(),
            _ => "Waiting for lobby…".to_string(),
        };
        let p = Paragraph::new(body)
            .block(Block::default().title(" agent ").borders(Borders::ALL))
            .wrap(Wrap { trim: false });
        f.render_widget(p, area);
    }
}

/// Pane shown when the agent runs in its own window.
///
/// A blank pane reads as a crash, so say plainly where the agent went, and be
/// straight about the consequence: `ololo` cannot see a desktop agent's
/// conversation the way it sees a terminal one. Probes and scoring are
/// unaffected — they run against the working directory either way — so the
/// participant is not at a disadvantage, just less observed.
fn render_desktop_agent(f: &mut Frame, area: Rect, app: &TuiApp) {
    let dim = Style::default().fg(Color::DarkGray);
    let strong = Style::default().add_modifier(Modifier::BOLD);
    let ok = Style::default().fg(Color::Green);
    let muted = Style::default().fg(Color::Yellow);

    let mut lines: Vec<Line> = vec![
        Line::from(""),
        Line::from(vec![
            Span::styled("  ", Style::default()),
            Span::styled(app.agent_label.clone(), strong),
            Span::styled(" is running in its own window.", Style::default()),
        ]),
        Line::from(""),
        Line::from(vec![Span::styled(
            "  Switch to it to work on the task — this pane stays here.",
            dim,
        )]),
        Line::from(""),
        Line::from(vec![Span::styled("  Still tracked:", strong)]),
        Line::from(vec![
            Span::styled("    ✓ ", ok),
            Span::styled("probes and scoring — they run against your files", dim),
        ]),
        Line::from(vec![
            Span::styled("    ✓ ", ok),
            Span::styled("token usage — read from the agent's own store", dim),
        ]),
        Line::from(vec![
            Span::styled("    ✓ ", ok),
            Span::styled("your commits and diffs", dim),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("    – ", muted),
            Span::styled("the conversation itself is not visible here", dim),
        ]),
    ];

    if app.header.status != Status::Running {
        lines.push(Line::from(""));
        lines.push(Line::from(vec![Span::styled(
            "  Waiting for the session to start…",
            muted,
        )]));
    }

    let title = format!(" {} (desktop app) ", app.agent_label);
    let p = Paragraph::new(Text::from(lines))
        .block(Block::default().title(title).borders(Borders::ALL))
        .wrap(Wrap { trim: false });
    f.render_widget(p, area);
}
