//! ZCode CLI: one sqlite ledger at `$ZCODE_HOME/cli/db/db.sqlite` (default
//! root `~/.zcode`; `ZCODE_HOME` may list several comma-separated roots).
//! Ported from ccusage's `adapters/zcode` (MIT).
//!
//! The store keeps one `model_usage` row per completed model request:
//! `id`, `session_id`, `started_at` (epoch ms), `model_id`, optional
//! `provider_id`, `status`, `input_tokens` (inclusive of both cache slices),
//! `output_tokens`, optional `cache_creation_input_tokens`,
//! `cache_read_input_tokens` and `computed_total_tokens`. The `session` table
//! carries `id`, `directory` (the project cwd) and, in newer layouts, a
//! `version`. Only `status = 'completed'` rows count. There is no per-request
//! cost, no message transcript and no tool-call record, so `stats` stays at
//! the trait default.

use crate::paths;
use crate::sqlite;
use crate::trait_::TokenExtractor;
use crate::types::{AgentId, SessionCounts, TokenCounts};
use std::collections::{BTreeMap, HashSet};
use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// Same override ccusage honours: one root, or several separated by commas.
const ZCODE_HOME_ENV: &str = "ZCODE_HOME";
const ZCODE_DB_RELATIVE_PATH: &str = "cli/db/db.sqlite";

const REQUIRED_MODEL_COLUMNS: &[&str] = &[
    "id",
    "session_id",
    "started_at",
    "model_id",
    "status",
    "input_tokens",
    "output_tokens",
];
const REQUIRED_SESSION_COLUMNS: &[&str] = &["id", "directory"];

pub struct ZCode;

impl TokenExtractor for ZCode {
    fn id(&self) -> AgentId {
        AgentId::ZCode
    }

    fn detect(&self) -> bool {
        !db_paths().is_empty()
    }

    fn extract(&self, since: Option<i64>) -> Vec<SessionCounts> {
        // Several configured roots may hold copies of the same ledger; a
        // usage id is counted once, the first configured root winning.
        let mut seen = HashSet::new();
        let mut rows = Vec::new();
        for db in db_paths() {
            for row in read_rows(&db) {
                if seen.insert(row.id.clone()) {
                    rows.push(row);
                }
            }
        }
        aggregate(rows, since)
    }
}

/// ZCode roots: the non-empty entries of `ZCODE_HOME` when it is set to
/// something, else `<home>/.zcode`. Mirrors ccusage: a configured-but-invalid
/// root list does not fall back to the default.
fn zcode_roots() -> Vec<PathBuf> {
    roots_from(std::env::var_os(ZCODE_HOME_ENV), paths::home_dir())
}

fn roots_from(configured: Option<OsString>, home: PathBuf) -> Vec<PathBuf> {
    if let Some(raw) = configured {
        let configured = configured_roots(raw);
        if !configured.is_empty() {
            return unique_dirs(configured);
        }
    }
    unique_dirs([home.join(".zcode")])
}

fn configured_roots(raw: OsString) -> Vec<PathBuf> {
    match raw.into_string() {
        Ok(value) => value
            .split(',')
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .collect(),
        Err(raw) => vec![PathBuf::from(raw)],
    }
}

fn unique_dirs(roots: impl IntoIterator<Item = PathBuf>) -> Vec<PathBuf> {
    let mut resolved = Vec::new();
    let mut seen = HashSet::new();
    for root in roots {
        let Ok(canonical) = std::fs::canonicalize(&root) else {
            continue;
        };
        if canonical.is_dir() && seen.insert(canonical.clone()) {
            resolved.push(canonical);
        }
    }
    resolved
}

/// Every existing ZCode database, canonicalized and deduplicated.
fn db_paths() -> Vec<PathBuf> {
    db_paths_under(zcode_roots())
}

fn db_paths_under(roots: impl IntoIterator<Item = PathBuf>) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for root in roots {
        let Ok(db) = std::fs::canonicalize(root.join(ZCODE_DB_RELATIVE_PATH)) else {
            continue;
        };
        if db.is_file() && seen.insert(db.clone()) {
            out.push(db);
        }
    }
    out
}

/// One completed `model_usage` row, tokens already normalized into
/// non-overlapping buckets.
struct UsageRow {
    id: String,
    session_id: String,
    started_at: i64,
    model_id: String,
    directory: Option<String>,
    counts: TokenCounts,
    source_file: PathBuf,
}

