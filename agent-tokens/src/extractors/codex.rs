//! OpenAI Codex CLI rollout logs: `~/.codex/sessions/YYYY/MM/DD/rollout-*.jsonl`.
//! Line shapes (all have a top-level ISO `timestamp`):
//! - `{"type":"session_meta","payload":{"id","cwd","timestamp",...}}`
//! - `{"type":"turn_context","payload":{"model",...}}`
//! - `{"type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{...},"last_token_usage":{...}}}}`
//!   where usage objects carry `input_tokens` (cached included), `cached_input_tokens`,
//!   `output_tokens`, `reasoning_output_tokens`.
//! - `{"type":"response_item","payload":{"type":"function_call","name":...,"arguments":"<json string>"}}`

use crate::jsonl;
use crate::paths;
use crate::trait_::TokenExtractor;
use crate::types::{AgentId, SessionCounts, SessionStats, TokenCounts};
use std::path::{Path, PathBuf};

pub struct Codex;

impl TokenExtractor for Codex {
    fn id(&self) -> AgentId {
        AgentId::Codex
    }

    fn detect(&self) -> bool {
        paths::codex_session_dirs().iter().any(|d| d.exists())
    }

    fn extract(&self, since: Option<i64>) -> Vec<SessionCounts> {
        paths::codex_session_dirs()
            .iter()
            .flat_map(|d| rollout_files(d))
            .collect::<Vec<_>>()
            .iter()
            .filter_map(|f| parse_counts(f, since))
            .collect()
    }

    fn stats(&self, since: Option<i64>) -> Vec<SessionStats> {
        paths::codex_session_dirs()
            .iter()
            .flat_map(|d| rollout_files(d))
            .collect::<Vec<_>>()
            .iter()
            .filter_map(|f| parse_stats(f, since))
            .collect()
    }
}

/// Recursively collect `*.jsonl` under the sessions root (YYYY/MM/DD nesting).
fn rollout_files(root: &Path) -> Vec<PathBuf> {
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
            } else if path.extension().and_then(|e| e.to_str()) == Some("jsonl") {
                files.push(path);
            }
        }
    }
    files
}

/// Parse one rollout file into token counts. Public for fixture tests.
pub fn parse_counts(path: &Path, since: Option<i64>) -> Option<SessionCounts> {
    let mut out = SessionCounts {
        agent: AgentId::Codex,
        session_id: path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "unknown".into()),
        model: None,
        cwd: None,
        started_at_ms: None,
        last_seen_at_ms: None,
        counts: TokenCounts::default(),
        cost: None,
        source_file: Some(path.to_path_buf()),
    };
    // Lifetime mode uses the last cumulative total; window mode sums the
    // per-turn `last_token_usage` of events inside the window.
    let mut window = TokenCounts::default();
    let mut lifetime = TokenCounts::default();
    let mut saw_tokens = false;
    let mut saw_window_tokens = false;

    for line in jsonl::read_lines(path) {
        if line.is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&line) else {
            continue;
        };
        let ts = jsonl::parse_ts_ms(v.get("timestamp").unwrap_or(&serde_json::Value::Null));
        if let Some(t) = ts {
            out.last_seen_at_ms = Some(out.last_seen_at_ms.unwrap_or(t).max(t));
        }
        let Some(payload) = v.get("payload") else {
            continue;
        };
        match v.get("type").and_then(|t| t.as_str()).unwrap_or("") {
            "session_meta" => {
                if let Some(id) = payload.get("id").and_then(|i| i.as_str()) {
                    out.session_id = id.to_string();
                }
                out.cwd = payload
                    .get("cwd")
                    .and_then(|c| c.as_str())
                    .map(String::from);
                out.started_at_ms = jsonl::parse_ts_ms(
                    payload.get("timestamp").unwrap_or(&serde_json::Value::Null),
                )
                .or(ts);
            }
            "turn_context" => {
                if let Some(m) = payload.get("model").and_then(|m| m.as_str()) {
                    out.model = Some(m.to_string());
                }
            }
            "event_msg" => {
                if payload.get("type").and_then(|t| t.as_str()) != Some("token_count") {
                    continue;
                }
                let Some(info) = payload.get("info").filter(|i| !i.is_null()) else {
                    continue;
                };
                if let Some(total) = info.get("total_token_usage") {
                    lifetime = usage_to_counts(total);
                    saw_tokens = true;
                }
                if let Some(cutoff) = since
                    && ts.is_some_and(|t| t >= cutoff)
                    && let Some(last) = info.get("last_token_usage")
                {
                    let c = usage_to_counts(last);
                    window.input += c.input;
                    window.output += c.output;
                    window.cache_read += c.cache_read;
                    window.reasoning += c.reasoning;
                    saw_window_tokens = true;
                }
            }
            _ => {}
        }
    }

    if since.is_some() {
        if !saw_window_tokens {
            return None;
        }
        out.counts = window;
    } else {
        if !saw_tokens {
            return None;
        }
        out.counts = lifetime;
    }
    Some(out)
}

