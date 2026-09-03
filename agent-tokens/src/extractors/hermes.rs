//! Hermes Agent: one sqlite `state.db` per Hermes home. The home is
//! `$HERMES_HOME` (ccusage-compatible: a comma-separated list of homes is
//! accepted, each contributing its own `state.db`) or `~/.hermes`.
//!
//! Ported from ccusage's hermes adapter (MIT). The `sessions` table carries
//! one row per session with lifetime totals: `id`, `model`, `billing_provider`,
//! `started_at` (unix seconds as REAL; ms tolerated), `message_count`,
//! `input_tokens` (uncached), `output_tokens`, `cache_read_tokens`,
//! `cache_write_tokens`, `reasoning_tokens`, `estimated_cost_usd`,
//! `actual_cost_usd`. Rows without a model, or with no tokens and no cost,
//! are not usage and are skipped — same rule as ccusage. There is no
//! per-turn token breakdown, so `since` filters at session granularity.
//!
//! Beyond the reference: when the store also has Hermes's `messages` table
//! (`session_id`, `role`, `tool_calls` JSON in OpenAI function-call shape,
//! `timestamp` unix seconds) it feeds `stats()` and sharpens
//! `last_seen_at_ms`; a store without it still yields counts.

use crate::sqlite;
use crate::trait_::TokenExtractor;
use crate::types::{AgentId, SessionCounts, SessionStats, TokenCounts};
use rusqlite::types::Value;
use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

pub struct Hermes;

const HERMES_HOME_ENV: &str = "HERMES_HOME";

impl TokenExtractor for Hermes {
    fn id(&self) -> AgentId {
        AgentId::Hermes
    }

    fn detect(&self) -> bool {
        !state_db_paths().is_empty()
    }

    fn extract(&self, since: Option<i64>) -> Vec<SessionCounts> {
        // A session id seen in an earlier home wins, as in ccusage.
        let mut seen = HashSet::new();
        state_db_paths()
            .iter()
            .flat_map(|db| parse_db(db, since))
            .filter(|s| seen.insert(s.session_id.clone()))
            .collect()
    }

    fn stats(&self, since: Option<i64>) -> Vec<SessionStats> {
        let mut seen = HashSet::new();
        state_db_paths()
            .iter()
            .flat_map(|db| parse_stats_db(db, since))
            .filter(|s| seen.insert(s.session_id.clone()))
            .collect()
    }
}

/// Existing `state.db` files, in home order, deduplicated.
fn state_db_paths() -> Vec<PathBuf> {
    let env = std::env::var(HERMES_HOME_ENV).ok();
    state_db_paths_for(env.as_deref(), &crate::paths::home_dir())
}

/// Discovery rule with its inputs made explicit so tests need not mutate the
/// process environment: `hermes_home` is the raw `HERMES_HOME` value.
fn state_db_paths_for(hermes_home: Option<&str>, home: &Path) -> Vec<PathBuf> {
    let homes: Vec<PathBuf> = match hermes_home {
        Some(list) => list
            .split(',')
            .map(str::trim)
            .filter(|p| !p.is_empty())
            .map(PathBuf::from)
            .collect(),
        None => vec![home.join(".hermes")],
    };
    let mut seen = HashSet::new();
    homes
        .into_iter()
        .map(|h| h.join("state.db"))
        .filter(|p| p.is_file())
        .filter(|p| seen.insert(p.clone()))
        .collect()
}

/// Coerce a sqlite value to a non-negative integer the way the reference
/// does: integers clamp at zero, finite positive reals truncate, numeric
/// text parses, anything else is zero.
fn value_u64(v: &Value) -> u64 {
    match v {
        Value::Integer(i) => (*i).max(0) as u64,
        Value::Real(f) if f.is_finite() && *f > 0.0 => f.trunc() as u64,
        Value::Text(s) => {
            let s = s.trim();
            s.parse::<i64>()
                .ok()
                .map(|i| i.max(0) as u64)
                .or_else(|| s.parse::<f64>().ok().map(|f| value_u64(&Value::Real(f))))
                .unwrap_or(0)
        }
        _ => 0,
    }
}