/// Parse one ZCode `db.sqlite` into per-session counts. Public for fixture
/// tests; the extractor deduplicates usage ids across roots before
/// aggregating, so this is the single-database view.
pub fn parse_db(db: &Path, since: Option<i64>) -> Vec<SessionCounts> {
    aggregate(read_rows(db), since)
}

fn read_rows(db: &Path) -> Vec<UsageRow> {
    let conn = match sqlite::open_ro(db) {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!("zcode db open failed: {e}");
            return vec![];
        }
    };
    // query_only is connection-local; the journal mode is left alone so a
    // live WAL stays readable while ZCode is running.
    if let Err(e) = conn.execute_batch("PRAGMA query_only = ON") {
        tracing::warn!("zcode db read-only setup failed: {e}");
        return vec![];
    }

    let Some(schema) = read_schema(&conn, db) else {
        return vec![];
    };

    let cache_creation = if schema.has_cache_creation {
        "m.cache_creation_input_tokens"
    } else {
        "0"
    };
    let cache_read = if schema.has_cache_read {
        "m.cache_read_input_tokens"
    } else {
        "0"
    };
    let computed_total = if schema.has_computed_total {
        "m.computed_total_tokens"
    } else {
        "m.input_tokens + m.output_tokens"
    };
    let sql = format!(
        "SELECT m.id, m.session_id, m.started_at, m.model_id, m.input_tokens, m.output_tokens, \
         {cache_creation}, {cache_read}, {computed_total}, s.directory \
         FROM model_usage m LEFT JOIN session s ON s.id = m.session_id \
         WHERE m.status = 'completed'"
    );
    let mut stmt = match conn.prepare(&sql) {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!("zcode db query failed: {e}");
            return vec![];
        }
    };
    let rows = stmt.query_map([], |row| {
        Ok(RawRow {
            id: row.get::<_, Option<String>>(0)?,
            session_id: row.get::<_, Option<String>>(1)?,
            started_at: read_timestamp_ms(row, 2),
            model_id: row.get::<_, Option<String>>(3)?,
            input_tokens: read_token_column(row, 4),
            output_tokens: read_token_column(row, 5),
            cache_creation: read_token_column(row, 6),
            cache_read: read_token_column(row, 7),
            computed_total: read_token_column(row, 8),
            directory: row.get::<_, Option<String>>(9).ok().flatten(),
        })
    });
    match rows {
        Ok(iter) => iter
            .filter_map(|r| r.ok())
            .filter_map(|r| r.normalize(db))
            .collect(),
        Err(e) => {
            tracing::warn!("zcode db read failed: {e}");
            vec![]
        }
    }
}

struct RawRow {
    id: Option<String>,
    session_id: Option<String>,
    started_at: Option<i64>,
    model_id: Option<String>,
    input_tokens: u64,
    output_tokens: u64,
    cache_creation: u64,
    cache_read: u64,
    computed_total: u64,
    directory: Option<String>,
}

impl RawRow {
    /// ccusage's `row_to_entry`: drop rows without identity or a positive
    /// timestamp, peel both cache slices out of the inclusive input count
    /// (bounded by it), and drop rows that carry no tokens at all.
    fn normalize(self, db: &Path) -> Option<UsageRow> {
        let id = self.id.filter(|s| !s.trim().is_empty())?;
        let session_id = self.session_id.filter(|s| !s.trim().is_empty())?;
        let model_id = self.model_id.map(|m| m.trim().to_string())?;
        if model_id.is_empty() {
            return None;
        }
        let started_at = self.started_at.filter(|t| *t > 0)?;

        let cache_read = self.cache_read.min(self.input_tokens);
        let cache_write = self
            .cache_creation
            .min(self.input_tokens.saturating_sub(cache_read));
        let input = self
            .input_tokens
            .saturating_sub(cache_read)
            .saturating_sub(cache_write);
        // ZCode has no reasoning column. ccusage keeps any positive remainder
        // of computed_total_tokens over the four buckets as "extra" tokens and
        // bills them at the output rate; the reasoning bucket is the one slot
        // we have for that remainder, and is priced the same way centrally.
        let accounted = input + self.output_tokens + cache_read + cache_write;
        let reasoning = self.computed_total.saturating_sub(accounted);
        let counts = TokenCounts {
            input,
            output: self.output_tokens,
            cache_read,
            cache_write,
            reasoning,
        };
        if counts.input + counts.output + counts.cache_read + counts.cache_write + counts.reasoning
            == 0
        {
            return None;
        }
        Some(UsageRow {
            id,
            session_id,
            started_at,
            model_id,
            directory: self.directory.filter(|d| !d.trim().is_empty()),
            counts,
            source_file: db.to_path_buf(),
        })
    }
}

