//! Qwen Code CLI chat logs: `~/.qwen/projects/<project>/chats/*.jsonl`
//! (format per tokscale's qwen parser). Lines are messages; assistant lines
//! carry `usageMetadata` with Gemini-style field names:
//! `promptTokenCount`, `candidatesTokenCount`, `thoughtsTokenCount`,
//! `cachedContentTokenCount`, plus `sessionId`, `model`, ISO `timestamp`.

use crate::jsonl;
use crate::paths;
use crate::trait_::TokenExtractor;
use crate::types::{AgentId, SessionCounts, SessionStats, TokenCounts};
use std::path::{Path, PathBuf};

pub struct Qwen;

impl TokenExtractor for Qwen {
    fn id(&self) -> AgentId {
        AgentId::Qwen
    }

    fn detect(&self) -> bool {
        paths::qwen_projects_dir().exists()
    }

    fn extract(&self, since: Option<i64>) -> Vec<SessionCounts> {
        chat_files(&paths::qwen_projects_dir())
            .iter()
            .filter_map(|f| parse_counts(f, since))
            .collect()
    }

    fn stats(&self, since: Option<i64>) -> Vec<SessionStats> {
        chat_files(&paths::qwen_projects_dir())
            .iter()
            .filter_map(|f| parse_stats(f, since))
            .collect()
    }
}

/// `projects/<project>/chats/*.jsonl`
fn chat_files(root: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let Ok(projects) = std::fs::read_dir(root) else {
        return files;
    };
    for project in projects.flatten() {
        let chats = project.path().join("chats");
        let Ok(entries) = std::fs::read_dir(&chats) else {
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

fn file_session_id(path: &Path) -> String {
    path.file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "unknown".into())
}

/// Parse one qwen chat jsonl into token counts. Public for fixture tests.
pub fn parse_counts(path: &Path, since: Option<i64>) -> Option<SessionCounts> {
    let mut out = SessionCounts {
        agent: AgentId::Qwen,
        session_id: file_session_id(path),
        model: None,
        cwd: None,
        started_at_ms: None,
        last_seen_at_ms: None,
        counts: TokenCounts::default(),
        cost: None,
        source_file: Some(path.to_path_buf()),
    };
    let mut saw_usage = false;

    for line in jsonl::read_lines(path) {
        if line.is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&line) else {
            continue;
        };
        if let Some(id) = v.get("sessionId").and_then(|s| s.as_str())
            && !id.is_empty()
        {
            out.session_id = id.to_string();
        }
        let ts = jsonl::parse_ts_ms(v.get("timestamp").unwrap_or(&serde_json::Value::Null));
        if let Some(t) = ts {
            if out.started_at_ms.is_none_or(|s| t < s) {
                out.started_at_ms = Some(t);
            }
            if out.last_seen_at_ms.is_none_or(|l| t > l) {
                out.last_seen_at_ms = Some(t);
            }
        }
        if v.get("type").and_then(|t| t.as_str()) != Some("assistant") {
            continue;
        }
        if let Some(cutoff) = since
            && ts.is_some_and(|t| t < cutoff)
        {
            continue;
        }
        let Some(usage) = v.get("usageMetadata") else {
            continue;
        };
        saw_usage = true;
        let get = |k: &str| usage.get(k).and_then(|x| x.as_u64()).unwrap_or(0);
        out.counts.input += get("promptTokenCount");
        out.counts.output += get("candidatesTokenCount");
        out.counts.reasoning += get("thoughtsTokenCount");
        out.counts.cache_read += get("cachedContentTokenCount");
        if let Some(m) = v.get("model").and_then(|m| m.as_str()) {
            out.model = Some(m.to_string());
        }
    }

    if since.is_some() && !saw_usage {
        return None;
    }
    Some(out)
}

/// Parse one qwen chat jsonl into behavioural stats. Public for fixture tests.
pub fn parse_stats(path: &Path, since: Option<i64>) -> Option<SessionStats> {
    let mut out = SessionStats::new(AgentId::Qwen, file_session_id(path));
    out.source_file = Some(path.to_path_buf());
    let mut saw_activity = false;

    for line in jsonl::read_lines(path) {
        if line.is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&line) else {
            continue;
        };
        if let Some(id) = v.get("sessionId").and_then(|s| s.as_str())
            && !id.is_empty()
        {
            out.session_id = id.to_string();
        }
        let ts = jsonl::parse_ts_ms(v.get("timestamp").unwrap_or(&serde_json::Value::Null));
        if let Some(cutoff) = since
            && ts.is_some_and(|t| t < cutoff)
        {
            continue;
        }
        match v.get("type").and_then(|t| t.as_str()) {
            Some("user") => {
                saw_activity = true;
                out.user_messages += 1;
                out.touch_ts(ts);
            }
            Some("assistant") => {
                saw_activity = true;
                out.assistant_messages += 1;
                out.touch_ts(ts);
                // Qwen is a Gemini-CLI fork; tool calls appear as
                // content parts with functionCall {name, args}.
                if let Some(parts) = v
                    .get("content")
                    .and_then(|c| c.get("parts"))
                    .and_then(|p| p.as_array())
                {
                    for part in parts {
                        if let Some(fc) = part.get("functionCall")
                            && let Some(name) = fc.get("name").and_then(|n| n.as_str())
                        {
                            let empty = serde_json::Value::Null;
                            let args = fc.get("args").unwrap_or(&empty);
                            out.record_tool(name, args);
                        }
                    }
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