fn value_f64(v: &Value) -> Option<f64> {
    match v {
        Value::Integer(i) => Some(*i as f64),
        Value::Real(f) if f.is_finite() => Some(*f),
        Value::Text(s) => s.trim().parse::<f64>().ok().filter(|f| f.is_finite()),
        _ => None,
    }
}

fn value_str(v: &Value) -> Option<String> {
    match v {
        Value::Text(s) => Some(s.clone()),
        _ => None,
    }
}

/// Hermes stores unix seconds as REAL; a value already in milliseconds is
/// recognised by magnitude. Non-positive stamps are absent.
fn timestamp_ms(value: f64) -> Option<i64> {
    if !value.is_finite() {
        return None;
    }
    let millis = if value > 1e12 { value } else { value * 1000.0 };
    (millis > 0.0).then(|| millis.trunc() as i64)
}

/// ccusage's provider normalisation, so the same billing provider spells the
/// same way whichever adapter reported it.
fn normalize_provider(value: Option<&str>, model: &str) -> Option<String> {
    let Some(value) = value.map(str::trim).filter(|v| !v.is_empty()) else {
        return infer_provider_from_model(model).map(String::from);
    };
    let normalized = value.to_ascii_lowercase().replace('-', "_");
    Some(
        match normalized.as_str() {
            "anthropic" | "claude" => "anthropic",
            "openai" | "openai_codex" => "openai",
            "google" | "google_ai" | "gemini" | "vertex" | "vertex_ai" => "google",
            other => other,
        }
        .to_string(),
    )
}

fn infer_provider_from_model(model: &str) -> Option<&'static str> {
    let model = model.to_ascii_lowercase();
    if model.starts_with("claude-") || model.starts_with("claude/") {
        Some("anthropic")
    } else if model.starts_with("gpt")
        || model.starts_with("chatgpt")
        || model.starts_with('o') && model.as_bytes().get(1).is_some_and(u8::is_ascii_digit)
    {
        Some("openai")
    } else if model.starts_with("gemini-") || model.starts_with("gemini/") {
        Some("google")
    } else {
        None
    }
}

/// A `SELECT *` with columns resolved by name, so a store from another
/// Hermes version (or another program entirely) degrades to "no rows"
/// instead of a failed prepare or a panic.
struct Table {
    cols: Vec<String>,
    rows: Vec<Vec<Value>>,
}

impl Table {
    fn load(conn: &rusqlite::Connection, table: &str) -> Option<Table> {
        let mut stmt = conn.prepare(&format!("SELECT * FROM {table}")).ok()?;
        let cols: Vec<String> = stmt.column_names().iter().map(|c| c.to_string()).collect();
        let n = cols.len();
        let rows = stmt
            .query_map([], |row| {
                (0..n)
                    .map(|i| row.get::<_, Value>(i))
                    .collect::<rusqlite::Result<Vec<Value>>>()
            })
            .ok()?
            .filter_map(|r| r.ok())
            .collect();
        Some(Table { cols, rows })
    }

    fn idx(&self, name: &str) -> Option<usize> {
        self.cols.iter().position(|c| c == name)
    }
}

fn cell(row: &[Value], i: Option<usize>) -> &Value {
    i.and_then(|i| row.get(i)).unwrap_or(&Value::Null)
}

/// Latest message timestamp per session, when the store has a `messages`
/// table. Absent table or columns: empty map.
fn last_message_ts(conn: &rusqlite::Connection) -> BTreeMap<String, i64> {
    let mut out = BTreeMap::new();
    let Some(messages) = Table::load(conn, "messages") else {
        return out;
    };
    let (Some(sid_i), Some(ts_i)) = (messages.idx("session_id"), messages.idx("timestamp")) else {
        return out;
    };
    for row in &messages.rows {
        let Some(sid) = value_str(cell(row, Some(sid_i))) else {
            continue;
        };
        let Some(ts) = value_f64(cell(row, Some(ts_i))).and_then(timestamp_ms) else {
            continue;
        };
        out.entry(sid)
            .and_modify(|t: &mut i64| *t = (*t).max(ts))
            .or_insert(ts);
    }
    out
}

