//! Grok Build CLI sessions: `$GROK_HOME` (else `~/.grok`) `/sessions/<url-encoded-cwd>/<session-id>/`
//! holding `updates.jsonl` (the ACP `session/update` stream) and an optional
//! `summary.json` (session id, cwd, current model). Ported from ccusage's grok
//! adapter (MIT).
//!
//! Every `updates.jsonl` line is a JSON-RPC notification envelope:
//! `{"timestamp":<unix seconds>,"method":"_x.ai/session/update","params":{"sessionId",
//! "update":{"sessionUpdate":<kind>,...},"_meta":{"eventId","agentTimestampMs"}}}`.
//! Kinds used here:
//! - `turn_completed` with `usage` (`inputTokens` INCLUDING cache, `outputTokens`
//!   INCLUDING `reasoningTokens`, `cachedReadTokens`, `cacheCreationTokens`,
//!   `costUsdTicks` = USD × 1e10, and a per-model `modelUsage` map) — tokens.
//! - `user_message_chunk` / `agent_message_chunk` (streamed; a run of chunks is
//!   one message) and `tool_call` (`title`/`kind`/`rawInput`) — stats.
//!
//! Timestamps prefer `_meta.agentTimestampMs`, then the envelope's Unix seconds.

use crate::jsonl;
use crate::paths;
use crate::trait_::TokenExtractor;
use crate::types::{AgentId, SessionCounts, SessionStats, TokenCounts};
use serde_json::Value;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// Grok Build CLI's own home override (single root, replaces `~/.grok`).
pub const GROK_HOME_ENV: &str = "GROK_HOME";

/// `costUsdTicks` are fixed-point USD: one tick is 1e-10 USD.
const COST_USD_TICKS_PER_USD: f64 = 1e10;

pub struct Grok;

impl TokenExtractor for Grok {
    fn id(&self) -> AgentId {
        AgentId::Grok
    }

    fn detect(&self) -> bool {
        grok_sessions_dir().is_some_and(|d| d.is_dir())
    }

    fn extract(&self, since: Option<i64>) -> Vec<SessionCounts> {
        grok_sessions_dir()
            .map(|d| extract_from(&d, since))
            .unwrap_or_default()
    }

    fn stats(&self, since: Option<i64>) -> Vec<SessionStats> {
        grok_sessions_dir()
            .map(|d| {
                update_files(&d)
                    .iter()
                    .filter_map(|f| parse_stats(f, since))
                    .collect()
            })
            .unwrap_or_default()
    }
}

/// Grok data root: a non-empty `GROK_HOME` (only honoured when it is a
/// directory, as the CLI does), else `<home>/.grok`. Goes through
/// `paths::home_dir()` so `AGENT_TOKENS_HOME` redirects the default.
fn grok_root() -> Option<PathBuf> {
    if let Some(home) = std::env::var_os(GROK_HOME_ENV) {
        let trimmed = home.to_string_lossy().trim().to_string();
        if !trimmed.is_empty() {
            let root = PathBuf::from(trimmed);
            return root.is_dir().then_some(root);
        }
    }
    let root = paths::home_dir().join(".grok");
    root.is_dir().then_some(root)
}

fn grok_sessions_dir() -> Option<PathBuf> {
    grok_root().map(|r| r.join("sessions"))
}

/// Every `updates.jsonl` under the sessions root, any nesting depth, sorted by
/// path so load order (and therefore cross-file dedupe) is stable.
fn update_files(root: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.file_name().and_then(|n| n.to_str()) == Some("updates.jsonl") {
                files.push(path);
            }
        }
    }
    files.sort();
    files
}

/// Parse every session under `sessions_root`, deduping turns that share an
/// `eventId` across files (the same server event can be exported into more
/// than one session; the first file in path order keeps it).
fn extract_from(sessions_root: &Path, since: Option<i64>) -> Vec<SessionCounts> {
    let mut seen = HashSet::new();
    update_files(sessions_root)
        .iter()
        .filter_map(|f| parse_counts_dedup(f, since, &mut seen))
        .collect()
}

struct SessionMeta {
    session_id: String,
    cwd: Option<String>,
    default_model: Option<String>,
}

