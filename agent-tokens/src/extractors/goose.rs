//! Goose sessions: sqlite `sessions.db` (location varies by platform, see
//! `paths::goose_db_paths`). Per tokscale's goose parser the `sessions` table
//! has `id`, `provider_name`, `model_config_json` ({"model_name": ...}),
//! `created_at`, and token columns — `accumulated_*` variants preferred over
//! the plain ones. Goose has no dedicated reasoning column; the gap between
//! total and input+output is attributed to reasoning.

use crate::paths;
use crate::sqlite;
use crate::trait_::TokenExtractor;
use crate::types::{AgentId, SessionCounts, TokenCounts};
use std::path::{Path, PathBuf};

pub struct Goose;

impl TokenExtractor for Goose {
    fn id(&self) -> AgentId {
        AgentId::Goose
    }

    fn detect(&self) -> bool {
        db_path().is_some()
    }

    fn extract(&self, since: Option<i64>) -> Vec<SessionCounts> {
        let Some(db) = db_path() else {
            return vec![];
        };
        parse_db(&db, since)
    }
}

fn db_path() -> Option<PathBuf> {
    paths::goose_db_paths().into_iter().find(|p| p.exists())
}

/// Normalize goose timestamps: seconds vs milliseconds heuristic.
fn norm_ts(v: Option<i64>) -> Option<i64> {
    match v {
        Some(t) if t <= 0 => None,
        Some(t) if t < 100_000_000_000 => Some(t * 1000),
        other => other,
    }
}

/// Parse a goose sessions.db. Public for fixture tests.
pub fn parse_db(db: &Path, since: Option<i64>) -> Vec<SessionCounts> {
    let conn = match sqlite::open_ro(db) {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!("goose db open failed: {e}");
            return vec![];
        }
    };

    // Column set differs between goose versions — read dynamically.
    let mut stmt = match conn.prepare("SELECT * FROM sessions") {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!("goose db query failed: {e}");
            return vec![];
        }
    };
    let cols: Vec<String> = stmt.column_names().iter().map(|c| c.to_string()).collect();
    let idx = |name: &str| cols.iter().position(|c| c == name);

    let (Some(id_i),) = (idx("id"),) else {
        return vec![];
    };
    let get_i64 = |row: &rusqlite::Row, i: Option<usize>| -> Option<i64> {
        i.and_then(|i| row.get::<_, Option<i64>>(i).ok().flatten())
    };
    let get_str = |row: &rusqlite::Row, i: Option<usize>| -> Option<String> {
        i.and_then(|i| row.get::<_, Option<String>>(i).ok().flatten())
    };

    let provider_i = idx("provider_name");
    let model_cfg_i = idx("model_config_json");
    let created_i = idx("created_at");
    let updated_i = idx("updated_at");
    let dir_i = idx("working_dir");
    let in_i = idx("input_tokens");
    let out_i = idx("output_tokens");
    let total_i = idx("total_tokens");
    let acc_in_i = idx("accumulated_input_tokens");
    let acc_out_i = idx("accumulated_output_tokens");
    let acc_total_i = idx("accumulated_total_tokens");

    let rows = stmt.query_map([], |row| {
        let input = get_i64(row, acc_in_i)
            .or_else(|| get_i64(row, in_i))
            .unwrap_or(0)
            .max(0) as u64;
        let output = get_i64(row, acc_out_i)
            .or_else(|| get_i64(row, out_i))
            .unwrap_or(0)
            .max(0) as u64;
        let total = get_i64(row, acc_total_i)
            .or_else(|| get_i64(row, total_i))
            .unwrap_or(0)
            .max(0) as u64;
        let model_name = get_str(row, model_cfg_i)
            .and_then(|cfg| serde_json::from_str::<serde_json::Value>(&cfg).ok())
            .and_then(|v| {
                v.get("model_name")
                    .and_then(|m| m.as_str())
                    .map(String::from)
            });
        let provider = get_str(row, provider_i);
        let model = match (provider, model_name) {
            (Some(p), Some(m)) => Some(serde_json::json!({"id": m, "providerID": p}).to_string()),
            (None, Some(m)) => Some(m),
            _ => None,
        };
        Ok(SessionCounts {
            agent: AgentId::Goose,
            session_id: row
                .get::<_, Option<String>>(id_i)
                .ok()
                .flatten()
                .unwrap_or_else(|| "unknown".into()),
            model,
            cwd: get_str(row, dir_i),
            started_at_ms: norm_ts(get_i64(row, created_i)),
            last_seen_at_ms: norm_ts(get_i64(row, updated_i)).or(norm_ts(get_i64(row, created_i))),
            counts: TokenCounts {
                input,
                output,
                cache_read: 0,
                cache_write: 0,
                // goose lacks a reasoning column; infer from the total gap
                reasoning: total.saturating_sub(input + output),
            },
            cost: None,
            source_file: Some(db.to_path_buf()),
        })
    });

    let sessions: Vec<SessionCounts> = match rows {
        Ok(iter) => iter.filter_map(|r| r.ok()).collect(),
        Err(_) => vec![],
    };

    sessions
        .into_iter()
        .filter(|s| s.counts.input + s.counts.output + s.counts.reasoning > 0)
        .filter(|s| {
            // Session-granularity window filter (no per-message data in the db)
            since.is_none_or(|cutoff| s.last_seen_at_ms.is_some_and(|t| t >= cutoff))
        })
        .collect()
}