/// Parse one Hermes `state.db` into per-session token counts. Public for
/// fixture tests.
pub fn parse_db(db: &Path, since: Option<i64>) -> Vec<SessionCounts> {
    let conn = match sqlite::open_ro(db) {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!("hermes db open failed: {e}");
            return vec![];
        }
    };
    let Some(sessions) = Table::load(&conn, "sessions") else {
        tracing::warn!("hermes db has no readable sessions table: {}", db.display());
        return vec![];
    };
    let (Some(id_i), Some(model_i), Some(started_i)) = (
        sessions.idx("id"),
        sessions.idx("model"),
        sessions.idx("started_at"),
    ) else {
        return vec![];
    };
    let ended_i = sessions.idx("ended_at");
    let provider_i = sessions.idx("billing_provider");
    let input_i = sessions.idx("input_tokens");
    let output_i = sessions.idx("output_tokens");
    let cache_read_i = sessions.idx("cache_read_tokens");
    let cache_write_i = sessions.idx("cache_write_tokens");
    let reasoning_i = sessions.idx("reasoning_tokens");
    let estimated_i = sessions.idx("estimated_cost_usd");
    let actual_i = sessions.idx("actual_cost_usd");
    let cwd_i = sessions
        .idx("working_directory")
        .or_else(|| sessions.idx("cwd"));

    let last_message = last_message_ts(&conn);
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for row in &sessions.rows {
        let Some(session_id) = value_str(cell(row, Some(id_i))).filter(|s| !s.is_empty()) else {
            continue;
        };
        let Some(model) = value_str(cell(row, Some(model_i)))
            .map(|m| m.trim().to_string())
            .filter(|m| !m.is_empty())
        else {
            continue;
        };
        let Some(started_at_ms) = value_f64(cell(row, Some(started_i))).and_then(timestamp_ms)
        else {
            continue;
        };
        let counts = TokenCounts {
            input: value_u64(cell(row, input_i)),
            output: value_u64(cell(row, output_i)),
            cache_read: value_u64(cell(row, cache_read_i)),
            cache_write: value_u64(cell(row, cache_write_i)),
            reasoning: value_u64(cell(row, reasoning_i)),
        };
        let estimated = value_f64(cell(row, estimated_i)).map(|c| c.max(0.0));
        let actual = value_f64(cell(row, actual_i)).map(|c| c.max(0.0));
        let cost_usd = actual.or(estimated);
        let no_tokens = counts.input == 0
            && counts.output == 0
            && counts.cache_read == 0
            && counts.cache_write == 0
            && counts.reasoning == 0;
        if no_tokens && cost_usd.unwrap_or(0.0) == 0.0 {
            continue;
        }
        if !seen.insert(session_id.clone()) {
            continue;
        }
        let ended_at_ms = value_f64(cell(row, ended_i)).and_then(timestamp_ms);
        let last_seen_at_ms = [
            Some(started_at_ms),
            ended_at_ms,
            last_message.get(&session_id).copied(),
        ]
        .into_iter()
        .flatten()
        .max();
        if let Some(cutoff) = since
            && last_seen_at_ms.is_some_and(|t| t < cutoff)
        {
            continue;
        }
        let provider = normalize_provider(value_str(cell(row, provider_i)).as_deref(), &model);
        let model = match provider {
            Some(p) => serde_json::json!({"id": model, "providerID": p}).to_string(),
            None => model,
        };
        out.push(SessionCounts {
            agent: AgentId::Hermes,
            session_id,
            model: Some(model),
            cwd: value_str(cell(row, cwd_i)).filter(|c| !c.trim().is_empty()),
            started_at_ms: Some(started_at_ms),
            last_seen_at_ms,
            counts,
            // A recorded zero means "included in a subscription"; leave it to
            // central pricing like any other unpriced session.
            cost: cost_usd.filter(|c| *c > 0.0),
            source_file: Some(db.to_path_buf()),
        });
    }
    out
}

