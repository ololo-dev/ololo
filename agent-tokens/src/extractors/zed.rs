//! Zed's built-in agent: one sqlite DB of threads at
//! `~/Library/Application Support/Zed/threads/threads.db` (macOS),
//! `$XDG_DATA_HOME/zed/threads/threads.db` (Linux), `%LOCALAPPDATA%` on
//! Windows. Opened read-only + immutable — Zed keeps it open while running.
//!
//! Each `threads` row holds one thread; `data` is the payload and `data_type`
//! says how it is encoded (`json` verbatim, `zstd` compressed). Decompression
//! is capped so a corrupt or hostile blob cannot exhaust memory.
//!
//! Usage lives on the thread, not the message: `request_token_usage` is a map
//! of message id -> usage, `cumulative_token_usage` a single running total.
//! Per-request is preferred and cumulative is the fallback, as in tokscale
//! (junhoyeo/tokscale, MIT, `sessions/zed.rs`). Zed persists
//! `language_model::TokenUsage`, which carries input/output/cache only —
//! reasoning tokens are not stored, so they stay 0 rather than being guessed.
//!
//! Which threads count is where this deliberately departs from tokscale.
//! tokscale keeps only `provider == "zed.dev"`, reasoning that everything
//! else is billed and logged by its own tool. That holds when you have an
//! extractor per provider; here we do not. Real Zed installs run mostly
//! non-zed.dev providers — Ollama, Copilot, OpenAI — and dropping them
//! reports zero while millions of tokens go unattributed. So the rule is
//! inverted: count every thread except providers we already read straight
//! from their own store, which would otherwise be counted twice.
//!
//! Contract: malformed rows are skipped, never panic.

use crate::paths;
use crate::sqlite;
use crate::trait_::TokenExtractor;
use crate::types::{AgentId, SessionCounts, TokenCounts};
use std::path::PathBuf;

/// Ceiling for a single decoded thread payload (32 MB), matching tokscale.
const MAX_PAYLOAD: usize = 32 * 1024 * 1024;

pub struct Zed;

/// First existing threads DB across the platform-specific locations.
fn threads_db() -> Option<PathBuf> {
    paths::zed_threads_db_paths()
        .into_iter()
        .find(|p| p.exists())
}

impl TokenExtractor for Zed {
    fn id(&self) -> AgentId {
        AgentId::Zed
    }

    fn detect(&self) -> bool {
        threads_db().is_some()
    }

    fn extract(&self, since: Option<i64>) -> Vec<SessionCounts> {
        let Some(db) = threads_db() else {
            return vec![];
        };
        parse_db(&db, since)
    }
}

fn parse_db(path: &std::path::Path, since: Option<i64>) -> Vec<SessionCounts> {
    let Ok(conn) = sqlite::open_ro_immutable(path) else {
        return vec![];
    };
    // created_at / folder_paths arrived in later schemas; tolerate their
    // absence so an older Zed still reports rather than erroring out.
    let has_created_at = column_exists(&conn, "threads", "created_at");
    let created_at = if has_created_at { "created_at" } else { "NULL" };
    let sql = format!("SELECT id, updated_at, {created_at}, summary, data_type, data FROM threads");
    let Ok(mut stmt) = conn.prepare(&sql) else {
        return vec![];
    };
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, Option<String>>(1)?,
            row.get::<_, Option<String>>(2)?,
            row.get::<_, Option<String>>(3)?,
            row.get::<_, String>(4)?,
            row.get::<_, Vec<u8>>(5)?,
        ))
    });
    let Ok(rows) = rows else {
        return vec![];
    };

    let mut out = Vec::new();
    for row in rows.flatten() {
        let (id, updated_at, created_at, _summary, data_type, data) = row;
        let updated_ms = updated_at.as_deref().and_then(parse_ts_ms);
        // `since` filters whole threads: the per-message timestamps Zed keeps
        // are not reliable enough to slice a thread mid-way.
        if let (Some(cutoff), Some(ts)) = (since, updated_ms)
            && ts < cutoff
        {
            continue;
        }
        let Some(payload) = decode(&data_type, &data) else {
            continue;
        };
        let Ok(json) = serde_json::from_slice::<serde_json::Value>(&payload) else {
            continue;
        };
        let Some((counts, model)) = thread_tokens(&json) else {
            continue;
        };
        if counts.input == 0
            && counts.output == 0
            && counts.cache_read == 0
            && counts.cache_write == 0
        {
            continue;
        }
        out.push(SessionCounts {
            agent: AgentId::Zed,
            session_id: id,
            model,
            cwd: json
                .get("folder_paths")
                .and_then(|v| v.as_array())
                .and_then(|a| a.first())
                .and_then(|v| v.as_str())
                .map(str::to_string),
            started_at_ms: created_at.as_deref().and_then(parse_ts_ms),
            last_seen_at_ms: updated_ms,
            counts,
            cost: None,
            source_file: Some(path.to_path_buf()),
        });
    }
    out
}

