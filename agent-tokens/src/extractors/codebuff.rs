//! Codebuff CLI chats (ported from ccusage's codebuff adapter, MIT).
//!
//! Layout: `~/.config/<channel>/projects/<project>/chats/<chat-id>/chat-messages.json`
//! where `<channel>` is one of `manicode`, `manicode-dev`, `manicode-staging`
//! and `<chat-id>` is an ISO timestamp with `-` in place of `:`
//! (`2026-01-02T03-04-05.000Z`). `CODEBUFF_DATA_DIR` (comma-separated; each
//! entry is either a channel dir or its `projects/` dir) replaces the default
//! channel roots. A session is one chat file, identified as
//! `<channel>/<project>/<chat-id>`.
//!
//! Each file is a JSON array of messages. A message is an assistant turn when
//! `variant` (else `role`) is `ai` / `agent` / `assistant`. Usage lives in
//! `metadata.model` + `metadata.usage` (Anthropic-style `inputTokens`,
//! `outputTokens`, `cacheCreationInputTokens`, `cacheReadInputTokens`), with
//! fallbacks to `metadata.codebuff.usage` and to the last assistant entry of
//! `metadata.runState.sessionState.mainAgentState.messageHistory[]`, whose
//! `providerOptions.codebuff.{model,usage}` uses OpenAI-style `prompt_tokens` /
//! `completion_tokens` / `prompt_tokens_details.cached_tokens`. A bare
//! `totalTokens` is attributed to output. `credits` is Codebuff credit spend,
//! not USD. Timestamps: `timestamp` | `createdAt` | `metadata.timestamp`
//! (ISO string, or seconds/ms number) → chat-id timestamp → file mtime.
//! Messages sharing an `id` are the same turn re-emitted: last one wins.

use crate::jsonl;
use crate::paths;
use crate::trait_::TokenExtractor;
use crate::types::{AgentId, SessionCounts, SessionStats, TokenCounts};
use serde_json::{Map, Value};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

/// Same override ccusage honours: comma-separated list of channel dirs (or
/// their `projects/` subdirs) that replaces the default `~/.config/<channel>`.
pub const CODEBUFF_DATA_DIR_ENV: &str = "CODEBUFF_DATA_DIR";
const CHANNELS: &[&str] = &["manicode", "manicode-dev", "manicode-staging"];
const CHAT_FILE: &str = "chat-messages.json";

pub struct Codebuff;

impl TokenExtractor for Codebuff {
    fn id(&self) -> AgentId {
        AgentId::Codebuff
    }

    fn detect(&self) -> bool {
        !project_roots().is_empty()
    }

    fn extract(&self, since: Option<i64>) -> Vec<SessionCounts> {
        chat_files(&project_roots())
            .iter()
            .filter_map(|f| parse_counts(f, since))
            .collect()
    }

    fn stats(&self, since: Option<i64>) -> Vec<SessionStats> {
        chat_files(&project_roots())
            .iter()
            .filter_map(|f| parse_stats(f, since))
            .collect()
    }
}

/// The `projects/` roots to scan, resolved from the environment and
/// `crate::paths::home_dir()` (so `AGENT_TOKENS_HOME` redirects the default).
fn project_roots() -> Vec<PathBuf> {
    let data_dirs = std::env::var(CODEBUFF_DATA_DIR_ENV).ok();
    project_roots_from(data_dirs.as_deref(), &paths::home_dir())
}

/// Resolve the existing `projects/` roots from an explicit `CODEBUFF_DATA_DIR`
/// value (comma-separated) or, when absent, the default channel dirs under
/// `home`. Public so the discovery rule is testable without touching the env.
pub fn project_roots_from(data_dirs: Option<&str>, home: &Path) -> Vec<PathBuf> {
    let candidates: Vec<PathBuf> = match data_dirs {
        Some(list) => list
            .split(',')
            .map(str::trim)
            .filter(|p| !p.is_empty())
            .map(PathBuf::from)
            .collect(),
        None => CHANNELS
            .iter()
            .map(|c| home.join(".config").join(c))
            .collect(),
    };
    let mut seen = HashSet::new();
    let mut roots = Vec::new();
    for root in candidates {
        let root = if root.file_name().is_some_and(|n| n == "projects") {
            root
        } else {
            root.join("projects")
        };
        if root.is_dir() && seen.insert(root.clone()) {
            roots.push(root);
        }
    }
    roots
}