/// Session identity from the directory layout, refined by `summary.json`
/// when present: `info.id` beats the dir name, `info.cwd` (then
/// `git_root_dir`) beats the url-decoded project segment.
fn load_session_meta(updates: &Path) -> SessionMeta {
    let session_dir = updates.parent();
    let session_id = session_dir
        .and_then(|d| d.file_name())
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "unknown".into());
    // `sessions/<url-encoded-cwd>/<id>/updates.jsonl`; a file sitting directly
    // under `sessions/` has no project segment to decode.
    let cwd = session_dir
        .and_then(|d| d.parent())
        .and_then(|p| p.file_name())
        .and_then(|n| n.to_str())
        .filter(|n| *n != "sessions")
        .map(url_decode_lightweight);

    let mut meta = SessionMeta {
        session_id,
        cwd,
        default_model: None,
    };
    let summary_path = updates.with_file_name("summary.json");
    if let Ok(content) = std::fs::read_to_string(&summary_path)
        && let Ok(summary) = serde_json::from_str::<Value>(&content)
    {
        let info = summary.get("info");
        if let Some(id) = info.and_then(|i| non_empty_str(i.get("id"))) {
            meta.session_id = id;
        }
        if let Some(cwd) = info
            .and_then(|i| non_empty_str(i.get("cwd")))
            .or_else(|| non_empty_str(summary.get("git_root_dir")))
        {
            meta.cwd = Some(cwd);
        }
        meta.default_model = non_empty_str(summary.get("current_model_id"));
    }
    meta
}

/// Parse one session's `updates.jsonl` into token counts. Public for fixture
/// tests; dedupes only within the file (see [`extract_from`] for cross-file).
pub fn parse_counts(updates: &Path, since: Option<i64>) -> Option<SessionCounts> {
    parse_counts_dedup(updates, since, &mut HashSet::new())
}

fn parse_counts_dedup(
    updates: &Path,
    since: Option<i64>,
    seen: &mut HashSet<String>,
) -> Option<SessionCounts> {
    let meta = load_session_meta(updates);
    let mut out = SessionCounts {
        agent: AgentId::Grok,
        session_id: meta.session_id.clone(),
        model: None,
        cwd: meta.cwd.clone(),
        started_at_ms: None,
        last_seen_at_ms: None,
        counts: TokenCounts::default(),
        cost: None,
        source_file: Some(updates.to_path_buf()),
    };
    let mut line_session_id: Option<String> = None;
    let mut saw_turn = false;
    let mut cost_ticks: u64 = 0;
    // (timestamp, model) of the newest turn that named a model.
    let mut latest_model: Option<(i64, String)> = None;

    for line in jsonl::read_lines(updates) {
        if line.is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        let Some(params) = v.get("params").filter(|p| p.is_object()) else {
            continue;
        };
        let ts = record_ts_ms(&v, params);
        touch_bounds(&mut out.started_at_ms, &mut out.last_seen_at_ms, ts);
        if line_session_id.is_none()
            && let Some(id) = non_empty_str(params.get("sessionId"))
        {
            line_session_id = Some(id);
        }
        let Some(update) = params.get("update").filter(|u| u.is_object()) else {
            continue;
        };
        if update.get("sessionUpdate").and_then(|k| k.as_str()) != Some("turn_completed") {
            continue;
        }
        let Some(usage) = update.get("usage").filter(|u| u.is_object()) else {
            continue;
        };
        let event_id = non_empty_str(params.get("_meta").and_then(|m| m.get("eventId")));
        let session_key = line_session_id
            .clone()
            .unwrap_or_else(|| meta.session_id.clone());

        for (model, mu) in model_usage_rows(usage, meta.default_model.as_deref()) {
            let (uncached, cache_read, cache_write) =
                split_input_tokens(mu.input, mu.cached_read, mu.cache_creation);
            if uncached == 0
                && cache_read == 0
                && cache_write == 0
                && mu.output == 0
                && mu.reasoning == 0
            {
                continue;
            }
            let model_key = model.as_deref().unwrap_or("");
            let key = match &event_id {
                Some(id) => format!("{id}|{model_key}"),
                None => format!(
                    "{session_key}|{}|{model_key}|{uncached}|{}|{cache_read}|{cache_write}|{}",
                    ts.unwrap_or(0),
                    mu.output,
                    mu.reasoning
                ),
            };
            if !seen.insert(key) {
                continue;
            }
            if let Some(cutoff) = since
                && ts.is_none_or(|t| t < cutoff)
            {
                continue;
            }
            saw_turn = true;
            out.counts.input += uncached;
            out.counts.cache_read += cache_read;
            out.counts.cache_write += cache_write;
            out.counts.output += mu.output;
            out.counts.reasoning += mu.reasoning;
            cost_ticks = cost_ticks.saturating_add(mu.cost_usd_ticks);
            if let Some(m) = model {
                let t = ts.unwrap_or(0);
                if latest_model.as_ref().is_none_or(|(prev, _)| t >= *prev) {
                    latest_model = Some((t, m));
                }
            }
        }
    }

    if !saw_turn {
        return None;
    }
    if let Some(id) = line_session_id {
        out.session_id = id;
    }
    out.model = latest_model.map(|(_, m)| m).or(meta.default_model);
    out.cost = (cost_ticks > 0).then(|| cost_ticks as f64 / COST_USD_TICKS_PER_USD);
    Some(out)
}

