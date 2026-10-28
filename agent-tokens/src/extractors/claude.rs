use crate::jsonl;
use crate::paths;
use crate::trait_::TokenExtractor;
use crate::types::{AgentId, SessionCounts, SessionStats, TokenCounts};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

pub struct Claude;

impl TokenExtractor for Claude {
    fn id(&self) -> AgentId {
        AgentId::Claude
    }

    fn detect(&self) -> bool {
        paths::claude_projects_dir().exists()
    }

    fn extract(&self, since: Option<i64>) -> Vec<SessionCounts> {
        let cwd = std::env::current_dir().unwrap_or_default();

        // Keyed by (session, model) so a mid-session model switch yields
        // one row per model instead of the last model absorbing all
        // tokens (the TUI/web stats join rows back per session).
        let mut sessions: HashMap<(String, Option<String>), SessionAgg> = HashMap::new();
        // Claude Code streams one `assistant` line per content block, each
        // carrying the whole message's usage: a text block and a tool call
        // from one reply are two lines with the same `message.id`, and
        // counting both doubles the tokens.
        let mut counted: HashSet<String> = HashSet::new();

        for file in transcripts(&cwd, since) {
            for line in jsonl::read_lines(&file.path) {
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
                if !file.admits(&v, &cwd) {
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

                if let Some(id) = message.get("id").and_then(|i| i.as_str())
                    && !counted.insert(format!("{session_id}/{id}"))
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
                        source_file: Some(file.path.clone()),
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
        let mut sessions: HashMap<String, SessionStats> = HashMap::new();
        // One reply, several lines (see `extract`): the message counts once,
        // while every line's tool call is its own.
        let mut counted: HashSet<String> = HashSet::new();

        for file in transcripts(&cwd, since) {
            for line in jsonl::read_lines(&file.path) {
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
                if !file.admits(&v, &cwd) {
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
                    let mut s = SessionStats::new(AgentId::Claude, session_id.clone());
                    s.cwd = v.get("cwd").and_then(|c| c.as_str()).map(String::from);
                    s.source_file = Some(file.path.clone());
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

                let message_id = v
                    .get("message")
                    .and_then(|m| m.get("id"))
                    .and_then(|i| i.as_str());
                let first_line = match message_id {
                    Some(id) => counted.insert(format!("{session_id}/{id}")),
                    None => true,
                };
                if first_line {
                    stats.assistant_messages += 1;
                }
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

/// A transcript file to read, and on what terms.
struct Transcript {
    path: PathBuf,
    /// Found outside the folder named after the working directory: its
    /// lines count only when they say they were written in that directory.
    by_cwd_only: bool,
}

impl Transcript {
    fn admits(&self, line: &serde_json::Value, cwd: &Path) -> bool {
        if !self.by_cwd_only {
            return true;
        }
        line.get("cwd")
            .and_then(|c| c.as_str())
            .is_some_and(|recorded| paths::same_dir(recorded, cwd))
    }
}

/// The transcripts of sessions run in `cwd`.
///
/// Claude Code keeps them under `~/.claude/projects/<escaped cwd>/`. That
/// folder is read whole. Every line also records its `cwd`, which is the
/// authoritative link — so when a window is given, the recently written
/// transcripts of every other project folder are read too, admitting only
/// the lines written in `cwd`. That is what finds the session when the
/// folder name and the escaping disagree (a Claude Code release changing
/// the rule, a path this escaper gets wrong) instead of reporting nothing.
fn transcripts(cwd: &Path, since: Option<i64>) -> Vec<Transcript> {
    let projects = paths::claude_projects_dir();
    let own = projects.join(paths::escape_cwd(cwd));
    let mut out: Vec<Transcript> = jsonl_files(&own)
        .into_iter()
        .map(|path| Transcript {
            path,
            by_cwd_only: false,
        })
        .collect();

    let Some(since) = since else {
        return out;
    };
    // Modification time, not content: a transcript last written before the
    // window opened has nothing in it.
    let fresh = |path: &Path| {
        std::fs::metadata(path)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .is_some_and(|d| d.as_millis() as i64 >= since)
    };
    let Ok(dirs) = std::fs::read_dir(&projects) else {
        return out;
    };
    for dir in dirs.flatten() {
        let dir = dir.path();
        if dir == own || !dir.is_dir() {
            continue;
        }
        out.extend(
            jsonl_files(&dir)
                .into_iter()
                .filter(|p| fresh(p))
                .map(|path| Transcript {
                    path,
                    by_cwd_only: true,
                }),
        );
    }
    out
}

fn jsonl_files(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return vec![];
    };
    entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("jsonl"))
        .collect()
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_folder_name_follows_claude_codes_escaping() {
        assert_eq!(
            paths::escape_cwd(Path::new("/Users/apk/lab/my_app.v2 x")),
            "-Users-apk-lab-my-app-v2-x"
        );
        assert_eq!(
            paths::escape_cwd(Path::new("/private/tmp/claude-502/-Users-apk")),
            "-private-tmp-claude-502--Users-apk"
        );
        assert_eq!(
            paths::escape_cwd(Path::new(r"C:\Users\apk\play")),
            "C--Users-apk-play"
        );
    }

    #[test]
    fn a_recorded_cwd_is_matched_as_a_path() {
        assert!(paths::same_dir(
            "/Users/apk/play",
            Path::new("/Users/apk/play")
        ));
        assert!(paths::same_dir(
            "/Users/apk/play/",
            Path::new("/Users/apk/play")
        ));
        assert!(!paths::same_dir(
            "/Users/apk/play2",
            Path::new("/Users/apk/play")
        ));
    }
}