/// Recursively collect every `chat-messages.json` under the given roots.
fn chat_files(roots: &[PathBuf]) -> Vec<PathBuf> {
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
            } else if path.file_name().is_some_and(|n| n == CHAT_FILE) {
                files.push(path);
            }
        }
    }
    files.sort();
    files
}

/// One chat file with its identity derived from the directory layout.
struct ChatFile {
    session_id: String,
    /// Timestamp encoded in the chat directory name, if it parses.
    chat_ts: Option<i64>,
    file_ts: i64,
    messages: Vec<Value>,
}

fn read_chat(path: &Path) -> Option<ChatFile> {
    let content = std::fs::read(path).ok()?;
    let messages = serde_json::from_slice::<Vec<Value>>(&content).ok()?;
    let (session_id, chat_id) = derive_context(path);
    Some(ChatFile {
        session_id,
        chat_ts: chat_id_timestamp(&chat_id),
        file_ts: file_modified_ms(path).unwrap_or(0),
        messages,
    })
}

/// `<channel>/<project>/<chat-id>` from
/// `<channel>/projects/<project>/chats/<chat-id>/chat-messages.json`.
fn derive_context(path: &Path) -> (String, String) {
    let name_of = |p: Option<&Path>, default: &str| -> String {
        p.and_then(Path::file_name)
            .and_then(|n| n.to_str())
            .filter(|n| !n.is_empty())
            .unwrap_or(default)
            .to_string()
    };
    let chat_dir = path.parent();
    let chat_id = name_of(chat_dir, "unknown");
    let project_dir = chat_dir.and_then(Path::parent).and_then(Path::parent);
    let project = name_of(project_dir, "unknown");
    let channel_dir = project_dir.and_then(Path::parent).and_then(Path::parent);
    let channel = name_of(channel_dir, "manicode");
    (format!("{channel}/{project}/{chat_id}"), chat_id)
}

/// `2026-01-02T03-04-05.000Z` → `2026-01-02T03:04:05.000Z` → epoch ms.
fn chat_id_timestamp(chat_id: &str) -> Option<i64> {
    let (date, time) = chat_id.split_once('T')?;
    let mut time = time.to_string();
    for _ in 0..2 {
        if let Some(i) = time.find('-') {
            time.replace_range(i..=i, ":");
        }
    }
    jsonl::parse_ts_ms(&Value::String(format!("{date}T{time}")))
}

fn file_modified_ms(path: &Path) -> Option<i64> {
    let modified = std::fs::metadata(path).ok()?.modified().ok()?;
    let ms = modified
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_millis()
        .min(i64::MAX as u128);
    Some(ms as i64)
}

/// ISO string, or a number in seconds (< 1e10) or milliseconds.
fn timestamp_value(value: Option<&Value>) -> Option<i64> {
    match value? {
        Value::String(s) => jsonl::parse_ts_ms(&Value::String(s.clone())),
        Value::Number(n) => {
            let raw = n.as_i64()?;
            let ms = if raw < 10_000_000_000 {
                raw.checked_mul(1_000)?
            } else {
                raw
            };
            (ms > 0).then_some(ms)
        }
        _ => None,
    }
}

fn message_timestamp(message: &Map<String, Value>) -> Option<i64> {
    timestamp_value(message.get("timestamp"))
        .or_else(|| timestamp_value(message.get("createdAt")))
        .or_else(|| {
            object_field(message, "metadata").and_then(|m| timestamp_value(m.get("timestamp")))
        })
}

fn resolve_timestamp(chat: &ChatFile, message: &Map<String, Value>) -> i64 {
    message_timestamp(message)
        .or(chat.chat_ts)
        .unwrap_or(chat.file_ts)
}

fn is_assistant_message(message: &Map<String, Value>) -> bool {
    matches!(
        string_field(message, "variant")
            .or_else(|| string_field(message, "role"))
            .as_deref(),
        Some("ai" | "agent" | "assistant")
    )
}

fn is_user_message(message: &Map<String, Value>) -> bool {
    matches!(
        string_field(message, "variant")
            .or_else(|| string_field(message, "role"))
            .as_deref(),
        Some("user" | "human")
    )
}

// ---- usage extraction -------------------------------------------------------