/// Parse one session's `updates.jsonl` into behavioural stats. Public for
/// fixture tests.
pub fn parse_stats(updates: &Path, since: Option<i64>) -> Option<SessionStats> {
    let meta = load_session_meta(updates);
    let mut out = SessionStats::new(AgentId::Grok, meta.session_id);
    out.cwd = meta.cwd;
    out.source_file = Some(updates.to_path_buf());
    let mut line_session_id: Option<String> = None;
    let mut saw_activity = false;
    // Messages stream as chunks; consecutive chunks of one role are one message.
    #[derive(PartialEq)]
    enum Run {
        None,
        User,
        Agent,
    }
    let mut run = Run::None;

    for line in jsonl::read_lines(updates) {
        if line.is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        let Some(params) = v.get("params").filter(|p| p.is_object()) else {
            continue;
        };
        let ts = record_ts_ms(&v, params);
        touch_bounds(&mut out.started_at_ms, &mut out.last_seen_at_ms, ts);
        if line_session_id.is_none()
            && let Some(id) = non_empty_str(params.get("sessionId"))
        {
            line_session_id = Some(id);
        }
        let in_window = since.is_none_or(|cutoff| ts.is_some_and(|t| t >= cutoff));

        // A client-side prompt request, if the CLI ever logs it here, is a
        // user message too.
        if v.get("method")
            .and_then(|m| m.as_str())
            .is_some_and(|m| m.ends_with("session/prompt"))
        {
            run = Run::None;
            if in_window {
                saw_activity = true;
                out.user_messages += 1;
            }
            continue;
        }

        let Some(update) = params.get("update").filter(|u| u.is_object()) else {
            continue;
        };
        match update
            .get("sessionUpdate")
            .and_then(|k| k.as_str())
            .unwrap_or("")
        {
            "user_message_chunk" => {
                if run != Run::User {
                    run = Run::User;
                    if in_window {
                        saw_activity = true;
                        out.user_messages += 1;
                    }
                }
            }
            "agent_message_chunk" => {
                if run != Run::Agent {
                    run = Run::Agent;
                    if in_window {
                        saw_activity = true;
                        out.assistant_messages += 1;
                    }
                }
            }
            // Thinking belongs to the assistant message around it.
            "agent_thought_chunk" => {}
            "tool_call" => {
                run = Run::None;
                if in_window {
                    saw_activity = true;
                    let name = ["name", "toolName", "tool", "title", "kind"]
                        .iter()
                        .find_map(|k| non_empty_str(update.get(k)))
                        .unwrap_or_else(|| "unknown".into());
                    // `rawInput` is the tool's arguments; fall back to the whole
                    // update so `locations[].path` still reveals a SKILL.md read.
                    let args = ["rawInput", "input", "arguments"]
                        .iter()
                        .find_map(|k| update.get(k).filter(|a| !a.is_null()))
                        .unwrap_or(update);
                    out.record_tool(&name, args);
                }
            }
            "turn_completed" => run = Run::None,
            // tool_call_update / plan / mode updates do not split a message.
            _ => {}
        }
    }

    if since.is_some() && !saw_activity {
        return None;
    }
    if let Some(id) = line_session_id {
        out.session_id = id;
    }
    Some(out)
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct ModelUsage {
    input: u64,
    output: u64,
    cached_read: u64,
    cache_creation: u64,
    reasoning: u64,
    cost_usd_ticks: u64,
}

fn model_usage_from(v: &Value) -> ModelUsage {
    let get = |k: &str| lenient_u64(v.get(k));
    ModelUsage {
        input: get("inputTokens"),
        output: get("outputTokens"),
        cached_read: get("cachedReadTokens"),
        cache_creation: get("cacheCreationTokens"),
        reasoning: get("reasoningTokens"),
        cost_usd_ticks: get("costUsdTicks"),
    }
}

/// One row per model from `modelUsage` (sorted by name), else the top-level
/// usage attributed to the session's default model (or no model at all).
fn model_usage_rows(
    usage: &Value,
    default_model: Option<&str>,
) -> Vec<(Option<String>, ModelUsage)> {
    if let Some(map) = usage.get("modelUsage").and_then(|m| m.as_object())
        && !map.is_empty()
    {
        let mut rows: Vec<_> = map
            .iter()
            .map(|(model, mu)| (Some(model.clone()), model_usage_from(mu)))
            .collect();
        rows.sort_by(|a, b| a.0.cmp(&b.0));
        return rows;
    }
    vec![(default_model.map(String::from), model_usage_from(usage))]
}

/// Split `inputTokens` into (uncached, cache_read, cache_write).
/// `cachedReadTokens` is a subset of `inputTokens`; `cacheCreationTokens` is
/// carved out of the uncached remainder so the three parts sum back to input.
fn split_input_tokens(input: u64, cached_read: u64, cache_creation: u64) -> (u64, u64, u64) {
    let cache_read = cached_read.min(input);
    let uncached = input - cache_read;
    let cache_write = cache_creation.min(uncached);
    (uncached - cache_write, cache_read, cache_write)
}

/// `_meta.agentTimestampMs` when positive, else the envelope `timestamp`
/// (Unix seconds as Grok writes it; an ISO string or a millisecond value is
/// tolerated for foreign data).
fn record_ts_ms(line: &Value, params: &Value) -> Option<i64> {
    if let Some(ms) = params
        .get("_meta")
        .and_then(|m| m.get("agentTimestampMs"))
        .and_then(value_as_i64)
        && ms > 0
    {
        return Some(ms);
    }
    let ts = line.get("timestamp")?;
    if ts.is_string() {
        return jsonl::parse_ts_ms(ts).filter(|t| *t > 0);
    }
    let n = value_as_i64(ts).filter(|n| *n > 0)?;
    // Anything below 1e11 cannot be a millisecond timestamp after 1973.
    Some(if n < 100_000_000_000 {
        n.saturating_mul(1000)
    } else {
        n
    })
}

fn touch_bounds(started: &mut Option<i64>, last: &mut Option<i64>, ts: Option<i64>) {
    if let Some(t) = ts {
        if started.is_none_or(|s| t < s) {
            *started = Some(t);
        }
        if last.is_none_or(|l| t > l) {
            *last = Some(t);
        }
    }
}

fn value_as_i64(v: &Value) -> Option<i64> {
    v.as_i64()
        .or_else(|| v.as_u64().and_then(|n| i64::try_from(n).ok()))
        .or_else(|| v.as_f64().map(|n| n as i64))
}

/// Integers, floats and numeric strings all count; anything else is zero.
fn lenient_u64(v: Option<&Value>) -> u64 {
    match v {
        Some(Value::Number(n)) => n
            .as_u64()
            .or_else(|| n.as_f64().filter(|f| *f >= 0.0).map(|f| f as u64))
            .unwrap_or(0),
        Some(Value::String(s)) => s.trim().parse::<u64>().unwrap_or(0),
        _ => 0,
    }
}

fn non_empty_str(v: Option<&Value>) -> Option<String> {
    v.and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from)
}

