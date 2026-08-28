//! Tokens (cost/usage) panel renderer.
//!
//! Mirrors the Stats tab of the web player-session page: a summary block
//! (tokens in/out, cache read/write, messages, tool calls, implementation
//! time) followed by one block per agent session (agent · model, session id,
//! token line, tool-call breakdown, skills loaded).

use crate::tui::app::TuiApp;
use crate::tui::render::common::{fmt_mins_secs, fmt_tokens, provider_model, truncate, wrap_text};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Paragraph};
use std::collections::BTreeMap;

/// Token usage attributed to one model within a session.
#[derive(Default)]
pub(crate) struct ModelUsage {
    pub input: u64,
    pub output: u64,
    pub cost: f64,
}

/// One agent session joined from token counts and behavioural stats —
/// the TUI equivalent of the web's `AgentSessionStats`. A session that
/// switched models keeps one entry per model in `models`; the flat
/// token fields are session totals.
#[derive(Default)]
pub(crate) struct SessionView {
    /// Per-model usage, keyed by `provider/model-id` display label.
    /// Extractors report one `SessionCounts` row per (session, model),
    /// so a model switch mid-session must not overwrite the previous
    /// model — it accumulates here instead.
    pub models: BTreeMap<String, ModelUsage>,
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
    pub cost: f64,
    pub user_messages: u64,
    pub assistant_messages: u64,
    pub tool_calls: u64,
    pub tools: BTreeMap<String, u64>,
    pub skills: BTreeMap<String, u64>,
    pub started_at_ms: Option<i64>,
    pub last_seen_at_ms: Option<i64>,
}

/// Join counts and stats by (agent, session id) — same merge the task-stats
/// reporter does for the web page (`task_stats::merge_agent_stats`), kept
/// local because the panel also needs timestamps for implementation time.
pub(crate) fn merge_session_views(
    counts: &[agent_tokens::SessionCounts],
    stats: &[agent_tokens::SessionStats],
) -> BTreeMap<(String, String), SessionView> {
    let mut merged: BTreeMap<(String, String), SessionView> = BTreeMap::new();

    for c in counts {
        let key = (c.agent.as_kebab().to_string(), c.session_id.clone());
        let v = merged.entry(key).or_default();
        let (provider, model_id) = provider_model(c);
        let model_label = if provider == "—" {
            model_id
        } else {
            format!("{provider}/{model_id}")
        };
        let m = v.models.entry(model_label).or_default();
        m.input += c.counts.input;
        m.output += c.counts.output;
        m.cost += c.cost.unwrap_or(0.0);
        v.input += c.counts.input;
        v.output += c.counts.output;
        v.cache_read += c.counts.cache_read;
        v.cache_write += c.counts.cache_write;
        v.cost += c.cost.unwrap_or(0.0);
        v.started_at_ms = min_opt(v.started_at_ms, c.started_at_ms);
        v.last_seen_at_ms = v.last_seen_at_ms.max(c.last_seen_at_ms);
    }

    for s in stats {
        let key = (s.agent.as_kebab().to_string(), s.session_id.clone());
        // Stats-only sessions (no token counts) are skipped: the panel is
        // gated on counts being present, and a session with zero tokens has
        // nothing to show under a "tokens" title.
        let Some(v) = merged.get_mut(&key) else {
            continue;
        };
        v.user_messages += s.user_messages;
        v.assistant_messages += s.assistant_messages;
        v.tool_calls += s.tool_calls;
        for (tool, n) in &s.tools {
            *v.tools.entry(tool.clone()).or_insert(0) += n;
        }
        for (skill, n) in &s.skills {
            *v.skills.entry(skill.clone()).or_insert(0) += n;
        }
        v.started_at_ms = min_opt(v.started_at_ms, s.started_at_ms);
        v.last_seen_at_ms = v.last_seen_at_ms.max(s.last_seen_at_ms);
    }

    merged
}

fn min_opt(a: Option<i64>, b: Option<i64>) -> Option<i64> {
    match (a, b) {
        (Some(x), Some(y)) => Some(x.min(y)),
        (x, y) => x.or(y),
    }
}

/// Highest-count-first tool/skill ordering, ties alphabetical — matches the
/// web stats tab's `sortedCounts`.
fn sorted_counts(rec: &BTreeMap<String, u64>) -> Vec<(&str, u64)> {
    let mut v: Vec<(&str, u64)> = rec.iter().map(|(k, n)| (k.as_str(), *n)).collect();
    v.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
    v
}

/// Number of text lines the panel body needs for `sessions` merged views —
/// used by the sidebar splitter to size the panel before rendering.
pub(crate) fn panel_line_count(views: &BTreeMap<(String, String), SessionView>) -> u16 {
    // Summary: in/out, cache, msgs, tool calls, time.
    let mut lines = 5u16;
    for v in views.values() {
        // blank + header + session id + tokens + tools + skills
        lines = lines.saturating_add(6);
        // Multi-model sessions replace the single token line with a
        // name + counts pair per model.
        if v.models.len() > 1 {
            lines = lines.saturating_add(v.models.len() as u16 * 2 - 1);
        }
        // Tool breakdowns longer than one row wrap; rough +1 per 4 tools.
        lines = lines.saturating_add((v.tools.len() as u16) / 4);
    }
    lines
}