#[derive(Clone, Default, Debug, PartialEq)]
struct AssistantUsage {
    model: Option<String>,
    credits: f64,
    input: u64,
    output: u64,
    cache_write: u64,
    cache_read: u64,
    /// Tokens a bare `totalTokens` reported beyond the itemised columns when
    /// output was already known; billed as output downstream.
    extra_total: u64,
}

impl AssistantUsage {
    fn has_signal(&self) -> bool {
        self.input > 0
            || self.output > 0
            || self.cache_write > 0
            || self.cache_read > 0
            || self.extra_total > 0
            || self.credits > 0.0
    }

    fn to_counts(&self) -> TokenCounts {
        TokenCounts {
            input: self.input,
            output: self.output.saturating_add(self.extra_total),
            cache_read: self.cache_read,
            cache_write: self.cache_write,
            reasoning: 0,
        }
    }
}

fn extract_assistant_usage(message: &Map<String, Value>) -> AssistantUsage {
    let mut usage = AssistantUsage::default();
    if let Some(metadata) = object_field(message, "metadata") {
        usage.model = string_field(metadata, "model");
        merge_fallback(&mut usage, parse_usage_object(metadata.get("usage")));
        merge_fallback(
            &mut usage,
            parse_usage_object(object_field(metadata, "codebuff").and_then(|c| c.get("usage"))),
        );
        if let Some(run_state_usage) = usage_from_run_state(metadata) {
            merge_fallback(&mut usage, run_state_usage);
        }
    }
    let credits = number_field(message, "credits");
    if credits > 0.0 && usage.credits <= 0.0 {
        usage.credits = credits;
    }
    usage
}

fn run_state_history(metadata: &Map<String, Value>) -> Option<&Vec<Value>> {
    metadata
        .get("runState")?
        .get("sessionState")?
        .get("mainAgentState")?
        .get("messageHistory")?
        .as_array()
}

/// Newest assistant entry in the run-state history wins; older ones only fill
/// columns it left at zero.
fn usage_from_run_state(metadata: &Map<String, Value>) -> Option<AssistantUsage> {
    let history = run_state_history(metadata)?;
    let mut usage = AssistantUsage::default();
    let mut found = false;
    for item in history.iter().rev() {
        let Some(entry) = item.as_object() else {
            continue;
        };
        if string_field(entry, "role").as_deref() != Some("assistant") {
            continue;
        }
        let Some(provider_options) = object_field(entry, "providerOptions") else {
            continue;
        };
        let mut entry_usage = AssistantUsage::default();
        merge_fallback(
            &mut entry_usage,
            parse_usage_object(provider_options.get("usage")),
        );
        if let Some(codebuff) = object_field(provider_options, "codebuff") {
            merge_fallback(&mut entry_usage, parse_usage_object(codebuff.get("usage")));
            entry_usage.model = string_field(codebuff, "model").or(entry_usage.model);
        }
        if entry_usage.has_signal() || entry_usage.model.is_some() {
            found = true;
        }
        merge_fallback(&mut usage, entry_usage);
    }
    found.then_some(usage)
}

/// Parse one usage record in any of the spellings Codebuff has written.
fn parse_usage_object(value: Option<&Value>) -> AssistantUsage {
    let mut usage = AssistantUsage::default();
    let Some(record) = value.and_then(Value::as_object) else {
        return usage;
    };
    let anthropic_input = pick_u64(record, &["inputTokens", "input_tokens"]);
    let prompt_input = pick_u64(record, &["promptTokens", "prompt_tokens"]);
    let detail_cached = pick_nested_u64(record, "promptTokensDetails", &["cachedTokens"]).max(
        pick_nested_u64(record, "prompt_tokens_details", &["cached_tokens"]),
    );
    // Anthropic-style inputTokens already excludes the cache columns;
    // OpenAI-style prompt_tokens includes cached_tokens, so split it out to
    // keep `input` = uncached input like every other extractor.
    usage.input = if anthropic_input > 0 {
        anthropic_input
    } else {
        prompt_input.saturating_sub(detail_cached)
    };
    usage.output = pick_u64(
        record,
        &[
            "outputTokens",
            "output_tokens",
            "completionTokens",
            "completion_tokens",
        ],
    );
    usage.cache_read =
        pick_u64(record, &["cacheReadInputTokens", "cache_read_input_tokens"]).max(detail_cached);
    usage.cache_write = pick_u64(
        record,
        &[
            "cacheCreationInputTokens",
            "cache_creation_input_tokens",
            "cacheCreationTokens",
            "cache_creation_tokens",
            "cachedTokensCreated",
            "cached_tokens_created",
        ],
    );
    let total = pick_u64(record, &["totalTokens", "total_tokens", "total"]);
    let known = usage
        .input
        .saturating_add(usage.output)
        .saturating_add(usage.cache_write)
        .saturating_add(usage.cache_read);
    let missing = total.saturating_sub(known);
    if missing > 0 {
        if usage.output == 0 {
            usage.output = missing;
        } else {
            usage.extra_total = missing;
        }
    }
    usage.credits = number_field(record, "credits");
    usage.model = string_field(record, "model");
    usage
}

