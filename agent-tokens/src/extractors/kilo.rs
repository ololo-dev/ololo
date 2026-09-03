//! Kilo CLI (Kilo Code's terminal agent, an OpenCode fork).
//!
//! One sqlite store, `kilo.db`, in `${KILO_DATA_DIR:-~/.local/share/kilo}`
//! (ported from ccusage's kilo adapter: `KILO_DATA_DIR` is a comma-separated
//! list of data dirs and, when set, is authoritative). The schema is
//! OpenCode's: a `message` table whose `data` column is the message JSON —
//! assistant rows carry `modelID`, `providerID`, `time.created`, `cost` and a
//! `tokens` block (`input`, `output`, `reasoning`, `cache.{read,write}`,
//! `total`); a `session` table (`id`, `directory`, …) that names the project
//! path; and a `part` table whose `{"type":"tool"}` rows are the tool calls.
//! Only `message` is required — the other two are read when present.

use crate::paths;
use crate::sqlite;
use crate::trait_::TokenExtractor;
use crate::types::{AgentId, SessionCounts, SessionStats, TokenCounts};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

/// Comma-separated list of Kilo data directories; overrides the default
/// location entirely when set (same variable and semantics as ccusage).
pub const KILO_DATA_DIR_ENV: &str = "KILO_DATA_DIR";
pub const KILO_DB_FILE_NAME: &str = "kilo.db";

pub struct Kilo;

impl TokenExtractor for Kilo {
    fn id(&self) -> AgentId {
        AgentId::Kilo
    }

    fn detect(&self) -> bool {
        !db_paths().is_empty()
    }

    fn extract(&self, since: Option<i64>) -> Vec<SessionCounts> {
        // Several data dirs may hold copies of the same rows (ccusage dedupes
        // on the embedded message id, first data dir wins) — so read every
        // store first and aggregate once.
        let mut seen: HashSet<String> = HashSet::new();
        let mut messages = Vec::new();
        let mut dirs: HashMap<(PathBuf, String), Option<String>> = HashMap::new();
        for db in db_paths() {
            let Ok(conn) = open(&db) else {
                continue;
            };
            for (sid, dir) in session_dirs(&conn) {
                dirs.insert((db.clone(), sid), dir);
            }
            for m in read_messages(&conn, &db) {
                if !seen.insert(m.id.clone()) {
                    continue;
                }
                messages.push(m);
            }
        }
        aggregate_counts(messages, since, &dirs)
    }

    fn stats(&self, since: Option<i64>) -> Vec<SessionStats> {
        db_paths()
            .iter()
            .flat_map(|db| parse_stats(db, since))
            .collect()
    }
}

/// Kilo data directories, in priority order. `KILO_DATA_DIR` (comma-separated)
/// is authoritative when set; otherwise `~/.local/share/kilo`. Only existing
/// directories are returned.
pub fn data_dirs() -> Vec<PathBuf> {
    data_dirs_from(
        std::env::var(KILO_DATA_DIR_ENV).ok().as_deref(),
        &paths::home_dir(),
    )
}

/// Pure form of [`data_dirs`] so the override semantics can be tested without
/// touching the process environment.
pub fn data_dirs_from(env_override: Option<&str>, home: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    if let Some(raw) = env_override {
        for dir in raw.split(',').map(str::trim).filter(|s| !s.is_empty()) {
            let dir = PathBuf::from(dir);
            if dir.is_dir() && seen.insert(dir.clone()) {
                out.push(dir);
            }
        }
        return out;
    }
    let dir = home.join(".local").join("share").join("kilo");
    if dir.is_dir() {
        out.push(dir);
    }
    out
}

/// Every `kilo.db` that exists under the data directories.
pub fn db_paths() -> Vec<PathBuf> {
    data_dirs()
        .iter()
        .filter_map(|dir| {
            let db = dir.join(KILO_DB_FILE_NAME);
            db.is_file().then_some(db)
        })
        .collect()
}

/// One parsed `message` row. `tokens` is `Some` only for assistant messages
/// that report a non-zero token block.
#[derive(Debug, Clone)]
pub struct Message {
    /// Embedded message id, else `<db path>:<row id>`.
    pub id: String,
    pub session_id: String,
    pub role: Option<String>,
    pub ts_ms: Option<i64>,
    pub model: Option<String>,
    pub provider: Option<String>,
    /// `path.cwd` when the message records it (OpenCode-family messages do).
    pub cwd: Option<String>,
    pub tokens: Option<TokenCounts>,
    pub cost: Option<f64>,
    pub source: PathBuf,
}

