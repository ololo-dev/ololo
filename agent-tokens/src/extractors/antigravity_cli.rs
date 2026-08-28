//! Antigravity CLI conversations:
//! `${GEMINI_CLI_HOME:-~/.gemini}/antigravity-cli/conversations/<uuid>.db`,
//! one sqlite DB per conversation, opened read-only + immutable.
//!
//! Each `gen_metadata` row (ordered by `idx`) is one generation encoded as a
//! `GeneratorMetadata` protobuf. There is no published `.proto`, so this
//! module ships a minimal wire-format reader and pulls only the fields it
//! needs. The field numbers were reverse-engineered by the tokscale project
//! (junhoyeo/tokscale, MIT) and are ported from its `sessions/antigravity_cli.rs`
//! (cross-checked there against 6 sessions / 140 turns; `#9 + #10 == #3`,
//! i.e. output + thinking == total output):
//!
//! - `gen_metadata.#1`            → chatModel message
//!   - `#19` (string)            → responseModel (e.g. `gemini-3-flash-a`)
//!   - `#9.#4` = `{#1: seconds, #2: nanos}` → per-generation wall-clock time
//!   - `#4`                      → usage message
//!     - `#1` (varint, const)    → fixed system-prompt tokens (≈1132)
//!     - `#2` (varint)           → newly-processed (non-cached) input tokens
//!     - `#5` (varint)           → cacheRead tokens
//!     - `#9` (varint)           → output (text) tokens
//!     - `#10` (varint)          → thinking / reasoning tokens
//!     - `#11` (string)          → responseId (dedup key)
//! - `trajectory_metadata_blob.#2` = `{#1: seconds, #2: nanos}` → created-at
//! - `trajectory_metadata_blob.#1.#1` (string)                  → workspace URI
//!
//! Contract kept from the tokscale port: malformed data degrades to
//! `None`/skip, never panics — untrusted varints are clamped into `i64`
//! (saturating) before use so corrupt blobs cannot wrap to negative counts or
//! overflow timestamps.

use crate::paths;
use crate::sqlite;
use crate::trait_::TokenExtractor;
use crate::types::{AgentId, SessionCounts, SessionStats, TokenCounts};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

pub struct AntigravityCli;

impl TokenExtractor for AntigravityCli {
    fn id(&self) -> AgentId {
        AgentId::AntigravityCli
    }

    fn detect(&self) -> bool {
        paths::antigravity_cli_conversations_dir().exists()
    }

    fn extract(&self, since: Option<i64>) -> Vec<SessionCounts> {
        conversation_dbs(&paths::antigravity_cli_conversations_dir())
            .iter()
            .flat_map(|db| parse_db_counts(db, since))
            .collect()
    }

    fn stats(&self, since: Option<i64>) -> Vec<SessionStats> {
        conversation_dbs(&paths::antigravity_cli_conversations_dir())
            .iter()
            .filter_map(|db| parse_db_stats(db, since))
            .collect()
    }
}

fn conversation_dbs(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return vec![];
    };
    let mut dbs: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("db"))
        .collect();
    dbs.sort();
    dbs
}

/// One deduped generation pulled out of a `gen_metadata` row.
struct Generation {
    timestamp_ms: i64,
    model: Option<String>,
    counts: TokenCounts,
}

fn session_id_of(path: &Path) -> String {
    path.file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown")
        .to_string()
}

/// Read all generations from one conversation DB, deduped by responseId and
/// filtered by `since` on the per-generation timestamp.
fn read_generations(path: &Path, since: Option<i64>) -> Option<(Vec<Generation>, Option<String>)> {
    let conn = match sqlite::open_ro_immutable(path) {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!("antigravity-cli db open failed: {e}");
            return None;
        }
    };

    let (session_ts, cwd) = read_trajectory_meta(&conn, path);

    // Not an Antigravity CLI database (table missing) — nothing to count.
    let mut stmt = conn
        .prepare("SELECT data FROM gen_metadata ORDER BY idx")
        .ok()?;
    let rows = stmt.query_map([], |row| row.get::<_, Vec<u8>>(0)).ok()?;

    let mut generations = Vec::new();
    let mut seen_response_ids: HashSet<String> = HashSet::new();
    for blob in rows.flatten() {
        let Some(generation) = parse_gen_metadata(&blob, session_ts, &mut seen_response_ids) else {
            continue;
        };
        if let Some(cutoff) = since
            && generation.timestamp_ms < cutoff
        {
            continue;
        }
        generations.push(generation);
    }
    Some((generations, cwd))
}