fn merge_fallback(target: &mut AssistantUsage, fallback: AssistantUsage) {
    if target.input == 0 {
        target.input = fallback.input;
    }
    if target.output == 0 {
        target.output = fallback.output;
    }
    if target.cache_write == 0 {
        target.cache_write = fallback.cache_write;
    }
    if target.cache_read == 0 {
        target.cache_read = fallback.cache_read;
    }
    if target.extra_total == 0 {
        target.extra_total = fallback.extra_total;
    }
    if target.credits <= 0.0 {
        target.credits = fallback.credits;
    }
    if target.model.is_none() {
        target.model = fallback.model;
    }
}

/// Codebuff's run state carries the project file context; `cwd` (else
/// `projectRoot`) is the working directory the chat ran in.
fn run_state_cwd(message: &Map<String, Value>) -> Option<String> {
    let file_context = object_field(message, "metadata")?
        .get("runState")?
        .get("sessionState")?
        .get("fileContext")?
        .as_object()?;
    string_field(file_context, "cwd").or_else(|| string_field(file_context, "projectRoot"))
}

// ---- counts -----------------------------------------------------------------

struct Entry {
    ts: i64,
    model: Option<String>,
    usage: AssistantUsage,
}

/// Assistant turns with any usage signal, deduplicated by message `id`
/// (last emission wins, keeping the first position).
fn usage_entries(chat: &ChatFile) -> Vec<Entry> {
    let mut entries: Vec<Entry> = Vec::new();
    let mut by_key: HashMap<String, usize> = HashMap::new();
    for (ordinal, message) in chat.messages.iter().enumerate() {
        let Some(message) = message.as_object() else {
            continue;
        };
        if !is_assistant_message(message) {
            continue;
        }
        let usage = extract_assistant_usage(message);
        if !usage.has_signal() {
            continue;
        }
        let ts = resolve_timestamp(chat, message);
        let key = match string_field(message, "id") {
            Some(id) => format!("id:{id}"),
            None => format!(
                "ord:{ordinal}:{ts}:{}:{}:{}:{}:{}:{}",
                usage.model.as_deref().unwrap_or(""),
                usage.input,
                usage.output,
                usage.cache_read,
                usage.cache_write,
                usage.extra_total
            ),
        };
        let entry = Entry {
            ts,
            model: usage.model.clone(),
            usage,
        };
        match by_key.get(&key) {
            Some(&i) => entries[i] = entry,
            None => {
                by_key.insert(key, entries.len());
                entries.push(entry);
            }
        }
    }
    entries
}

/// Parse one chat file into token counts. Public for fixture tests.
pub fn parse_counts(path: &Path, since: Option<i64>) -> Option<SessionCounts> {
    let chat = read_chat(path)?;
    let entries = usage_entries(&chat);
    if entries.is_empty() {
        return None;
    }

    let mut counts = TokenCounts::default();
    let mut in_window = false;
    let mut started_at_ms: Option<i64> = None;
    let mut last_seen_at_ms: Option<i64> = None;
    let mut model: Option<(i64, String)> = None;
    for entry in &entries {
        started_at_ms = Some(started_at_ms.map_or(entry.ts, |s| s.min(entry.ts)));
        last_seen_at_ms = Some(last_seen_at_ms.map_or(entry.ts, |l| l.max(entry.ts)));
        if let Some(m) = &entry.model
            && model.as_ref().is_none_or(|(t, _)| entry.ts >= *t)
        {
            model = Some((entry.ts, m.clone()));
        }
        if since.is_some_and(|cutoff| entry.ts < cutoff) {
            continue;
        }
        in_window = true;
        let c = entry.usage.to_counts();
        counts.input += c.input;
        counts.output += c.output;
        counts.cache_read += c.cache_read;
        counts.cache_write += c.cache_write;
    }
    if since.is_some() && !in_window {
        return None;
    }

    let cwd = chat
        .messages
        .iter()
        .filter_map(Value::as_object)
        .filter_map(run_state_cwd)
        .next_back();

    Some(SessionCounts {
        agent: AgentId::Codebuff,
        session_id: chat.session_id,
        model: model.map(|(_, m)| m),
        cwd,
        started_at_ms,
        last_seen_at_ms,
        counts,
        // `credits` is Codebuff credit spend, not dollars — not comparable.
        cost: None,
        source_file: Some(path.to_path_buf()),
    })
}