fn open(db: &Path) -> rusqlite::Result<rusqlite::Connection> {
    sqlite::open_ro(db)
        .inspect_err(|e| tracing::warn!("kilo db open failed ({}): {e}", db.display()))
}

/// `session.id -> session.directory`; empty when the table is absent.
fn session_dirs(conn: &rusqlite::Connection) -> Vec<(String, Option<String>)> {
    let Ok(mut stmt) = conn.prepare("SELECT id, directory FROM session") else {
        return vec![];
    };
    let Ok(rows) = stmt.query_map([], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?))
    }) else {
        return vec![];
    };
    rows.flatten().collect()
}

/// Read and parse every `message` row. Public for fixture tests.
pub fn parse_messages(db: &Path) -> Vec<Message> {
    match open(db) {
        Ok(conn) => read_messages(&conn, db),
        Err(_) => vec![],
    }
}

fn read_messages(conn: &rusqlite::Connection, db: &Path) -> Vec<Message> {
    let mut stmt = match conn.prepare("SELECT id, session_id, data FROM message") {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!("kilo db query failed ({}): {e}", db.display());
            return vec![];
        }
    };
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, Option<String>>(0)?,
            r.get::<_, Option<String>>(1)?,
            r.get::<_, Option<String>>(2)?,
        ))
    });
    let Ok(rows) = rows else {
        return vec![];
    };
    rows.flatten()
        .filter_map(|(row_id, row_session, data)| {
            let value = serde_json::from_str::<Value>(&data?).ok()?;
            parse_message(&value, row_id.as_deref(), row_session.as_deref(), db)
        })
        .collect()
}

/// Parse one message JSON payload. `None` when neither the payload nor the
/// row names a session.
pub fn parse_message(
    value: &Value,
    row_id: Option<&str>,
    row_session_id: Option<&str>,
    db: &Path,
) -> Option<Message> {
    let session_id = non_empty(value.get("session_id"))
        .or_else(|| non_empty(value.get("sessionID")))
        .or_else(|| row_session_id.map(String::from))
        .filter(|s| !s.is_empty())?;
    let id = non_empty(value.get("id"))
        .unwrap_or_else(|| format!("{}:{}", db.display(), row_id.unwrap_or("")));
    let role = non_empty(value.get("role"));
    let ts_ms = value
        .get("time")
        .and_then(|t| t.get("created"))
        .and_then(Value::as_i64)
        .and_then(norm_ts);
    let tokens = if role.as_deref() == Some("assistant") {
        tokens_of(value)
    } else {
        None
    };
    Some(Message {
        id,
        session_id,
        role,
        ts_ms,
        model: non_empty(value.get("modelID")),
        provider: non_empty(value.get("providerID")),
        cwd: value
            .get("path")
            .and_then(|p| p.get("cwd"))
            .and_then(Value::as_str)
            .map(String::from),
        tokens,
        cost: value.get("cost").and_then(Value::as_f64),
        source: db.to_path_buf(),
    })
}

fn non_empty(v: Option<&Value>) -> Option<String> {
    v.and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(String::from)
}

/// Kilo writes epoch milliseconds; tolerate seconds and reject non-positive.
fn norm_ts(v: i64) -> Option<i64> {
    if v <= 0 {
        None
    } else if v < 1_000_000_000_000 {
        v.checked_mul(1000)
    } else {
        Some(v)
    }
}

