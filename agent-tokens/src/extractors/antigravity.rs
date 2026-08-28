//! Antigravity IDE usage, read from synced JSONL caches.
//!
//! The Antigravity IDE exposes usage only via a live language-server RPC —
//! there is no on-disk usage log to read directly. This extractor reads the
//! JSONL artifacts produced by a sync step, from two sources sharing one
//! format (line format ported from tokscale, junhoyeo/tokscale, MIT,
//! `sessions/antigravity.rs`):
//! - ololo's built-in sync (`ololo::antigravity_sync`) —
//!   `~/.config/ololo/antigravity-cache/sessions/*.jsonl`;
//! - `tokscale antigravity sync` —
//!   `~/.config/tokscale/antigravity-cache/sessions/*.jsonl`.
//!
//! Both syncs derive the artifact file name from the session id the same
//! way, so a session synced by both appears under one name in each dir;
//! files are deduped by name (newest mtime wins) to keep usage counted once.
//!
//! Lines are JSON objects tagged by `type`:
//! - `"session_meta"`: carries `modelId` (fallback model for later usage rows)
//! - `"usage"`: carries `sessionId`, `timestamp` (epoch ms), token fields
//!   (`input`, `output`, `cacheRead`, `cacheWrite`, `reasoning`), optional
//!   `modelId` and `responseId` (dedup key). Junk / unknown lines are skipped.

use crate::paths;
use crate::trait_::TokenExtractor;
use crate::types::{AgentId, SessionCounts, SessionStats, TokenCounts};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

pub struct Antigravity;

impl TokenExtractor for Antigravity {
    fn id(&self) -> AgentId {
        AgentId::Antigravity
    }

    fn detect(&self) -> bool {
        paths::ololo_antigravity_cache_dir().exists()
            || paths::tokscale_antigravity_cache_dir().exists()
    }

    fn extract(&self, since: Option<i64>) -> Vec<SessionCounts> {
        merged_cache_files()
            .iter()
            .flat_map(|f| parse_cache_counts(f, since))
            .collect()
    }

    fn stats(&self, since: Option<i64>) -> Vec<SessionStats> {
        merged_cache_files()
            .iter()
            .flat_map(|f| parse_cache_stats(f, since))
            .collect()
    }
}

/// Artifacts from both sync sources, deduped by file name so a session
/// synced by both is counted once — the newer file wins.
fn merged_cache_files() -> Vec<PathBuf> {
    merge_by_name(&[
        cache_files(&paths::ololo_antigravity_cache_dir()),
        cache_files(&paths::tokscale_antigravity_cache_dir()),
    ])
}

fn merge_by_name(sources: &[Vec<PathBuf>]) -> Vec<PathBuf> {
    let mut best: HashMap<std::ffi::OsString, PathBuf> = HashMap::new();
    for path in sources.iter().flatten() {
        let Some(name) = path.file_name().map(|n| n.to_os_string()) else {
            continue;
        };
        match best.entry(name) {
            std::collections::hash_map::Entry::Occupied(mut e) => {
                if modified_ms(path) > modified_ms(e.get()) {
                    e.insert(path.clone());
                }
            }
            std::collections::hash_map::Entry::Vacant(e) => {
                e.insert(path.clone());
            }
        }
    }
    let mut files: Vec<PathBuf> = best.into_values().collect();
    files.sort();
    files
}

fn modified_ms(path: &Path) -> i64 {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn cache_files(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return vec![];
    };
    let mut files: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("jsonl"))
        .collect();
    files.sort();
    files
}

/// One parsed `usage` row.
struct UsageRow {
    session_id: String,
    timestamp_ms: i64,
    model: Option<String>,
    counts: TokenCounts,
}

/// Tolerant numeric extraction ported from tokscale's `to_safe_i64`: accepts
/// i64 / u64 (clamped into i64) / numeric strings; anything else or negative
/// values collapse to 0.
fn to_safe_i64(value: Option<&Value>) -> i64 {
    value
        .and_then(|inner| {
            inner
                .as_i64()
                .or_else(|| inner.as_u64().and_then(|number| i64::try_from(number).ok()))
                .or_else(|| inner.as_str().and_then(|text| text.parse::<i64>().ok()))
        })
        .unwrap_or(0)
        .max(0)
}

fn parse_usage_row(value: &Value, fallback_model: Option<&str>) -> Option<UsageRow> {
    let session_id = value.get("sessionId").and_then(Value::as_str)?.to_string();
    let timestamp_ms = to_safe_i64(value.get("timestamp"));
    if timestamp_ms <= 0 {
        return None;
    }

    let model = value
        .get("modelId")
        .and_then(Value::as_str)
        .filter(|text| !text.trim().is_empty())
        .map(String::from)
        .or_else(|| fallback_model.map(String::from));

    let input = to_safe_i64(value.get("input")) as u64;
    let output = to_safe_i64(value.get("output")) as u64;
    let cache_read = to_safe_i64(value.get("cacheRead")) as u64;
    let cache_write = to_safe_i64(value.get("cacheWrite")) as u64;
    let reasoning = to_safe_i64(value.get("reasoning")) as u64;
    if input == 0 && output == 0 && cache_read == 0 && cache_write == 0 && reasoning == 0 {
        return None;
    }

    Some(UsageRow {
        session_id,
        timestamp_ms,
        model,
        counts: TokenCounts {
            input,
            output,
            cache_read,
            cache_write,
            reasoning,
        },
    })
}

