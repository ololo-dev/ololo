//! Shared session-log parsing for pi and omp (omp is a pi fork; both write
//! `<root>/<escaped-cwd>/<timestamp>_<uuid>.jsonl` with the same line format:
//! a `{"type":"session",...}` header, then `{"type":"message",...}` entries
//! where assistant messages carry `usage` and `provider`/`model`).

use crate::jsonl;
use crate::types::{AgentId, SessionCounts, SessionStats, TokenCounts};
use std::path::{Path, PathBuf};

/// Walk `<root>/*/ *.jsonl` and return one SessionCounts per session file.
pub fn extract_from_root(root: &Path, agent: AgentId, since: Option<i64>) -> Vec<SessionCounts> {
    session_files(root)
        .iter()
        .filter_map(|f| parse_counts(f, agent, since))
        .collect()
}

/// Walk `<root>/*/ *.jsonl` and return one SessionStats per session file.
pub fn stats_from_root(root: &Path, agent: AgentId, since: Option<i64>) -> Vec<SessionStats> {
    session_files(root)
        .iter()
        .filter_map(|f| parse_stats(f, agent, since))
        .collect()
}

fn session_files(root: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let Ok(dirs) = std::fs::read_dir(root) else {
        return files;
    };
    for dir in dirs.flatten() {
        let dir_path = dir.path();
        if !dir_path.is_dir() {
            continue;
        }
        let Ok(entries) = std::fs::read_dir(&dir_path) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("jsonl") {
                files.push(path);
            }
        }
    }
    files
}

/// Parse one pi/omp session file into token counts. Public for fixture tests.
pub fn parse_counts(path: &Path, agent: AgentId, since: Option<i64>) -> Option<SessionCounts> {
    let mut out = SessionCounts {
        agent,
        session_id: file_stem_session_id(path),
        model: None,
        cwd: None,
        started_at_ms: None,
        last_seen_at_ms: None,
        counts: TokenCounts::default(),
        cost: None,
        source_file: Some(path.to_path_buf()),
    };
    let mut saw_usage = false;
    let mut cost = 0.0f64;

    for line in jsonl::read_lines(path) {
        if line.is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&line) else {
            continue;
        };
        match v.get("type").and_then(|t| t.as_str()).unwrap_or("") {
            "session" => {
                if let Some(id) = v.get("id").and_then(|i| i.as_str()) {
                    out.session_id = id.to_string();
                }
                out.cwd = v.get("cwd").and_then(|c| c.as_str()).map(String::from);
                out.started_at_ms =
                    jsonl::parse_ts_ms(v.get("timestamp").unwrap_or(&serde_json::Value::Null));
            }
            "message" => {
                let Some(msg) = v.get("message") else {
                    continue;
                };
                let ts = message_ts_ms(&v, msg);
                if let Some(t) = ts {
                    out.last_seen_at_ms = Some(out.last_seen_at_ms.unwrap_or(t).max(t));
                }
                if msg.get("role").and_then(|r| r.as_str()) != Some("assistant") {
                    continue;
                }
                if let Some(cutoff) = since
                    && ts.is_some_and(|t| t < cutoff)
                {
                    continue;
                }
                let Some(usage) = msg.get("usage") else {
                    continue;
                };
                saw_usage = true;
                out.counts.input += u64_field(usage, "input");
                out.counts.output += u64_field(usage, "output");
                out.counts.cache_read += u64_field(usage, "cacheRead");
                out.counts.cache_write += u64_field(usage, "cacheWrite");
                cost += usage
                    .get("cost")
                    .and_then(|c| c.get("total"))
                    .and_then(|t| t.as_f64())
                    .unwrap_or(0.0);
                // Store model as JSON matching the opencode format so the
                // table render splits provider/model uniformly.
                if let (Some(provider), Some(model)) = (
                    msg.get("provider").and_then(|p| p.as_str()),
                    msg.get("model").and_then(|m| m.as_str()),
                ) {
                    out.model =
                        Some(serde_json::json!({"id": model, "providerID": provider}).to_string());
                }
            }
            _ => {}
        }
    }

    if since.is_some() && !saw_usage {
        return None; // nothing in the window
    }
    out.cost = saw_usage.then_some(cost);
    Some(out)
}

/// Parse one pi/omp session file into behavioural stats. Public for fixture tests.
pub fn parse_stats(path: &Path, agent: AgentId, since: Option<i64>) -> Option<SessionStats> {
    let mut out = SessionStats::new(agent, file_stem_session_id(path));
    out.source_file = Some(path.to_path_buf());
    let mut saw_activity = false;

    for line in jsonl::read_lines(path) {
        if line.is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&line) else {
            continue;
        };
        match v.get("type").and_then(|t| t.as_str()).unwrap_or("") {
            "session" => {
                if let Some(id) = v.get("id").and_then(|i| i.as_str()) {
                    out.session_id = id.to_string();
                }
                out.cwd = v.get("cwd").and_then(|c| c.as_str()).map(String::from);
                out.touch_ts(jsonl::parse_ts_ms(
                    v.get("timestamp").unwrap_or(&serde_json::Value::Null),
                ));
            }
            "message" => {
                let Some(msg) = v.get("message") else {
                    continue;
                };
                let ts = message_ts_ms(&v, msg);
                if let Some(cutoff) = since
                    && ts.is_some_and(|t| t < cutoff)
                {
                    continue;
                }
                out.touch_ts(ts);
                match msg.get("role").and_then(|r| r.as_str()) {
                    Some("user") => {
                        saw_activity = true;
                        out.user_messages += 1;
                    }
                    Some("assistant") => {
                        saw_activity = true;
                        out.assistant_messages += 1;
                        if let Some(content) = msg.get("content").and_then(|c| c.as_array()) {
                            for item in content {
                                if item.get("type").and_then(|t| t.as_str()) == Some("toolCall")
                                    && let Some(name) = item.get("name").and_then(|n| n.as_str())
                                {
                                    let empty = serde_json::Value::Null;
                                    let args = item.get("arguments").unwrap_or(&empty);
                                    out.record_tool(name, args);
                                }
                            }
                        }
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

/// Prefer the inner message timestamp (epoch-ms number), fall back to the
/// outer ISO timestamp.
fn message_ts_ms(line: &serde_json::Value, msg: &serde_json::Value) -> Option<i64> {
    msg.get("timestamp")
        .and_then(|t| t.as_i64())
        .or_else(|| jsonl::parse_ts_ms(line.get("timestamp").unwrap_or(&serde_json::Value::Null)))
}

fn u64_field(v: &serde_json::Value, key: &str) -> u64 {
    v.get(key).and_then(|x| x.as_u64()).unwrap_or(0)
}

fn file_stem_session_id(path: &Path) -> String {
    path.file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "unknown".to_string())
}