// ---- stats ------------------------------------------------------------------

/// Parse one chat file into behavioural stats. Public for fixture tests.
///
/// User/assistant counts come from the top-level messages (windowed by their
/// own timestamps). Tool calls come from the newest run-state snapshot in the
/// file — every assistant message carries the full cumulative history, so only
/// the last one is read to avoid counting each call once per turn; they are
/// therefore windowed at session granularity.
pub fn parse_stats(path: &Path, since: Option<i64>) -> Option<SessionStats> {
    let chat = read_chat(path)?;
    let mut out = SessionStats::new(AgentId::Codebuff, chat.session_id.clone());
    out.source_file = Some(path.to_path_buf());

    let mut seen_ids: HashSet<String> = HashSet::new();
    let mut saw_activity = false;
    let mut latest_history: Option<&Vec<Value>> = None;
    for message in &chat.messages {
        let Some(message) = message.as_object() else {
            continue;
        };
        if let Some(cwd) = run_state_cwd(message) {
            out.cwd = Some(cwd);
        }
        if let Some(history) = object_field(message, "metadata").and_then(run_state_history) {
            latest_history = Some(history);
        }
        let is_user = is_user_message(message);
        let is_assistant = is_assistant_message(message);
        if !is_user && !is_assistant {
            continue;
        }
        if let Some(id) = string_field(message, "id")
            && !seen_ids.insert(id)
        {
            continue;
        }
        let ts = resolve_timestamp(&chat, message);
        if since.is_some_and(|cutoff| ts < cutoff) {
            continue;
        }
        saw_activity = true;
        out.touch_ts(Some(ts));
        if is_user {
            out.user_messages += 1;
        } else {
            out.assistant_messages += 1;
        }
        if let Some(parts) = message.get("content").and_then(Value::as_array) {
            record_tool_parts(&mut out, parts);
        }
    }
    if since.is_some() && !saw_activity {
        return None;
    }

    if let Some(history) = latest_history {
        for item in history {
            let Some(entry) = item.as_object() else {
                continue;
            };
            if string_field(entry, "role").as_deref() != Some("assistant") {
                continue;
            }
            if let Some(parts) = entry.get("content").and_then(Value::as_array) {
                record_tool_parts(&mut out, parts);
            }
        }
    }
    Some(out)
}

/// AI-SDK style content parts: `{type: "tool-call", toolName, input|args}`.
fn record_tool_parts(out: &mut SessionStats, parts: &[Value]) {
    for part in parts {
        let Some(part) = part.as_object() else {
            continue;
        };
        if !matches!(
            string_field(part, "type").as_deref(),
            Some("tool-call" | "tool_call" | "toolCall" | "tool_use")
        ) {
            continue;
        }
        let name = string_field(part, "toolName")
            .or_else(|| string_field(part, "name"))
            .unwrap_or_else(|| "unknown".into());
        let args = ["input", "args", "arguments"]
            .iter()
            .find_map(|k| part.get(*k))
            .map(|v| match v {
                Value::String(s) => serde_json::from_str(s).unwrap_or(Value::Null),
                other => other.clone(),
            })
            .unwrap_or(Value::Null);
        out.record_tool(&name, &args);
    }
}

// ---- lenient field access ---------------------------------------------------

fn object_field<'a>(record: &'a Map<String, Value>, key: &str) -> Option<&'a Map<String, Value>> {
    record.get(key).and_then(Value::as_object)
}

