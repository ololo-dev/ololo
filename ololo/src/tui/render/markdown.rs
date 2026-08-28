//! Markdown → styled terminal lines, for the chat pane's bubbles.
//!
//! Briefs, done-notes and judge feedback arrive as markdown. Showing the
//! raw sigils (`**`, `#`, `-`) makes the reader parse markup; rendering it
//! keeps the bubble readable at a glance. This stays a chat bubble, not a
//! browser: everything becomes wrapped ratatui lines in the bubble's own
//! palette — emphasis as terminal modifiers, headings and code one shade
//! brighter than the body, lists with hanging indents.
//!
//! Single newlines inside a paragraph stay line breaks (chat convention,
//! and Gherkin-style briefs depend on it); blank lines separate blocks.

use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

/// Render `text` as markdown wrapped to `width`, then push every line
/// right by `indent` — the caller's bubble indentation.
pub(crate) fn md_indented(
    text: &str,
    indent: &str,
    width: usize,
    base: Style,
) -> Vec<Line<'static>> {
    let indent_w = indent.chars().count();
    md_lines(text, width.saturating_sub(indent_w).max(1), base)
        .into_iter()
        .map(|line| {
            let mut spans = vec![Span::raw(indent.to_string())];
            spans.extend(line.spans);
            Line::from(spans)
        })
        .collect()
}

/// Render `text` as markdown into lines of at most `width` chars, styled
/// relative to `base`. Always returns at least one (possibly empty) line.
pub(crate) fn md_lines(text: &str, width: usize, base: Style) -> Vec<Line<'static>> {
    let mut r = Renderer::new(width.max(1), base);
    let opts = Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS;
    for ev in Parser::new_ext(text, opts) {
        match ev {
            Event::Start(tag) => r.start(tag),
            Event::End(tag) => r.end(tag),
            Event::Text(t) => match r.code.as_mut() {
                Some(code) => code.push_str(&t),
                None => r.push_text(&t, r.style()),
            },
            Event::Code(t) => r.push_text(&t, r.code_style()),
            // Chat convention: a single newline is a line break, not a
            // space — Gherkin steps in briefs must not flow together.
            Event::SoftBreak | Event::HardBreak => r.flush(),
            Event::Rule => r.rule(),
            Event::TaskListMarker(done) => {
                r.push_text(if done { "[x] " } else { "[ ] " }, r.style())
            }
            Event::Html(t) | Event::InlineHtml(t) => r.push_text(&t, r.style()),
            _ => {}
        }
    }
    r.finish()
}

/// One shade brighter than the body — how headings and code stand apart
/// without leaving the bubble's palette.
fn lift(c: Color) -> Color {
    match c {
        Color::Gray => Color::White,
        Color::DarkGray => Color::Gray,
        other => other,
    }
}

/// A word: styled fragments with no whitespace inside (`**bold**tail` is
/// one word of two fragments).
type Word = Vec<(String, Style)>;

struct Renderer {
    width: usize,
    base: Style,
    out: Vec<Line<'static>>,
    /// Words of the block being collected, plus the fragment in progress.
    words: Vec<Word>,
    cur: Word,
    /// Inline style stack; `last()` is what new text gets.
    styles: Vec<Style>,
    /// One entry per open list: next ordinal for ordered, None for bullets.
    lists: Vec<Option<u64>>,
    /// The current block's first-line prefix (list marker) and the hanging
    /// indent every following line gets.
    first_prefix: String,
    cont_prefix: String,
    /// The block already emitted a line — its marker is spent.
    block_started: bool,
    /// Put a blank spacer row before the next emitted line.
    want_blank: bool,
    /// The current list item has not yet seen its first paragraph.
    item_fresh: bool,
    quote_depth: usize,
    /// A fenced/indented code block being collected verbatim.
    code: Option<String>,
}

impl Renderer {
    fn new(width: usize, base: Style) -> Self {
        Self {
            width,
            base,
            out: Vec::new(),
            words: Vec::new(),
            cur: Vec::new(),
            styles: vec![base],
            lists: Vec::new(),
            first_prefix: String::new(),
            cont_prefix: String::new(),
            block_started: false,
            want_blank: false,
            item_fresh: false,
            quote_depth: 0,
            code: None,
        }
    }

