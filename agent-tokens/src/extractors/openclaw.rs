//! OpenClaw session transcripts (the agent was previously shipped as
//! clawdbot, moltbot and moldbot, and every one of those homes may still hold
//! history).
//!
//! Discovery mirrors ccusage's OpenClaw adapter: `OPENCLAW_DIR` (one root or a
//! comma-separated list) wins; otherwise `~/.openclaw`, `~/.clawdbot`,
//! `~/.moltbot` and `~/.moldbot` are all scanned. Each root is walked
//! recursively (typically `<root>/agents/<agent>/sessions/<uuid>.jsonl`),
//! symlinks are skipped, and archived transcripts named
//! `<uuid>.jsonl.deleted.<ts>` / `<uuid>.jsonl.reset.<ts>` still count. The
//! session id is the filename up to `.jsonl`, so a live file and its archived
//! copies fold into one session (identical records are counted once).
//!
//! Line shapes (pi-agent lineage):
//! - `{"type":"session","id","timestamp","cwd"}` header (cwd when present)
//! - `{"type":"model_change","provider","modelId"}` or
//!   `{"type":"custom","customType":"model-snapshot","data":{"provider","modelId"}}`
//!   set the active model for later messages that do not name their own
//! - `{"type":"message","message":{"role","content":[...],"usage":{"input",
//!   "output","cacheRead","cacheWrite","totalTokens","cost":{"total"}},
//!   "timestamp"}}` — only assistant messages carry usage; `content` items of
//!   `{"type":"toolCall","name","arguments"}` are the tool calls

use crate::jsonl;
use crate::paths;
use crate::trait_::TokenExtractor;
use crate::types::{AgentId, SessionCounts, SessionStats, TokenCounts};
use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

/// Same override ccusage honours for this source.
const OPENCLAW_DIR_ENV: &str = "OPENCLAW_DIR";

pub struct OpenClaw;

impl TokenExtractor for OpenClaw {
    fn id(&self) -> AgentId {
        AgentId::OpenClaw
    }

    fn detect(&self) -> bool {
        !data_dirs().is_empty()
    }

    fn extract(&self, since: Option<i64>) -> Vec<SessionCounts> {
        sessions()
            .into_iter()
            .filter_map(|(id, files)| counts_from_files(&id, &files, since))
            .collect()
    }

    fn stats(&self, since: Option<i64>) -> Vec<SessionStats> {
        sessions()
            .into_iter()
            .filter_map(|(id, files)| stats_from_files(&id, &files, since))
            .collect()
    }
}

// ---------------------------------------------------------------------------
// Discovery
// ---------------------------------------------------------------------------

/// OpenClaw roots that exist on this machine. `OPENCLAW_DIR` (comma-separated
/// list allowed) replaces the defaults; otherwise every known home-dir name is
/// tried under `paths::home_dir()` so `AGENT_TOKENS_HOME` redirects it.
fn data_dirs() -> Vec<PathBuf> {
    let env = std::env::var(OPENCLAW_DIR_ENV).ok();
    data_dirs_from(&paths::home_dir(), env.as_deref())
}

/// Pure form of [`data_dirs`] so discovery can be tested without touching the
/// process environment.
fn data_dirs_from(home: &Path, env_override: Option<&str>) -> Vec<PathBuf> {
    if let Some(raw) = env_override.filter(|v| !v.trim().is_empty()) {
        let mut seen = HashSet::new();
        return raw
            .split(',')
            .map(str::trim)
            .filter(|p| !p.is_empty())
            .map(PathBuf::from)
            .filter(|p| p.is_dir() && seen.insert(p.clone()))
            .collect();
    }
    [".openclaw", ".clawdbot", ".moltbot", ".moldbot"]
        .iter()
        .map(|d| home.join(d))
        .filter(|p| p.is_dir())
        .collect()
}

/// Every session file under every root, grouped by session id and sorted so
/// the live transcript and its archived copies are read in a stable order.
fn sessions() -> BTreeMap<String, Vec<PathBuf>> {
    let mut files = Vec::new();
    for root in data_dirs() {
        files.extend(session_files(&root));
    }
    files.sort();
    let mut grouped: BTreeMap<String, Vec<PathBuf>> = BTreeMap::new();
    for f in files {
        grouped.entry(session_id_of(&f)).or_default().push(f);
    }
    grouped
}

