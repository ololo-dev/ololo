//! Gemini CLI session data: `~/.gemini/tmp/<project-hash>/`.
//!
//! Primary source is the chat checkpoints `chats/*.json` (format per
//! tokscale's gemini parser). Each file is one session:
//! `{"sessionId","startTime","lastUpdated","messages":[{"id","type","model",
//! "timestamp","tokens":{"input","output","cached","thoughts","tool","total"}}]}`.
//! `tokens.input` includes cached tokens — cached is subtracted to avoid
//! double-counting.
//!
//! When a project dir has no chat checkpoints (chat auto-save off, or an
//! older CLI), `logs.json` — the user-prompt log gemini-cli has always
//! written — still yields per-session behavioural stats, so gemini activity
//! is visible even without token data.
//!
//! `<project-hash>` is sha256-hex of the project root path (gemini-cli's
//! `getProjectHash`). The hash can't be reversed, but hashing a candidate
//! directory forward identifies its sessions — sessions under the hash of
//! the current working directory get `cwd` set, which per-task stats
//! consumers use for project attribution.

use crate::jsonl;
use crate::paths;
use crate::trait_::TokenExtractor;
use crate::types::{AgentId, SessionCounts, SessionStats, TokenCounts};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

pub struct Gemini;

impl TokenExtractor for Gemini {
    fn id(&self) -> AgentId {
        AgentId::Gemini
    }

    fn detect(&self) -> bool {
        paths::gemini_tmp_dir().exists()
    }

    fn extract(&self, since: Option<i64>) -> Vec<SessionCounts> {
        let cwd = std::env::current_dir().ok();
        let cwd_hash = cwd.as_ref().map(|p| project_hash(p));
        let mut out = Vec::new();
        for dir in project_dirs(&paths::gemini_tmp_dir()) {
            let attributed_cwd = resolved_cwd(&dir.hash, cwd.as_deref(), cwd_hash.as_deref());
            for f in &dir.chats {
                if let Some(mut s) = parse_counts(f, since) {
                    s.cwd = attributed_cwd.clone();
                    out.push(s);
                }
            }
        }
        out
    }

    fn stats(&self, since: Option<i64>) -> Vec<SessionStats> {
        let cwd = std::env::current_dir().ok();
        let cwd_hash = cwd.as_ref().map(|p| project_hash(p));
        let mut out = Vec::new();
        for dir in project_dirs(&paths::gemini_tmp_dir()) {
            let attributed_cwd = resolved_cwd(&dir.hash, cwd.as_deref(), cwd_hash.as_deref());
            let mut sessions = if !dir.chats.is_empty() {
                dir.chats
                    .iter()
                    .filter_map(|f| parse_stats(f, since))
                    .collect()
            } else if let Some(logs) = &dir.logs {
                parse_logs_stats(logs, since)
            } else {
                Vec::new()
            };
            for s in &mut sessions {
                s.cwd = attributed_cwd.clone();
            }
            out.extend(sessions);
        }
        out
    }
}

/// sha256-hex of the project root path — gemini-cli's `getProjectHash`
/// scheme for naming `~/.gemini/tmp/<hash>/`. Public for consumers that
/// want to match sessions against a known directory.
pub fn project_hash(path: &Path) -> String {
    let mut hasher = Sha256::new();
    hasher.update(path.to_string_lossy().as_bytes());
    format!("{:x}", hasher.finalize())
}

fn resolved_cwd(hash: &str, cwd: Option<&Path>, cwd_hash: Option<&str>) -> Option<String> {
    match (cwd, cwd_hash) {
        (Some(dir), Some(h)) if h == hash => Some(dir.to_string_lossy().into_owned()),
        _ => None,
    }
}

/// One `tmp/<hash>/` project directory and its session sources.
struct ProjectDir {
    hash: String,
    /// `chats/*.json` checkpoint files.
    chats: Vec<PathBuf>,
    /// `logs.json` user-prompt log, when present.
    logs: Option<PathBuf>,
}

fn project_dirs(root: &Path) -> Vec<ProjectDir> {
    let mut dirs = Vec::new();
    let Ok(hashes) = std::fs::read_dir(root) else {
        return dirs;
    };
    for hash_entry in hashes.flatten() {
        let hash_path = hash_entry.path();
        if !hash_path.is_dir() {
            continue;
        }
        let mut dir = ProjectDir {
            hash: hash_entry.file_name().to_string_lossy().into_owned(),
            chats: Vec::new(),
            logs: None,
        };
        if let Ok(entries) = std::fs::read_dir(hash_path.join("chats")) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) == Some("json") {
                    dir.chats.push(path);
                }
            }
        }
        let logs = hash_path.join("logs.json");
        if logs.is_file() {
            dir.logs = Some(logs);
        }
        dirs.push(dir);
    }
    dirs
}

fn token_field(tokens: &serde_json::Value, keys: &[&str]) -> u64 {
    keys.iter()
        .find_map(|k| tokens.get(*k).and_then(|v| v.as_u64()))
        .unwrap_or(0)
}

