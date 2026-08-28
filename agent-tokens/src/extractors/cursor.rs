//! Cursor IDE usage, from two sources merged:
//!
//! 1. Cursor's global state DB (`state.vscdb`, see
//!    `paths::cursor_state_db_paths`), opened read-only + `immutable=1` so a
//!    live IDE is never locked. The `cursorDiskKV` table holds
//!    `composerData:<composerId>` rows (JSON: `composerId`, `createdAt` epoch
//!    ms, `fullConversationHeadersOnly` = `[{bubbleId, type}]` with type
//!    1=user / 2=assistant) and `bubbleId:<composerId>:<bubbleId>` rows whose
//!    JSON carries a per-bubble `tokenCount` (`{inputTokens, outputTokens}`).
//!    Real DBs currently store zeros there, so only non-zero counts are
//!    emitted; message stats come from the composer headers.
//! 2. tokscale's Cursor usage-export cache (`~/.config/tokscale/cursor-cache/
//!    usage*.csv`, produced by `tokscale cursor sync`). CSV parsing (v1/v2/v3
//!    header detection, column indices, date handling) is ported from
//!    tokscale (junhoyeo/tokscale, MIT) `sessions/cursor.rs`.
//!
//! `since` granularity is coarse for the state DB: composer headers carry no
//! per-message timestamps, so a composer is included whole when its
//! `createdAt >= since` and skipped entirely otherwise.

use crate::jsonl;
use crate::paths;
use crate::sqlite;
use crate::trait_::TokenExtractor;
use crate::types::{AgentId, SessionCounts, SessionStats, TokenCounts};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub struct Cursor;

impl TokenExtractor for Cursor {
    fn id(&self) -> AgentId {
        AgentId::Cursor
    }

    fn detect(&self) -> bool {
        state_db_path().is_some() || paths::tokscale_cursor_cache_dir().exists()
    }

    fn extract(&self, since: Option<i64>) -> Vec<SessionCounts> {
        let mut out = Vec::new();
        if let Some(db) = state_db_path() {
            out.extend(parse_state_db_counts(&db, since));
        }
        for csv in cache_csv_files(&paths::tokscale_cursor_cache_dir()) {
            out.extend(parse_cache_csv(&csv, since));
        }
        out
    }

    fn stats(&self, since: Option<i64>) -> Vec<SessionStats> {
        match state_db_path() {
            Some(db) => parse_state_db_stats(&db, since),
            None => vec![],
        }
    }
}

fn state_db_path() -> Option<PathBuf> {
    paths::cursor_state_db_paths()
        .into_iter()
        .find(|p| p.exists())
}

fn cache_csv_files(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return vec![];
    };
    let mut files: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("csv"))
        .collect();
    files.sort();
    files
}

// ---------------------------------------------------------------------------
// state.vscdb (cursorDiskKV)
// ---------------------------------------------------------------------------

struct Composer {
    id: String,
    created_at_ms: Option<i64>,
    user_messages: u64,
    assistant_messages: u64,
}

fn read_composers(conn: &rusqlite::Connection) -> Vec<Composer> {
    let mut stmt =
        match conn.prepare("SELECT key, value FROM cursorDiskKV WHERE key LIKE 'composerData:%'") {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!("cursor state db composer query failed: {e}");
                return vec![];
            }
        };
    // `value` is TEXT in some DB versions and BLOB in others — read bytes.
    let rows = stmt.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, Vec<u8>>(1)?))
    });
    let Ok(rows) = rows else {
        return vec![];
    };

    let mut out = Vec::new();
    for (key, value) in rows.flatten() {
        let Ok(v) = serde_json::from_slice::<serde_json::Value>(&value) else {
            continue;
        };
        let id = v
            .get("composerId")
            .and_then(|c| c.as_str())
            .map(String::from)
            .or_else(|| key.strip_prefix("composerData:").map(String::from));
        let Some(id) = id else { continue };
        let created_at_ms = v
            .get("createdAt")
            .and_then(|t| t.as_i64())
            .filter(|&t| t > 0);
        let mut user_messages = 0u64;
        let mut assistant_messages = 0u64;
        if let Some(headers) = v
            .get("fullConversationHeadersOnly")
            .and_then(|h| h.as_array())
        {
            for header in headers {
                match header.get("type").and_then(|t| t.as_i64()) {
                    Some(1) => user_messages += 1,
                    Some(2) => assistant_messages += 1,
                    _ => {}
                }
            }
        }
        out.push(Composer {
            id,
            created_at_ms,
            user_messages,
            assistant_messages,
        });
    }
    out
}

