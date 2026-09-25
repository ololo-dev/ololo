//! The F3 picker: when several things are open for the agent — artifact
//! requests, the task brief, a failed check — the player chooses which one
//! to paste. Artifact requests carry their live countdown; what the agent
//! already has is marked `✓ pasted`.

use crate::tui::app::{PasteKind, TuiApp, fmt_countdown, time_left};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Clear, Padding, Paragraph};

/// The mark of an item the agent already has.
const PASTED: &str = " ✓ pasted ";

pub(crate) fn render_paste_picker(f: &mut Frame, app: &TuiApp) {
    let Some(picker) = app.paste_picker.as_ref() else {
        return;
    };
    let area = f.area();
    let w = area.width.saturating_sub(4).clamp(30, 96);
    // Number, glyph, borders and padding, the countdown and the mark.
    let label_w = (w as usize)
        .saturating_sub(4 + 4 + 3 + 16 + PASTED.chars().count())
        .max(10);

    let mut lines: Vec<Line> = Vec::new();
    for (i, item) in picker.items.iter().enumerate() {
        let selected = i == picker.cursor;
        let (glyph, glyph_color) = match item.kind {
            PasteKind::Request => ("⚖", Color::Yellow),
            PasteKind::Brief => ("▸", Color::Cyan),
            PasteKind::FailedCheck => ("✗", Color::Red),
        };
        let mut label: String = item.label.chars().take(label_w).collect();
        if item.label.chars().count() > label_w {
            label.pop();
            label.push('…');
        }
        let base = if selected {
            Style::default()
                .bg(Color::Rgb(40, 52, 72))
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };
        // Pasted before: the label steps back, the mark says why.
        let label_color = if item.pasted {
            Color::Gray
        } else {
            Color::White
        };
        let mut spans = vec![
            Span::styled(format!(" {} ", i + 1), base.fg(Color::DarkGray)),
            Span::styled(format!("{glyph} "), base.fg(glyph_color)),
            Span::styled(format!("{label:<label_w$}"), base.fg(label_color)),
            if item.pasted {
                Span::styled(PASTED, base.fg(Color::Green))
            } else {
                Span::styled(" ".repeat(PASTED.chars().count()), base)
            },
        ];
        match time_left(item.deadline) {
            Some(left) => {
                let color = if left.as_secs() < 60 {
                    Color::Red
                } else {
                    Color::Yellow
                };
                spans.push(Span::styled(
                    format!(" {:>6} left ", fmt_countdown(left)),
                    base.fg(color).add_modifier(Modifier::BOLD),
                ));
            }
            None => spans.push(Span::styled(" ".repeat(13), base)),
        }
        lines.push(Line::from(spans));
    }

    let h = (lines.len() as u16 + 4).clamp(6, area.height);
    let rect = Rect::new(
        area.width.saturating_sub(w) / 2,
        area.height.saturating_sub(h) / 2,
        w,
        h,
    );
    f.render_widget(Clear, rect);
    let hint = if picker.items.iter().any(|i| i.pasted) {
        " ✓ the agent has it · ↑/↓ choose · ⏎ or 1–9 paste · Esc close "
    } else {
        " ↑/↓ choose · ⏎ or 1–9 paste · Esc close "
    };
    let block = Block::default()
        .title(" paste to the agent ")
        .title_bottom(Line::from(Span::styled(
            hint,
            Style::default().fg(Color::DarkGray),
        )))
        .borders(Borders::ALL)
        .padding(Padding::new(1, 1, 1, 1));
    let inner = block.inner(rect);
    f.render_widget(block, rect);
    lines.truncate(inner.height as usize);
    f.render_widget(Paragraph::new(Text::from(lines)), inner);
}