fn column_exists(conn: &rusqlite::Connection, table: &str, column: &str) -> bool {
    let Ok(mut stmt) = conn.prepare(&format!("PRAGMA table_info({table})")) else {
        return false;
    };
    let Ok(rows) = stmt.query_map([], |r| r.get::<_, String>(1)) else {
        return false;
    };
    rows.flatten().any(|name| name == column)
}

/// Decode a thread payload according to its `data_type`.
fn decode(data_type: &str, data: &[u8]) -> Option<Vec<u8>> {
    match data_type {
        "json" => (data.len() <= MAX_PAYLOAD).then(|| data.to_vec()),
        "zstd" => zstd::stream::decode_all(data)
            .ok()
            .filter(|d| d.len() <= MAX_PAYLOAD),
        _ => None,
    }
}

/// Providers whose usage another extractor already reports from that tool's
/// own store. Counting the Zed-side copy as well would double them.
const EXTERNALLY_TRACKED: &[&str] = &["opencode", "copilot_chat", "copilot"];

/// A thread's usage and the model that produced it.
///
/// Returns `None` when the thread belongs to a provider we read elsewhere.
fn thread_tokens(thread: &serde_json::Value) -> Option<(TokenCounts, Option<String>)> {
    let provider = thread
        .pointer("/model/provider")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    if EXTERNALLY_TRACKED.contains(&provider) {
        return None;
    }
    let model = thread
        .pointer("/model/model")
        .and_then(|v| v.as_str())
        .map(str::to_string);

    let mut counts = TokenCounts::default();
    // `request_token_usage` is keyed by message id; sum its values.
    if !add_usage(&mut counts, thread.get("request_token_usage")) {
        add_usage(&mut counts, thread.get("cumulative_token_usage"));
    }
    Some((counts, model))
}

/// Add a usage value to `counts`. Accepts the shapes Zed has used: a single
/// object, an array of them, or a map of message id -> object.
/// Returns whether anything non-zero was added.
fn add_usage(counts: &mut TokenCounts, usage: Option<&serde_json::Value>) -> bool {
    let Some(usage) = usage else {
        return false;
    };
    let entries: Vec<&serde_json::Value> = match usage {
        serde_json::Value::Array(a) => a.iter().collect(),
        serde_json::Value::Object(o) if !o.contains_key("input_tokens") => o.values().collect(),
        v => vec![v],
    };
    let mut any = false;
    for e in entries {
        let input = field(e, "input_tokens");
        let output = field(e, "output_tokens");
        let cache_write = field(e, "cache_creation_input_tokens");
        let cache_read = field(e, "cache_read_input_tokens");
        if input == 0 && output == 0 && cache_write == 0 && cache_read == 0 {
            continue;
        }
        counts.input += input;
        counts.output += output;
        counts.cache_write += cache_write;
        counts.cache_read += cache_read;
        any = true;
    }
    any
}

fn field(v: &serde_json::Value, key: &str) -> u64 {
    v.get(key).and_then(|n| n.as_u64()).unwrap_or(0)
}

