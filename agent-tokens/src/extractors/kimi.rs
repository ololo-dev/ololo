//! Kimi CLI wire logs: `~/.kimi/sessions/<group>/<session-uuid>/wire.jsonl`
//! (format per tokscale's kimi parser). Usage arrives as StatusUpdate lines:
//! `{"timestamp":<unix-secs float>,"message":{"type":"StatusUpdate","payload":
//! {"message_id":...,"token_usage":{"input_other","output","input_cache_read",
//! "input_cache_creation"}}}}`. StatusUpdates repeat per message_id with
//! growing totals — keep the largest per id, then sum.

use crate::paths;
use crate::trait_::TokenExtractor;
use crate::types::{AgentId, SessionCounts, TokenCounts};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub struct Kimi;

impl TokenExtractor for Kimi {
    fn id(&self) -> AgentId {
        AgentId::Kimi
    }

    fn detect(&self) -> bool {
        paths::kimi_sessions_dir().exists()
    }

    fn extract(&self, since: Option<i64>) -> Vec<SessionCounts> {
        wire_files(&paths::kimi_sessions_dir())
            .iter()
            .filter_map(|f| parse_counts(f, since))
            .collect()
    }

    // wire.jsonl carries only usage telemetry, no tool/message transcript —
    // no stats() implementation.
}

/// `sessions/<group>/<uuid>/wire.jsonl`
fn wire_files(root: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let Ok(groups) = std::fs::read_dir(root) else {
        return files;
    };
    for group in groups.flatten() {
        let Ok(sessions) = std::fs::read_dir(group.path()) else {
            continue;
        };
        for session in sessions.flatten() {
            let wire = session.path().join("wire.jsonl");
            if wire.is_file() {
                files.push(wire);
            }
        }
    }
    files
}

/// Parse one kimi wire.jsonl into token counts. Public for fixture tests.
pub fn parse_counts(path: &Path, since: Option<i64>) -> Option<SessionCounts> {
    // session id = parent dir (uuid)
    let session_id = path
        .parent()
        .and_then(|d| d.file_name())
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "unknown".into());

    // message_id -> (counts, total, ts); later updates with higher totals win
    let mut per_message: HashMap<String, (TokenCounts, u64, Option<i64>)> = HashMap::new();
    let mut first_ts: Option<i64> = None;
    let mut last_ts: Option<i64> = None;
    let mut model: Option<String> = None;

    let Ok(content) = std::fs::read_to_string(path) else {
        return None;
    };
    for line in content.lines() {
        let line = line.trim_end_matches('\r');
        if line.is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        let ts = v
            .get("timestamp")
            .and_then(|t| t.as_f64())
            .map(|secs| (secs * 1000.0) as i64);
        if let Some(t) = ts {
            if first_ts.is_none_or(|f| t < f) {
                first_ts = Some(t);
            }
            if last_ts.is_none_or(|l| t > l) {
                last_ts = Some(t);
            }
        }
        let Some(msg) = v.get("message") else {
            continue;
        };
        if msg.get("type").and_then(|t| t.as_str()) != Some("StatusUpdate") {
            continue;
        }
        let Some(payload) = msg.get("payload") else {
            continue;
        };
        if let Some(m) = payload.get("model").and_then(|m| m.as_str()) {
            model = Some(m.to_string());
        }
        let Some(usage) = payload.get("token_usage") else {
            continue;
        };
        if let Some(cutoff) = since
            && ts.is_some_and(|t| t < cutoff)
        {
            continue;
        }
        let get = |k: &str| usage.get(k).and_then(|x| x.as_u64()).unwrap_or(0);
        let counts = TokenCounts {
            input: get("input_other"),
            output: get("output"),
            cache_read: get("input_cache_read"),
            cache_write: get("input_cache_creation"),
            reasoning: 0,
        };
        let total = counts.input + counts.output + counts.cache_read + counts.cache_write;
        if total == 0 {
            continue;
        }
        let message_id = payload
            .get("message_id")
            .map(|m| m.to_string())
            .unwrap_or_else(|| format!("line-{}", per_message.len()));
        let entry = per_message
            .entry(message_id)
            .or_insert((TokenCounts::default(), 0, ts));
        if total >= entry.1 {
            *entry = (counts, total, ts);
        }
    }

    if per_message.is_empty() {
        return None;
    }
    let mut out = TokenCounts::default();
    for (counts, _, _) in per_message.values() {
        out.input += counts.input;
        out.output += counts.output;
        out.cache_read += counts.cache_read;
        out.cache_write += counts.cache_write;
    }
    Some(SessionCounts {
        agent: AgentId::Kimi,
        session_id,
        model,
        cwd: None,
        started_at_ms: first_ts,
        last_seen_at_ms: last_ts,
        counts: out,
        cost: None,
        source_file: Some(path.to_path_buf()),
    })
}
