//! The F3 menu: what the player can hand the hosted agent — artifact
//! requests, the task brief, the failing check, the task's check results,
//! code health, the judges' verdicts, earlier briefs. Every row says what
//! it carries on a second line; artifact requests show their live
//! countdown; what the agent already has is marked `✓ pasted`.

use crate::tui::app::{
    PasteItem, PasteKind, PastePicker, PasteTab, TuiApp, fmt_countdown, time_left,
};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Clear, Padding, Paragraph};

/// The mark of an item the agent already has.
const PASTED: &str = " ✓ pasted ";
/// Rows one item takes: its label and what it hands over.
const ITEM_ROWS: usize = 2;

fn clip(text: &str, width: usize) -> String {
    let mut out: String = text.chars().take(width).collect();
    if text.chars().count() > width {
        out.pop();
        out.push('…');
    }
    out
}

fn glyph(kind: PasteKind) -> (&'static str, Color) {
    match kind {
        PasteKind::Request => ("⚖", Color::Yellow),
        PasteKind::Brief => ("▸", Color::Cyan),
        PasteKind::FailedCheck => ("✗", Color::Red),
        PasteKind::Checks => ("≡", Color::Cyan),
        PasteKind::Health => ("♥", Color::Green),
        PasteKind::Verdicts => ("★", Color::Yellow),
        PasteKind::EarlierBrief => ("▹", Color::DarkGray),
        PasteKind::Check => ("·", Color::Cyan),
        PasteKind::Verdict => ("☆", Color::Yellow),
    }
}

fn item_lines(i: usize, item: &PasteItem, selected: bool, label_w: usize) -> [Line<'static>; 2] {
    let (glyph, glyph_color) = glyph(item.kind);
    let base = if selected {
        Style::default().bg(Color::Rgb(40, 52, 72))
    } else {
        Style::default()
    };
    let bold = base.add_modifier(Modifier::BOLD);
    // Pasted before: the label steps back, the mark says why.
    let label_color = if item.pasted {
        Color::Gray
    } else {
        Color::White
    };
    // Only the first nine rows answer to a digit.
    let number = if i < 9 {
        format!(" {} ", i + 1)
    } else {
        "   ".to_string()
    };
    let label = clip(&item.label, label_w);
    let mut spans = vec![
        Span::styled(number, base.fg(Color::DarkGray)),
        Span::styled(format!("{glyph} "), bold.fg(glyph_color)),
        Span::styled(format!("{label:<label_w$}"), bold.fg(label_color)),
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
                bold.fg(color),
            ));
        }
        None => spans.push(Span::styled(" ".repeat(13), base)),
    }
    let hint_w = label_w + PASTED.chars().count() + 13;
    let hint = clip(&item.hint, hint_w);
    let hint_line = Line::from(vec![
        Span::styled("     ", base),
        Span::styled(format!("{hint:<hint_w$}"), base.fg(Color::DarkGray)),
    ]);
    [Line::from(spans), hint_line]
}

/// Rows above the items: the tab bar and a blank line under it.
const TAB_ROWS: usize = 2;

/// ` All 7   Tasks 2   Judges 3  …` — the open tab reversed, the empty dimmed.
fn tab_bar(picker: &PastePicker) -> Line<'static> {
    let mut spans = vec![Span::raw(" ")];
    for (tab, count) in PasteTab::ALL.iter().zip(picker.counts) {
        let style = if *tab == picker.tab {
            Style::default()
                .fg(Color::Black)
                .bg(Color::Cyan)
                .add_modifier(Modifier::BOLD)
        } else if count == 0 {
            Style::default().fg(Color::DarkGray)
        } else {
            Style::default().fg(Color::Gray)
        };
        spans.push(Span::styled(format!(" {} {count} ", tab.title()), style));
        spans.push(Span::raw(" "));
    }
    Line::from(spans)
}

pub(crate) fn render_paste_picker(f: &mut Frame, app: &TuiApp) {
    let Some(picker) = app.paste_picker.as_ref() else {
        return;
    };
    let area = f.area();
    let w = area.width.saturating_sub(4).clamp(30, 104);
    // Number, glyph, borders and padding, the countdown and the mark.
    let label_w = (w as usize)
        .saturating_sub(4 + 4 + 3 + 16 + PASTED.chars().count())
        .max(10);

    // Sized for the fullest tab, so switching tabs does not resize the menu.
    let most = picker.counts.iter().copied().max().unwrap_or(0).max(1);
    let h = ((most * ITEM_ROWS + TAB_ROWS) as u16 + 4).clamp(8, area.height);
    let rect = Rect::new(
        area.width.saturating_sub(w) / 2,
        area.height.saturating_sub(h) / 2,
        w,
        h,
    );
    f.render_widget(Clear, rect);
    let hint = if picker.items.iter().any(|i| i.pasted) {
        " ✓ the agent has it · ←/→ tabs · ↑/↓ choose · ⏎ or 1–9 paste · Esc close "
    } else {
        " ←/→ tabs · ↑/↓ choose · ⏎ or 1–9 paste · Esc close "
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

    // A window of whole items that keeps the cursor in view.
    let fit = ((inner.height as usize).saturating_sub(TAB_ROWS) / ITEM_ROWS).max(1);
    let first = (picker.cursor + 1).saturating_sub(fit);
    let mut lines: Vec<Line> = vec![tab_bar(picker), Line::default()];
    lines.extend(
        picker
            .items
            .iter()
            .enumerate()
            .skip(first)
            .take(fit)
            .flat_map(|(i, item)| item_lines(i, item, i == picker.cursor, label_w)),
    );
    f.render_widget(Paragraph::new(Text::from(lines)), inner);
}