/// The `tokens` block of an assistant message, with ccusage's `total`
/// fallback: whatever `total` exceeds the itemised counts by lands in
/// `output` when that is zero, else in `reasoning`. `None` when the block is
/// missing, not an object, or all zero.
fn tokens_of(value: &Value) -> Option<TokenCounts> {
    let t = value.get("tokens")?.as_object()?;
    let num = |k: &str| t.get(k).and_then(Value::as_u64).unwrap_or(0);
    let (cache_read, cache_write) = match t.get("cache") {
        Some(Value::Object(c)) => (
            c.get("read").and_then(Value::as_u64).unwrap_or(0),
            c.get("write").and_then(Value::as_u64).unwrap_or(0),
        ),
        _ => (0, 0),
    };
    let mut counts = TokenCounts {
        input: num("input"),
        output: num("output"),
        cache_read,
        cache_write,
        reasoning: num("reasoning"),
    };
    let known = counts
        .input
        .saturating_add(counts.output)
        .saturating_add(counts.cache_read)
        .saturating_add(counts.cache_write)
        .saturating_add(counts.reasoning);
    let missing = num("total").saturating_sub(known);
    if missing > 0 {
        if counts.output == 0 {
            counts.output = missing;
        } else {
            counts.reasoning = counts.reasoning.saturating_add(missing);
        }
    }
    (known.saturating_add(missing) > 0).then_some(counts)
}

fn in_window(ts: Option<i64>, since: Option<i64>) -> bool {
    match since {
        None => true,
        Some(cutoff) => ts.is_some_and(|t| t >= cutoff),
    }
}

#[derive(Default)]
struct Agg {
    counts: TokenCounts,
    cost: Option<f64>,
    first_ts: Option<i64>,
    last_ts: Option<i64>,
    /// (timestamp, model, provider) of the latest message naming a model.
    model: Option<(i64, String, Option<String>)>,
    /// (timestamp, cwd) of the latest message naming a cwd.
    cwd: Option<(i64, String)>,
    source: Option<PathBuf>,
}

fn aggregate_counts(
    messages: Vec<Message>,
    since: Option<i64>,
    dirs: &HashMap<(PathBuf, String), Option<String>>,
) -> Vec<SessionCounts> {
    let mut agg: HashMap<String, Agg> = HashMap::new();
    for m in messages {
        // Only timestamped assistant turns with tokens count (as in ccusage).
        let Some(tokens) = m.tokens else {
            continue;
        };
        let Some(ts) = m.ts_ms else {
            continue;
        };
        if !in_window(Some(ts), since) {
            continue;
        }
        let a = agg.entry(m.session_id.clone()).or_default();
        a.counts.input = a.counts.input.saturating_add(tokens.input);
        a.counts.output = a.counts.output.saturating_add(tokens.output);
        a.counts.cache_read = a.counts.cache_read.saturating_add(tokens.cache_read);
        a.counts.cache_write = a.counts.cache_write.saturating_add(tokens.cache_write);
        a.counts.reasoning = a.counts.reasoning.saturating_add(tokens.reasoning);
        if let Some(c) = m.cost {
            a.cost = Some(a.cost.unwrap_or(0.0) + c);
        }
        if a.first_ts.is_none_or(|f| ts < f) {
            a.first_ts = Some(ts);
        }
        if a.last_ts.is_none_or(|l| ts > l) {
            a.last_ts = Some(ts);
        }
        if let Some(model) = m.model
            && a.model.as_ref().is_none_or(|(t, _, _)| ts >= *t)
        {
            a.model = Some((ts, model, m.provider));
        }
        if let Some(cwd) = m.cwd
            && a.cwd.as_ref().is_none_or(|(t, _)| ts >= *t)
        {
            a.cwd = Some((ts, cwd));
        }
        if a.source.is_none() {
            a.source = Some(m.source);
        }
    }

    let mut out: Vec<SessionCounts> = agg
        .into_iter()
        .map(|(session_id, a)| {
            let source = a.source.unwrap_or_default();
            let cwd = dirs
                .get(&(source.clone(), session_id.clone()))
                .cloned()
                .flatten()
                .or_else(|| a.cwd.map(|(_, c)| c));
            SessionCounts {
                agent: AgentId::Kilo,
                session_id,
                model: a.model.map(|(_, id, provider)| model_string(&id, provider)),
                cwd,
                started_at_ms: a.first_ts,
                last_seen_at_ms: a.last_ts,
                counts: a.counts,
                cost: a.cost,
                source_file: Some(source),
            }
        })
        .collect();
    out.sort_by(|x, y| x.session_id.cmp(&y.session_id));
    out
}

/// Same shape the OpenCode extractor emits so the TUI/model cleaners flatten
/// provider + id uniformly; a bare id when the provider is unknown.
fn model_string(id: &str, provider: Option<String>) -> String {
    match provider {
        Some(p) => serde_json::json!({"id": id, "providerID": p}).to_string(),
        None => id.to_string(),
    }
}