/// Token counts for one conversation DB, one row per model (a mid-session
/// model switch yields one row per model, matching the claude extractor).
/// Public for fixture tests.
pub fn parse_db_counts(path: &Path, since: Option<i64>) -> Vec<SessionCounts> {
    let Some((generations, cwd)) = read_generations(path, since) else {
        return vec![];
    };
    let session_id = session_id_of(path);

    let mut per_model: HashMap<Option<String>, SessionCounts> = HashMap::new();
    for generation in generations {
        let agg = per_model
            .entry(generation.model.clone())
            .or_insert_with(|| SessionCounts {
                agent: AgentId::AntigravityCli,
                session_id: session_id.clone(),
                model: generation.model.clone(),
                cwd: cwd.clone(),
                started_at_ms: None,
                last_seen_at_ms: None,
                counts: TokenCounts::default(),
                cost: None,
                source_file: Some(path.to_path_buf()),
            });
        agg.counts.input = agg.counts.input.saturating_add(generation.counts.input);
        agg.counts.output = agg.counts.output.saturating_add(generation.counts.output);
        agg.counts.cache_read = agg
            .counts
            .cache_read
            .saturating_add(generation.counts.cache_read);
        agg.counts.reasoning = agg
            .counts
            .reasoning
            .saturating_add(generation.counts.reasoning);
        let t = generation.timestamp_ms;
        if t > 0 {
            if agg.started_at_ms.is_none_or(|s| t < s) {
                agg.started_at_ms = Some(t);
            }
            if agg.last_seen_at_ms.is_none_or(|l| t > l) {
                agg.last_seen_at_ms = Some(t);
            }
        }
    }
    per_model.into_values().collect()
}

/// One SessionStats per conversation DB: assistant message count = deduped
/// generations in the window. (SessionStats carries no model field, so the
/// per-generation model only surfaces via `parse_db_counts`.) Public for
/// fixture tests.
pub fn parse_db_stats(path: &Path, since: Option<i64>) -> Option<SessionStats> {
    let (generations, cwd) = read_generations(path, since)?;
    if generations.is_empty() {
        return None;
    }
    let mut stats = SessionStats::new(AgentId::AntigravityCli, session_id_of(path));
    stats.cwd = cwd;
    stats.source_file = Some(path.to_path_buf());
    for generation in &generations {
        stats.assistant_messages += 1;
        stats.touch_ts(Some(generation.timestamp_ms).filter(|&t| t > 0));
    }
    Some(stats)
}

fn parse_gen_metadata(
    blob: &[u8],
    session_timestamp: i64,
    seen_response_ids: &mut HashSet<String>,
) -> Option<Generation> {
    let chat_model = message_field(blob, 1)?;
    let usage = message_field(chat_model, 4)?;

    // Per-generation wall-clock time: `chatModel.#9.#4` is an absolute
    // `{#1: seconds, #2: nanos}` Timestamp for this turn. Fall back to the
    // session-created stamp when absent, zero, or malformed.
    let timestamp = message_field(chat_model, 9)
        .and_then(|generated| message_field(generated, 4))
        .and_then(proto_timestamp_ms)
        .filter(|&ms| ms > 0)
        .unwrap_or(session_timestamp);

    // input = fixed system prompt (#1) + newly-processed input (#2). Clamp
    // untrusted u64 varints into i64 (a corrupt/malicious blob could encode a
    // value > i64::MAX, which `as i64` would wrap to a negative count) and
    // combine with saturating_add so totals never overflow.
    let to_i64 = |v: u64| i64::try_from(v).unwrap_or(i64::MAX);
    let input = to_i64(varint_field(usage, 1).unwrap_or(0))
        .saturating_add(to_i64(varint_field(usage, 2).unwrap_or(0)));
    let cache_read = to_i64(varint_field(usage, 5).unwrap_or(0));
    let output = to_i64(varint_field(usage, 9).unwrap_or(0));
    let reasoning = to_i64(varint_field(usage, 10).unwrap_or(0));
    if input == 0 && output == 0 && cache_read == 0 && reasoning == 0 {
        return None;
    }

    // Dedup by responseId (#11): retried/replayed generations appear more
    // than once but bill once.
    if let Some(key) = string_field(usage, 11).filter(|text| !text.trim().is_empty())
        && !seen_response_ids.insert(key.to_string())
    {
        return None;
    }

    let model = string_field(chat_model, 19)
        .filter(|text| !text.trim().is_empty())
        .map(String::from);

    Some(Generation {
        timestamp_ms: timestamp,
        model,
        counts: TokenCounts {
            input: input as u64,
            output: output as u64,
            cache_read: cache_read as u64,
            cache_write: 0,
            reasoning: reasoning as u64,
        },
    })
}

/// Session-level created-at timestamp and workspace path from the single
/// `trajectory_metadata_blob` row; created-at falls back to the file mtime
/// when the blob is absent or undecodable.
fn read_trajectory_meta(conn: &rusqlite::Connection, path: &Path) -> (i64, Option<String>) {
    let blob: Option<Vec<u8>> = conn
        .query_row(
            "SELECT data FROM trajectory_metadata_blob LIMIT 1",
            [],
            |row| row.get::<_, Vec<u8>>(0),
        )
        .ok();

    let mut timestamp = None;
    let mut cwd = None;
    if let Some(blob) = &blob {
        timestamp = session_created_ms(blob).filter(|&ms| ms > 0);
        cwd = message_field(blob, 1)
            .and_then(|folder| string_field(folder, 1))
            .and_then(file_uri_to_path);
    }
    (timestamp.unwrap_or_else(|| file_modified_ms(path)), cwd)
}