/// Composer-level `since` filter: headers carry no per-message timestamps, so
/// the whole composer is in or out based on `createdAt` (coarse by design).
fn composer_in_window(created_at_ms: Option<i64>, since: Option<i64>) -> bool {
    match since {
        None => true,
        Some(cutoff) => created_at_ms.is_some_and(|t| t >= cutoff),
    }
}

/// One SessionStats per composer conversation. Public for fixture tests.
pub fn parse_state_db_stats(db: &Path, since: Option<i64>) -> Vec<SessionStats> {
    let conn = match sqlite::open_ro_immutable(db) {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!("cursor state db open failed: {e}");
            return vec![];
        }
    };
    read_composers(&conn)
        .into_iter()
        .filter(|c| composer_in_window(c.created_at_ms, since))
        .filter(|c| c.user_messages + c.assistant_messages > 0)
        .map(|c| {
            let mut s = SessionStats::new(AgentId::Cursor, c.id);
            s.started_at_ms = c.created_at_ms;
            s.last_seen_at_ms = c.created_at_ms;
            s.user_messages = c.user_messages;
            s.assistant_messages = c.assistant_messages;
            s.source_file = Some(db.to_path_buf());
            s
        })
        .collect()
}

/// Per-composer token counts summed from non-zero per-bubble `tokenCount`
/// rows. Public for fixture tests.
pub fn parse_state_db_counts(db: &Path, since: Option<i64>) -> Vec<SessionCounts> {
    let conn = match sqlite::open_ro_immutable(db) {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!("cursor state db open failed: {e}");
            return vec![];
        }
    };

    let created: HashMap<String, Option<i64>> = read_composers(&conn)
        .into_iter()
        .map(|c| (c.id, c.created_at_ms))
        .collect();

    let mut stmt =
        match conn.prepare("SELECT key, value FROM cursorDiskKV WHERE key LIKE 'bubbleId:%'") {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!("cursor state db bubble query failed: {e}");
                return vec![];
            }
        };
    let rows = stmt.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, Vec<u8>>(1)?))
    });
    let Ok(rows) = rows else {
        return vec![];
    };

    let mut per_composer: HashMap<String, TokenCounts> = HashMap::new();
    for (key, value) in rows.flatten() {
        // Key format: bubbleId:<composerId>:<bubbleId>
        let Some(rest) = key.strip_prefix("bubbleId:") else {
            continue;
        };
        let Some((composer_id, _bubble_id)) = rest.split_once(':') else {
            continue;
        };
        let Ok(v) = serde_json::from_slice::<serde_json::Value>(&value) else {
            continue;
        };
        let Some(tc) = v.get("tokenCount") else {
            continue;
        };
        let input = tc.get("inputTokens").and_then(|t| t.as_u64()).unwrap_or(0);
        let output = tc.get("outputTokens").and_then(|t| t.as_u64()).unwrap_or(0);
        // Real DBs currently store zeros here — only non-zero counts matter.
        if input == 0 && output == 0 {
            continue;
        }
        let agg = per_composer.entry(composer_id.to_string()).or_default();
        agg.input += input;
        agg.output += output;
    }

    per_composer
        .into_iter()
        .filter(|(id, _)| {
            // Bubbles carry no timestamps: inherit the composer's createdAt
            // window membership (composers unknown to composerData are only
            // included when no window is requested).
            composer_in_window(created.get(id).copied().flatten(), since)
        })
        .map(|(id, counts)| {
            let created_at = created.get(&id).copied().flatten();
            SessionCounts {
                agent: AgentId::Cursor,
                session_id: id,
                model: None,
                cwd: None,
                started_at_ms: created_at,
                last_seen_at_ms: created_at,
                counts,
                cost: None,
                source_file: Some(db.to_path_buf()),
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// tokscale cursor-cache CSVs (ported from tokscale sessions/cursor.rs)
// ---------------------------------------------------------------------------

/// `usage.csv` is the legacy single-account cache; extra accounts use
/// `usage.<account>.csv`.
fn account_id_from_cache_path(path: &Path) -> String {
    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("usage.csv");
    if file_name == "usage.csv" {
        return "active".to_string();
    }
    if let Some(stem) = file_name
        .strip_prefix("usage.")
        .and_then(|s| s.strip_suffix(".csv"))
    {
        let cleaned: String = stem
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' {
                    c
                } else {
                    '-'
                }
            })
            .collect();
        if cleaned.is_empty() {
            return "unknown".to_string();
        }
        return cleaned;
    }
    "unknown".to_string()
}

/// Parse a cost string like "$0.50" or "1,234.56"; "Included" / "-" / NaN → 0.
fn parse_cost(cost_str: &str) -> f64 {
    let cleaned = cost_str.replace(['$', ','], "");
    let trimmed = cleaned.trim();
    if trimmed.is_empty()
        || trimmed.eq_ignore_ascii_case("nan")
        || !trimmed.chars().any(|c| c.is_ascii_digit())
    {
        return 0.0;
    }
    trimmed.parse().unwrap_or(0.0)
}