    fn style(&self) -> Style {
        *self.styles.last().unwrap_or(&self.base)
    }

    fn code_style(&self) -> Style {
        match self.base.fg {
            Some(c) => self.base.fg(lift(c)),
            None => self.base,
        }
    }

    fn heading_style(&self) -> Style {
        self.code_style().add_modifier(Modifier::BOLD)
    }

    fn start(&mut self, tag: Tag<'_>) {
        match tag {
            Tag::Paragraph => {
                if self.item_fresh {
                    // The item's first paragraph rides the marker row.
                    self.item_fresh = false;
                } else {
                    self.begin_block();
                }
            }
            Tag::Heading { .. } => {
                self.begin_block();
                self.styles.push(self.heading_style());
            }
            Tag::List(start) => self.lists.push(start),
            Tag::Item => {
                let depth = self.lists.len().max(1);
                let indent = "  ".repeat(depth - 1);
                let marker = match self.lists.last_mut().and_then(|s| s.as_mut()) {
                    Some(n) => {
                        let m = format!("{n}. ");
                        *n += 1;
                        m
                    }
                    None => "• ".to_string(),
                };
                self.cont_prefix = format!("{}{}", indent, " ".repeat(marker.chars().count()));
                self.first_prefix = format!("{indent}{marker}");
                self.block_started = false;
                self.item_fresh = true;
                // Tight lists read as one bubble paragraph — no spacer
                // between sibling items.
                self.want_blank = false;
            }
            Tag::CodeBlock(_) => {
                self.begin_block();
                self.code = Some(String::new());
            }
            Tag::BlockQuote(_) => self.quote_depth += 1,
            Tag::Emphasis => self
                .styles
                .push(self.style().add_modifier(Modifier::ITALIC)),
            Tag::Strong => self.styles.push(self.style().add_modifier(Modifier::BOLD)),
            Tag::Strikethrough => self
                .styles
                .push(self.style().add_modifier(Modifier::CROSSED_OUT)),
            // The link's own words, underlined; a brief has no use for a
            // URL nobody can click.
            Tag::Link { .. } | Tag::Image { .. } => self
                .styles
                .push(self.style().add_modifier(Modifier::UNDERLINED)),
            _ => {}
        }
    }

