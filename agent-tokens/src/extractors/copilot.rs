//! GitHub Copilot CLI sessions: `~/.copilot/session-state/<uuid>/events.jsonl`.
//! Events carry an ISO `timestamp`; `session.start` has cwd + selectedModel,
//! `assistant.message` has the resolved model, `tool.execution_start` names
//! each tool call. Copilot does not persist token usage locally, so token
//! counts are zero — sessions are still reported for model/activity/stats.

use crate::jsonl;
use crate::paths;
use crate::trait_::TokenExtractor;
use crate::types::{AgentId, SessionCounts, SessionStats, TokenCounts};
use std::path::{Path, PathBuf};

pub struct Copilot;

impl TokenExtractor for Copilot {
    fn id(&self) -> AgentId {
        AgentId::Copilot
    }

    fn detect(&self) -> bool {
        paths::copilot_state_dir().exists()
    }

    fn extract(&self, since: Option<i64>) -> Vec<SessionCounts> {
        event_files(&paths::copilot_state_dir())
            .iter()
            .filter_map(|f| parse_counts(f, since))
            .collect()
    }

    fn stats(&self, since: Option<i64>) -> Vec<SessionStats> {
        event_files(&paths::copilot_state_dir())
            .iter()
            .filter_map(|f| parse_stats(f, since))
            .collect()
    }
}

fn event_files(root: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(root) else {
        return vec![];
    };
    entries
        .flatten()
        .map(|e| e.path().join("events.jsonl"))
        .filter(|p| p.is_file())
        .collect()
}

fn dir_session_id(events_path: &Path) -> String {
    events_path
        .parent()
        .and_then(|d| d.file_name())
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "unknown".into())
}

/// Parse one copilot events file into session counts (tokens always zero).
/// Public for fixture tests.
pub fn parse_counts(events_path: &Path, since: Option<i64>) -> Option<SessionCounts> {
    let mut out = SessionCounts {
        agent: AgentId::Copilot,
        session_id: dir_session_id(events_path),
        model: None,
        cwd: None,
        started_at_ms: None,
        last_seen_at_ms: None,
        counts: TokenCounts::default(),
        cost: None,
        source_file: Some(events_path.to_path_buf()),
    };

    for line in jsonl::read_lines(events_path) {
        if line.is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&line) else {
            continue;
        };
        let data = v.get("data");
        let ts = jsonl::parse_ts_ms(v.get("timestamp").unwrap_or(&serde_json::Value::Null));
        if let Some(t) = ts {
            out.last_seen_at_ms = Some(out.last_seen_at_ms.unwrap_or(t).max(t));
        }
        match v.get("type").and_then(|t| t.as_str()).unwrap_or("") {
            "session.start" => {
                let Some(d) = data else { continue };
                if let Some(id) = d.get("sessionId").and_then(|s| s.as_str()) {
                    out.session_id = id.to_string();
                }
                out.started_at_ms =
                    jsonl::parse_ts_ms(d.get("startTime").unwrap_or(&serde_json::Value::Null));
                out.cwd = d
                    .get("context")
                    .and_then(|c| c.get("cwd"))
                    .and_then(|c| c.as_str())
                    .map(String::from);
                if let Some(m) = d.get("selectedModel").and_then(|m| m.as_str()) {
                    out.model = Some(m.to_string());
                }
            }
            "session.model_change" => {
                if let Some(m) = data
                    .and_then(|d| d.get("newModel"))
                    .and_then(|m| m.as_str())
                {
                    out.model = Some(m.to_string());
                }
            }
            "assistant.message" => {
                if let Some(m) = data.and_then(|d| d.get("model")).and_then(|m| m.as_str()) {
                    out.model = Some(m.to_string());
                }
            }
            _ => {}
        }
    }

    // No local token data: in window mode only report sessions active in the window.
    if let Some(cutoff) = since
        && out.last_seen_at_ms.is_none_or(|t| t < cutoff)
    {
        return None;
    }
    Some(out)
}

/// Parse one copilot events file into behavioural stats. Public for fixture tests.
pub fn parse_stats(events_path: &Path, since: Option<i64>) -> Option<SessionStats> {
    let mut out = SessionStats::new(AgentId::Copilot, dir_session_id(events_path));
    out.source_file = Some(events_path.to_path_buf());
    let mut saw_activity = false;

    for line in jsonl::read_lines(events_path) {
        if line.is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&line) else {
            continue;
        };
        let data = v.get("data");
        let ts = jsonl::parse_ts_ms(v.get("timestamp").unwrap_or(&serde_json::Value::Null));
        let event_type = v.get("type").and_then(|t| t.as_str()).unwrap_or("");

        if event_type == "session.start" {
            let Some(d) = data else { continue };
            if let Some(id) = d.get("sessionId").and_then(|s| s.as_str()) {
                out.session_id = id.to_string();
            }
            out.cwd = d
                .get("context")
                .and_then(|c| c.get("cwd"))
                .and_then(|c| c.as_str())
                .map(String::from);
            out.touch_ts(jsonl::parse_ts_ms(
                d.get("startTime").unwrap_or(&serde_json::Value::Null),
            ));
            continue;
        }

        if let Some(cutoff) = since
            && ts.is_some_and(|t| t < cutoff)
        {
            continue;
        }
        match event_type {
            "user.message" => {
                saw_activity = true;
                out.user_messages += 1;
                out.touch_ts(ts);
            }
            "assistant.message" => {
                saw_activity = true;
                out.assistant_messages += 1;
                out.touch_ts(ts);
            }
            "tool.execution_start" => {
                saw_activity = true;
                out.touch_ts(ts);
                let Some(d) = data else { continue };
                let name = d
                    .get("toolName")
                    .and_then(|n| n.as_str())
                    .unwrap_or("unknown");
                let empty = serde_json::Value::Null;
                let args = d.get("arguments").unwrap_or(&empty);
                out.record_tool(name, args);
            }
            _ => {}
        }
    }

    if since.is_some() && !saw_activity {
        return None;
    }
    Some(out)
}