fn aggregate(rows: Vec<UsageRow>, since: Option<i64>) -> Vec<SessionCounts> {
    let mut sessions: BTreeMap<String, SessionAgg> = BTreeMap::new();
    for row in rows {
        let agg = sessions.entry(row.session_id.clone()).or_default();
        // Session identity (cwd, first/last activity, latest model) comes
        // from the whole lifetime; only the counts respect the window.
        agg.touch(&row);
        if since.is_none_or(|cutoff| row.started_at >= cutoff) {
            agg.in_window += 1;
            agg.counts.input += row.counts.input;
            agg.counts.output += row.counts.output;
            agg.counts.cache_read += row.counts.cache_read;
            agg.counts.cache_write += row.counts.cache_write;
            agg.counts.reasoning += row.counts.reasoning;
        }
    }
    sessions
        .into_iter()
        .filter(|(_, agg)| since.is_none() || agg.in_window > 0)
        .map(|(session_id, agg)| SessionCounts {
            agent: AgentId::ZCode,
            session_id,
            model: agg.latest_model.map(|(_, m)| m),
            cwd: agg.cwd,
            started_at_ms: agg.started_at_ms,
            last_seen_at_ms: agg.last_seen_at_ms,
            counts: agg.counts,
            cost: None,
            source_file: agg.source_file,
        })
        .collect()
}

#[derive(Default)]
struct SessionAgg {
    counts: TokenCounts,
    in_window: usize,
    cwd: Option<String>,
    started_at_ms: Option<i64>,
    last_seen_at_ms: Option<i64>,
    /// (started_at, model_id) of the most recent completed request.
    latest_model: Option<(i64, String)>,
    source_file: Option<PathBuf>,
}