/// Parse one Hermes `state.db` into behavioural stats from its `messages`
/// table. Public for fixture tests. A store without that table yields nothing.
pub fn parse_stats_db(db: &Path, since: Option<i64>) -> Vec<SessionStats> {
    let conn = match sqlite::open_ro(db) {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!("hermes db open failed: {e}");
            return vec![];
        }
    };
    let Some(messages) = Table::load(&conn, "messages") else {
        return vec![];
    };
    let (Some(sid_i), Some(role_i)) = (messages.idx("session_id"), messages.idx("role")) else {
        return vec![];
    };
    let ts_i = messages.idx("timestamp");
    let tool_calls_i = messages.idx("tool_calls");

    // Session metadata (cwd, start) comes from the sessions table when present.
    let mut cwd_by_session: BTreeMap<String, String> = BTreeMap::new();
    if let Some(sessions) = Table::load(&conn, "sessions")
        && let (Some(id_i), Some(cwd_i)) = (
            sessions.idx("id"),
            sessions
                .idx("working_directory")
                .or_else(|| sessions.idx("cwd")),
        )
    {
        for row in &sessions.rows {
            if let (Some(id), Some(cwd)) = (
                value_str(cell(row, Some(id_i))),
                value_str(cell(row, Some(cwd_i))).filter(|c| !c.trim().is_empty()),
            ) {
                cwd_by_session.insert(id, cwd);
            }
        }
    }

    let mut by_session: BTreeMap<String, SessionStats> = BTreeMap::new();
    for row in &messages.rows {
        let Some(sid) = value_str(cell(row, Some(sid_i))).filter(|s| !s.is_empty()) else {
            continue;
        };
        let ts = value_f64(cell(row, ts_i)).and_then(timestamp_ms);
        if let Some(cutoff) = since
            && ts.is_some_and(|t| t < cutoff)
        {
            continue;
        }
        let role = value_str(cell(row, Some(role_i))).unwrap_or_default();
        let stats = by_session.entry(sid.clone()).or_insert_with(|| {
            let mut s = SessionStats::new(AgentId::Hermes, sid.clone());
            s.cwd = cwd_by_session.get(&sid).cloned();
            s.source_file = Some(db.to_path_buf());
            s
        });
        match role.trim() {
            "user" => {
                stats.user_messages += 1;
                stats.touch_ts(ts);
            }
            "assistant" => {
                stats.assistant_messages += 1;
                stats.touch_ts(ts);
                for (name, args) in tool_calls(cell(row, tool_calls_i)) {
                    stats.record_tool(&name, &args);
                }
            }
            _ => {}
        }
    }
    by_session.into_values().collect()
}