/// Parse one rollout file into behavioural stats. Public for fixture tests.
pub fn parse_stats(path: &Path, since: Option<i64>) -> Option<SessionStats> {
    let mut out = SessionStats::new(
        AgentId::Codex,
        path.file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "unknown".into()),
    );
    out.source_file = Some(path.to_path_buf());
    let mut saw_activity = false;

    for line in jsonl::read_lines(path) {
        if line.is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&line) else {
            continue;
        };
        let ts = jsonl::parse_ts_ms(v.get("timestamp").unwrap_or(&serde_json::Value::Null));
        let Some(payload) = v.get("payload") else {
            continue;
        };
        match v.get("type").and_then(|t| t.as_str()).unwrap_or("") {
            "session_meta" => {
                if let Some(id) = payload.get("id").and_then(|i| i.as_str()) {
                    out.session_id = id.to_string();
                }
                out.cwd = payload
                    .get("cwd")
                    .and_then(|c| c.as_str())
                    .map(String::from);
                out.touch_ts(ts);
            }
            "response_item" => {
                if let Some(cutoff) = since
                    && ts.is_some_and(|t| t < cutoff)
                {
                    continue;
                }
                match payload.get("type").and_then(|t| t.as_str()).unwrap_or("") {
                    "message" => {
                        saw_activity = true;
                        out.touch_ts(ts);
                        match payload.get("role").and_then(|r| r.as_str()) {
                            Some("user") => out.user_messages += 1,
                            Some("assistant") => out.assistant_messages += 1,
                            _ => {}
                        }
                    }
                    "function_call" | "custom_tool_call" => {
                        saw_activity = true;
                        out.touch_ts(ts);
                        let name = payload
                            .get("name")
                            .and_then(|n| n.as_str())
                            .unwrap_or("unknown");
                        // arguments is a JSON-encoded string
                        let args = payload
                            .get("arguments")
                            .and_then(|a| a.as_str())
                            .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok())
                            .unwrap_or(serde_json::Value::Null);
                        out.record_tool(name, &args);
                    }
                    "local_shell_call" => {
                        saw_activity = true;
                        out.touch_ts(ts);
                        out.record_tool("shell", &serde_json::Value::Null);
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }

    if since.is_some() && !saw_activity {
        return None;
    }
    Some(out)
}

fn usage_to_counts(usage: &serde_json::Value) -> TokenCounts {
    let get = |k: &str| usage.get(k).and_then(|v| v.as_u64()).unwrap_or(0);
    let input_total = get("input_tokens");
    // Codex has used both spellings; take whichever reports more so a
    // rename upstream cannot silently zero the cache column.
    let cached = get("cached_input_tokens").max(get("cache_read_input_tokens"));
    TokenCounts {
        // codex input_tokens includes cached; split so columns match other agents
        input: input_total.saturating_sub(cached),
        output: get("output_tokens"),
        cache_read: cached,
        cache_write: 0,
        reasoning: get("reasoning_output_tokens"),
    }
}

#[cfg(test)]
mod usage_tests {
    use super::*;

    #[test]
    fn input_excludes_cached_so_columns_match_other_agents() {
        let u = serde_json::json!({
            "input_tokens": 100, "cached_input_tokens": 40,
            "output_tokens": 7, "reasoning_output_tokens": 3
        });
        let c = usage_to_counts(&u);
        assert_eq!(
            (c.input, c.cache_read, c.output, c.reasoning),
            (60, 40, 7, 3)
        );
    }

    #[test]
    fn accepts_either_spelling_of_the_cache_field() {
        // Codex has used both names; whichever reports more wins, so an
        // upstream rename cannot silently zero the cache column.
        let renamed = serde_json::json!({"input_tokens": 50, "cache_read_input_tokens": 20});
        assert_eq!(usage_to_counts(&renamed).cache_read, 20);
        let both = serde_json::json!({
            "input_tokens": 50, "cached_input_tokens": 20, "cache_read_input_tokens": 35
        });
        assert_eq!(usage_to_counts(&both).cache_read, 35);
    }

    #[test]
    fn cached_larger_than_input_does_not_underflow() {
        let u = serde_json::json!({"input_tokens": 5, "cached_input_tokens": 9});
        assert_eq!(usage_to_counts(&u).input, 0);
    }

    #[test]
    fn archived_sessions_are_scanned_too() {
        // Archiving a session must not erase the usage it recorded.
        let dirs = crate::paths::codex_session_dirs();
        assert!(dirs.iter().any(|d| d.ends_with("sessions")));
        assert!(
            dirs.iter().any(|d| d.ends_with("archived_sessions")),
            "archived rollouts share the schema and still count: {dirs:?}"
        );
    }
}