impl SessionAgg {
    fn touch(&mut self, row: &UsageRow) {
        if self.cwd.is_none() {
            self.cwd = row.directory.clone();
        }
        if self.source_file.is_none() {
            self.source_file = Some(row.source_file.clone());
        }
        if self.started_at_ms.is_none_or(|s| row.started_at < s) {
            self.started_at_ms = Some(row.started_at);
        }
        if self.last_seen_at_ms.is_none_or(|l| row.started_at > l) {
            self.last_seen_at_ms = Some(row.started_at);
        }
        if self
            .latest_model
            .as_ref()
            .is_none_or(|(t, _)| row.started_at >= *t)
        {
            self.latest_model = Some((row.started_at, row.model_id.clone()));
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct Schema {
    has_cache_creation: bool,
    has_cache_read: bool,
    has_computed_total: bool,
}

/// Inspect both tables through `LIMIT 0` statements so no row data (prompts,
/// content) is ever materialized, and refuse layouts missing a required column.
fn read_schema(conn: &rusqlite::Connection, db: &Path) -> Option<Schema> {
    let model_columns = table_columns(conn, "model_usage")?;
    let session_columns = table_columns(conn, "session")?;
    let missing_model = missing_columns(&model_columns, REQUIRED_MODEL_COLUMNS);
    let missing_session = missing_columns(&session_columns, REQUIRED_SESSION_COLUMNS);
    if !missing_model.is_empty() || !missing_session.is_empty() {
        tracing::debug!(
            "unsupported zcode schema at {}: model_usage missing {:?}, session missing {:?}",
            db.display(),
            missing_model,
            missing_session
        );
        return None;
    }
    Some(Schema {
        has_cache_creation: model_columns.contains("cache_creation_input_tokens"),
        has_cache_read: model_columns.contains("cache_read_input_tokens"),
        has_computed_total: model_columns.contains("computed_total_tokens"),
    })
}

fn table_columns(conn: &rusqlite::Connection, table: &str) -> Option<HashSet<String>> {
    let stmt = conn
        .prepare(&format!("SELECT * FROM \"{table}\" LIMIT 0"))
        .ok()?;
    Some(stmt.column_names().iter().map(|c| c.to_string()).collect())
}

fn missing_columns(columns: &HashSet<String>, required: &[&str]) -> Vec<String> {
    required
        .iter()
        .filter(|column| !columns.contains(**column))
        .map(|column| (*column).to_string())
        .collect()
}

/// Token counters may be stored as INTEGER or REAL; negatives clamp to zero.
fn read_token_column(row: &rusqlite::Row, index: usize) -> u64 {
    match row.get_ref(index) {
        Ok(rusqlite::types::ValueRef::Integer(v)) => v.max(0) as u64,
        Ok(rusqlite::types::ValueRef::Real(v)) if v.is_finite() && v > 0.0 => {
            v.min(u64::MAX as f64).round() as u64
        }
        _ => 0,
    }
}

fn read_timestamp_ms(row: &rusqlite::Row, index: usize) -> Option<i64> {
    match row.get_ref(index) {
        Ok(rusqlite::types::ValueRef::Integer(v)) => Some(v),
        Ok(rusqlite::types::ValueRef::Real(v))
            if v.is_finite() && v >= i64::MIN as f64 && v <= i64::MAX as f64 =>
        {
            Some(v.round() as i64)
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    const T1: i64 = 1_786_909_042_666;
    const T2: i64 = 1_786_909_043_666;
    const T3: i64 = 1_786_909_044_666;

    fn create_db(path: &Path, legacy: bool) -> Connection {
        let db = Connection::open(path).unwrap();
        if legacy {
            db.execute_batch(
                "CREATE TABLE model_usage (
                    id TEXT PRIMARY KEY, session_id TEXT, started_at INTEGER, model_id TEXT,
                    status TEXT, input_tokens INTEGER, output_tokens INTEGER,
                    cache_read_input_tokens INTEGER, computed_total_tokens INTEGER
                );
                CREATE TABLE session (id TEXT PRIMARY KEY, directory TEXT);",
            )
            .unwrap();
        } else {
            db.execute_batch(
                "CREATE TABLE model_usage (
                    id TEXT PRIMARY KEY, session_id TEXT, started_at INTEGER, model_id TEXT,
                    provider_id TEXT, status TEXT, input_tokens INTEGER, output_tokens INTEGER,
                    cache_creation_input_tokens INTEGER, cache_read_input_tokens INTEGER,
                    computed_total_tokens INTEGER
                );
                CREATE TABLE session (id TEXT PRIMARY KEY, directory TEXT, version TEXT);",
            )
            .unwrap();
        }
        db
    }

    #[allow(clippy::too_many_arguments)]
    fn insert_usage(
        db: &Connection,
        id: &str,
        session: &str,
        started_at: i64,
        model: &str,
        status: &str,
        tokens: (i64, i64, i64, i64, i64),
    ) {
        db.execute(
            "INSERT INTO model_usage
             (id, session_id, started_at, model_id, provider_id, status, input_tokens,
              output_tokens, cache_creation_input_tokens, cache_read_input_tokens,
              computed_total_tokens)
             VALUES (?1, ?2, ?3, ?4, 'builtin:zai-coding-plan', ?5, ?6, ?7, ?8, ?9, ?10)",
            rusqlite::params![
                id, session, started_at, model, status, tokens.0, tokens.1, tokens.2, tokens.3,
                tokens.4
            ],
        )
        .unwrap();
    }

    fn fixture_db(dir: &Path) -> PathBuf {
        let path = dir.join("db.sqlite");
        let db = create_db(&path, false);
        db.execute(
            "INSERT INTO session VALUES ('session-1', '/project', '0.16.3')",
            [],
        )
        .unwrap();
        db.execute("INSERT INTO session VALUES ('session-2', '', NULL)", [])
            .unwrap();
        // ccusage's canonical row: 100 inclusive input = 60 fresh + 15 + 25.
        insert_usage(
            &db,
            "usage-1",
            "session-1",
            T1,
            "GLM-5.2",
            "completed",
            (100, 10, 15, 25, 110),
        );
        insert_usage(
            &db,
            "usage-2",
            "session-1",
            T2,
            "GLM-5.3",
            "completed",
            (50, 5, 0, 0, 55),
        );
        // Still running: never counted.
        insert_usage(
            &db,
            "usage-3",
            "session-1",
            T3,
            "GLM-5.3",
            "running",
            (1000, 1000, 0, 0, 2000),
        );
        // Total exceeds the four buckets by 13: remainder becomes reasoning.
        insert_usage(
            &db,
            "usage-4",
            "session-2",
            T3,
            "custom-model",
            "completed",
            (40, 5, 0, 10, 58),
        );
        // Zero-token completed row: dropped.
        insert_usage(
            &db,
            "usage-5",
            "session-3",
            T3,
            "GLM-5.3",
            "completed",
            (0, 0, 0, 0, 0),
        );
        path
    }

    #[test]
    fn parses_completed_rows_per_session() {
        let dir = tempfile::tempdir().unwrap();
        let db = fixture_db(dir.path());
        let mut out = parse_db(&db, None);
        out.sort_by(|a, b| a.session_id.cmp(&b.session_id));
        assert_eq!(out.len(), 2, "session-3 has no tokens");

        let s1 = &out[0];
        assert_eq!(s1.agent, AgentId::ZCode);
        assert_eq!(s1.session_id, "session-1");
        assert_eq!(s1.counts.input, 60 + 50);
        assert_eq!(s1.counts.output, 15);
        assert_eq!(s1.counts.cache_write, 15);
        assert_eq!(s1.counts.cache_read, 25);
        assert_eq!(s1.counts.reasoning, 0);
        assert_eq!(
            s1.model.as_deref(),
            Some("GLM-5.3"),
            "latest completed model"
        );
        assert_eq!(s1.cwd.as_deref(), Some("/project"));
        assert_eq!(s1.started_at_ms, Some(T1));
        assert_eq!(s1.last_seen_at_ms, Some(T2), "running row is invisible");
        assert_eq!(s1.cost, None);
        assert_eq!(s1.source_file.as_deref(), Some(db.as_path()));

        let s2 = &out[1];
        assert_eq!(s2.session_id, "session-2");
        assert_eq!(s2.counts.input, 30);
        assert_eq!(s2.counts.cache_read, 10);
        assert_eq!(s2.counts.reasoning, 13);
        assert_eq!(s2.cwd, None, "blank directory is not a cwd");
        assert_eq!(s2.model.as_deref(), Some("custom-model"));
    }

    #[test]
    fn since_filters_rows_and_drops_idle_sessions() {
        let dir = tempfile::tempdir().unwrap();
        let db = fixture_db(dir.path());

        let out = parse_db(&db, Some(T2));
        assert_eq!(out.len(), 2);
        let s1 = out.iter().find(|s| s.session_id == "session-1").unwrap();
        assert_eq!(s1.counts.input, 50, "only usage-2 is in the window");
        assert_eq!(s1.counts.cache_read, 0);
        assert_eq!(
            s1.started_at_ms,
            Some(T1),
            "lifetime start survives the window"
        );

        let out = parse_db(&db, Some(T3));
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].session_id, "session-2");

        assert!(parse_db(&db, Some(T3 + 1)).is_empty());
    }

    #[test]
    fn legacy_schema_without_optional_columns() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db.sqlite");
        let db = create_db(&path, true);
        db.execute(
            "INSERT INTO model_usage VALUES ('usage-legacy', 'session-legacy', ?1, 'custom-model',
             'completed', 100, 10, 25, 110)",
            [T1],
        )
        .unwrap();
        db.execute("INSERT INTO session VALUES ('session-legacy', '')", [])
            .unwrap();
        drop(db);

