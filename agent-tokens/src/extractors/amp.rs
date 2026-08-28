//! Sourcegraph Amp threads: `~/.local/share/amp/threads/*.json` (format per
//! tokscale's amp parser). Each file is one thread:
//! `{"id","created":<epoch-ms>,"messages":[{"role",...,"usage"?}], "usageLedger"?}`.
//! Per-message usage fields: `inputTokens`, `outputTokens`,
//! `cacheReadInputTokens`, `cacheCreationInputTokens`, `credits`, `model` —
//! found either inline on the message or under its `usage` key.

use crate::jsonl;
use crate::paths;
use crate::trait_::TokenExtractor;
use crate::types::{AgentId, SessionCounts, SessionStats, TokenCounts};
use std::path::{Path, PathBuf};

pub struct Amp;

impl TokenExtractor for Amp {
    fn id(&self) -> AgentId {
        AgentId::Amp
    }

    fn detect(&self) -> bool {
        paths::amp_threads_dir().exists()
    }

    fn extract(&self, since: Option<i64>) -> Vec<SessionCounts> {
        thread_files(&paths::amp_threads_dir())
            .iter()
            .filter_map(|f| parse_counts(f, since))
            .collect()
    }

    fn stats(&self, since: Option<i64>) -> Vec<SessionStats> {
        thread_files(&paths::amp_threads_dir())
            .iter()
            .filter_map(|f| parse_stats(f, since))
            .collect()
    }
}

fn thread_files(root: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(root) else {
        return vec![];
    };
    entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("json"))
        .collect()
}

fn thread_id(v: &serde_json::Value, path: &Path) -> String {
    v.get("id")
        .and_then(|i| i.as_str())
        .map(String::from)
        .unwrap_or_else(|| {
            path.file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| "unknown".into())
        })
}

/// Usage fields may sit inline on the message or under `usage`.
fn usage_of(msg: &serde_json::Value) -> Option<serde_json::Value> {
    let candidate = msg.get("usage").unwrap_or(msg);
    if candidate.get("inputTokens").is_some() || candidate.get("outputTokens").is_some() {
        Some(candidate.clone())
    } else {
        None
    }
}

/// Parse one amp thread json into token counts. Public for fixture tests.
pub fn parse_counts(path: &Path, since: Option<i64>) -> Option<SessionCounts> {
    let content = std::fs::read_to_string(path).ok()?;
    let v: serde_json::Value = serde_json::from_str(&content).ok()?;

    let created = v.get("created").and_then(|c| c.as_i64());
    let mut out = SessionCounts {
        agent: AgentId::Amp,
        session_id: thread_id(&v, path),
        model: None,
        cwd: None,
        started_at_ms: created,
        last_seen_at_ms: created,
        counts: TokenCounts::default(),
        cost: None,
        source_file: Some(path.to_path_buf()),
    };

    let mut saw_usage = false;
    let mut credits = 0.0f64;
    if let Some(messages) = v.get("messages").and_then(|m| m.as_array()) {
        for msg in messages {
            let ts = jsonl::parse_ts_ms(msg.get("timestamp").unwrap_or(&serde_json::Value::Null))
                .or(created);
            if let Some(t) = ts {
                out.last_seen_at_ms = Some(out.last_seen_at_ms.unwrap_or(t).max(t));
            }
            if let Some(cutoff) = since
                && ts.is_some_and(|t| t < cutoff)
            {
                continue;
            }
            let Some(usage) = usage_of(msg) else {
                continue;
            };
            saw_usage = true;
            let get = |k: &str| usage.get(k).and_then(|x| x.as_u64()).unwrap_or(0);
            out.counts.input += get("inputTokens");
            out.counts.output += get("outputTokens");
            out.counts.cache_read += get("cacheReadInputTokens");
            out.counts.cache_write += get("cacheCreationInputTokens");
            credits += usage.get("credits").and_then(|c| c.as_f64()).unwrap_or(0.0);
            if let Some(m) = usage
                .get("model")
                .or_else(|| msg.get("model"))
                .and_then(|m| m.as_str())
            {
                out.model = Some(m.to_string());
            }
        }
    }

    if !saw_usage {
        // Threads with no local usage (server-side only) are not reported.
        return None;
    }
    out.cost = Some(credits);
    Some(out)
}

/// Parse one amp thread json into behavioural stats. Public for fixture tests.
pub fn parse_stats(path: &Path, since: Option<i64>) -> Option<SessionStats> {
    let content = std::fs::read_to_string(path).ok()?;
    let v: serde_json::Value = serde_json::from_str(&content).ok()?;

    let created = v.get("created").and_then(|c| c.as_i64());
    if let Some(cutoff) = since
        && created.is_some_and(|t| t < cutoff)
    {
        return None;
    }

    let mut out = SessionStats::new(AgentId::Amp, thread_id(&v, path));
    out.source_file = Some(path.to_path_buf());
    out.touch_ts(created);

    let mut saw_activity = false;
    if let Some(messages) = v.get("messages").and_then(|m| m.as_array()) {
        for msg in messages {
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
                            if item.get("type").and_then(|t| t.as_str()) == Some("tool_use")
                                && let Some(name) = item.get("name").and_then(|n| n.as_str())
                            {
                                let empty = serde_json::Value::Null;
                                let args = item.get("input").unwrap_or(&empty);
                                out.record_tool(name, args);
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }

    if !saw_activity {
        return None;
    }
    Some(out)
}