/// Simple CSV line splitter that respects quoted fields.
fn parse_csv_line(line: &str) -> Vec<&str> {
    let mut fields = Vec::new();
    let mut start = 0;
    let mut in_quotes = false;
    for (i, &byte) in line.as_bytes().iter().enumerate() {
        match byte {
            b'"' => in_quotes = !in_quotes,
            b',' if !in_quotes => {
                fields.push(&line[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    if start <= line.len() {
        fields.push(&line[start..]);
    }
    fields
}

/// Date column → epoch ms. ISO-8601 datetimes go through the shared parser;
/// date-only rows anchor at noon UTC (keeps the local date stable for
/// UTC-12..UTC+14, matching tokscale).
fn parse_date_to_ms(date_str: &str) -> Option<i64> {
    if date_str.len() == 10 && !date_str.contains('T') {
        return jsonl::parse_ts_ms(&serde_json::Value::String(format!("{date_str}T12:00:00Z")));
    }
    jsonl::parse_ts_ms(&serde_json::Value::String(date_str.to_string()))
}

/// Parse one tokscale cursor-cache CSV. One SessionCounts per usage row, keyed
/// `cursor-csv:<account>:<date>`. Public for fixture tests.
///
/// Header formats (v1/v2/v3 detection ported from tokscale):
/// - v1: `Date,Model,Input (w/ Cache Write),Input (w/o Cache Write),Cache Read,Output Tokens,Total Tokens,Cost,Cost to you`
/// - v2: `Date,Kind,Model,Max Mode,Input (w/ Cache Write),...,Cost`
/// - v3: `Date,Cloud Agent ID,Automation ID,Kind,Model,Max Mode,...,Cost`
pub fn parse_cache_csv(path: &Path, since: Option<i64>) -> Vec<SessionCounts> {
    let Ok(content) = std::fs::read_to_string(path) else {
        return vec![];
    };
    let mut lines = content.lines();
    let Some(header) = lines.next() else {
        return vec![];
    };
    if !header.contains("Date") || !header.contains("Model") {
        return vec![];
    }

    let header_fields = parse_csv_line(header);
    let has_kind_column = header_fields.iter().any(|f| f.trim() == "Kind");
    let column_count = header_fields.len();

    // (model, input w/ cache write, input w/o cache write, cache read, output, cost)
    let (model_idx, input_cw_idx, input_idx, cache_read_idx, output_idx, cost_idx) =
        if has_kind_column && column_count >= 11 {
            (4, 6, 7, 8, 9, 11) // v3
        } else if has_kind_column {
            (2, 4, 5, 6, 7, 9) // v2
        } else {
            (1, 2, 3, 4, 5, 7) // v1
        };

    let account_id = account_id_from_cache_path(path);
    let mut out = Vec::new();

    for line in lines {
        if line.trim().is_empty() {
            continue;
        }
        let fields = parse_csv_line(line);
        if fields.len() < cost_idx + 1 {
            continue;
        }
        let field = |i: usize| fields[i].trim().trim_matches('"');
        let parse_i64 = |i: usize| field(i).parse::<i64>().unwrap_or(0);

        let date_str = field(0);
        let model = field(model_idx);
        if model.is_empty() {
            continue;
        }
        let Some(timestamp) = parse_date_to_ms(date_str).filter(|&t| t > 0) else {
            continue;
        };
        if let Some(cutoff) = since
            && timestamp < cutoff
        {
            continue;
        }

        let input_with_cache_write = parse_i64(input_cw_idx);
        let input = parse_i64(input_idx);
        let cache_read = parse_i64(cache_read_idx);
        let output = parse_i64(output_idx);
        let cost = parse_cost(field(cost_idx));
        // Cache write = input w/ cache write minus input w/o cache write.
        let cache_write = (input_with_cache_write - input).max(0) as u64;

        out.push(SessionCounts {
            agent: AgentId::Cursor,
            session_id: format!("cursor-csv:{account_id}:{date_str}"),
            model: Some(model.to_string()),
            cwd: None,
            started_at_ms: Some(timestamp),
            last_seen_at_ms: Some(timestamp),
            counts: TokenCounts {
                input: input.max(0) as u64,
                output: output.max(0) as u64,
                cache_read: cache_read.max(0) as u64,
                cache_write,
                reasoning: 0,
            },
            cost: (cost > 0.0).then_some(cost),
            source_file: Some(path.to_path_buf()),
        });
    }
    out
}
