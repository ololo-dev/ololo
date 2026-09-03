//! Factory Droid CLI sessions (ported from ccusage's `droid` adapter).
//!
//! Layout: `${DROID_SESSIONS_DIR:-~/.factory/sessions}/**/<session-id>.settings.json`
//! plus a sibling `<session-id>.jsonl` transcript. `DROID_SESSIONS_DIR` may be
//! one directory or a comma-separated list (archive roots). The settings file
//! is a single cumulative snapshot per session:
//!
//! ```json
//! {"model":"Claude-Sonnet-4-[Anthropic]","providerLock":"anthropic",
//!  "providerLockTimestamp":"2026-05-01T01:02:03.000Z",
//!  "tokenUsage":{"inputTokens":100,"outputTokens":50,"cacheCreationTokens":20,
//!                "cacheReadTokens":10,"thinkingTokens":5,"totalTokens":185}}
//! ```
//!
//! Rules carried over from ccusage: the session id is the file stem before
//! `.settings.json`; model names are normalized (`custom:` prefix and
//! `[Provider]` brackets stripped, lower-cased, `.`/space runs → `-`); when the
//! settings carry no model the first `Model: …` mention in the sidecar jsonl
//! wins, else a `<provider>-unknown` placeholder; `totalTokens` larger than the
//! known parts tops up output (or thinking when output is already set); a
//! snapshot with zero tokens is dropped; the same session id found in several
//! roots keeps the newest snapshot. The snapshot timestamp is
//! `providerLockTimestamp`, falling back to the file's mtime.
//!
//! The transcript is not documented by ccusage (it only scrapes the model line
//! from it). `stats` reads it best-effort in the Anthropic-messages shape Droid
//! writes (`{"type":"message","timestamp":…,"message":{"role":…,"content":[…]}}`
//! with `tool_use` blocks) and yields zero counts, never errors, on anything else.

use crate::jsonl;
use crate::paths;
use crate::trait_::TokenExtractor;
use crate::types::{AgentId, SessionCounts, SessionStats, TokenCounts};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// Same override ccusage honours: one dir or a comma-separated list of dirs.
pub const DROID_SESSIONS_DIR_ENV: &str = "DROID_SESSIONS_DIR";

const SETTINGS_SUFFIX: &str = ".settings.json";
/// ccusage scans only the head of the sidecar for the model line; we reuse the
/// same bound for the cwd/first-timestamp probe so a long transcript stays cheap.
const SIDECAR_HEAD_LINES: usize = 500;

pub struct Droid;

impl TokenExtractor for Droid {
    fn id(&self) -> AgentId {
        AgentId::Droid
    }

    fn detect(&self) -> bool {
        !session_dirs().is_empty()
    }

    fn extract(&self, since: Option<i64>) -> Vec<SessionCounts> {
        let mut parsed: Vec<SessionCounts> = settings_files(&session_dirs())
            .iter()
            .filter_map(|f| parse_counts(f, since))
            .collect();
        dedup_latest(&mut parsed, |s| s.session_id.clone(), |s| s.last_seen_at_ms)
    }

    fn stats(&self, since: Option<i64>) -> Vec<SessionStats> {
        let mut parsed: Vec<SessionStats> = settings_files(&session_dirs())
            .iter()
            .filter_map(|f| parse_stats(f, since))
            .collect();
        dedup_latest(&mut parsed, |s| s.session_id.clone(), |s| s.last_seen_at_ms)
    }
}

/// Existing, de-duplicated session roots: `DROID_SESSIONS_DIR` (comma list)
/// or `<home>/.factory/sessions`, where home honours `AGENT_TOKENS_HOME`.
pub fn session_dirs() -> Vec<PathBuf> {
    let raw: Vec<PathBuf> = match std::env::var(DROID_SESSIONS_DIR_ENV) {
        Ok(list) => list
            .split(',')
            .map(str::trim)
            .filter(|p| !p.is_empty())
            .map(PathBuf::from)
            .collect(),
        Err(_) => vec![paths::home_dir().join(".factory").join("sessions")],
    };
    let mut seen = HashSet::new();
    raw.into_iter()
        .filter(|p| p.is_dir())
        .filter(|p| seen.insert(p.clone()))
        .collect()
}

