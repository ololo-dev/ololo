//! Cursor CLI agent transcripts:
//! `~/.cursor/projects/<escaped>/agent-transcripts/<uuid>/<uuid>.jsonl`.
//! Lines are `{"role": "user"|"assistant", "message": {"content": [...]}}` with
//! `tool_use` content blocks. No token usage and no timestamps are persisted,
//! so counts are zero and session times come from file mtime.

use crate::jsonl;
use crate::paths;
use crate::trait_::TokenExtractor;
use crate::types::{AgentId, SessionCounts, SessionStats, TokenCounts};
use std::path::{Path, PathBuf};

pub struct CursorCli;

impl TokenExtractor for CursorCli {
    fn id(&self) -> AgentId {
        AgentId::CursorCli
    }

    fn detect(&self) -> bool {
        paths::cursor_projects_dir().exists()
    }

    fn extract(&self, since: Option<i64>) -> Vec<SessionCounts> {
        transcript_files(&paths::cursor_projects_dir())
            .iter()
            .filter_map(|f| parse_counts(f, since))
            .collect()
    }

    fn stats(&self, since: Option<i64>) -> Vec<SessionStats> {
        transcript_files(&paths::cursor_projects_dir())
            .iter()
            .filter_map(|f| parse_stats(f, since))
            .collect()
    }
}

/// `projects/*/agent-transcripts/*/*.jsonl`
fn transcript_files(root: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let Ok(projects) = std::fs::read_dir(root) else {
        return files;
    };
    for project in projects.flatten() {
        let transcripts = project.path().join("agent-transcripts");
        let Ok(sessions) = std::fs::read_dir(&transcripts) else {
            continue;
        };
        for session in sessions.flatten() {
            let Ok(entries) = std::fs::read_dir(session.path()) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) == Some("jsonl") {
                    files.push(path);
                }
            }
        }
    }
    files
}

fn session_id_of(path: &Path) -> String {
    path.file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "unknown".into())
}

fn mtime_ms(path: &Path) -> Option<i64> {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .map(|d| d.as_millis() as i64)
}

/// Parse one cursor transcript into session counts (tokens always zero).
/// Public for fixture tests.
pub fn parse_counts(path: &Path, since: Option<i64>) -> Option<SessionCounts> {
    let last_seen = mtime_ms(path);
    if let Some(cutoff) = since
        && last_seen.is_none_or(|t| t < cutoff)
    {
        return None;
    }
    // Skip empty transcripts
    if jsonl::read_lines(path).iter().all(|l| l.is_empty()) {
        return None;
    }
    Some(SessionCounts {
        agent: AgentId::CursorCli,
        session_id: session_id_of(path),
        model: None,
        cwd: None,
        started_at_ms: None,
        last_seen_at_ms: last_seen,
        counts: TokenCounts::default(),
        cost: None,
        source_file: Some(path.to_path_buf()),
    })
}

/// Parse one cursor transcript into behavioural stats. Public for fixture tests.
pub fn parse_stats(path: &Path, since: Option<i64>) -> Option<SessionStats> {
    let last_seen = mtime_ms(path);
    if let Some(cutoff) = since
        && last_seen.is_none_or(|t| t < cutoff)
    {
        return None;
    }

    let mut out = SessionStats::new(AgentId::CursorCli, session_id_of(path));
    out.last_seen_at_ms = last_seen;
    out.source_file = Some(path.to_path_buf());
    let mut saw_activity = false;

    for line in jsonl::read_lines(path) {
        if line.is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&line) else {
            continue;
        };
        match v.get("role").and_then(|r| r.as_str()) {
            Some("user") => {
                saw_activity = true;
                out.user_messages += 1;
            }
            Some("assistant") => {
                saw_activity = true;
                out.assistant_messages += 1;
                if let Some(content) = v
                    .get("message")
                    .and_then(|m| m.get("content"))
                    .and_then(|c| c.as_array())
                {
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

    if !saw_activity {
        return None;
    }
    Some(out)
}