pub(crate) fn render_tokens(f: &mut Frame, area: Rect, app: &TuiApp) {
    let counts = match &app.tokens {
        Some(v) if !v.is_empty() => v,
        _ => return,
    };
    let empty: Vec<agent_tokens::SessionStats> = Vec::new();
    let stats = app.token_stats.as_ref().unwrap_or(&empty);

    let inner_w = area.width.saturating_sub(2).max(1) as usize;
    let views = merge_session_views(counts, stats);

    // ── Summary across all sessions (web stats tab's tile row) ──
    let mut input = 0u64;
    let mut output = 0u64;
    let mut cache_read = 0u64;
    let mut cache_write = 0u64;
    let mut user_msgs = 0u64;
    let mut agent_msgs = 0u64;
    let mut tool_calls = 0u64;
    let mut cost = 0f64;
    let mut started: Option<i64> = None;
    let mut last_seen: Option<i64> = None;
    for v in views.values() {
        input += v.input;
        output += v.output;
        cache_read += v.cache_read;
        cache_write += v.cache_write;
        user_msgs += v.user_messages;
        agent_msgs += v.assistant_messages;
        tool_calls += v.tool_calls;
        cost += v.cost;
        started = min_opt(started, v.started_at_ms);
        last_seen = last_seen.max(v.last_seen_at_ms);
    }

    let label = Style::default().fg(Color::DarkGray);
    let value = Style::default().fg(Color::White);
    let mut lines: Vec<Line> = Vec::new();
    let mut summary = |name: &str, val: String| {
        lines.push(Line::from(vec![
            Span::styled(format!("{name:<11}"), label),
            Span::styled(truncate(&val, inner_w.saturating_sub(11)), value),
        ]));
    };
    summary(
        "in/out",
        format!("{} / {}", fmt_tokens(input), fmt_tokens(output)),
    );
    summary(
        "cache r/w",
        format!("{} / {}", fmt_tokens(cache_read), fmt_tokens(cache_write)),
    );
    summary("msgs u/a", format!("{user_msgs} / {agent_msgs}"));
    summary("tool calls", tool_calls.to_string());
    let time = match (started, last_seen) {
        (Some(s), Some(e)) if e >= s => fmt_mins_secs(((e - s) / 1000) as u64),
        _ => "—".to_string(),
    };
    summary("time", time);

    // ── Per-session blocks (web stats tab's per-agent breakdown) ──
    for ((agent, session_id), v) in &views {
        lines.push(Line::default());

        // Single-model session: compact "agent · model" header + one
        // token line. Multi-model session (model switched mid-run): the
        // header counts the models and each model gets its own name +
        // token line — nothing is overwritten or summed away.
        let header = match v.models.len() {
            0 | 1 => {
                let model = v.models.keys().next().map(String::as_str).unwrap_or("—");
                format!("{agent} · {model}")
            }
            n => format!("{agent} · {n} models"),
        };
        lines.push(Line::from(Span::styled(
            truncate(&header, inner_w),
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )));
        lines.push(Line::from(Span::styled(
            truncate(session_id, inner_w),
            Style::default().fg(Color::DarkGray),
        )));

        if v.models.len() > 1 {
            for (model, m) in &v.models {
                lines.push(Line::from(Span::styled(
                    truncate(model, inner_w),
                    Style::default().fg(Color::Cyan),
                )));
                let mut token_line = format!(
                    "  {} in · {} out",
                    fmt_tokens(m.input),
                    fmt_tokens(m.output)
                );
                if m.cost > 0.0 {
                    token_line.push_str(&format!(" · ${:.4}", m.cost));
                }
                lines.push(Line::from(Span::styled(
                    truncate(&token_line, inner_w),
                    Style::default().fg(Color::White),
                )));
            }
        } else {
            let mut token_line =
                format!("{} in · {} out", fmt_tokens(v.input), fmt_tokens(v.output));
            if v.cost > 0.0 {
                token_line.push_str(&format!(" · ${:.4}", v.cost));
            }
            lines.push(Line::from(Span::styled(
                truncate(&token_line, inner_w),
                Style::default().fg(Color::White),
            )));
        }

        let tools_text = if v.tools.is_empty() {
            format!("tools ({}): —", v.tool_calls)
        } else {
            let list = sorted_counts(&v.tools)
                .iter()
                .map(|(tool, n)| format!("{tool} {n}"))
                .collect::<Vec<_>>()
                .join(", ");
            format!("tools ({}): {}", v.tool_calls, list)
        };
        for wrapped in wrap_text(&tools_text, inner_w) {
            lines.push(Line::from(Span::styled(
                wrapped,
                Style::default().fg(Color::Green),
            )));
        }

        let skills_text = if v.skills.is_empty() {
            "skills: —".to_string()
        } else {
            let list = sorted_counts(&v.skills)
                .iter()
                .map(|(skill, n)| {
                    if *n > 1 {
                        format!("{skill} ×{n}")
                    } else {
                        (*skill).to_string()
                    }
                })
                .collect::<Vec<_>>()
                .join(", ");
            format!("skills: {list}")
        };
        for wrapped in wrap_text(&skills_text, inner_w) {
            lines.push(Line::from(Span::styled(
                wrapped,
                Style::default().fg(Color::Magenta),
            )));
        }
    }

    let total_str = if cost > 0.0 {
        format!(" stats ${cost:.4} ")
    } else {
        " stats ".to_string()
    };
    let block = Block::default().title(total_str).borders(Borders::ALL);
    f.render_widget(Paragraph::new(Text::from(lines)).block(block), area);
}