/// Session parents are URL-encoded cwd paths (e.g. `D%3A%5Cproj`). Decodes
/// into bytes so a multi-byte scalar split across triplets survives; invalid
/// UTF-8 degrades to U+FFFD rather than truncating the path.
fn url_decode_lightweight(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let (Some(hi), Some(lo)) = (from_hex(bytes[i + 1]), from_hex(bytes[i + 2]))
        {
            out.push(hi * 16 + lo);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn from_hex(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixtures_root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/grok")
    }

    fn widget_updates() -> PathBuf {
        fixtures_root().join(
            "sessions/%2Fhome%2Fdev%2Fwidget/019fa1b1-0000-7000-8000-000000000001/updates.jsonl",
        )
    }

    fn garbage_updates() -> PathBuf {
        fixtures_root()
            .join("sessions/D%3A%5Cwork%5Cproj/019fa1b1-0000-7000-8000-000000000002/updates.jsonl")
    }

    // Turn 2 of the widget session: agentTimestampMs 1750003605500.
    const TURN2_MS: i64 = 1_750_003_600_000;

    #[test]
    fn counts_split_cache_out_of_input_and_sum_turns() {
        let s = parse_counts(&widget_updates(), None).expect("session parses");
        assert_eq!(s.agent, AgentId::Grok);
        assert_eq!(s.session_id, "sess-widget-1");
        assert_eq!(s.model.as_deref(), Some("grok-4.5-build"));
        assert_eq!(s.cwd.as_deref(), Some("/home/dev/widget"));
        // turn 1: 18444 - 11264 = 7180 uncached; turn 2: 2000 - 1500 - 100 = 400.
        assert_eq!(s.counts.input, 7180 + 400);
        assert_eq!(s.counts.cache_read, 11264 + 1500);
        assert_eq!(s.counts.cache_write, 100);
        assert_eq!(s.counts.output, 130 + 300);
        assert_eq!(s.counts.reasoning, 73 + 20);
        // costUsdTicks / 1e10.
        let cost = s.cost.expect("ticks recorded");
        assert!((cost - 0.021_519_2).abs() < 1e-12, "cost {cost}");
        assert_eq!(s.started_at_ms, Some(1_750_000_000_100));
        assert_eq!(s.last_seen_at_ms, Some(1_750_003_605_500));
        assert_eq!(s.source_file.as_deref(), Some(widget_updates().as_path()));
    }

    #[test]
    fn since_keeps_only_turns_completed_in_the_window() {
        let s = parse_counts(&widget_updates(), Some(TURN2_MS)).expect("turn 2 is in window");
        assert_eq!(s.counts.input, 400);
        assert_eq!(s.counts.cache_read, 1500);
        assert_eq!(s.counts.cache_write, 100);
        assert_eq!(s.counts.output, 300);
        assert_eq!(s.counts.reasoning, 20);
        assert!((s.cost.unwrap() - 0.003).abs() < 1e-12);
        // Session bounds still describe the whole session.
        assert_eq!(s.started_at_ms, Some(1_750_000_000_100));

        assert!(
            parse_counts(&widget_updates(), Some(1_750_010_000_000)).is_none(),
            "a session with no turn in the window is dropped"
        );
        assert!(
            parse_counts(&widget_updates(), Some(0)).is_some(),
            "since = 0 admits every dated turn"
        );
    }

    #[test]
    fn stats_count_message_runs_and_tool_calls() {
        let st = parse_stats(&widget_updates(), None).expect("stats parse");
        assert_eq!(st.session_id, "sess-widget-1");
        assert_eq!(st.cwd.as_deref(), Some("/home/dev/widget"));
        // Two prompts, each streamed as chunks; the first one arrives in two pieces.
        assert_eq!(st.user_messages, 2);
        // Agent chunk runs are split by tool calls: 3 in turn 1, 2 in turn 2.
        assert_eq!(st.assistant_messages, 5);
        assert_eq!(st.tool_calls, 3);
        assert_eq!(st.tools.get("read_file"), Some(&1));
        assert_eq!(st.tools.get("write_file"), Some(&1));
        assert_eq!(st.tools.get("bash"), Some(&1));
        // read_file of <name>/SKILL.md via rawInput.path is a skill load.
        assert_eq!(st.skills.get("reconnaissance"), Some(&1));
        assert_eq!(st.started_at_ms, Some(1_750_000_000_100));
        assert_eq!(st.last_seen_at_ms, Some(1_750_003_605_500));
    }

    #[test]
    fn stats_since_window_counts_only_later_activity() {
        let st = parse_stats(&widget_updates(), Some(TURN2_MS)).expect("activity in window");
        assert_eq!(st.user_messages, 1);
        assert_eq!(st.assistant_messages, 2);
        assert_eq!(st.tool_calls, 1);
        assert_eq!(st.tools.get("bash"), Some(&1));
        assert!(st.skills.is_empty());
        assert!(parse_stats(&widget_updates(), Some(1_750_010_000_000)).is_none());
    }

    #[test]
    fn garbage_lines_are_skipped_and_duplicates_dropped() {
        let s = parse_counts(&garbage_updates(), None).expect("the good turns survive");
        // No summary and no sessionId on the lines: the dir name is the id and
        // the url-encoded project segment is the cwd.
        assert_eq!(s.session_id, "019fa1b1-0000-7000-8000-000000000002");
        assert_eq!(s.cwd.as_deref(), Some("D:\\work\\proj"));
        // evt-top counted once (50 - 10 = 40) + lenient "7"/1.0 row.
        assert_eq!(s.counts.input, 40 + 7);
        assert_eq!(s.counts.cache_read, 10);
        assert_eq!(s.counts.output, 5 + 1);
        assert_eq!(s.counts.reasoning, 2);
        assert_eq!(s.model, None, "no modelUsage and no summary model");
        assert_eq!(s.cost, None, "no ticks recorded");
        // Envelope seconds become millis; the undated row does not move the bounds.
        assert_eq!(s.started_at_ms, Some(1_750_100_000_000));
        assert_eq!(s.last_seen_at_ms, Some(1_750_100_003_000));

        // The undated lenient row has no timestamp, so a window excludes it.
        let w = parse_counts(&garbage_updates(), Some(1_750_100_003_000)).unwrap();
        assert_eq!((w.counts.input, w.counts.output), (40, 5));

        let st = parse_stats(&garbage_updates(), None).unwrap();
        assert_eq!(st.tool_calls, 1);
        assert_eq!(st.tools.get("bash"), Some(&1));
        assert_eq!((st.user_messages, st.assistant_messages), (0, 0));
    }

    #[test]
    fn missing_file_and_empty_root_are_harmless() {
        let missing = fixtures_root().join("sessions/nope/updates.jsonl");
        assert!(parse_counts(&missing, None).is_none());
        assert!(parse_stats(&missing, None).is_some());
        assert!(update_files(&fixtures_root().join("does-not-exist")).is_empty());
    }

    #[test]
    fn discovery_finds_every_updates_jsonl_and_dedupes_across_sessions() {
        let root = fixtures_root().join("sessions");
        let files = update_files(&root);
        assert_eq!(files.len(), 2);
        assert!(files.iter().all(|f| f.ends_with("updates.jsonl")));
        assert!(files[0] < files[1], "sorted for stable dedupe order");

        let sessions = extract_from(&root, None);
        assert_eq!(sessions.len(), 2);

        // A second copy of a session's turns under another id counts once.
        let tmp = tempfile::tempdir().unwrap();
        let copy = tmp.path().join("sessions/proj/a");
        let dup = tmp.path().join("sessions/proj/b");
        std::fs::create_dir_all(&copy).unwrap();
        std::fs::create_dir_all(&dup).unwrap();
        std::fs::copy(widget_updates(), copy.join("updates.jsonl")).unwrap();
        std::fs::copy(widget_updates(), dup.join("updates.jsonl")).unwrap();
        let sessions = extract_from(&tmp.path().join("sessions"), None);
        assert_eq!(sessions.len(), 1, "the duplicate export has nothing left");
        assert_eq!(sessions[0].counts.input, 7580);
    }

    #[test]
    fn grok_home_env_points_detection_at_the_fixture_tree() {
        // SAFETY: the only reader of GROK_HOME in this binary is this module,
        // and no other test here sets it.
        unsafe { std::env::set_var(GROK_HOME_ENV, fixtures_root()) };
        assert!(Grok.detect());
        let sessions = Grok.extract(None);
        assert_eq!(sessions.len(), 2);
        assert_eq!(Grok.stats(None).len(), 2);
        unsafe { std::env::set_var(GROK_HOME_ENV, fixtures_root().join("not-a-dir")) };
        assert!(
            !Grok.detect(),
            "a GROK_HOME that is not a directory is ignored"
        );
        unsafe { std::env::remove_var(GROK_HOME_ENV) };
    }

    #[test]
    fn multi_model_turn_attributes_each_model_and_picks_the_latest() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("sessions/proj/multi");
        std::fs::create_dir_all(&dir).unwrap();
        let lines = [
            r#"{"timestamp":1750000100,"params":{"sessionId":"sess-m","update":{"sessionUpdate":"turn_completed","usage":{"modelUsage":{"model-a":{"inputTokens":10,"outputTokens":2,"cachedReadTokens":0,"reasoningTokens":1},"model-b":{"inputTokens":20,"outputTokens":4,"cachedReadTokens":5,"reasoningTokens":0}}}},"_meta":{"eventId":"evt-multi"}}}"#,
            r#"{"timestamp":1750000200,"params":{"sessionId":"sess-m","update":{"sessionUpdate":"turn_completed","usage":{"modelUsage":{"grok-4.5-build":{"inputTokens":100,"outputTokens":20,"cachedReadTokens":40,"reasoningTokens":10}}}},"_meta":{"eventId":"evt-later"}}}"#,
        ];
        std::fs::write(dir.join("updates.jsonl"), lines.join("\n")).unwrap();
        let s = parse_counts(&dir.join("updates.jsonl"), None).unwrap();
        assert_eq!(s.session_id, "sess-m");
        assert_eq!(s.cwd.as_deref(), Some("proj"));
        assert_eq!(s.counts.input, 10 + 15 + 60);
        assert_eq!(s.counts.cache_read, 5 + 40);
        assert_eq!(s.counts.output, 2 + 4 + 20);
        assert_eq!(s.counts.reasoning, 1 + 10);
        assert_eq!(s.model.as_deref(), Some("grok-4.5-build"));
    }

    #[test]
    fn split_input_tokens_clamps_and_carves_cache_creation() {
        assert_eq!(split_input_tokens(100, 40, 0), (60, 40, 0));
        assert_eq!(split_input_tokens(100, 40, 25), (35, 40, 25));
        assert_eq!(split_input_tokens(10, 40, 0), (0, 10, 0));
        assert_eq!(split_input_tokens(100, 0, 500), (0, 0, 100));
    }

    #[test]
    fn timestamps_prefer_agent_millis_over_envelope_seconds() {
        let with_ms = serde_json::json!({"timestamp": 1750000000, "params": {"_meta": {"agentTimestampMs": 1785328986355i64}}});
        assert_eq!(
            record_ts_ms(&with_ms, &with_ms["params"]),
            Some(1_785_328_986_355)
        );
        let seconds = serde_json::json!({"timestamp": 1750000000, "params": {}});
        assert_eq!(
            record_ts_ms(&seconds, &seconds["params"]),
            Some(1_750_000_000_000)
        );
        let iso = serde_json::json!({"timestamp": "2025-06-15T15:06:40Z", "params": {}});
        assert_eq!(record_ts_ms(&iso, &iso["params"]), Some(1_750_000_000_000));
        let none = serde_json::json!({"params": {"_meta": {"agentTimestampMs": 0}}});
        assert_eq!(record_ts_ms(&none, &none["params"]), None);
    }

    #[test]
    fn url_decode_recombines_wide_scalars_and_tolerates_invalid_bytes() {
        assert_eq!(
            url_decode_lightweight("%2Ftmp%2F%E6%97%A5%E6%9C%AC"),
            "/tmp/日本"
        );
        assert_eq!(
            url_decode_lightweight("D%3A%5Cwork%5Cproj"),
            "D:\\work\\proj"
        );
        assert_eq!(url_decode_lightweight("%2Ftmp%2F%FFx"), "/tmp/\u{fffd}x");
        assert_eq!(url_decode_lightweight("plain%"), "plain%");
    }
}