fn session_created_ms(blob: &[u8]) -> Option<i64> {
    proto_timestamp_ms(message_field(blob, 2)?)
}

/// Decode a protobuf `{#1: seconds, #2: nanos}` Timestamp message to epoch ms.
///
/// `seconds` is an unbounded wire varint, so a malformed blob can carry a
/// value whose `* 1000` overflows `i64` and panics in debug builds. Use
/// checked arithmetic and return `None` on overflow. `nanos` is
/// range-validated against the protobuf Timestamp spec (`0..=999_999_999`);
/// out-of-range marks the whole stamp malformed so the caller's fallback
/// takes over instead of producing a skewed time.
fn proto_timestamp_ms(ts: &[u8]) -> Option<i64> {
    let seconds = varint_field(ts, 1)? as i64;
    let nanos = i64::try_from(varint_field(ts, 2).unwrap_or(0)).ok()?;
    if !(0..=999_999_999).contains(&nanos) {
        return None;
    }
    seconds.checked_mul(1000)?.checked_add(nanos / 1_000_000)
}

fn file_modified_ms(path: &Path) -> i64 {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Convert a `file://` URI to a filesystem path, percent-decoding UTF-8
/// escapes. After the scheme the remainder is `authority + path`; the three
/// shapes RFC 8089 (and Antigravity) produce:
/// - `file:///C:/x`        → `C:/x`           (empty authority, Windows drive)
/// - `file:///home/x`      → `/home/x`        (empty authority, POSIX absolute)
/// - `file://host/share/x` → `//host/share/x` (non-empty authority → UNC)
fn file_uri_to_path(uri: &str) -> Option<String> {
    let decoded = percent_decode(uri.strip_prefix("file://")?);
    let bytes = decoded.as_bytes();
    let path = if bytes.first() == Some(&b'/') {
        if bytes.len() >= 3 && bytes[2] == b':' {
            decoded[1..].to_string()
        } else {
            decoded
        }
    } else {
        format!("//{decoded}")
    };
    Some(path)
}

fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let (Some(hi), Some(lo)) = (hex_value(bytes[i + 1]), hex_value(bytes[i + 2]))
        {
            out.push((hi << 4) | lo);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Minimal protobuf wire-format reader (no prost / schema dependency).
// ---------------------------------------------------------------------------

enum Wire<'a> {
    Varint(u64),
    Len(&'a [u8]),
    Fixed64,
    Fixed32,
}

struct ProtoReader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> ProtoReader<'a> {
    fn new(buf: &'a [u8]) -> Self {
        Self { buf, pos: 0 }
    }

    fn read_varint(&mut self) -> Option<u64> {
        let mut result: u64 = 0;
        let mut shift = 0u32;
        loop {
            let byte = *self.buf.get(self.pos)?;
            self.pos += 1;
            result |= u64::from(byte & 0x7f) << shift;
            if byte & 0x80 == 0 {
                return Some(result);
            }
            shift += 7;
            if shift >= 64 {
                return None;
            }
        }
    }

    /// Yield the next `(field_number, value)` pair, or `None` at end-of-buffer
    /// or on a malformed/unsupported wire type. Group wire types (3/4) are
    /// deprecated and never appear here; we stop rather than risk desync.
    fn next_field(&mut self) -> Option<(u64, Wire<'a>)> {
        if self.pos >= self.buf.len() {
            return None;
        }
        let tag = self.read_varint()?;
        let field = tag >> 3;
        let wire = match tag & 0x7 {
            0 => Wire::Varint(self.read_varint()?),
            1 => {
                self.pos = self.pos.checked_add(8).filter(|&p| p <= self.buf.len())?;
                Wire::Fixed64
            }
            2 => {
                let len = self.read_varint()? as usize;
                let end = self.pos.checked_add(len).filter(|&p| p <= self.buf.len())?;
                let bytes = &self.buf[self.pos..end];
                self.pos = end;
                Wire::Len(bytes)
            }
            5 => {
                self.pos = self.pos.checked_add(4).filter(|&p| p <= self.buf.len())?;
                Wire::Fixed32
            }
            _ => return None,
        };
        Some((field, wire))
    }
}

/// First length-delimited (sub-message / string / bytes) value for `field`.
fn message_field(buf: &[u8], field: u64) -> Option<&[u8]> {
    let mut reader = ProtoReader::new(buf);
    while let Some((found, wire)) = reader.next_field() {
        if found == field
            && let Wire::Len(bytes) = wire
        {
            return Some(bytes);
        }
    }
    None
}

/// First varint value for `field`.
fn varint_field(buf: &[u8], field: u64) -> Option<u64> {
    let mut reader = ProtoReader::new(buf);
    while let Some((found, wire)) = reader.next_field() {
        if found == field
            && let Wire::Varint(value) = wire
        {
            return Some(value);
        }
    }
    None
}

/// First UTF-8 string value for `field`.
fn string_field(buf: &[u8], field: u64) -> Option<&str> {
    message_field(buf, field).and_then(|bytes| std::str::from_utf8(bytes).ok())
}