/// Recursively collect session transcripts under `root`. Symlinks are skipped
/// (a loop or a link out of the store must never be followed).
fn session_files(root: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let Ok(ft) = entry.file_type() else {
                continue;
            };
            if ft.is_symlink() {
                continue;
            }
            let path = entry.path();
            if ft.is_dir() {
                stack.push(path);
            } else if ft.is_file()
                && path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(is_session_file)
            {
                files.push(path);
            }
        }
    }
    files
}

/// `<id>.jsonl`, plus the archived `<id>.jsonl.deleted.<ts>` and
/// `<id>.jsonl.reset.<ts>` copies OpenClaw leaves behind.
fn is_session_file(name: &str) -> bool {
    let Some(index) = name.find(".jsonl") else {
        return false;
    };
    let suffix = &name[index..];
    suffix == ".jsonl"
        || suffix.starts_with(".jsonl.deleted.")
        || suffix.starts_with(".jsonl.reset.")
}

/// Filename up to `.jsonl`, so archived copies share the live session's id.
fn session_id_of(path: &Path) -> String {
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown");
    match name.find(".jsonl") {
        Some(0) | None => name.to_string(),
        Some(index) => name[..index].to_string(),
    }
}

// ---------------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------------

/// One assistant message with usage.
#[derive(Debug, Clone)]
struct UsageEntry {
    ts: i64,
    model: String,
    counts: TokenCounts,
    cost: Option<f64>,
}

impl UsageEntry {
    /// Identity used to fold repeated records (a transcript and its archived
    /// copy, or a line duplicated by a crash-and-resume) into one.
    fn key(&self) -> String {
        format!(
            "{}:{}:{}:{}:{}:{}:{:?}",
            self.ts,
            self.model,
            self.counts.input,
            self.counts.output,
            self.counts.cache_write,
            self.counts.cache_read,
            self.cost
        )
    }
}

/// One user/assistant message, for the behavioural stats.
#[derive(Debug)]
struct Message {
    ts: i64,
    role: String,
    /// (tool name, arguments) per `toolCall` content item.
    tools: Vec<(String, serde_json::Value)>,
}

/// Everything one transcript file yields.
#[derive(Debug, Default)]
struct Transcript {
    cwd: Option<String>,
    started_at_ms: Option<i64>,
    entries: Vec<UsageEntry>,
    /// Every user/assistant message, in file order.
    messages: Vec<Message>,
}

fn read_transcript(path: &Path) -> Option<Transcript> {
    let content = std::fs::read_to_string(path).ok()?;
    let fallback_ts = file_modified_ms(path);
    let mut out = Transcript::default();
    let mut current_model: Option<String> = None;

    for line in content.lines() {
        let line = line.trim_end_matches('\r');
        if line.is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        let Some(obj) = v.as_object() else {
            continue;
        };
        let kind = obj.get("type").and_then(|t| t.as_str()).unwrap_or("");
        match kind {
            "session" => {
                out.cwd = non_empty_str(obj.get("cwd"));
                out.started_at_ms = obj.get("timestamp").and_then(jsonl::parse_ts_ms);
            }
            "model_change" => apply_model_change(&v, &mut current_model),
            "custom"
                if obj.get("customType").and_then(|c| c.as_str()) == Some("model-snapshot") =>
            {
                apply_model_change(&v, &mut current_model)
            }
            "message" => {
                // A non-object `message` is simply a line without usage.
                let Some(msg) = obj.get("message").and_then(|m| m.as_object()) else {
                    continue;
                };
                let role = msg.get("role").and_then(|r| r.as_str()).unwrap_or("");
                let ts = msg
                    .get("timestamp")
                    .and_then(jsonl::parse_ts_ms)
                    .or_else(|| obj.get("timestamp").and_then(jsonl::parse_ts_ms))
                    .unwrap_or(fallback_ts);

                if role == "user" || role == "assistant" {
                    let mut tools = Vec::new();
                    if role == "assistant"
                        && let Some(content) = msg.get("content").and_then(|c| c.as_array())
                    {
                        for item in content {
                            if item.get("type").and_then(|t| t.as_str()) == Some("toolCall")
                                && let Some(name) = item.get("name").and_then(|n| n.as_str())
                            {
                                let args = item
                                    .get("arguments")
                                    .cloned()
                                    .unwrap_or(serde_json::Value::Null);
                                tools.push((name.to_string(), args));
                            }
                        }
                    }
                    out.messages.push(Message {
                        ts,
                        role: role.to_string(),
                        tools,
                    });
                }

                if role != "assistant" {
                    continue;
                }
                let Some(usage) = msg.get("usage").filter(|u| u.is_object()) else {
                    continue;
                };
                let Some(counts) = usage_to_counts(usage) else {
                    continue;
                };
                let model = non_empty_str(msg.get("modelId"))
                    .or_else(|| non_empty_str(msg.get("model")))
                    .or_else(|| current_model.clone())
                    .unwrap_or_else(|| "unknown".to_string());
                let cost = usage
                    .get("cost")
                    .filter(|c| c.is_object())
                    .and_then(|c| c.get("total"))
                    .and_then(|t| t.as_f64());
                out.entries.push(UsageEntry {
                    ts,
                    model,
                    counts,
                    cost,
                });
            }
            _ => {}
        }
    }
    Some(out)
}