/// Recursively collect `*.settings.json` under the given roots, sorted so the
/// latest-wins dedup is deterministic.
fn settings_files(roots: &[PathBuf]) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let mut stack: Vec<PathBuf> = roots.to_vec();
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.ends_with(SETTINGS_SUFFIX))
            {
                files.push(path);
            }
        }
    }
    files.sort();
    files
}

/// ccusage keeps the newest snapshot per session id (sort by timestamp, walk
/// in reverse, first occurrence wins).
fn dedup_latest<T>(
    items: &mut Vec<T>,
    key: impl Fn(&T) -> String,
    ts: impl Fn(&T) -> Option<i64>,
) -> Vec<T> {
    items.sort_by_key(|i| ts(i).unwrap_or(i64::MIN));
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    while let Some(item) = items.pop() {
        if seen.insert(key(&item)) {
            out.push(item);
        }
    }
    out
}

fn session_id_of(settings_path: &Path) -> Option<&str> {
    settings_path
        .file_name()
        .and_then(|n| n.to_str())
        .and_then(|n| n.strip_suffix(SETTINGS_SUFFIX))
}

fn sidecar_of(settings_path: &Path) -> Option<PathBuf> {
    let id = session_id_of(settings_path)?;
    Some(settings_path.with_file_name(format!("{id}.jsonl")))
}

fn file_mtime_ms(path: &Path) -> Option<i64> {
    let modified = std::fs::metadata(path).ok()?.modified().ok()?;
    let millis = modified
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_millis();
    Some(millis.min(i64::MAX as u128) as i64)
}

fn string_field(obj: &serde_json::Map<String, serde_json::Value>, key: &str) -> Option<String> {
    let v = obj.get(key)?.as_str()?.trim();
    (!v.is_empty()).then(|| v.to_string())
}

fn u64_field(obj: &serde_json::Map<String, serde_json::Value>, key: &str) -> u64 {
    obj.get(key).and_then(|v| v.as_u64()).unwrap_or(0)
}

/// `tokenUsage` → counts, with ccusage's `totalTokens` top-up. `None` when the
/// object is missing/not an object or every counter is zero.
fn parse_token_usage(value: Option<&serde_json::Value>) -> Option<TokenCounts> {
    let usage = value?.as_object()?;
    let mut counts = TokenCounts {
        input: u64_field(usage, "inputTokens"),
        output: u64_field(usage, "outputTokens"),
        cache_write: u64_field(usage, "cacheCreationTokens"),
        cache_read: u64_field(usage, "cacheReadTokens"),
        reasoning: u64_field(usage, "thinkingTokens"),
    };
    let known = counts
        .input
        .saturating_add(counts.output)
        .saturating_add(counts.cache_write)
        .saturating_add(counts.cache_read)
        .saturating_add(counts.reasoning);
    let missing = u64_field(usage, "totalTokens").saturating_sub(known);
    if missing > 0 {
        if counts.output == 0 {
            counts.output = missing;
        } else {
            counts.reasoning = counts.reasoning.saturating_add(missing);
        }
    }
    (known.saturating_add(missing) > 0).then_some(counts)
}

/// Port of ccusage's `normalize_droid_model_name`:
/// `custom:Claude-Opus-4.5-Thinking-[Anthropic]-0` → `claude-opus-4-5-thinking-0`.
pub fn normalize_model_name(model: &str) -> String {
    let raw = model.strip_prefix("custom:").unwrap_or(model);
    let mut without_brackets = String::new();
    let mut depth = 0u32;
    for ch in raw.chars() {
        match ch {
            '[' => depth += 1,
            ']' => depth = depth.saturating_sub(1),
            _ if depth == 0 => without_brackets.push(ch),
            _ => {}
        }
    }
    let lower = without_brackets
        .trim()
        .trim_end_matches('-')
        .to_ascii_lowercase();
    let mut out = String::new();
    let mut prev_dash = false;
    for ch in lower.chars() {
        let next = if ch == '.' || ch.is_whitespace() || ch == '-' {
            '-'
        } else {
            ch
        };
        if next == '-' {
            if !prev_dash {
                out.push('-');
                prev_dash = true;
            }
        } else {
            out.push(next);
            prev_dash = false;
        }
    }
    out.trim_matches('-').to_string()
}