/// Zed writes RFC3339 timestamps; fall back to a bare epoch-ms integer.
fn parse_ts_ms(raw: &str) -> Option<i64> {
    if let Ok(n) = raw.parse::<i64>() {
        return Some(n);
    }
    crate::jsonl::parse_ts_ms(&serde_json::Value::String(raw.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn thread(provider: &str, body: serde_json::Value) -> serde_json::Value {
        let mut v = body;
        v["model"] = json!({"model": "glm-5:cloud", "provider": provider});
        v
    }

    #[test]
    fn sums_request_usage_keyed_by_message_id() {
        // Zed stores request usage as a map of message id -> usage, not a list.
        let t = thread(
            "ollama",
            json!({"request_token_usage": {
                "id-a": {"input_tokens": 10, "output_tokens": 5},
                "id-b": {"input_tokens": 7, "output_tokens": 1},
            }}),
        );
        let (c, model) = thread_tokens(&t).expect("counted");
        assert_eq!((c.input, c.output), (17, 6));
        assert_eq!(model.as_deref(), Some("glm-5:cloud"));
        assert_eq!(c.reasoning, 0, "Zed does not persist reasoning tokens");
    }

    #[test]
    fn falls_back_to_cumulative_when_request_usage_is_missing_or_zero() {
        let missing = thread(
            "ollama",
            json!({"cumulative_token_usage": {"input_tokens": 9, "output_tokens": 4}}),
        );
        let (c, _) = thread_tokens(&missing).expect("counted");
        assert_eq!((c.input, c.output), (9, 4));

        let zeroed = thread(
            "ollama",
            json!({
                "request_token_usage": {"id-a": {"input_tokens": 0, "output_tokens": 0}},
                "cumulative_token_usage": {"input_tokens": 11, "output_tokens": 2},
            }),
        );
        let (c, _) = thread_tokens(&zeroed).expect("counted");
        assert_eq!((c.input, c.output), (11, 2));
    }

    #[test]
    fn skips_providers_another_extractor_already_reports() {
        // Driving opencode from Zed's UI must not be counted twice — the
        // opencode extractor already reads it from opencode's own store.
        for provider in ["opencode", "copilot_chat", "copilot"] {
            let t = thread(
                provider,
                json!({"cumulative_token_usage": {"input_tokens": 999, "output_tokens": 999}}),
            );
            assert!(
                thread_tokens(&t).is_none(),
                "{provider} must be left to its own extractor"
            );
        }
    }

    #[test]
    fn counts_providers_no_other_extractor_covers() {
        // The departure from tokscale: a real Zed install runs Ollama/OpenAI
        // far more than zed.dev, and nothing else reports those.
        for provider in ["zed.dev", "ollama", "openai", "anthropic", ""] {
            let t = thread(
                provider,
                json!({"cumulative_token_usage": {"input_tokens": 5, "output_tokens": 6}}),
            );
            let (c, _) = thread_tokens(&t).expect(provider);
            assert_eq!((c.input, c.output), (5, 6), "{provider}");
        }
    }

    #[test]
    fn reads_cache_fields_when_the_provider_reports_them() {
        let t = thread(
            "anthropic",
            json!({"request_token_usage": {"id": {
            "input_tokens": 1, "output_tokens": 2,
            "cache_creation_input_tokens": 3, "cache_read_input_tokens": 4}}}),
        );
        let (c, _) = thread_tokens(&t).expect("counted");
        assert_eq!((c.cache_write, c.cache_read), (3, 4));
    }

    #[test]
    fn malformed_threads_yield_nothing_rather_than_panicking() {
        for v in [
            json!({}),
            json!({"request_token_usage": "nope"}),
            json!({"model": 42}),
        ] {
            let counted = thread_tokens(&v)
                .map(|(c, _)| c.input + c.output)
                .unwrap_or(0);
            assert_eq!(counted, 0);
        }
    }

    #[test]
    fn decode_rejects_unknown_encodings_and_oversized_payloads() {
        assert!(decode("json", b"{}").is_some());
        assert!(decode("protobuf", b"{}").is_none());
        assert!(decode("json", &vec![b'x'; MAX_PAYLOAD + 1]).is_none());
        assert!(decode("zstd", b"not-really-zstd").is_none());
    }
}