        let out = parse_db(&path, None);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].counts.input, 75);
        assert_eq!(out[0].counts.cache_read, 25);
        assert_eq!(out[0].counts.cache_write, 0);
        assert_eq!(out[0].counts.output, 10);
        assert_eq!(out[0].cwd, None);
    }

    #[test]
    fn tolerates_foreign_or_broken_databases() {
        let dir = tempfile::tempdir().unwrap();

        let unrelated = dir.path().join("unrelated.sqlite");
        Connection::open(&unrelated)
            .unwrap()
            .execute_batch("CREATE TABLE unrelated (id TEXT)")
            .unwrap();
        assert!(parse_db(&unrelated, None).is_empty());

        // Right table names, required column missing.
        let partial = dir.path().join("partial.sqlite");
        Connection::open(&partial)
            .unwrap()
            .execute_batch(
                "CREATE TABLE model_usage (id TEXT, session_id TEXT, started_at INTEGER);
                 CREATE TABLE session (id TEXT, directory TEXT);",
            )
            .unwrap();
        assert!(parse_db(&partial, None).is_empty());

        let garbage = dir.path().join("garbage.sqlite");
        std::fs::write(&garbage, "not a database").unwrap();
        assert!(parse_db(&garbage, None).is_empty());

        assert!(parse_db(&dir.path().join("missing.sqlite"), None).is_empty());
    }

    #[test]
    fn skips_rows_with_broken_identity_or_odd_column_types() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db.sqlite");
        let db = create_db(&path, false);
        db.execute_batch(
            "INSERT INTO model_usage VALUES ('', 's', 1786909042666, 'm', NULL, 'completed', 1, 1, 0, 0, 2);
             INSERT INTO model_usage VALUES ('u-nots', NULL, 1786909042666, 'm', NULL, 'completed', 1, 1, 0, 0, 2);
             INSERT INTO model_usage VALUES ('u-nomodel', 's', 1786909042666, '  ', NULL, 'completed', 1, 1, 0, 0, 2);
             INSERT INTO model_usage VALUES ('u-ts0', 's', 0, 'm', NULL, 'completed', 1, 1, 0, 0, 2);
             INSERT INTO model_usage VALUES ('u-tstext', 's', 'yesterday', 'm', NULL, 'completed', 1, 1, 0, 0, 2);
             INSERT INTO model_usage VALUES ('u-real', 's', 1786909042666.4, 'm', NULL, 'completed', 7.6, -3, 'x', NULL, NULL);",
        )
        .unwrap();
        drop(db);

        let out = parse_db(&path, None);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].session_id, "s");
        assert_eq!(out[0].started_at_ms, Some(1_786_909_042_666));
        assert_eq!(
            out[0].counts.input, 8,
            "REAL rounds, negatives and text clamp to 0"
        );
        assert_eq!(out[0].counts.output, 0);
        assert_eq!(out[0].counts.reasoning, 0);
    }

    #[test]
    fn reads_a_wal_database_without_touching_journal_mode() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db.sqlite");
        let db = create_db(&path, false);
        db.pragma_update(None, "journal_mode", "WAL").unwrap();
        db.pragma_update(None, "wal_autocheckpoint", 0).unwrap();
        db.execute_batch("BEGIN").unwrap();
        db.execute(
            "INSERT INTO session VALUES ('session-1', '/project', '0.16.3')",
            [],
        )
        .unwrap();
        insert_usage(
            &db,
            "usage-1",
            "session-1",
            T1,
            "GLM-5.3",
            "completed",
            (100, 10, 15, 25, 110),
        );
        db.execute_batch("COMMIT").unwrap();
        assert!(path.with_extension("sqlite-wal").is_file());

        let out = parse_db(&path, None);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].counts.input, 60);
        let mode: String = db
            .pragma_query_value(None, "journal_mode", |r| r.get(0))
            .unwrap();
        assert_eq!(mode, "wal");
    }

    #[test]
    fn root_discovery_honours_zcode_home_without_fallback() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        std::fs::create_dir_all(home.join(".zcode/cli/db")).unwrap();
        std::fs::write(home.join(".zcode/cli/db/db.sqlite"), "x").unwrap();
        let first = dir.path().join("first");
        let second = dir.path().join("second");
        std::fs::create_dir_all(first.join("cli/db")).unwrap();
        std::fs::create_dir_all(&second).unwrap();
        std::fs::write(first.join("cli/db/db.sqlite"), "x").unwrap();

        // Unset or empty: the default home.
        let default_root = home.join(".zcode").canonicalize().unwrap();
        assert_eq!(roots_from(None, home.clone()), vec![default_root.clone()]);
        assert_eq!(
            roots_from(Some(OsString::new()), home.clone()),
            vec![default_root.clone()]
        );
        assert_eq!(
            db_paths_under(roots_from(None, home.clone())),
            vec![default_root.join(ZCODE_DB_RELATIVE_PATH)]
        );

        // Comma list: trimmed, canonicalized, deduplicated, order kept.
        let raw = format!(
            " {}, {}, {} ",
            first.display(),
            second.display(),
            first.display()
        );
        let roots = roots_from(Some(OsString::from(raw)), home.clone());
        assert_eq!(
            roots,
            vec![
                first.canonicalize().unwrap(),
                second.canonicalize().unwrap()
            ]
        );
        // Only the first root has a database.
        assert_eq!(
            db_paths_under(roots),
            vec![first.canonicalize().unwrap().join(ZCODE_DB_RELATIVE_PATH)]
        );

        // Configured but invalid: no silent fallback to ~/.zcode.
        let missing = dir.path().join("missing");
        assert!(roots_from(Some(missing.into_os_string()), home.clone()).is_empty());
        // A file is not a root either.
        let file_root = first.join("cli/db/db.sqlite");
        assert!(roots_from(Some(file_root.into_os_string()), home).is_empty());
    }

    #[test]
    fn extractor_has_no_stats() {
        let z = ZCode;
        assert_eq!(z.id(), AgentId::ZCode);
        assert!(
            z.stats(None).is_empty(),
            "zcode records no messages or tools"
        );
    }
}