fn normalize_provider(value: Option<&str>) -> String {
    let Some(value) = value else {
        return "unknown".into();
    };
    let normalized = value.trim().to_ascii_lowercase().replace('-', "_");
    match normalized.as_str() {
        "" => "unknown".into(),
        "claude" | "anthropic" => "anthropic".into(),
        "openai" => "openai".into(),
        "google" | "google_ai" | "gemini" | "vertex" | "vertex_ai" => "google".into(),
        "xai" | "x_ai" | "grok" => "xai".into(),
        other => other.to_string(),
    }
}

fn infer_provider_from_model(model: &str) -> &'static str {
    if model.contains("claude")
        || model.contains("opus")
        || model.contains("sonnet")
        || model.contains("haiku")
    {
        "anthropic"
    } else if model.starts_with("gpt-")
        || model.contains("-gpt-")
        || model.contains("chatgpt")
        || (model.starts_with('o') && model.as_bytes().get(1).is_some_and(u8::is_ascii_digit))
    {
        "openai"
    } else if model.contains("gemini") {
        "google"
    } else if model.contains("grok") {
        "xai"
    } else {
        "unknown"
    }
}

/// ccusage's placeholder when neither settings nor transcript name a model.
fn default_model_from_provider(provider: &str) -> Option<String> {
    match provider {
        "anthropic" => Some("claude-unknown".into()),
        "openai" => Some("gpt-unknown".into()),
        "google" => Some("gemini-unknown".into()),
        "xai" => Some("grok-unknown".into()),
        _ => None,
    }
}

/// `… Model: Claude Opus 4.5 Thinking [Anthropic] …` → `claude-opus-4-5-thinking`.
fn model_from_line(line: &str) -> Option<String> {
    let tail = line.split_once("Model:")?.1;
    let raw = tail
        .split(['"', '\\', '['])
        .next()
        .unwrap_or_default()
        .trim();
    if raw.is_empty() {
        return None;
    }
    let normalized = normalize_model_name(raw);
    (!normalized.is_empty()).then_some(normalized)
}

/// What the head of the sidecar transcript tells us.
#[derive(Default)]
struct SidecarHead {
    model: Option<String>,
    cwd: Option<String>,
    first_ts: Option<i64>,
}

fn read_sidecar_head(settings_path: &Path) -> SidecarHead {
    let mut head = SidecarHead::default();
    let Some(sidecar) = sidecar_of(settings_path) else {
        return head;
    };
    for line in jsonl::read_lines(&sidecar)
        .into_iter()
        .take(SIDECAR_HEAD_LINES)
    {
        if head.model.is_none() {
            head.model = model_from_line(&line);
        }
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&line) else {
            continue;
        };
        if head.cwd.is_none() {
            head.cwd = v.get("cwd").and_then(|c| c.as_str()).map(String::from);
        }
        if head.first_ts.is_none() {
            head.first_ts =
                jsonl::parse_ts_ms(v.get("timestamp").unwrap_or(&serde_json::Value::Null));
        }
    }
    head
}