    fn end(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Paragraph => self.flush(),
            TagEnd::Heading(_) => {
                self.flush();
                self.styles.pop();
            }
            TagEnd::Item => {
                self.flush();
                self.item_fresh = false;
            }
            TagEnd::List(_) => {
                self.lists.pop();
                if self.lists.is_empty() {
                    self.first_prefix.clear();
                    self.cont_prefix.clear();
                }
            }
            TagEnd::CodeBlock => {
                let code = self.code.take().unwrap_or_default();
                self.emit_code(&code);
            }
            TagEnd::BlockQuote(_) => self.quote_depth = self.quote_depth.saturating_sub(1),
            TagEnd::Emphasis
            | TagEnd::Strong
            | TagEnd::Strikethrough
            | TagEnd::Link
            | TagEnd::Image => {
                self.styles.pop();
            }
            _ => {}
        }
    }

    /// A new block begins outside any list marker: default prefixes, and a
    /// spacer before its first line.
    fn begin_block(&mut self) {
        if self.lists.is_empty() {
            self.first_prefix.clear();
            self.cont_prefix.clear();
            self.block_started = false;
        }
        self.want_blank = true;
    }

    fn push_text(&mut self, s: &str, style: Style) {
        for ch in s.chars() {
            if ch.is_whitespace() {
                self.end_word();
            } else {
                match self.cur.last_mut() {
                    Some((frag, st)) if *st == style => frag.push(ch),
                    _ => self.cur.push((ch.to_string(), style)),
                }
            }
        }
    }

    fn end_word(&mut self) {
        if !self.cur.is_empty() {
            self.words.push(std::mem::take(&mut self.cur));
        }
    }

    /// The prefix the `i`-th wrapped line of the current flush carries.
    fn prefix_at(&self, i: usize) -> String {
        let p = if !self.block_started && i == 0 {
            &self.first_prefix
        } else {
            &self.cont_prefix
        };
        format!("{}{}", "▏ ".repeat(self.quote_depth), p)
    }

    fn spacer(&mut self) {
        if self.want_blank && !self.out.is_empty() {
            self.out.push(Line::default());
        }
        self.want_blank = false;
    }

    /// Greedy-wrap the collected words into lines and emit them. Words
    /// longer than a line are hard-broken, fragment styles intact.
    fn flush(&mut self) {
        self.end_word();
        if self.words.is_empty() {
            return;
        }
        self.spacer();
        let words = std::mem::take(&mut self.words);

        let mut built: Vec<Line<'static>> = Vec::new();
        let mut cur: Word = Vec::new();
        let mut cur_len = 0usize;
        for word in words {
            let wlen: usize = word.iter().map(|(s, _)| s.chars().count()).sum();
            let avail = |i: usize, s: &Self| {
                s.width
                    .saturating_sub(s.prefix_at(i).chars().count())
                    .max(1)
            };
            if cur_len > 0 && cur_len + 1 + wlen > avail(built.len(), self) {
                built.push(self.mk_line(built.len(), std::mem::take(&mut cur)));
                cur_len = 0;
            }
            if wlen > avail(built.len(), self) {
                // Hard-break: spill the word's chars across lines; the
                // final chunk stays open so following words can join it.
                let mut frags: Word = word.into_iter().collect();
                loop {
                    let mut room = avail(built.len(), self);
                    let mut chunk: Word = Vec::new();
                    while room > 0 && !frags.is_empty() {
                        let (s, st) = frags.remove(0);
                        let n = s.chars().count();
                        if n <= room {
                            room -= n;
                            chunk.push((s, st));
                        } else {
                            let head: String = s.chars().take(room).collect();
                            let tail: String = s.chars().skip(room).collect();
                            chunk.push((head, st));
                            frags.insert(0, (tail, st));
                            room = 0;
                        }
                    }
                    if frags.is_empty() {
                        cur_len = chunk.iter().map(|(s, _)| s.chars().count()).sum();
                        cur = chunk;
                        break;
                    }
                    built.push(self.mk_line(built.len(), chunk));
                }
                continue;
            }
            if cur_len > 0 {
                cur.push((" ".to_string(), self.base));
                cur_len += 1;
            }
            cur_len += wlen;
            cur.extend(word);
        }
        if !cur.is_empty() {
            built.push(self.mk_line(built.len(), cur));
        }
        self.out.extend(built);
        self.block_started = true;
    }

    fn mk_line(&self, i: usize, frags: Word) -> Line<'static> {
        let mut spans = vec![Span::styled(self.prefix_at(i), self.base)];
        spans.extend(frags.into_iter().map(|(s, st)| Span::styled(s, st)));
        Line::from(spans)
    }

    /// A fenced block, verbatim: no re-wrapping into prose, over-long rows
    /// hard-broken, in the lifted code shade.
    fn emit_code(&mut self, code: &str) {
        self.spacer();
        let style = self.code_style();
        let prefix = format!("{}{}", "▏ ".repeat(self.quote_depth), self.cont_prefix);
        let avail = self.width.saturating_sub(prefix.chars().count()).max(1);
        for raw in code.trim_end().lines() {
            let chars: Vec<char> = raw.chars().collect();
            let mut start = 0;
            loop {
                let end = (start + avail).min(chars.len());
                let seg: String = chars[start..end].iter().collect();
                self.out.push(Line::from(vec![
                    Span::styled(prefix.clone(), self.base),
                    Span::styled(seg, style),
                ]));
                start = end;
                if start >= chars.len() {
                    break;
                }
            }
        }
        self.block_started = true;
    }

    fn rule(&mut self) {
        self.want_blank = true;
        self.spacer();
        self.out.push(Line::from(Span::styled(
            "─".repeat(self.width.min(24)),
            self.base,
        )));
    }

    fn finish(mut self) -> Vec<Line<'static>> {
        self.flush();
        if self.out.is_empty() {
            self.out.push(Line::default());
        }
        self.out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat(lines: &[Line<'_>]) -> Vec<String> {
        lines
            .iter()
            .map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect())
            .collect()
    }

    #[test]
    fn bold_sigils_become_style_not_text() {
        let lines = md_lines(
            "build a **weather** widget",
            40,
            Style::default().fg(Color::Gray),
        );
        let text = flat(&lines).join(" ");
        assert!(!text.contains("**"), "sigils must not survive: {text}");
        assert!(text.contains("weather"));
        let bold = lines[0]
            .spans
            .iter()
            .any(|s| s.content == "weather" && s.style.add_modifier.contains(Modifier::BOLD));
        assert!(bold, "the word must carry the bold modifier");
    }

    #[test]
    fn inline_code_is_lifted_a_shade() {
        let lines = md_lines("run `sh answer.sh`", 40, Style::default().fg(Color::Gray));
        let lifted = lines[0]
            .spans
            .iter()
            .any(|s| s.content.contains("answer.sh") && s.style.fg == Some(Color::White));
        assert!(lifted, "code spans render one shade brighter");
    }

    #[test]
    fn list_items_get_bullets_and_hanging_indent() {
        let lines = md_lines(
            "- a fairly long first item that wraps around\n- second",
            20,
            Style::default().fg(Color::Gray),
        );
        let rows = flat(&lines);
        assert!(rows[0].starts_with("• a fairly"), "bullet marker: {rows:?}");
        assert!(rows[1].starts_with("  "), "hanging indent: {rows:?}");
        assert!(
            rows.iter().any(|r| r.starts_with("• second")),
            "second bullet: {rows:?}"
        );
        assert!(
            rows.iter().all(|r| !r.is_empty()),
            "tight list has no spacer rows: {rows:?}"
        );
    }

    #[test]
    fn ordered_lists_count() {
        let rows = flat(&md_lines("1. one\n2. two", 20, Style::default()));
        assert_eq!(rows, vec!["1. one", "2. two"]);
    }

    #[test]
    fn headings_are_bold_and_unhashed() {
        let lines = md_lines(
            "## Scenario: first search",
            40,
            Style::default().fg(Color::Gray),
        );
        let text = flat(&lines).join(" ");
        assert!(!text.contains('#'), "hash sigils must not survive: {text}");
        let styled = lines[0].spans.iter().any(|s| {
            s.content.contains("Scenario")
                && s.style.add_modifier.contains(Modifier::BOLD)
                && s.style.fg == Some(Color::White)
        });
        assert!(styled, "headings render bold and lifted");
    }

    #[test]
    fn single_newlines_stay_line_breaks() {
        // Gherkin steps in briefs are authored one per line and must not
        // flow together into a paragraph.
        let rows = flat(&md_lines(
            "Given a city\nWhen searched",
            40,
            Style::default(),
        ));
        assert_eq!(rows, vec!["Given a city", "When searched"]);
    }

    #[test]
    fn code_fences_stay_verbatim() {
        let rows = flat(&md_lines(
            "use it:\n\n```\nsh answer.sh \"q\"\n```",
            40,
            Style::default(),
        ));
        assert!(
            rows.contains(&"sh answer.sh \"q\"".to_string()),
            "fence contents survive verbatim: {rows:?}"
        );
        assert!(
            !rows.iter().any(|r| r.contains("```")),
            "fence sigils must not survive: {rows:?}"
        );
    }

    #[test]
    fn paragraphs_are_separated_by_a_spacer() {
        let rows = flat(&md_lines(
            "first block\n\nsecond block",
            40,
            Style::default(),
        ));
        assert_eq!(rows, vec!["first block", "", "second block"]);
    }

    #[test]
    fn everything_survives_wrapping() {
        let text = "a brief that is fairly long and wraps a few times over";
        let rows = flat(&md_lines(text, 12, Style::default()));
        assert_eq!(
            rows.join(" ")
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" "),
            text
        );
        assert!(rows.iter().all(|r| r.chars().count() <= 12));
    }

    #[test]
    fn empty_input_yields_one_empty_line() {
        assert_eq!(flat(&md_lines("", 10, Style::default())), vec![""]);
    }
}