/// Parse one `kilo.db` into per-session token counts. Public for fixture tests.
pub fn parse_counts(db: &Path, since: Option<i64>) -> Vec<SessionCounts> {
    let Ok(conn) = open(db) else {
        return vec![];
    };
    let dirs = session_dirs(&conn)
        .into_iter()
        .map(|(sid, dir)| ((db.to_path_buf(), sid), dir))
        .collect();
    let mut seen = HashSet::new();
    let messages = read_messages(&conn, db)
        .into_iter()
        .filter(|m| seen.insert(m.id.clone()))
        .collect();
    aggregate_counts(messages, since, &dirs)
}

/// One `part` row: `(session_id, message_id, time_created, data)`.
type PartRow = (String, Option<String>, Option<i64>, Value);

/// Tool parts live in the `part` table; columns vary by version so they are
/// looked up by name. Empty when the table is missing.
fn read_parts(conn: &rusqlite::Connection) -> Vec<PartRow> {
    let Ok(mut stmt) = conn.prepare("SELECT * FROM part") else {
        return vec![];
    };
    let cols: Vec<String> = stmt.column_names().iter().map(|c| c.to_string()).collect();
    let idx = |name: &str| cols.iter().position(|c| c == name);
    let (Some(session_i), Some(data_i)) = (idx("session_id"), idx("data")) else {
        return vec![];
    };
    let message_i = idx("message_id");
    let created_i = idx("time_created");
    let Ok(rows) = stmt.query_map([], |r| {
        let session_id = r.get::<_, Option<String>>(session_i)?;
        let data = r.get::<_, Option<String>>(data_i)?;
        let message_id = message_i.and_then(|i| r.get::<_, Option<String>>(i).ok().flatten());
        let created = created_i.and_then(|i| r.get::<_, Option<i64>>(i).ok().flatten());
        Ok((session_id, message_id, created, data))
    }) else {
        return vec![];
    };
    rows.flatten()
        .filter_map(|(session_id, message_id, created, data)| {
            let data = serde_json::from_str::<Value>(&data?).ok()?;
            Some((session_id?, message_id, created.and_then(norm_ts), data))
        })
        .collect()
}