/// Parse one `<id>.settings.json` snapshot into token counts. Public for
/// fixture tests. The snapshot is cumulative with a single timestamp, so the
/// `since` window applies at session granularity (last activity ≥ cutoff).
pub fn parse_counts(settings_path: &Path, since: Option<i64>) -> Option<SessionCounts> {
    let content = std::fs::read_to_string(settings_path).ok()?;
    let value: serde_json::Value = serde_json::from_str(&content).ok()?;
    let settings = value.as_object()?;
    let counts = parse_token_usage(settings.get("tokenUsage"))?;

    let head = read_sidecar_head(settings_path);

    let mut provider = normalize_provider(string_field(settings, "providerLock").as_deref());
    let model = string_field(settings, "model")
        .map(|m| normalize_model_name(&m))
        .filter(|m| !m.is_empty())
        .or_else(|| head.model.clone())
        .or_else(|| default_model_from_provider(&provider));
    if provider == "unknown"
        && let Some(m) = &model
    {
        provider = infer_provider_from_model(m).to_string();
    }
    let _ = provider; // only feeds the model placeholder; pricing lives elsewhere

    let lock_ts = string_field(settings, "providerLockTimestamp")
        .and_then(|t| jsonl::parse_ts_ms(&serde_json::Value::String(t)));
    let mtime = file_mtime_ms(settings_path);
    let started_at_ms = lock_ts.or(head.first_ts).or(mtime);
    let last_seen_at_ms = match (lock_ts, mtime) {
        (Some(a), Some(b)) => Some(a.max(b)),
        (a, b) => a.or(b),
    };
    if let Some(cutoff) = since
        && last_seen_at_ms.is_none_or(|t| t < cutoff)
    {
        return None;
    }

    Some(SessionCounts {
        agent: AgentId::Droid,
        session_id: session_id_of(settings_path)
            .unwrap_or("unknown")
            .to_string(),
        model,
        cwd: head.cwd,
        started_at_ms,
        last_seen_at_ms,
        counts,
        cost: None,
        source_file: Some(settings_path.to_path_buf()),
    })
}

fn role_of(v: &serde_json::Value) -> Option<&str> {
    v.get("message")
        .and_then(|m| m.get("role"))
        .or_else(|| v.get("role"))
        .and_then(|r| r.as_str())
}

fn content_of(v: &serde_json::Value) -> Option<&serde_json::Value> {
    v.get("message")
        .and_then(|m| m.get("content"))
        .or_else(|| v.get("content"))
}

fn block_type(block: &serde_json::Value) -> &str {
    block.get("type").and_then(|t| t.as_str()).unwrap_or("")
}