/// `model_change` carries the fields at the root; `model-snapshot` nests them
/// under `data`. `modelId` outranks `model`.
fn apply_model_change(v: &serde_json::Value, current: &mut Option<String>) {
    let source = match v.get("data") {
        Some(data) if data.is_object() => data,
        _ => v,
    };
    if let Some(model) =
        non_empty_str(source.get("modelId")).or_else(|| non_empty_str(source.get("model")))
    {
        *current = Some(model);
    }
}

/// Map an OpenClaw `usage` block onto our columns. `input` is already the
/// uncached part (cache reads are reported separately). When the parts are
/// missing but `totalTokens` is set, the remainder is booked as output, as
/// ccusage does. Returns `None` for an all-zero record.
fn usage_to_counts(usage: &serde_json::Value) -> Option<TokenCounts> {
    let get = |k: &str| usage.get(k).and_then(|x| x.as_u64()).unwrap_or(0);
    let mut counts = TokenCounts {
        input: get("input"),
        output: get("output"),
        cache_read: get("cacheRead"),
        cache_write: get("cacheWrite"),
        reasoning: 0,
    };
    let known = counts
        .input
        .saturating_add(counts.output)
        .saturating_add(counts.cache_read)
        .saturating_add(counts.cache_write);
    let missing = get("totalTokens").saturating_sub(known);
    // ccusage keeps a separate "extra total" column when output is already
    // set; we have no such column, so only the output fallback is applied.
    if missing > 0 && counts.output == 0 {
        counts.output = missing;
    }
    (known > 0 || missing > 0).then_some(counts)
}

fn non_empty_str(v: Option<&serde_json::Value>) -> Option<String> {
    let s = v?.as_str()?.trim();
    (!s.is_empty()).then(|| s.to_string())
}

fn file_modified_ms(path: &Path) -> i64 {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|m| m.duration_since(std::time::UNIX_EPOCH).ok())
        .and_then(|d| i64::try_from(d.as_millis()).ok())
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------
// Aggregation
// ---------------------------------------------------------------------------

/// Fold one session's transcript files into token counts. Repeated records
/// are counted once; first file (sorted order) wins.
fn counts_from_files(
    session_id: &str,
    files: &[PathBuf],
    since: Option<i64>,
) -> Option<SessionCounts> {
    let mut out = SessionCounts {
        agent: AgentId::OpenClaw,
        session_id: session_id.to_string(),
        model: None,
        cwd: None,
        started_at_ms: None,
        last_seen_at_ms: None,
        counts: TokenCounts::default(),
        cost: None,
        source_file: files.first().cloned(),
    };
    let mut seen = HashSet::new();
    let mut saw_usage = false;
    let mut saw_cost = false;
    let mut cost = 0.0f64;
    let mut latest: Option<(i64, String)> = None;

    for file in files {
        let Some(t) = read_transcript(file) else {
            continue;
        };
        if out.cwd.is_none() {
            out.cwd = t.cwd;
        }
        if let Some(s) = t.started_at_ms {
            out.started_at_ms = Some(out.started_at_ms.map_or(s, |cur| cur.min(s)));
        }
        for e in t.entries {
            if !seen.insert(e.key()) {
                continue;
            }
            out.last_seen_at_ms = Some(out.last_seen_at_ms.map_or(e.ts, |cur| cur.max(e.ts)));
            if out.started_at_ms.is_none_or(|s| e.ts < s) {
                out.started_at_ms = Some(e.ts);
            }
            if let Some(cutoff) = since
                && e.ts < cutoff
            {
                continue;
            }
            saw_usage = true;
            out.counts.input += e.counts.input;
            out.counts.output += e.counts.output;
            out.counts.cache_read += e.counts.cache_read;
            out.counts.cache_write += e.counts.cache_write;
            if let Some(c) = e.cost {
                saw_cost = true;
                cost += c;
            }
            if latest.as_ref().is_none_or(|(ts, _)| e.ts >= *ts) {
                latest = Some((e.ts, e.model));
            }
        }
    }

    if !saw_usage {
        // Nothing in the window (or an empty transcript in lifetime mode).
        return None;
    }
    out.model = latest.map(|(_, m)| m);
    out.cost = saw_cost.then_some(cost);
    Some(out)
}