/// Parse one `kilo.db` into per-session behavioural stats: user/assistant
/// message counts from `message`, tool calls (and skill loads) from the
/// `part` table. Public for fixture tests.
pub fn parse_stats(db: &Path, since: Option<i64>) -> Vec<SessionStats> {
    let Ok(conn) = open(db) else {
        return vec![];
    };
    let dirs: HashMap<String, Option<String>> = session_dirs(&conn).into_iter().collect();
    let mut sessions: HashMap<String, SessionStats> = HashMap::new();
    let mut message_ts: HashMap<String, i64> = HashMap::new();
    let mut seen = HashSet::new();

    let new_stats = |session_id: &str| {
        let mut s = SessionStats::new(AgentId::Kilo, session_id.to_string());
        s.cwd = dirs.get(session_id).cloned().flatten();
        s.source_file = Some(db.to_path_buf());
        s
    };

    for m in read_messages(&conn, db) {
        if !seen.insert(m.id.clone()) {
            continue;
        }
        if let Some(ts) = m.ts_ms {
            message_ts.insert(m.id.clone(), ts);
        }
        if !in_window(m.ts_ms, since) {
            continue;
        }
        let stats = sessions
            .entry(m.session_id.clone())
            .or_insert_with(|| new_stats(&m.session_id));
        stats.touch_ts(m.ts_ms);
        match m.role.as_deref() {
            Some("user") => stats.user_messages += 1,
            Some("assistant") => stats.assistant_messages += 1,
            _ => {}
        }
        if stats.cwd.is_none() {
            stats.cwd = m.cwd;
        }
    }

    for (session_id, message_id, created, data) in read_parts(&conn) {
        if data.get("type").and_then(Value::as_str) != Some("tool") {
            continue;
        }
        let Some(tool) = data.get("tool").and_then(Value::as_str) else {
            continue;
        };
        let state = data.get("state");
        let ts = created
            .or_else(|| {
                message_id
                    .as_ref()
                    .and_then(|id| message_ts.get(id).copied())
            })
            .or_else(|| {
                state
                    .and_then(|s| s.get("time"))
                    .and_then(|t| t.get("start"))
                    .and_then(Value::as_i64)
                    .and_then(norm_ts)
            });
        if !in_window(ts, since) {
            continue;
        }
        let stats = sessions
            .entry(session_id.clone())
            .or_insert_with(|| new_stats(&session_id));
        stats.touch_ts(ts);
        let args = state.and_then(|s| s.get("input")).unwrap_or(&Value::Null);
        stats.record_tool(tool, args);
    }

    let mut out: Vec<SessionStats> = sessions.into_values().collect();
    out.sort_by(|x, y| x.session_id.cmp(&y.session_id));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    const T0: i64 = 1_767_312_000_000; // 2026-01-02T00:00:00Z
    const HOUR: i64 = 3_600_000;

    /// ccusage's test schema: `message (id, session_id, data)`, no extras.
    fn message_only_db(path: &Path, rows: &[(&str, &str, &str)]) {
        let db = Connection::open(path).unwrap();
        db.execute(
            "CREATE TABLE message (id TEXT, session_id TEXT, data TEXT)",
            [],
        )
        .unwrap();
        for (id, sid, data) in rows {
            db.execute(
                "INSERT INTO message (id, session_id, data) VALUES (?1, ?2, ?3)",
                rusqlite::params![id, sid, data],
            )
            .unwrap();
        }
    }

    fn assistant(id: &str, model: &str, ts: i64, tokens: &str, cost: Option<f64>) -> String {
        let cost = cost.map_or(String::new(), |c| format!(r#","cost":{c}"#));
        format!(
            r#"{{"id":"{id}","role":"assistant","providerID":"anthropic","modelID":"{model}","time":{{"created":{ts}}},"tokens":{tokens}{cost},"agent":"build"}}"#
        )
    }

    fn user(id: &str, ts: i64) -> String {
        format!(
            r#"{{"id":"{id}","role":"user","time":{{"created":{ts}}},"path":{{"cwd":"/work/proj","root":"/work/proj"}}}}"#
        )
    }

    #[test]
    fn parses_counts_from_message_table() {
        let tmp = tempfile::tempdir().unwrap();
        let db = tmp.path().join(KILO_DB_FILE_NAME);
        let a1 = assistant(
            "msg-1",
            "claude-sonnet-4-20250514",
            T0,
            r#"{"input":100,"output":50,"reasoning":5,"cache":{"read":10,"write":20}}"#,
            Some(0.02),
        );
        // cache is not an object -> both cache counts are zero
        let a2 = assistant(
            "msg-2",
            "claude-opus-4-1",
            T0 + HOUR,
            r#"{"input":100,"output":10,"cache":0}"#,
            Some(0.03),
        );
        // only a total -> lands in output (ccusage's total fallback)
        let a3 = assistant(
            "msg-3",
            "claude-opus-4-1",
            T0 + 2 * HOUR,
            r#"{"total":234}"#,
            None,
        );
        // total exceeding the parts with output present -> the gap is reasoning
        let a4 = assistant(
            "msg-4",
            "claude-opus-4-1",
            T0 + 3 * HOUR,
            r#"{"input":10,"output":5,"total":20}"#,
            None,
        );
        // no timestamp -> ignored, as ccusage does
        let no_ts = r#"{"id":"msg-5","role":"assistant","providerID":"openai","modelID":"gpt-5","tokens":{"input":1,"output":1}}"#;
        // all-zero tokens -> ignored
        let zero = assistant(
            "msg-6",
            "gpt-5",
            T0 + 4 * HOUR,
            r#"{"input":0,"output":0}"#,
            None,
        );
        let u = user("msg-0", T0 - 1000);
        message_only_db(
            &db,
            &[
                ("r0", "session-a", &u),
                ("r1", "session-a", &a1),
                ("r2", "session-a", &a2),
                ("r3", "session-a", &a3),
                ("r4", "session-a", &a4),
                ("r5", "session-a", no_ts),
                ("r6", "session-a", &zero),
            ],
        );

        let sessions = parse_counts(&db, None);
        assert_eq!(sessions.len(), 1);
        let s = &sessions[0];
        assert_eq!(s.agent, AgentId::Kilo);
        assert_eq!(s.session_id, "session-a");
        assert_eq!(s.counts.input, 210);
        assert_eq!(s.counts.output, 50 + 10 + 234 + 5);
        assert_eq!(s.counts.reasoning, 5 + 5);
        assert_eq!(s.counts.cache_read, 10);
        assert_eq!(s.counts.cache_write, 20);
        assert!((s.cost.unwrap() - 0.05).abs() < 1e-9);
        assert_eq!(s.started_at_ms, Some(T0));
        assert_eq!(s.last_seen_at_ms, Some(T0 + 3 * HOUR));
        assert_eq!(
            s.model.as_deref(),
            Some(r#"{"id":"claude-opus-4-1","providerID":"anthropic"}"#)
        );
        // no session table: cwd is unknown (the user message's path is not a turn)
        assert_eq!(s.cwd, None);
        assert_eq!(s.source_file.as_deref(), Some(db.as_path()));
    }

    #[test]
    fn cost_is_none_when_no_message_records_one() {
        let tmp = tempfile::tempdir().unwrap();
        let db = tmp.path().join(KILO_DB_FILE_NAME);
        let a = assistant("m", "gpt-5", T0, r#"{"input":1,"output":1}"#, None);
        message_only_db(&db, &[("r1", "s", &a)]);
        let sessions = parse_counts(&db, None);
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].cost, None);
        assert_eq!(
            sessions[0].model.as_deref(),
            Some(r#"{"id":"gpt-5","providerID":"anthropic"}"#)
        );
    }

    #[test]
    fn seconds_timestamps_and_embedded_session_id_are_honoured() {
        let tmp = tempfile::tempdir().unwrap();
        let db = tmp.path().join(KILO_DB_FILE_NAME);
        let secs = T0 / 1000;
        let a = format!(
            r#"{{"role":"assistant","modelID":"gpt-5","session_id":"embedded","time":{{"created":{secs}}},"tokens":{{"input":3,"output":4}}}}"#
        );
        message_only_db(&db, &[("r1", "row-session", &a)]);
        let sessions = parse_counts(&db, None);
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].session_id, "embedded");
        assert_eq!(sessions[0].started_at_ms, Some(T0));
        // no providerID -> bare model id
        assert_eq!(sessions[0].model.as_deref(), Some("gpt-5"));
    }

    #[test]
    fn since_filters_turns_and_drops_empty_sessions() {
        let tmp = tempfile::tempdir().unwrap();
        let db = tmp.path().join(KILO_DB_FILE_NAME);
        let old_a = assistant("a-old", "gpt-5", T0, r#"{"input":100,"output":100}"#, None);
        let new_a = assistant(
            "a-new",
            "gpt-5",
            T0 + 2 * HOUR,
            r#"{"input":7,"output":8}"#,
            None,
        );
        let old_b = assistant(
            "b-old",
            "gpt-5",
            T0 + HOUR,
            r#"{"input":50,"output":50}"#,
            None,
        );
        message_only_db(
            &db,
            &[
                ("r1", "session-a", &old_a),
                ("r2", "session-a", &new_a),
                ("r3", "session-b", &old_b),
            ],
        );

        let all = parse_counts(&db, None);
        assert_eq!(all.len(), 2);

        let windowed = parse_counts(&db, Some(T0 + 2 * HOUR));
        assert_eq!(windowed.len(), 1, "session-b has nothing in the window");
        assert_eq!(windowed[0].session_id, "session-a");
        assert_eq!(windowed[0].counts.input, 7);
        assert_eq!(windowed[0].counts.output, 8);
        assert_eq!(windowed[0].started_at_ms, Some(T0 + 2 * HOUR));

        assert!(parse_counts(&db, Some(T0 + 3 * HOUR)).is_empty());
    }

    /// Full OpenCode-family schema: session + message + part tables.
    fn full_db(path: &Path) {
        let db = Connection::open(path).unwrap();
        db.execute_batch(
            "CREATE TABLE session (id TEXT PRIMARY KEY, directory TEXT, title TEXT, time_created INTEGER, time_updated INTEGER);
             CREATE TABLE message (id TEXT PRIMARY KEY, session_id TEXT, time_created INTEGER, time_updated INTEGER, data TEXT);
             CREATE TABLE part (id TEXT PRIMARY KEY, session_id TEXT, message_id TEXT, time_created INTEGER, time_updated INTEGER, data TEXT);",
        )
        .unwrap();
        db.execute(
            "INSERT INTO session VALUES ('ses-1', '/work/proj', 'title', ?1, ?2)",
            rusqlite::params![T0, T0 + HOUR],
        )
        .unwrap();
        let msgs: Vec<(&str, i64, String)> = vec![
            ("u1", T0, user("u1", T0)),
            (
                "a1",
                T0 + 1000,
                assistant(
                    "a1",
                    "gpt-5",
                    T0 + 1000,
                    r#"{"input":10,"output":5}"#,
                    Some(0.01),
                ),
            ),
            ("u2", T0 + HOUR, user("u2", T0 + HOUR)),
            (
                "a2",
                T0 + HOUR + 1000,
                assistant(
                    "a2",
                    "gpt-5",
                    T0 + HOUR + 1000,
                    r#"{"input":20,"output":6}"#,
                    Some(0.02),
                ),
            ),
        ];
        for (id, ts, data) in &msgs {
            db.execute(
                "INSERT INTO message VALUES (?1, 'ses-1', ?2, ?2, ?3)",
                rusqlite::params![id, ts, data],
            )
            .unwrap();
        }
        let parts: Vec<(&str, &str, i64, &str)> = vec![
            (
                "p1",
                "a1",
                T0 + 1500,
                r#"{"type":"tool","callID":"c1","tool":"read","state":{"status":"completed","input":{"filePath":"/work/.agents/skills/drill-tdd/SKILL.md"}}}"#,
            ),
            (
                "p2",
                "a1",
                T0 + 1600,
                r#"{"type":"tool","callID":"c2","tool":"bash","state":{"status":"completed","input":{"command":"ls"}}}"#,
            ),
            ("p3", "a1", T0 + 1700, r#"{"type":"text","text":"hello"}"#),
            (
                "p4",
                "a2",
                T0 + HOUR + 1500,
                r#"{"type":"tool","callID":"c3","tool":"bash","state":{"status":"completed","input":{"command":"cargo test"}}}"#,
            ),
        ];
        for (id, mid, ts, data) in &parts {
            db.execute(
                "INSERT INTO part VALUES (?1, 'ses-1', ?2, ?3, ?3, ?4)",
                rusqlite::params![id, mid, ts, data],
            )
            .unwrap();
        }
    }

    #[test]
    fn counts_take_cwd_from_session_table() {
        let tmp = tempfile::tempdir().unwrap();
        let db = tmp.path().join(KILO_DB_FILE_NAME);
        full_db(&db);
        let sessions = parse_counts(&db, None);
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].cwd.as_deref(), Some("/work/proj"));
        assert_eq!(sessions[0].counts.input, 30);
        assert_eq!(sessions[0].counts.output, 11);
        assert!((sessions[0].cost.unwrap() - 0.03).abs() < 1e-9);
    }

    #[test]
    fn stats_count_messages_tools_and_skills() {
        let tmp = tempfile::tempdir().unwrap();
        let db = tmp.path().join(KILO_DB_FILE_NAME);
        full_db(&db);

        let stats = parse_stats(&db, None);
        assert_eq!(stats.len(), 1);
        let s = &stats[0];
        assert_eq!(s.agent, AgentId::Kilo);
        assert_eq!(s.session_id, "ses-1");
        assert_eq!(s.cwd.as_deref(), Some("/work/proj"));
        assert_eq!(s.user_messages, 2);
        assert_eq!(s.assistant_messages, 2);
        assert_eq!(s.tool_calls, 3);
        assert_eq!(s.tools.get("read"), Some(&1));
        assert_eq!(s.tools.get("bash"), Some(&2));
        assert_eq!(s.skills.get("drill-tdd"), Some(&1));
        assert_eq!(s.started_at_ms, Some(T0));
        assert_eq!(s.last_seen_at_ms, Some(T0 + HOUR + 1500));
        assert_eq!(s.source_file.as_deref(), Some(db.as_path()));

        let windowed = parse_stats(&db, Some(T0 + HOUR));
        assert_eq!(windowed.len(), 1);
        let w = &windowed[0];
        assert_eq!(w.user_messages, 1);
        assert_eq!(w.assistant_messages, 1);
        assert_eq!(w.tool_calls, 1);
        assert!(w.skills.is_empty());

        assert!(parse_stats(&db, Some(T0 + 2 * HOUR)).is_empty());
    }

    #[test]
    fn stats_without_part_table_still_count_messages() {
        let tmp = tempfile::tempdir().unwrap();
        let db = tmp.path().join(KILO_DB_FILE_NAME);
        let u = user("u1", T0);
        let a = assistant("a1", "gpt-5", T0 + 1, r#"{"input":1,"output":1}"#, None);
        message_only_db(&db, &[("r1", "s", &u), ("r2", "s", &a)]);
        let stats = parse_stats(&db, None);
        assert_eq!(stats.len(), 1);
        assert_eq!(stats[0].user_messages, 1);
        assert_eq!(stats[0].assistant_messages, 1);
        assert_eq!(stats[0].tool_calls, 0);
        // the user message's path.cwd is the only project hint here
        assert_eq!(stats[0].cwd.as_deref(), Some("/work/proj"));
    }

    #[test]
    fn foreign_and_missing_databases_yield_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let garbage = tmp.path().join(KILO_DB_FILE_NAME);
        std::fs::write(&garbage, b"this is not a sqlite database at all\n\x00\xff").unwrap();
        assert!(parse_counts(&garbage, None).is_empty());
        assert!(parse_stats(&garbage, None).is_empty());
        assert!(parse_messages(&garbage).is_empty());

        let missing = tmp.path().join("nope").join(KILO_DB_FILE_NAME);
        assert!(parse_counts(&missing, None).is_empty());
        assert!(parse_stats(&missing, None).is_empty());

        let other = tmp.path().join("other.db");
        let conn = Connection::open(&other).unwrap();
        conn.execute("CREATE TABLE unrelated (x INTEGER)", [])
            .unwrap();
        conn.execute("INSERT INTO unrelated VALUES (1)", [])
            .unwrap();
        drop(conn);
        assert!(parse_counts(&other, None).is_empty());
        assert!(parse_stats(&other, None).is_empty());

        // message table with hostile payloads: bad JSON, NULLs, wrong types
        let hostile = tmp.path().join("hostile.db");
        let conn = Connection::open(&hostile).unwrap();
        conn.execute(
            "CREATE TABLE message (id TEXT, session_id TEXT, data TEXT)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO message VALUES ('r1', 's', 'not json'), (NULL, NULL, NULL), ('r3', 's', '[1,2,3]'), ('r4', NULL, '{\"role\":\"assistant\",\"tokens\":{\"input\":1}}'), ('r5', 's', '{\"role\":\"assistant\",\"time\":{\"created\":\"soon\"},\"tokens\":\"lots\",\"cost\":\"free\"}')",
            [],
        )
        .unwrap();
        drop(conn);
        assert!(parse_counts(&hostile, None).is_empty());
        // r5 is still an assistant message (junk tokens/cost/time are ignored),
        // r3/r4 are not usable rows; nothing panics and no tool is invented.
        let stats = parse_stats(&hostile, None);
        assert_eq!(stats.len(), 1);
        assert_eq!(stats[0].session_id, "s");
        assert_eq!(stats[0].assistant_messages, 1);
        assert_eq!(stats[0].user_messages, 0);
        assert_eq!(stats[0].tool_calls, 0);
        assert_eq!(stats[0].started_at_ms, None);
    }

    #[test]
    fn data_dir_override_is_a_comma_list_and_authoritative() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        let default_dir = home.join(".local/share/kilo");
        std::fs::create_dir_all(&default_dir).unwrap();
        let alt_a = tmp.path().join("a");
        let alt_b = tmp.path().join("b");
        std::fs::create_dir_all(&alt_a).unwrap();
        std::fs::create_dir_all(&alt_b).unwrap();

        assert_eq!(data_dirs_from(None, &home), vec![default_dir.clone()]);
        assert!(data_dirs_from(None, tmp.path()).is_empty());

        let list = format!(
            " {} ,{},{},{}, ,",
            alt_a.display(),
            alt_b.display(),
            alt_a.display(),
            tmp.path().join("missing").display()
        );
        assert_eq!(
            data_dirs_from(Some(&list), &home),
            vec![alt_a.clone(), alt_b.clone()]
        );
        // set but empty: the override wins and names nothing
        assert!(data_dirs_from(Some(""), &home).is_empty());
    }
}