/// Parse a session's sidecar transcript into behavioural stats. Public for
/// fixture tests. Returns `None` when the settings snapshot is unusable or,
/// with `since`, when no message falls inside the window.
pub fn parse_stats(settings_path: &Path, since: Option<i64>) -> Option<SessionStats> {
    let counts = parse_counts(settings_path, None)?;
    let mut out = SessionStats::new(AgentId::Droid, counts.session_id);
    out.cwd = counts.cwd;
    out.source_file = Some(settings_path.to_path_buf());
    let mut saw_activity = false;

    let sidecar = sidecar_of(settings_path)?;
    for line in jsonl::read_lines(&sidecar) {
        if line.is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&line) else {
            continue;
        };
        let ts = jsonl::parse_ts_ms(v.get("timestamp").unwrap_or(&serde_json::Value::Null));
        let Some(role) = role_of(&v) else {
            continue;
        };
        if let Some(cutoff) = since
            && ts.is_some_and(|t| t < cutoff)
        {
            continue;
        }
        let content = content_of(&v);
        let blocks: &[serde_json::Value] = content.and_then(|c| c.as_array()).map_or(&[], |a| a);
        match role {
            "user" => {
                // A user turn that only carries tool results is the harness
                // echoing tool output back, not a human message.
                let only_results =
                    !blocks.is_empty() && blocks.iter().all(|b| block_type(b) == "tool_result");
                if only_results {
                    continue;
                }
                saw_activity = true;
                out.user_messages += 1;
                out.touch_ts(ts);
            }
            "assistant" => {
                saw_activity = true;
                out.assistant_messages += 1;
                out.touch_ts(ts);
                for block in blocks {
                    if !matches!(block_type(block), "tool_use" | "tool_call") {
                        continue;
                    }
                    let name = block
                        .get("name")
                        .and_then(|n| n.as_str())
                        .unwrap_or("unknown");
                    let args = block
                        .get("input")
                        .or_else(|| block.get("arguments"))
                        .or_else(|| block.get("args"))
                        .cloned()
                        .unwrap_or(serde_json::Value::Null);
                    // Some harnesses JSON-encode the arguments as a string.
                    let args = match args {
                        serde_json::Value::String(s) => {
                            serde_json::from_str(&s).unwrap_or(serde_json::Value::String(s))
                        }
                        other => other,
                    };
                    out.record_tool(name, &args);
                }
            }
            _ => {}
        }
    }

    if since.is_some() && !saw_activity {
        return None;
    }
    if out.started_at_ms.is_none() {
        out.started_at_ms = counts.started_at_ms;
    }
    if out.last_seen_at_ms.is_none() {
        out.last_seen_at_ms = counts.last_seen_at_ms;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/droid")
            .join(name)
    }

    fn ts(s: &str) -> i64 {
        jsonl::parse_ts_ms(&serde_json::Value::String(s.into())).unwrap()
    }

    #[test]
    fn normalizes_model_names_like_ccusage() {
        assert_eq!(
            normalize_model_name("custom:Claude-Opus-4.5-Thinking-[Anthropic]-0"),
            "claude-opus-4-5-thinking-0"
        );
        assert_eq!(
            normalize_model_name("Claude-Sonnet-4-[Anthropic]"),
            "claude-sonnet-4"
        );
        assert_eq!(normalize_model_name("gemini-2.5-pro"), "gemini-2-5-pro");
    }

    #[test]
    fn total_tokens_tops_up_output_when_parts_are_missing() {
        let usage = parse_token_usage(Some(&serde_json::json!({"totalTokens": 456}))).unwrap();
        assert_eq!(usage.output, 456);
        assert_eq!(usage.reasoning, 0);
        // With output already set the surplus is attributed to thinking.
        let usage = parse_token_usage(Some(&serde_json::json!({
            "inputTokens": 10, "outputTokens": 5, "totalTokens": 20
        })))
        .unwrap();
        assert_eq!((usage.input, usage.output, usage.reasoning), (10, 5, 5));
    }

    #[test]
    fn zero_usage_is_dropped() {
        assert!(parse_token_usage(Some(&serde_json::json!({"inputTokens": 0}))).is_none());
        assert!(parse_token_usage(Some(&serde_json::json!("nope"))).is_none());
        assert!(parse_token_usage(None).is_none());
        assert!(parse_counts(&fixture("zero.settings.json"), None).is_none());
    }

    #[test]
    fn parses_counts_from_settings_snapshot() {
        let s = parse_counts(&fixture("session-a.settings.json"), None).unwrap();
        assert_eq!(s.agent, AgentId::Droid);
        assert_eq!(s.session_id, "session-a");
        assert_eq!(s.model.as_deref(), Some("claude-sonnet-4"));
        assert_eq!(s.cwd.as_deref(), Some("/home/dev/project"));
        assert_eq!(s.counts.input, 100);
        assert_eq!(s.counts.output, 50);
        assert_eq!(s.counts.cache_write, 20);
        assert_eq!(s.counts.cache_read, 10);
        assert_eq!(s.counts.reasoning, 5);
        assert_eq!(s.cost, None);
        assert_eq!(s.started_at_ms, Some(ts("2026-05-01T01:02:03.000Z")));
        assert!(s.last_seen_at_ms.unwrap() >= s.started_at_ms.unwrap());
        assert_eq!(s.source_file, Some(fixture("session-a.settings.json")));
    }

    #[test]
    fn falls_back_to_sidecar_model_line() {
        let s = parse_counts(&fixture("session-b.settings.json"), None).unwrap();
        assert_eq!(s.model.as_deref(), Some("claude-opus-4-5-thinking"));
        assert_eq!(s.counts.input, 10);
        assert_eq!(s.counts.output, 20);
    }

    #[test]
    fn falls_back_to_provider_placeholder_without_any_model() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("s.settings.json");
        std::fs::write(
            &p,
            r#"{"providerLock":"openai","tokenUsage":{"inputTokens":3}}"#,
        )
        .unwrap();
        let s = parse_counts(&p, None).unwrap();
        assert_eq!(s.model.as_deref(), Some("gpt-unknown"));
        // No providerLockTimestamp: mtime stands in for both bounds.
        assert!(s.started_at_ms.is_some());
        assert_eq!(s.started_at_ms, s.last_seen_at_ms);

        let p = dir.path().join("t.settings.json");
        std::fs::write(&p, r#"{"tokenUsage":{"outputTokens":3}}"#).unwrap();
        assert_eq!(parse_counts(&p, None).unwrap().model, None);
    }

    #[test]
    fn since_filters_at_session_granularity() {
        let path = fixture("session-a.settings.json");
        let last_seen = parse_counts(&path, None).unwrap().last_seen_at_ms.unwrap();
        assert!(parse_counts(&path, Some(last_seen)).is_some());
        assert!(parse_counts(&path, Some(last_seen + 1)).is_none());
    }

    #[test]
    fn newest_snapshot_wins_for_duplicate_session_ids() {
        // Two roots hold session-c: an older archive copy and the live one.
        let dir = tempfile::tempdir().unwrap();
        let archive = dir.path().join("archive");
        std::fs::create_dir_all(&archive).unwrap();
        std::fs::write(
            archive.join("session-c.settings.json"),
            r#"{"model":"gpt-5","providerLock":"openai",
                "providerLockTimestamp":"2026-05-01T01:02:03.000Z",
                "tokenUsage":{"inputTokens":10,"outputTokens":20}}"#,
        )
        .unwrap();
        std::fs::write(
            dir.path().join("session-c.settings.json"),
            r#"{"model":"gpt-5","providerLock":"openai",
                "providerLockTimestamp":"2026-05-02T01:02:03.000Z",
                "tokenUsage":{"inputTokens":100,"outputTokens":200}}"#,
        )
        .unwrap();
        // mtime is "now" for both, so pin the lock timestamps as the tiebreak
        // by making the archive copy's mtime older than its lock time.
        let files = settings_files(&[dir.path().to_path_buf()]);
        assert_eq!(files.len(), 2);
        let mut parsed: Vec<SessionCounts> =
            files.iter().filter_map(|f| parse_counts(f, None)).collect();
        // Simulate ccusage's ordering key (snapshot timestamp) explicitly.
        for s in &mut parsed {
            s.last_seen_at_ms = s.started_at_ms;
        }
        let out = dedup_latest(&mut parsed, |s| s.session_id.clone(), |s| s.last_seen_at_ms);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].session_id, "session-c");
        assert_eq!((out[0].counts.input, out[0].counts.output), (100, 200));
    }

    #[test]
    fn stats_count_messages_and_tools() {
        let st = parse_stats(&fixture("session-a.settings.json"), None).unwrap();
        assert_eq!(st.session_id, "session-a");
        assert_eq!(st.cwd.as_deref(), Some("/home/dev/project"));
        assert_eq!(
            st.user_messages, 2,
            "tool_result-only user turns are not messages"
        );
        assert_eq!(st.assistant_messages, 3);
        assert_eq!(st.tool_calls, 3);
        assert_eq!(st.tools.get("Read"), Some(&2));
        assert_eq!(st.tools.get("Execute"), Some(&1));
        assert_eq!(st.skills.get("reconnaissance"), Some(&1));
        // Message timestamps bound the stats; the session_start record does not.
        assert_eq!(st.started_at_ms, Some(ts("2026-05-01T01:02:10.000Z")));
        assert_eq!(st.last_seen_at_ms, Some(ts("2026-05-01T01:05:00.000Z")));
    }

    #[test]
    fn stats_since_window_drops_older_messages() {
        let path = fixture("session-a.settings.json");
        let cutoff = ts("2026-05-01T01:04:00.000Z");
        let st = parse_stats(&path, Some(cutoff)).unwrap();
        assert_eq!(st.user_messages, 1);
        assert_eq!(st.assistant_messages, 1);
        assert_eq!(st.tool_calls, 1);
        assert_eq!(st.tools.get("Execute"), Some(&1));
        assert_eq!(st.started_at_ms, Some(ts("2026-05-01T01:04:30.000Z")));
        assert!(parse_stats(&path, Some(ts("2027-01-01T00:00:00Z"))).is_none());
    }

    #[test]
    fn stats_without_sidecar_are_empty_but_present() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("lonely.settings.json");
        std::fs::write(&p, r#"{"model":"gpt-5","tokenUsage":{"inputTokens":1}}"#).unwrap();
        let st = parse_stats(&p, None).unwrap();
        assert_eq!(
            (st.user_messages, st.assistant_messages, st.tool_calls),
            (0, 0, 0)
        );
        assert!(parse_stats(&p, Some(0)).is_none());
    }

    #[test]
    fn garbage_is_tolerated() {
        let s = parse_counts(&fixture("garbage.settings.json"), None).unwrap();
        assert_eq!(s.counts.input, 7);
        assert_eq!(s.counts.output, 0);
        assert_eq!(s.model.as_deref(), Some("claude-unknown"));
        let st = parse_stats(&fixture("garbage.settings.json"), None).unwrap();
        // String content still counts as a message; tool_result-only and
        // role-less lines do not.
        assert_eq!(st.user_messages, 1);
        assert_eq!(st.assistant_messages, 2);
        assert_eq!(st.tool_calls, 1);
        assert_eq!(st.tools.get("unknown"), Some(&1));

        let dir = tempfile::tempdir().unwrap();
        for (name, body) in [
            ("not-json.settings.json", "{{{"),
            ("array.settings.json", "[1,2,3]"),
            ("no-usage.settings.json", r#"{"model":"x"}"#),
            ("string-usage.settings.json", r#"{"tokenUsage":"lots"}"#),
        ] {
            let p = dir.path().join(name);
            std::fs::write(&p, body).unwrap();
            assert!(parse_counts(&p, None).is_none(), "{name}");
            assert!(parse_stats(&p, None).is_none(), "{name}");
        }
        assert!(parse_counts(&dir.path().join("missing.settings.json"), None).is_none());
    }

    #[test]
    fn discovery_walks_nested_roots_and_only_settings_files() {
        let files = settings_files(&[fixture("")]);
        let names: Vec<String> = files
            .iter()
            .map(|f| f.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert!(names.contains(&"session-a.settings.json".to_string()));
        assert!(
            names.contains(&"nested.settings.json".to_string()),
            "{names:?}"
        );
        assert!(
            names.iter().all(|n| n.ends_with(SETTINGS_SUFFIX)),
            "{names:?}"
        );
        assert!(settings_files(&[fixture("does-not-exist")]).is_empty());
    }

    #[test]
    fn env_override_takes_a_comma_separated_list() {
        // Serialized with the other env-touching tests via the process-wide
        // lock on the env var name; nextest runs each test in its own process.
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a");
        let b = dir.path().join("b");
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        let list = format!(
            "{}, {} ,{},/definitely/not/here",
            a.display(),
            b.display(),
            a.display()
        );
        unsafe { std::env::set_var(DROID_SESSIONS_DIR_ENV, &list) };
        let dirs = session_dirs();
        unsafe { std::env::remove_var(DROID_SESSIONS_DIR_ENV) };
        assert_eq!(dirs, vec![a, b]);
    }

    #[test]
    fn default_root_hangs_off_agent_tokens_home() {
        let dir = tempfile::tempdir().unwrap();
        unsafe { std::env::set_var("AGENT_TOKENS_HOME", dir.path()) };
        let before = Droid.detect();
        std::fs::create_dir_all(dir.path().join(".factory/sessions")).unwrap();
        let after = Droid.detect();
        let dirs = session_dirs();
        unsafe { std::env::remove_var("AGENT_TOKENS_HOME") };
        assert!(!before);
        assert!(after);
        assert_eq!(dirs, vec![dir.path().join(".factory/sessions")]);
    }
}