/// Parse one gemini chat checkpoint into token counts. Public for fixture tests.
pub fn parse_counts(path: &Path, since: Option<i64>) -> Option<SessionCounts> {
    let content = std::fs::read_to_string(path).ok()?;
    let v: serde_json::Value = serde_json::from_str(&content).ok()?;

    let mut out = SessionCounts {
        agent: AgentId::Gemini,
        session_id: v
            .get("sessionId")
            .and_then(|s| s.as_str())
            .map(String::from)
            .unwrap_or_else(|| {
                path.file_stem()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "unknown".into())
            }),
        model: None,
        cwd: None,
        started_at_ms: jsonl::parse_ts_ms(v.get("startTime").unwrap_or(&serde_json::Value::Null)),
        last_seen_at_ms: jsonl::parse_ts_ms(
            v.get("lastUpdated").unwrap_or(&serde_json::Value::Null),
        ),
        counts: TokenCounts::default(),
        cost: None,
        source_file: Some(path.to_path_buf()),
    };

    let mut saw_usage = false;
    if let Some(messages) = v.get("messages").and_then(|m| m.as_array()) {
        for msg in messages {
            let ts = jsonl::parse_ts_ms(msg.get("timestamp").unwrap_or(&serde_json::Value::Null));
            if let Some(cutoff) = since
                && ts.is_some_and(|t| t < cutoff)
            {
                continue;
            }
            let Some(tokens) = msg.get("tokens") else {
                continue;
            };
            saw_usage = true;
            let input = token_field(tokens, &["input", "promptTokenCount", "prompt"]);
            let cached = token_field(tokens, &["cached", "cachedContentTokenCount"]);
            // input includes cached — split to match other agents
            out.counts.input += input.saturating_sub(cached);
            out.counts.cache_read += cached;
            out.counts.output +=
                token_field(tokens, &["output", "candidatesTokenCount", "candidates"]);
            out.counts.reasoning +=
                token_field(tokens, &["thoughts", "thoughtsTokenCount", "reasoning"]);
            if let Some(m) = msg.get("model").and_then(|m| m.as_str()) {
                out.model = Some(m.to_string());
            }
        }
    }

    if since.is_some() && !saw_usage {
        return None;
    }
    Some(out)
}

/// Parse one gemini chat checkpoint into behavioural stats. Public for fixture tests.
pub fn parse_stats(path: &Path, since: Option<i64>) -> Option<SessionStats> {
    let content = std::fs::read_to_string(path).ok()?;
    let v: serde_json::Value = serde_json::from_str(&content).ok()?;

    let mut out = SessionStats::new(
        AgentId::Gemini,
        v.get("sessionId")
            .and_then(|s| s.as_str())
            .map(String::from)
            .unwrap_or_else(|| {
                path.file_stem()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "unknown".into())
            }),
    );
    out.source_file = Some(path.to_path_buf());
    out.touch_ts(jsonl::parse_ts_ms(
        v.get("startTime").unwrap_or(&serde_json::Value::Null),
    ));
    out.touch_ts(jsonl::parse_ts_ms(
        v.get("lastUpdated").unwrap_or(&serde_json::Value::Null),
    ));

    let mut saw_activity = false;
    if let Some(messages) = v.get("messages").and_then(|m| m.as_array()) {
        for msg in messages {
            let ts = jsonl::parse_ts_ms(msg.get("timestamp").unwrap_or(&serde_json::Value::Null));
            if let Some(cutoff) = since
                && ts.is_some_and(|t| t < cutoff)
            {
                continue;
            }
            match msg.get("type").and_then(|t| t.as_str()) {
                Some("user") => {
                    saw_activity = true;
                    out.user_messages += 1;
                }
                Some("gemini") | Some("assistant") => {
                    saw_activity = true;
                    out.assistant_messages += 1;
                }
                _ => {}
            }
            // Tool calls, when present, appear as toolCalls: [{"name":...,"args":...}]
            if let Some(calls) = msg.get("toolCalls").and_then(|c| c.as_array()) {
                for call in calls {
                    if let Some(name) = call.get("name").and_then(|n| n.as_str()) {
                        saw_activity = true;
                        let empty = serde_json::Value::Null;
                        let args = call.get("args").unwrap_or(&empty);
                        out.record_tool(name, args);
                    }
                }
            }
        }
    }

    if since.is_some() && !saw_activity {
        return None;
    }
    Some(out)
}

/// Parse a project dir's `logs.json` (array of
/// `{"sessionId","messageId","type":"user","message","timestamp"}` entries)
/// into per-session stats. Only user prompts are recorded there, so entries
/// carry message counts and timestamps but no token or tool data. Public for
/// fixture tests.
pub fn parse_logs_stats(path: &Path, since: Option<i64>) -> Vec<SessionStats> {
    let Some(content) = std::fs::read_to_string(path).ok() else {
        return Vec::new();
    };
    let Ok(entries) = serde_json::from_str::<Vec<serde_json::Value>>(&content) else {
        return Vec::new();
    };

    let mut by_session: std::collections::BTreeMap<String, SessionStats> =
        std::collections::BTreeMap::new();
    for entry in &entries {
        if entry.get("type").and_then(|t| t.as_str()) != Some("user") {
            continue;
        }
        let ts = jsonl::parse_ts_ms(entry.get("timestamp").unwrap_or(&serde_json::Value::Null));
        if let Some(cutoff) = since
            && ts.is_some_and(|t| t < cutoff)
        {
            continue;
        }
        let Some(session_id) = entry.get("sessionId").and_then(|s| s.as_str()) else {
            continue;
        };
        let stats = by_session.entry(session_id.to_string()).or_insert_with(|| {
            let mut s = SessionStats::new(AgentId::Gemini, session_id.to_string());
            s.source_file = Some(path.to_path_buf());
            s
        });
        stats.user_messages += 1;
        stats.touch_ts(ts);
    }
    by_session.into_values().collect()
}