/// Decode the `tool_calls` column: a JSON array in OpenAI function-call shape
/// (`{"function":{"name","arguments":"<json string>"}}`), with a flat
/// `{"name","arguments"}` tolerated. Anything else is no tool calls.
fn tool_calls(v: &Value) -> Vec<(String, serde_json::Value)> {
    let Some(text) = value_str(v) else {
        return vec![];
    };
    let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&text) else {
        return vec![];
    };
    let Some(calls) = parsed.as_array() else {
        return vec![];
    };
    calls
        .iter()
        .filter_map(|call| {
            let func = call.get("function").unwrap_or(call);
            let name = func.get("name").and_then(|n| n.as_str())?;
            let args = match func.get("arguments") {
                Some(serde_json::Value::String(s)) => {
                    serde_json::from_str(s).unwrap_or(serde_json::Value::Null)
                }
                Some(other) => other.clone(),
                None => serde_json::Value::Null,
            };
            Some((name.to_string(), args))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::{Connection, params};

    /// The `sessions` schema from ccusage's loader tests (Hermes's real one has
    /// more columns; every extra is ignored), plus Hermes's `messages` table.
    fn create_state_db(path: &Path, with_messages: bool) -> Connection {
        let conn = Connection::open(path).unwrap();
        conn.execute_batch(
            "CREATE TABLE sessions (
                id TEXT PRIMARY KEY,
                source TEXT NOT NULL,
                model TEXT,
                started_at REAL NOT NULL,
                ended_at REAL,
                message_count INTEGER DEFAULT 0,
                input_tokens INTEGER DEFAULT 0,
                output_tokens INTEGER DEFAULT 0,
                cache_read_tokens INTEGER DEFAULT 0,
                cache_write_tokens INTEGER DEFAULT 0,
                reasoning_tokens INTEGER DEFAULT 0,
                billing_provider TEXT,
                estimated_cost_usd REAL,
                actual_cost_usd REAL
            );",
        )
        .unwrap();
        if with_messages {
            conn.execute_batch(
                "CREATE TABLE messages (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    session_id TEXT NOT NULL,
                    role TEXT NOT NULL,
                    content TEXT,
                    tool_calls TEXT,
                    tool_call_id TEXT,
                    timestamp REAL NOT NULL
                );",
            )
            .unwrap();
        }
        conn
    }

    #[allow(clippy::too_many_arguments)]
    fn insert_session(
        conn: &Connection,
        id: &str,
        model: &str,
        started_at: f64,
        tokens: (i64, i64, i64, i64, i64),
        provider: Option<&str>,
        estimated: Option<f64>,
        actual: Option<f64>,
    ) {
        conn.execute(
            "INSERT INTO sessions (
                id, source, model, started_at, message_count,
                input_tokens, output_tokens, cache_read_tokens, cache_write_tokens, reasoning_tokens,
                billing_provider, estimated_cost_usd, actual_cost_usd
            ) VALUES (?1, 'cli', ?2, ?3, 42, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                id, model, started_at, tokens.0, tokens.1, tokens.2, tokens.3, tokens.4, provider,
                estimated, actual
            ],
        )
        .unwrap();
    }

    fn insert_message(
        conn: &Connection,
        session_id: &str,
        role: &str,
        tool_calls: Option<&str>,
        timestamp: f64,
    ) {
        conn.execute(
            "INSERT INTO messages (session_id, role, content, tool_calls, timestamp)
             VALUES (?1, ?2, 'x', ?3, ?4)",
            params![session_id, role, tool_calls, timestamp],
        )
        .unwrap();
    }

    #[test]
    fn loads_billable_sessions_from_state_db() {
        // Mirrors ccusage's `loads_billable_hermes_sessions_from_state_db`.
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("state.db");
        let conn = create_state_db(&db, false);
        insert_session(
            &conn,
            "session-1",
            "claude-sonnet-4-20250514",
            1_750_000_000.25,
            (1200, 300, 50, 20, 10),
            Some("anthropic"),
            Some(0.12),
            Some(0.34),
        );
        drop(conn);

        let sessions = parse_db(&db, None);
        assert_eq!(sessions.len(), 1);
        let s = &sessions[0];
        assert_eq!(s.agent, AgentId::Hermes);
        assert_eq!(s.session_id, "session-1");
        assert_eq!(
            s.model.as_deref(),
            Some(r#"{"id":"claude-sonnet-4-20250514","providerID":"anthropic"}"#)
        );
        assert_eq!(s.counts.input, 1200);
        assert_eq!(s.counts.output, 300);
        assert_eq!(s.counts.cache_read, 50);
        assert_eq!(s.counts.cache_write, 20);
        assert_eq!(s.counts.reasoning, 10);
        // actual cost beats the estimate; seconds become milliseconds
        assert_eq!(s.cost, Some(0.34));
        assert_eq!(s.started_at_ms, Some(1_750_000_000_250));
        assert_eq!(s.last_seen_at_ms, Some(1_750_000_000_250));
        assert_eq!(s.cwd, None);
        assert_eq!(s.source_file.as_deref(), Some(db.as_path()));
    }

    #[test]
    fn skips_rows_that_are_not_usage() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("state.db");
        let conn = create_state_db(&db, false);
        // no model
        insert_session(
            &conn,
            "no-model",
            "  ",
            1.7e9,
            (10, 1, 0, 0, 0),
            None,
            None,
            None,
        );
        // no tokens and no cost
        insert_session(
            &conn,
            "empty",
            "gpt-5.5",
            1.7e9,
            (0, 0, 0, 0, 0),
            None,
            Some(0.0),
            None,
        );
        // no tokens but a recorded cost still counts (ccusage rule)
        insert_session(
            &conn,
            "cost-only",
            "gpt-5.5",
            1.7e9,
            (0, 0, 0, 0, 0),
            None,
            Some(0.05),
            None,
        );
        // zero-dollar subscription usage: tokens kept, cost left for pricing
        insert_session(
            &conn,
            "subscription",
            "gpt-5.5",
            1_750_000_000_250.0, // already milliseconds
            (244_075, 10_019, 3_339_776, 0, 3_216),
            None,
            Some(0.0),
            Some(0.0),
        );
        // negative token counts clamp to zero and never underflow
        insert_session(
            &conn,
            "negative",
            "grok-4.3",
            1.7e9,
            (-5, 7, -1, 0, 0),
            Some("xAI"),
            None,
            None,
        );
        drop(conn);

        let sessions = parse_db(&db, None);
        let ids: Vec<&str> = sessions.iter().map(|s| s.session_id.as_str()).collect();
        assert_eq!(ids, ["cost-only", "subscription", "negative"]);

        let cost_only = &sessions[0];
        assert_eq!(cost_only.cost, Some(0.05));
        // provider inferred from the model name when billing_provider is absent
        assert_eq!(
            cost_only.model.as_deref(),
            Some(r#"{"id":"gpt-5.5","providerID":"openai"}"#)
        );

        let sub = &sessions[1];
        assert_eq!(sub.cost, None);
        assert_eq!(sub.counts.cache_read, 3_339_776);
        assert_eq!(sub.started_at_ms, Some(1_750_000_000_250));

        let neg = &sessions[2];
        assert_eq!(
            (neg.counts.input, neg.counts.output, neg.counts.cache_read),
            (0, 7, 0)
        );
        assert_eq!(
            neg.model.as_deref(),
            Some(r#"{"id":"grok-4.3","providerID":"xai"}"#)
        );
    }

    #[test]
    fn unknown_provider_keeps_the_bare_model_id() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("state.db");
        let conn = create_state_db(&db, false);
        insert_session(
            &conn,
            "s",
            "hermes-local-7b",
            1.7e9,
            (1, 1, 0, 0, 0),
            None,
            None,
            None,
        );
        drop(conn);
        let sessions = parse_db(&db, None);
        assert_eq!(sessions[0].model.as_deref(), Some("hermes-local-7b"));
    }

    #[test]
    fn since_filters_at_session_granularity_and_messages_extend_last_seen() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("state.db");
        let conn = create_state_db(&db, true);
        // old session, nothing after the cutoff
        insert_session(
            &conn,
            "old",
            "gpt-5.5",
            1_000.0,
            (10, 5, 0, 0, 0),
            None,
            None,
            None,
        );
        // started before the cutoff but still active after it: its messages say so
        insert_session(
            &conn,
            "active",
            "gpt-5.5",
            1_000.0,
            (20, 5, 0, 0, 0),
            None,
            None,
            None,
        );
        insert_message(&conn, "active", "user", None, 1_000.0);
        insert_message(&conn, "active", "assistant", None, 5_000.0);
        // started after the cutoff
        insert_session(
            &conn,
            "new",
            "gpt-5.5",
            4_000.0,
            (30, 5, 0, 0, 0),
            None,
            None,
            None,
        );
        drop(conn);

        let all = parse_db(&db, None);
        assert_eq!(all.len(), 3);
        let active = all.iter().find(|s| s.session_id == "active").unwrap();
        assert_eq!(active.started_at_ms, Some(1_000_000));
        assert_eq!(active.last_seen_at_ms, Some(5_000_000));

        let cutoff = 3_000_000; // epoch ms
        let recent = parse_db(&db, Some(cutoff));
        let mut ids: Vec<&str> = recent.iter().map(|s| s.session_id.as_str()).collect();
        ids.sort_unstable();
        assert_eq!(ids, ["active", "new"]);
        // no per-turn split exists, so the whole session's totals are reported
        assert_eq!(
            recent
                .iter()
                .find(|s| s.session_id == "active")
                .unwrap()
                .counts
                .input,
            20
        );

        assert!(parse_db(&db, Some(10_000_000)).is_empty());
    }

    #[test]
    fn stats_count_messages_tools_and_skills() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("state.db");
        let conn = create_state_db(&db, true);
        insert_session(
            &conn,
            "s1",
            "gpt-5.5",
            1_000.0,
            (10, 5, 0, 0, 0),
            None,
            None,
            None,
        );
        insert_message(&conn, "s1", "system", None, 999.0);
        insert_message(&conn, "s1", "user", None, 1_000.0);
        insert_message(
            &conn,
            "s1",
            "assistant",
            Some(
                r#"[{"id":"call_1","type":"function","function":{"name":"terminal","arguments":"{\"command\":\"ls\"}"}},
                    {"id":"call_2","type":"function","function":{"name":"read_file","arguments":"{\"path\":\"/home/u/.hermes/skills/drill-tdd/SKILL.md\"}"}}]"#,
            ),
            1_001.0,
        );
        insert_message(&conn, "s1", "tool", None, 1_002.0);
        insert_message(&conn, "s1", "user", None, 2_000.0);
        // flat shape and unparsable JSON are tolerated
        insert_message(
            &conn,
            "s1",
            "assistant",
            Some(r#"[{"name":"terminal","arguments":{"command":"pwd"}}]"#),
            2_001.0,
        );
        insert_message(&conn, "s1", "assistant", Some("not json"), 2_002.0);
        drop(conn);

        let stats = parse_stats_db(&db, None);
        assert_eq!(stats.len(), 1);
        let s = &stats[0];
        assert_eq!(s.agent, AgentId::Hermes);
        assert_eq!(s.session_id, "s1");
        assert_eq!(s.user_messages, 2);
        assert_eq!(s.assistant_messages, 3);
        assert_eq!(s.tool_calls, 3);
        assert_eq!(s.tools.get("terminal"), Some(&2));
        assert_eq!(s.tools.get("read_file"), Some(&1));
        assert_eq!(s.skills.get("drill-tdd"), Some(&1));
        assert_eq!(s.started_at_ms, Some(1_000_000));
        assert_eq!(s.last_seen_at_ms, Some(2_002_000));
        assert_eq!(s.source_file.as_deref(), Some(db.as_path()));

        // window: only the second exchange
        let recent = parse_stats_db(&db, Some(1_500_000));
        assert_eq!(recent.len(), 1);
        assert_eq!(recent[0].user_messages, 1);
        assert_eq!(recent[0].assistant_messages, 2);
        assert_eq!(recent[0].tool_calls, 1);
        assert!(recent[0].skills.is_empty());

        assert!(parse_stats_db(&db, Some(9_000_000)).is_empty());
    }

    #[test]
    fn stats_are_empty_without_a_messages_table() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("state.db");
        let conn = create_state_db(&db, false);
        insert_session(
            &conn,
            "s1",
            "gpt-5.5",
            1_000.0,
            (10, 5, 0, 0, 0),
            None,
            None,
            None,
        );
        drop(conn);
        assert!(parse_stats_db(&db, None).is_empty());
        // counts are unaffected by the missing table
        assert_eq!(parse_db(&db, None).len(), 1);
    }

    #[test]
    fn foreign_or_broken_databases_yield_nothing() {
        let dir = tempfile::tempdir().unwrap();

        // a sqlite file from some other program
        let foreign = dir.path().join("foreign.db");
        let conn = Connection::open(&foreign).unwrap();
        conn.execute_batch("CREATE TABLE notes (id INTEGER PRIMARY KEY, body TEXT);")
            .unwrap();
        drop(conn);
        assert!(parse_db(&foreign, None).is_empty());
        assert!(parse_stats_db(&foreign, None).is_empty());

        // a sessions table with the wrong columns
        let odd = dir.path().join("odd.db");
        let conn = Connection::open(&odd).unwrap();
        conn.execute_batch(
            "CREATE TABLE sessions (pk INTEGER PRIMARY KEY, name TEXT);
             INSERT INTO sessions (name) VALUES ('x');
             CREATE TABLE messages (body TEXT);
             INSERT INTO messages (body) VALUES ('y');",
        )
        .unwrap();
        drop(conn);
        assert!(parse_db(&odd, None).is_empty());
        assert!(parse_stats_db(&odd, None).is_empty());

        // type-confused cells never panic
        let typed = dir.path().join("typed.db");
        let conn = create_state_db(&typed, true);
        conn.execute(
            "INSERT INTO sessions (id, source, model, started_at, input_tokens, output_tokens, actual_cost_usd)
             VALUES ('t', 'cli', 'gpt-5.5', 'soon', 'many', 3.9, 'free')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO sessions (id, source, model, started_at, input_tokens)
             VALUES ('u', 'cli', 'gpt-5.5', '1700', '12')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO messages (session_id, role, tool_calls, timestamp) VALUES (7, 'assistant', 42, 'never')",
            [],
        )
        .unwrap();
        drop(conn);
        let sessions = parse_db(&typed, None);
        // 't' has an unparsable start time and is dropped; 'u' parses its text cells
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].session_id, "u");
        assert_eq!(sessions[0].counts.input, 12);
        assert_eq!(sessions[0].started_at_ms, Some(1_700_000));
        // the message row survives as text cells: no tools, no timestamps, no panic
        let stats = parse_stats_db(&typed, None);
        assert_eq!(stats.len(), 1);
        assert_eq!(stats[0].session_id, "7");
        assert_eq!(stats[0].assistant_messages, 1);
        assert_eq!(stats[0].tool_calls, 0);
        assert_eq!(stats[0].last_seen_at_ms, None);

        // not sqlite at all, and not there at all
        let text = dir.path().join("state.db");
        std::fs::write(&text, "hello").unwrap();
        assert!(parse_db(&text, None).is_empty());
        assert!(parse_db(&dir.path().join("missing.db"), None).is_empty());
    }

    #[test]
    fn discovery_honours_hermes_home_list_and_falls_back_to_dot_hermes() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let a = dir.path().join("a");
        let b = dir.path().join("b");
        for h in [&home.join(".hermes"), &a, &b] {
            std::fs::create_dir_all(h).unwrap();
        }
        std::fs::write(home.join(".hermes/state.db"), "").unwrap();
        std::fs::write(a.join("state.db"), "").unwrap();
        // `b` has a directory named state.db: not a file, not a store
        std::fs::create_dir_all(b.join("state.db")).unwrap();

        assert_eq!(
            state_db_paths_for(None, &home),
            vec![home.join(".hermes/state.db")]
        );
        let list = format!(
            " {} ,{},, {} ,{}",
            a.display(),
            b.display(),
            a.display(),
            dir.path().join("nope").display()
        );
        assert_eq!(
            state_db_paths_for(Some(&list), &home),
            vec![a.join("state.db")]
        );
        assert!(state_db_paths_for(Some(""), &home).is_empty());
    }

    #[test]
    fn sessions_seen_in_an_earlier_home_win() {
        let dir = tempfile::tempdir().unwrap();
        let first = dir.path().join("first.db");
        let second = dir.path().join("second.db");
        for (db, input) in [(&first, 1), (&second, 2)] {
            let conn = create_state_db(db, false);
            insert_session(
                &conn,
                "dup",
                "gpt-5.5",
                1_000.0,
                (input, 0, 0, 0, 0),
                None,
                None,
                None,
            );
        }
        let mut seen = HashSet::new();
        let merged: Vec<SessionCounts> = [first, second]
            .iter()
            .flat_map(|db| parse_db(db, None))
            .filter(|s| seen.insert(s.session_id.clone()))
            .collect();
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].counts.input, 1);
    }

    #[test]
    fn timestamp_units_are_recognised_by_magnitude() {
        assert_eq!(timestamp_ms(1_750_000_000.25), Some(1_750_000_000_250));
        assert_eq!(timestamp_ms(1_750_000_000_250.0), Some(1_750_000_000_250));
        assert_eq!(timestamp_ms(0.0), None);
        assert_eq!(timestamp_ms(-5.0), None);
        assert_eq!(timestamp_ms(f64::NAN), None);
        assert_eq!(timestamp_ms(f64::INFINITY), None);
    }
}