/// Fold one session's transcript files into behavioural stats.
fn stats_from_files(
    session_id: &str,
    files: &[PathBuf],
    since: Option<i64>,
) -> Option<SessionStats> {
    let mut out = SessionStats::new(AgentId::OpenClaw, session_id.to_string());
    out.source_file = files.first().cloned();
    let mut saw_activity = false;

    for file in files {
        let Some(t) = read_transcript(file) else {
            continue;
        };
        if out.cwd.is_none() {
            out.cwd = t.cwd;
        }
        out.touch_ts(t.started_at_ms);
        for m in t.messages {
            if let Some(cutoff) = since
                && m.ts < cutoff
            {
                continue;
            }
            saw_activity = true;
            out.touch_ts(Some(m.ts));
            match m.role.as_str() {
                "user" => out.user_messages += 1,
                "assistant" => {
                    out.assistant_messages += 1;
                    for (name, args) in &m.tools {
                        out.record_tool(name, args);
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

/// Parse one transcript file into token counts. Public for fixture tests.
pub fn parse_counts(path: &Path, since: Option<i64>) -> Option<SessionCounts> {
    counts_from_files(
        &session_id_of(path),
        std::slice::from_ref(&path.to_path_buf()),
        since,
    )
}

/// Parse one transcript file into behavioural stats. Public for fixture tests.
pub fn parse_stats(path: &Path, since: Option<i64>) -> Option<SessionStats> {
    stats_from_files(
        &session_id_of(path),
        std::slice::from_ref(&path.to_path_buf()),
        since,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/openclaw")
            .join(name)
    }

    #[test]
    fn counts_parse_with_model_tracking_and_cost() {
        let s = parse_counts(&fixture("agents/main/sessions/abc.jsonl"), None).unwrap();
        assert_eq!(s.agent, AgentId::OpenClaw);
        assert_eq!(s.session_id, "abc");
        assert_eq!(s.cwd.as_deref(), Some("/tmp/proj"));
        // input stays uncached; cacheRead/cacheWrite land in their own columns
        assert_eq!(s.counts.input, 1660 + 200);
        assert_eq!(s.counts.output, 55 + 40);
        assert_eq!(s.counts.cache_read, 108_928 + 1000);
        assert_eq!(s.counts.cache_write, 500);
        assert_eq!(s.counts.reasoning, 0);
        // The message-level model outranks the model_change state, and the
        // latest message decides the session's model.
        assert_eq!(s.model.as_deref(), Some("claude-sonnet-4-5"));
        assert!((s.cost.unwrap() - 0.05).abs() < 1e-9);
        assert_eq!(s.started_at_ms, Some(1_769_753_900_000));
        assert_eq!(s.last_seen_at_ms, Some(1_769_753_960_000));
        assert!(s.source_file.unwrap().ends_with("abc.jsonl"));
    }

    #[test]
    fn model_snapshot_custom_event_sets_the_model() {
        let s = parse_counts(&fixture("agents/main/sessions/snapshot.jsonl"), None).unwrap();
        assert_eq!(s.model.as_deref(), Some("gpt-5.2"));
        assert_eq!(s.counts.input, 10);
        assert_eq!(s.counts.output, 20);
        assert!(s.cost.is_none(), "no cost in the log means no cost");
    }

    #[test]
    fn total_tokens_fallback_books_the_remainder_as_output() {
        let c = usage_to_counts(&serde_json::json!({"totalTokens": 222})).unwrap();
        assert_eq!((c.input, c.output), (0, 222));
        let c = usage_to_counts(
            &serde_json::json!({"input": 100, "cacheRead": 25, "totalTokens": 175}),
        )
        .unwrap();
        assert_eq!((c.input, c.output, c.cache_read), (100, 50, 25));
        // output already set: the remainder is ccusage's extra column, not ours
        let c = usage_to_counts(&serde_json::json!({"input": 1, "output": 1, "totalTokens": 9}))
            .unwrap();
        assert_eq!((c.input, c.output), (1, 1));
        assert!(usage_to_counts(&serde_json::json!({"input": 0, "output": 0})).is_none());
        assert!(usage_to_counts(&serde_json::json!({"input": -5, "output": 1.5})).is_none());
    }

    #[test]
    fn since_filters_messages_and_drops_empty_sessions() {
        let path = fixture("agents/main/sessions/abc.jsonl");
        // Window opens after the first assistant message.
        let s = parse_counts(&path, Some(1_769_753_950_000)).unwrap();
        assert_eq!(s.counts.input, 200);
        assert_eq!(s.counts.output, 40);
        assert_eq!(s.counts.cache_read, 1000);
        assert_eq!(s.counts.cache_write, 0);
        assert!((s.cost.unwrap() - 0.03).abs() < 1e-9);
        // Timestamps still describe the whole session.
        assert_eq!(s.started_at_ms, Some(1_769_753_900_000));
        assert_eq!(s.last_seen_at_ms, Some(1_769_753_960_000));
        // Window after everything: session dropped.
        assert!(parse_counts(&path, Some(1_769_800_000_000)).is_none());
        // Window at the exact timestamp is inclusive.
        let s = parse_counts(&path, Some(1_769_753_960_000)).unwrap();
        assert_eq!(s.counts.output, 40);
    }

    #[test]
    fn stats_count_messages_tools_and_skills() {
        let s = parse_stats(&fixture("agents/main/sessions/abc.jsonl"), None).unwrap();
        assert_eq!(s.session_id, "abc");
        assert_eq!(s.cwd.as_deref(), Some("/tmp/proj"));
        assert_eq!(s.user_messages, 2);
        assert_eq!(s.assistant_messages, 2);
        assert_eq!(s.tool_calls, 2);
        assert_eq!(s.tools.get("bash"), Some(&1));
        assert_eq!(s.tools.get("read"), Some(&1));
        assert_eq!(s.skills.get("drill-tdd"), Some(&1));
        assert_eq!(s.started_at_ms, Some(1_769_753_900_000));
        assert_eq!(s.last_seen_at_ms, Some(1_769_753_960_000));

        let s = parse_stats(
            &fixture("agents/main/sessions/abc.jsonl"),
            Some(1_769_753_950_000),
        )
        .unwrap();
        assert_eq!(s.user_messages, 1);
        assert_eq!(s.assistant_messages, 1);
        assert_eq!(s.tools.get("bash"), None);
        assert!(
            parse_stats(
                &fixture("agents/main/sessions/abc.jsonl"),
                Some(1_769_800_000_000)
            )
            .is_none()
        );
    }

    #[test]
    fn garbage_lines_are_skipped_and_foreign_shapes_do_not_panic() {
        let s = parse_counts(&fixture("agents/main/sessions/garbage.jsonl"), None).unwrap();
        // The model_change with a non-object `message` still applies, the
        // truncated/foreign lines are dropped, and the one good record counts.
        assert_eq!(s.model.as_deref(), Some("gpt-5.2"));
        assert_eq!(s.counts.input, 10);
        assert_eq!(s.counts.output, 20);
        let st = parse_stats(&fixture("agents/main/sessions/garbage.jsonl"), None).unwrap();
        // Both assistant-role lines are messages even though only one has
        // usable usage; the toolCall with a non-string name is not a tool.
        assert_eq!(st.assistant_messages, 2);
        assert_eq!(st.user_messages, 0);
        assert_eq!(st.tool_calls, 0);
        // A file with no usable record is not a session at all.
        assert!(parse_counts(&fixture("noise.jsonl"), None).is_none());
        assert!(parse_stats(&fixture("noise.jsonl"), None).is_none());
        assert!(parse_counts(Path::new("/nonexistent/openclaw.jsonl"), None).is_none());
    }

    #[test]
    fn repeated_records_are_counted_once() {
        let s = parse_counts(&fixture("agents/main/sessions/dup.jsonl"), None).unwrap();
        assert_eq!(s.counts.input, 1);
        assert_eq!(s.counts.output, 1);
    }

    #[test]
    fn archived_copies_fold_into_the_live_session() {
        let root = fixture("");
        let files = session_files(&root);
        let mut names: Vec<_> = files
            .iter()
            .map(|f| f.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        names.sort();
        assert!(names.contains(&"abc.jsonl".to_string()));
        assert!(names.contains(&"abc.jsonl.deleted.1769753970000".to_string()));
        assert!(names.contains(&"abc.jsonl.reset.2026-03-20T06-34-44.520Z".to_string()));
        assert!(!names.iter().any(|n| n.ends_with(".json")));

        let mut abc: Vec<_> = files
            .iter()
            .filter(|f| session_id_of(f) == "abc")
            .cloned()
            .collect();
        abc.sort();
        assert_eq!(abc.len(), 3);
        let s = counts_from_files("abc", &abc, None).unwrap();
        // The deleted copy duplicates the live file (counted once); the reset
        // copy holds one older, distinct message that still counts.
        assert_eq!(s.counts.input, 1660 + 200 + 7);
        assert_eq!(s.counts.output, 55 + 40 + 3);
        assert_eq!(s.started_at_ms, Some(1_769_753_800_000));
        assert_eq!(s.last_seen_at_ms, Some(1_769_753_960_000));
    }

    #[test]
    fn session_file_names_and_ids() {
        assert!(is_session_file("a.jsonl"));
        assert!(is_session_file("a.jsonl.deleted.1700000000000"));
        assert!(is_session_file("a.jsonl.reset.2026-03-20T06-34-44.520Z"));
        assert!(!is_session_file("a.json"));
        assert!(!is_session_file("a.jsonl.bak"));
        assert_eq!(session_id_of(Path::new("/x/abc.jsonl")), "abc");
        assert_eq!(session_id_of(Path::new("/x/abc.jsonl.deleted.170")), "abc");
        assert_eq!(session_id_of(Path::new("/x/.jsonl")), ".jsonl");
    }

    #[test]
    fn legacy_home_dirs_are_discovered() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        assert!(data_dirs_from(home, None).is_empty());

        for legacy in [".clawdbot", ".moltbot", ".moldbot"] {
            std::fs::create_dir_all(home.join(legacy).join("agents/main/sessions")).unwrap();
        }
        std::fs::write(
            home.join(".moltbot/agents/main/sessions/old.jsonl.deleted.1700000000000"),
            r#"{"type":"message","message":{"role":"assistant","model":"gpt-5.2","usage":{"input":3,"output":4},"timestamp":1769753935279}}"#,
        )
        .unwrap();
        // A regular file named like a root is not a root.
        std::fs::write(home.join(".openclaw"), "not a dir").unwrap();

        let dirs = data_dirs_from(home, None);
        assert_eq!(dirs.len(), 3);
        assert!(dirs.iter().all(|d| d.is_dir()));
        assert!(!dirs.iter().any(|d| d.ends_with(".openclaw")));

        let files: Vec<_> = dirs.iter().flat_map(|d| session_files(d)).collect();
        assert_eq!(files.len(), 1);
        let s = parse_counts(&files[0], None).unwrap();
        assert_eq!(s.session_id, "old");
        assert_eq!((s.counts.input, s.counts.output), (3, 4));
    }

    #[test]
    fn openclaw_dir_override_is_a_comma_separated_list_of_existing_dirs() {
        let tmp = tempfile::tempdir().unwrap();
        let a = tmp.path().join("a");
        let b = tmp.path().join("b");
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        let raw = format!(
            " {} , {} ,{},/definitely/not/here,",
            a.display(),
            b.display(),
            a.display()
        );
        let dirs = data_dirs_from(tmp.path(), Some(&raw));
        assert_eq!(dirs, vec![a, b]);
        // An override that names nothing real yields no roots — and does NOT
        // fall back to the home-dir defaults.
        std::fs::create_dir_all(tmp.path().join(".openclaw")).unwrap();
        assert!(data_dirs_from(tmp.path(), Some("/definitely/not/here")).is_empty());
        // Blank override means "not set".
        assert_eq!(data_dirs_from(tmp.path(), Some("  ")).len(), 1);
    }
}
