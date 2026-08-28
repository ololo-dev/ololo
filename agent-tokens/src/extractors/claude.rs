use crate::jsonl;
use crate::paths;
use crate::trait_::TokenExtractor;
use crate::types::{AgentId, SessionCounts, SessionStats, TokenCounts};
use std::collections::HashMap;
use std::path::PathBuf;

pub struct Claude;

impl TokenExtractor for Claude {
    fn id(&self) -> AgentId {
        AgentId::Claude
    }

    fn detect(&self) -> bool {
        let cwd = std::env::current_dir().unwrap_or_default();
        let escaped = paths::escape_cwd(&cwd);
        paths::claude_projects_dir().join(&escaped).exists()
    }

    fn extract(&self, since: Option<i64>) -> Vec<SessionCounts> {
        let cwd = std::env::current_dir().unwrap_or_default();
        let escaped = paths::escape_cwd(&cwd);
        let project_dir = paths::claude_projects_dir().join(&escaped);

        if !project_dir.exists() {
            return vec![];
        }

        // Keyed by (session, model) so a mid-session model switch yields
        // one row per model instead of the last model absorbing all
        // tokens (the TUI/web stats join rows back per session).
        let mut sessions: HashMap<(String, Option<String>), SessionAgg> = HashMap::new();

        let Ok(entries) = std::fs::read_dir(&project_dir) else {
            return vec![];
        };

        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
                continue;
            }

            let lines = jsonl::read_lines(&path);
            for line in lines {
                if line.is_empty() {
                    continue;
                }
                let Ok(v) = serde_json::from_str::<serde_json::Value>(&line) else {
                    tracing::warn!("failed to parse claude jsonl line");
                    continue;
                };

                // Only assistant messages carry usage
                let msg_type = v.get("type").and_then(|t| t.as_str()).unwrap_or("");
                if msg_type != "assistant" {
                    continue;
                }

                let session_id = v
                    .get("sessionId")
                    .and_then(|s| s.as_str())
                    .unwrap_or("unknown")
                    .to_string();

                let message = match v.get("message") {
                    Some(m) => m,
                    None => continue,
                };

                let usage = match message.get("usage") {
                    Some(u) => u,
                    None => continue,
                };

                let model = message
                    .get("model")
                    .and_then(|m| m.as_str())
                    .map(String::from);

                let ts = jsonl::parse_ts_ms(v.get("timestamp").unwrap_or(&serde_json::Value::Null));

                // When since is set, only count turns within the time window
                if let Some(cutoff) = since
                    && let Some(t) = ts
                    && t < cutoff
                {
                    continue;
                }

                let agg = sessions
                    .entry((session_id.clone(), model.clone()))
                    .or_insert_with(|| SessionAgg {
                        session_id: session_id.clone(),
                        model: model.clone(),
                        cwd: v.get("cwd").and_then(|c| c.as_str()).map(String::from),
                        started_at_ms: ts,
                        last_seen_at_ms: ts,
                        counts: TokenCounts::default(),
                        cost: None,
                        source_file: Some(path.clone()),
                    });

                agg.counts.input += usage
                    .get("input_tokens")
                    .and_then(|t| t.as_u64())
                    .unwrap_or(0);
                agg.counts.output += usage
                    .get("output_tokens")
                    .and_then(|t| t.as_u64())
                    .unwrap_or(0);
                agg.counts.cache_read += usage
                    .get("cache_read_input_tokens")
                    .and_then(|t| t.as_u64())
                    .unwrap_or(0);
                agg.counts.cache_write += usage
                    .get("cache_creation_input_tokens")
                    .and_then(|t| t.as_u64())
                    .unwrap_or(0);

                if let Some(t) = ts {
                    agg.last_seen_at_ms = Some(t);
                    if agg.started_at_ms.is_none() {
                        agg.started_at_ms = Some(t);
                    }
                }
            }
        }

        sessions
            .into_values()
            .map(|a| SessionCounts {
                agent: AgentId::Claude,
                session_id: a.session_id,
                model: a.model,
                cwd: a.cwd,
                started_at_ms: a.started_at_ms,
                last_seen_at_ms: a.last_seen_at_ms,
                counts: a.counts,
                cost: a.cost,
                source_file: a.source_file,
            })
            .collect()
    }

    fn stats(&self, since: Option<i64>) -> Vec<SessionStats> {
        let cwd = std::env::current_dir().unwrap_or_default();
        let escaped = paths::escape_cwd(&cwd);
        let project_dir = paths::claude_projects_dir().join(&escaped);

        let Ok(entries) = std::fs::read_dir(&project_dir) else {
            return vec![];
        };

        let mut sessions: HashMap<String, SessionStats> = HashMap::new();

        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
                continue;
            }
            for line in jsonl::read_lines(&path) {
                if line.is_empty() {
                    continue;
                }
                let Ok(v) = serde_json::from_str::<serde_json::Value>(&line) else {
                    continue;
                };
                let msg_type = v.get("type").and_then(|t| t.as_str()).unwrap_or("");
                if msg_type != "user" && msg_type != "assistant" {
                    continue;
                }
                let ts = jsonl::parse_ts_ms(v.get("timestamp").unwrap_or(&serde_json::Value::Null));
                if let Some(cutoff) = since
                    && ts.is_some_and(|t| t < cutoff)
                {
                    continue;
                }
                let session_id = v
                    .get("sessionId")
                    .and_then(|s| s.as_str())
                    .unwrap_or("unknown")
                    .to_string();
                let stats = sessions.entry(session_id.clone()).or_insert_with(|| {
                    let mut s = SessionStats::new(AgentId::Claude, session_id);
                    s.cwd = v.get("cwd").and_then(|c| c.as_str()).map(String::from);
                    s.source_file = Some(path.clone());
                    s
                });
                stats.touch_ts(ts);

                if msg_type == "user" {
                    // Tool results come back as type=user lines with a
                    // toolUseResult field — don't count them as human messages.
                    if v.get("toolUseResult").is_none() {
                        stats.user_messages += 1;
                    }
                    continue;
                }

                stats.assistant_messages += 1;
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
                            stats.record_tool(name, args);
                        }
                    }
                }
            }
        }

        sessions.into_values().collect()
    }
}

struct SessionAgg {
    session_id: String,
    model: Option<String>,
    cwd: Option<String>,
    started_at_ms: Option<i64>,
    last_seen_at_ms: Option<i64>,
    counts: TokenCounts,
    cost: Option<f64>,
    source_file: Option<PathBuf>,
}