fn string_field(record: &Map<String, Value>, key: &str) -> Option<String> {
    let value = record.get(key)?.as_str()?.trim();
    (!value.is_empty()).then(|| value.to_string())
}

fn number_field(record: &Map<String, Value>, key: &str) -> f64 {
    record
        .get(key)
        .and_then(Value::as_f64)
        .filter(|v| v.is_finite() && *v > 0.0)
        .unwrap_or(0.0)
}

/// First key holding a positive integer wins.
fn pick_u64(record: &Map<String, Value>, keys: &[&str]) -> u64 {
    keys.iter()
        .filter_map(|k| record.get(*k))
        .find_map(|v| v.as_u64().filter(|n| *n > 0))
        .unwrap_or(0)
}

fn pick_nested_u64(record: &Map<String, Value>, key: &str, keys: &[&str]) -> u64 {
    object_field(record, key).map_or(0, |nested| pick_u64(nested, keys))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn fixture(rel: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/codebuff")
            .join(rel)
    }

    fn chat_a() -> PathBuf {
        fixture("manicode/projects/project-a/chats/2026-01-02T03-04-05.000Z/chat-messages.json")
    }

    const T_03_04_06: i64 = 1_767_323_046_000;
    const T_03_10_00: i64 = 1_767_323_400_000;
    const T_03_12_00: i64 = 1_767_323_520_000;

    #[test]
    fn counts_parse_across_metadata_and_run_state_shapes() {
        let s = parse_counts(&chat_a(), None).expect("session");
        assert_eq!(s.session_id, "manicode/project-a/2026-01-02T03-04-05.000Z");
        // msg-a1 is emitted twice (last wins: output 60), the run-state turn
        // adds 200 uncached + 100 cached + 80 out, the bare totalTokens turn
        // adds 40 output; the usage-less streaming placeholder is skipped.
        assert_eq!(s.counts.input, 100 + 200);
        assert_eq!(s.counts.output, 60 + 80 + 40);
        assert_eq!(s.counts.cache_write, 20);
        assert_eq!(s.counts.cache_read, 10 + 100);
        assert_eq!(s.counts.reasoning, 0);
        assert_eq!(s.model.as_deref(), Some("openai/gpt-5"));
        assert_eq!(s.cwd.as_deref(), Some("/home/dev/project-a"));
        assert_eq!(s.started_at_ms, Some(T_03_04_06));
        assert_eq!(s.last_seen_at_ms, Some(T_03_12_00));
        assert_eq!(s.cost, None);
        assert_eq!(s.source_file.as_deref(), Some(chat_a().as_path()));
    }

    #[test]
    fn since_counts_only_turns_at_or_after_the_cutoff() {
        let s = parse_counts(&chat_a(), Some(T_03_10_00)).expect("in window");
        assert_eq!(s.counts.input, 200);
        assert_eq!(s.counts.output, 80 + 40);
        assert_eq!(s.counts.cache_read, 100);
        assert_eq!(s.counts.cache_write, 0);
        // Session span is still the whole chat.
        assert_eq!(s.started_at_ms, Some(T_03_04_06));

        assert!(parse_counts(&chat_a(), Some(T_03_12_00 + 1)).is_none());
    }

    #[test]
    fn stats_count_messages_and_tools_from_the_latest_snapshot() {
        let s = parse_stats(&chat_a(), None).expect("stats");
        assert_eq!(s.user_messages, 2);
        // msg-a1 twice counts once; the placeholder still is an assistant turn.
        assert_eq!(s.assistant_messages, 4);
        assert_eq!(s.tool_calls, 2);
        assert_eq!(s.tools.get("read_files"), Some(&1));
        assert_eq!(s.tools.get("run_terminal_command"), Some(&1));
        assert_eq!(s.skills.get("drill-tdd"), Some(&1));
        assert_eq!(s.cwd.as_deref(), Some("/home/dev/project-a"));
        assert_eq!(s.started_at_ms, Some(T_03_04_06 - 1_000));
        assert_eq!(s.last_seen_at_ms, Some(T_03_12_00));

        let w = parse_stats(&chat_a(), Some(T_03_10_00)).expect("in window");
        assert_eq!(w.user_messages, 0);
        assert_eq!(w.assistant_messages, 3);
        assert!(parse_stats(&chat_a(), Some(T_03_12_00 + 1)).is_none());
    }

    #[test]
    fn garbage_is_skipped_without_panicking() {
        let junk = fixture("manicode/projects/junk/chats/not-a-timestamp/chat-messages.json");
        // No record carries a usable usage signal, so no counts...
        assert!(parse_counts(&junk, None).is_none());
        // ...but the assistant-role records are still messages; nothing in
        // them parses as a tool call.
        let st = parse_stats(&junk, None).expect("messages without usage still count");
        assert_eq!(st.assistant_messages, 7);
        assert_eq!(st.tool_calls, 0);
        assert!(st.skills.is_empty());
        assert!(st.cwd.is_none());

        let not_json = fixture("manicode/projects/junk/chats/broken/chat-messages.json");
        assert!(parse_counts(&not_json, None).is_none());
        assert!(parse_stats(&not_json, None).is_none());

        assert!(parse_counts(&fixture("missing/chat-messages.json"), None).is_none());
    }

    #[test]
    fn chat_id_timestamp_backfills_messages_without_one() {
        let s = parse_counts(
            &fixture(
                "manicode-dev/projects/proj-b/chats/2026-02-03T10-20-30.500Z/chat-messages.json",
            ),
            None,
        )
        .expect("session");
        assert_eq!(s.session_id, "manicode-dev/proj-b/2026-02-03T10-20-30.500Z");
        let expected = jsonl::parse_ts_ms(&json!("2026-02-03T10:20:30.500Z"));
        assert_eq!(s.started_at_ms, expected);
        assert_eq!(s.counts.output, 7);
        assert_eq!(s.model.as_deref(), Some("claude-sonnet-4-20250514"));
        assert!(s.cwd.is_none());
    }

    #[test]
    fn openai_prompt_tokens_are_split_into_uncached_and_cached() {
        let u = parse_usage_object(Some(&json!({
            "prompt_tokens": 300, "completion_tokens": 80,
            "prompt_tokens_details": {"cached_tokens": 100}
        })));
        assert_eq!((u.input, u.cache_read, u.output), (200, 100, 80));

        let a = parse_usage_object(Some(&json!({
            "inputTokens": 100, "cacheReadInputTokens": 10, "outputTokens": 5
        })));
        assert_eq!((a.input, a.cache_read, a.output), (100, 10, 5));
    }

    #[test]
    fn total_tokens_fallback_lands_in_output() {
        let u = parse_usage_object(Some(&json!({"totalTokens": 789})));
        assert_eq!(u.output, 789);
        assert_eq!(u.extra_total, 0);

        let u = parse_usage_object(Some(&json!({
            "inputTokens": 100, "outputTokens": 50, "totalTokens": 175
        })));
        assert_eq!(u.output, 50);
        assert_eq!(u.extra_total, 25);
        assert_eq!(u.to_counts().output, 75);
    }

    #[test]
    fn numeric_timestamps_accept_seconds_and_millis() {
        assert_eq!(
            timestamp_value(Some(&json!(1_767_323_400))),
            Some(T_03_10_00)
        );
        assert_eq!(
            timestamp_value(Some(&json!(1_767_323_400_000i64))),
            Some(T_03_10_00)
        );
        assert_eq!(timestamp_value(Some(&json!(0))), None);
        assert_eq!(timestamp_value(Some(&json!("nope"))), None);
        assert_eq!(timestamp_value(Some(&json!(true))), None);
    }

    #[test]
    fn discovery_honours_the_override_and_default_channels() {
        let home = fixture("home");
        let roots = project_roots_from(None, &home);
        assert_eq!(
            roots,
            vec![
                home.join(".config/manicode/projects"),
                home.join(".config/manicode-dev/projects"),
            ],
            "only channels that exist under $HOME/.config are scanned"
        );

        let base = fixture("");
        let list = format!(
            "{}, {}/manicode/projects ,, /nonexistent/codebuff",
            base.join("manicode").display(),
            base.display()
        );
        let roots = project_roots_from(Some(&list), &home);
        assert_eq!(roots, vec![base.join("manicode/projects")]);

        let files = chat_files(&roots);
        assert_eq!(files.len(), 3);
        assert!(files.iter().all(|f| f.ends_with(CHAT_FILE)));
        assert!(project_roots_from(Some(""), &home).is_empty());
    }
}
