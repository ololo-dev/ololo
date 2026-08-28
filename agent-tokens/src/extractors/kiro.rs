//! AWS Kiro CLI sessions: `~/.kiro/sessions/cli/<uuid>.json` (metadata with
//! per-turn token counts and model info) plus `<uuid>.jsonl` (transcript with
//! Prompt / AssistantMessage / ToolResults events).

use crate::jsonl;
use crate::paths;
use crate::trait_::TokenExtractor;
use crate::types::{AgentId, SessionCounts, SessionStats, TokenCounts};
use std::path::{Path, PathBuf};

pub struct Kiro;

impl TokenExtractor for Kiro {
    fn id(&self) -> AgentId {
        AgentId::Kiro
    }

    fn detect(&self) -> bool {
        paths::kiro_sessions_dir().exists()
    }

    fn extract(&self, since: Option<i64>) -> Vec<SessionCounts> {
        meta_files(&paths::kiro_sessions_dir())
            .iter()
            .filter_map(|f| parse_counts(f, since))
            .collect()
    }

    fn stats(&self, since: Option<i64>) -> Vec<SessionStats> {
        meta_files(&paths::kiro_sessions_dir())
            .iter()
            .filter_map(|f| parse_stats(f, since))
            .collect()
    }
}

fn meta_files(root: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(root) else {
        return vec![];
    };
    entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("json"))
        .collect()
}

/// Parse one kiro session meta file into token counts. Public for fixture tests.
pub fn parse_counts(meta_path: &Path, since: Option<i64>) -> Option<SessionCounts> {
    let content = std::fs::read_to_string(meta_path).ok()?;
    let v: serde_json::Value = serde_json::from_str(&content).ok()?;

    let session_id = v
        .get("session_id")
        .and_then(|s| s.as_str())
        .map(String::from)
        .unwrap_or_else(|| {
            meta_path
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| "unknown".into())
        });
    let started_at_ms = jsonl::parse_ts_ms(v.get("created_at").unwrap_or(&serde_json::Value::Null));
    let last_seen_at_ms =
        jsonl::parse_ts_ms(v.get("updated_at").unwrap_or(&serde_json::Value::Null));

    let state = v.get("session_state")?;
    let model = state
        .get("rts_model_state")
        .and_then(|m| m.get("model_info"))
        .and_then(|i| i.get("model_id"))
        .and_then(|m| m.as_str())
        .map(String::from);

    let mut counts = TokenCounts::default();
    let mut in_window = false;
    if let Some(turns) = state
        .get("conversation_metadata")
        .and_then(|c| c.get("user_turn_metadatas"))
        .and_then(|t| t.as_array())
    {
        for turn in turns {
            if let Some(cutoff) = since {
                let end = jsonl::parse_ts_ms(
                    turn.get("end_timestamp")
                        .unwrap_or(&serde_json::Value::Null),
                );
                if end.is_some_and(|t| t < cutoff) {
                    continue;
                }
            }
            in_window = true;
            counts.input += turn
                .get("input_token_count")
                .and_then(|t| t.as_u64())
                .unwrap_or(0);
            counts.output += turn
                .get("output_token_count")
                .and_then(|t| t.as_u64())
                .unwrap_or(0);
        }
    }
    if since.is_some() && !in_window {
        return None;
    }

    Some(SessionCounts {
        agent: AgentId::Kiro,
        session_id,
        model,
        cwd: v.get("cwd").and_then(|c| c.as_str()).map(String::from),
        started_at_ms,
        last_seen_at_ms,
        counts,
        // metering_usage is in Kiro credits, not dollars — not comparable
        cost: None,
        source_file: Some(meta_path.to_path_buf()),
    })
}

/// Parse a kiro session (meta json + sibling jsonl transcript) into stats.
/// Public for fixture tests.
pub fn parse_stats(meta_path: &Path, since: Option<i64>) -> Option<SessionStats> {
    let counts = parse_counts(meta_path, None)?;
    // The transcript has no per-event timestamps on assistant messages, so the
    // window filter is applied at session granularity via updated_at.
    if let Some(cutoff) = since
        && counts.last_seen_at_ms.is_some_and(|t| t < cutoff)
    {
        return None;
    }

    let mut out = SessionStats::new(AgentId::Kiro, counts.session_id);
    out.cwd = counts.cwd;
    out.started_at_ms = counts.started_at_ms;
    out.last_seen_at_ms = counts.last_seen_at_ms;
    out.source_file = Some(meta_path.to_path_buf());

    let transcript = meta_path.with_extension("jsonl");
    for line in jsonl::read_lines(&transcript) {
        if line.is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&line) else {
            continue;
        };
        let Some(data) = v.get("data") else {
            continue;
        };
        match v.get("kind").and_then(|k| k.as_str()).unwrap_or("") {
            "Prompt" => out.user_messages += 1,
            "AssistantMessage" => {
                out.assistant_messages += 1;
                if let Some(content) = data.get("content").and_then(|c| c.as_array()) {
                    for item in content {
                        if item.get("kind").and_then(|k| k.as_str()) == Some("toolUse")
                            && let Some(tool) = item.get("data")
                        {
                            let name = tool
                                .get("name")
                                .and_then(|n| n.as_str())
                                .unwrap_or("unknown");
                            let empty = serde_json::Value::Null;
                            let args = tool.get("input").unwrap_or(&empty);
                            out.record_tool(name, args);
                        }
                    }
                }
            }
            _ => {}
        }
    }
    Some(out)
}