/// Read all usage rows from one cache file, deduped by `responseId` and
/// filtered by `since` on the row timestamp.
fn read_usage_rows(path: &Path, since: Option<i64>) -> Vec<UsageRow> {
    let Ok(content) = std::fs::read_to_string(path) else {
        return vec![];
    };

    let mut rows = Vec::new();
    let mut session_model: Option<String> = None;
    let mut seen_response_ids: HashSet<String> = HashSet::new();

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Ok(value) = serde_json::from_str::<Value>(trimmed) else {
            continue;
        };
        match value.get("type").and_then(Value::as_str).unwrap_or("") {
            "session_meta" => {
                if let Some(model_id) = value
                    .get("modelId")
                    .and_then(Value::as_str)
                    .filter(|text| !text.trim().is_empty())
                {
                    session_model = Some(model_id.to_string());
                }
            }
            "usage" => {
                if let Some(key) = value
                    .get("responseId")
                    .and_then(Value::as_str)
                    .filter(|text| !text.trim().is_empty())
                    && !seen_response_ids.insert(key.to_string())
                {
                    continue;
                }
                let Some(row) = parse_usage_row(&value, session_model.as_deref()) else {
                    continue;
                };
                if let Some(cutoff) = since
                    && row.timestamp_ms < cutoff
                {
                    continue;
                }
                rows.push(row);
            }
            _ => {}
        }
    }
    rows
}

/// Token counts from one cache file, aggregated per (session, model).
/// Public for fixture tests.
pub fn parse_cache_counts(path: &Path, since: Option<i64>) -> Vec<SessionCounts> {
    let mut sessions: HashMap<(String, Option<String>), SessionCounts> = HashMap::new();
    for row in read_usage_rows(path, since) {
        let agg = sessions
            .entry((row.session_id.clone(), row.model.clone()))
            .or_insert_with(|| SessionCounts {
                agent: AgentId::Antigravity,
                session_id: row.session_id.clone(),
                model: row.model.clone(),
                cwd: None,
                started_at_ms: None,
                last_seen_at_ms: None,
                counts: TokenCounts::default(),
                cost: None,
                source_file: Some(path.to_path_buf()),
            });
        agg.counts.input = agg.counts.input.saturating_add(row.counts.input);
        agg.counts.output = agg.counts.output.saturating_add(row.counts.output);
        agg.counts.cache_read = agg.counts.cache_read.saturating_add(row.counts.cache_read);
        agg.counts.cache_write = agg
            .counts
            .cache_write
            .saturating_add(row.counts.cache_write);
        agg.counts.reasoning = agg.counts.reasoning.saturating_add(row.counts.reasoning);
        let t = row.timestamp_ms;
        if agg.started_at_ms.is_none_or(|s| t < s) {
            agg.started_at_ms = Some(t);
        }
        if agg.last_seen_at_ms.is_none_or(|l| t > l) {
            agg.last_seen_at_ms = Some(t);
        }
    }
    sessions.into_values().collect()
}

/// Minimal stats: one SessionStats per session, assistant message count =
/// usage rows (the cache carries no user messages or tool calls). Public for
/// fixture tests.
pub fn parse_cache_stats(path: &Path, since: Option<i64>) -> Vec<SessionStats> {
    let mut sessions: HashMap<String, SessionStats> = HashMap::new();
    for row in read_usage_rows(path, since) {
        let stats = sessions.entry(row.session_id.clone()).or_insert_with(|| {
            let mut s = SessionStats::new(AgentId::Antigravity, row.session_id.clone());
            s.source_file = Some(path.to_path_buf());
            s
        });
        stats.assistant_messages += 1;
        stats.touch_ts(Some(row.timestamp_ms));
    }
    sessions.into_values().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_by_name_dedupes_across_dirs_newest_wins() {
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        let older = a.path().join("sess-abc.jsonl");
        let newer = b.path().join("sess-abc.jsonl");
        let only_a = a.path().join("sess-only.jsonl");
        std::fs::write(&older, "{}").unwrap();
        std::fs::write(&only_a, "{}").unwrap();
        std::fs::write(&newer, "{}").unwrap();
        let old_time = std::time::SystemTime::now() - std::time::Duration::from_secs(60);
        std::fs::File::open(&older)
            .unwrap()
            .set_modified(old_time)
            .unwrap();

        let merged = merge_by_name(&[cache_files(a.path()), cache_files(b.path())]);
        assert_eq!(merged.len(), 2);
        assert!(merged.contains(&newer), "newer duplicate should win");
        assert!(merged.contains(&only_a));
        assert!(!merged.contains(&older));
    }
}
