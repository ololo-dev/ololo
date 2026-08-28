use crate::paths;
use crate::sqlite;
use crate::trait_::TokenExtractor;
use crate::types::{AgentId, SessionCounts, SessionStats, TokenCounts};
use std::collections::HashMap;

pub struct OpenCode;

impl TokenExtractor for OpenCode {
    fn id(&self) -> AgentId {
        AgentId::OpenCode
    }

    fn detect(&self) -> bool {
        paths::opencode_db_path().exists()
    }

    fn extract(&self, since: Option<i64>) -> Vec<SessionCounts> {
        let db_path = paths::opencode_db_path();
        let conn = match sqlite::open_ro(&db_path) {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!("opencode db open failed: {e}");
                return vec![];
            }
        };

        if let Some(cutoff) = since {
            // Per-window mode: query individual messages within the time window,
            // sum their per-turn token counts per session.
            return extract_opencode_window(&conn, cutoff, &db_path);
        }

        // Session-lifetime mode: read pre-aggregated totals from session table
        let sql = "SELECT id, model, tokens_input, tokens_output, tokens_reasoning, tokens_cache_read, tokens_cache_write, cost, directory, time_created, time_updated FROM session";

        let mut stmt = match conn.prepare(sql) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!("opencode db query failed: {e}");
                return vec![];
            }
        };

        let rows = stmt
            .query_map([], |row| {
                Ok(SessionCounts {
                    agent: AgentId::OpenCode,
                    session_id: row.get::<_, String>(0)?,
                    model: row.get::<_, Option<String>>(1)?,
                    cwd: row.get::<_, Option<String>>(8)?,
                    started_at_ms: row.get::<_, Option<i64>>(9)?,
                    last_seen_at_ms: row.get::<_, Option<i64>>(10)?,
                    counts: TokenCounts {
                        input: row.get(2)?,
                        output: row.get(3)?,
                        reasoning: row.get(4)?,
                        cache_read: row.get(5)?,
                        cache_write: row.get(6)?,
                    },
                    cost: row.get::<_, Option<f64>>(7)?,
                    source_file: Some(db_path.clone()),
                })
            })
            .ok();

        match rows {
            Some(iter) => iter.filter_map(|r| r.ok()).collect(),
            None => vec![],
        }
    }

    fn stats(&self, since: Option<i64>) -> Vec<SessionStats> {
        let db_path = paths::opencode_db_path();
        let conn = match sqlite::open_ro(&db_path) {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!("opencode db open failed: {e}");
                return vec![];
            }
        };

        let mut sessions: HashMap<String, SessionStats> = HashMap::new();
        let mut session_cwd: HashMap<String, Option<String>> = HashMap::new();
        if let Ok(mut stmt) = conn.prepare("SELECT id, directory FROM session")
            && let Ok(rows) = stmt.query_map([], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?))
            })
        {
            for r in rows.flatten() {
                session_cwd.insert(r.0, r.1);
            }
        }

        // Message roles
        if let Ok(mut stmt) = conn
            .prepare("SELECT session_id, time_created, data FROM message WHERE time_created >= ?1")
            && let Ok(rows) = stmt.query_map(rusqlite::params![since.unwrap_or(0)], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, String>(2)?,
                ))
            })
        {
            for (session_id, ts, data_json) in rows.flatten() {
                let Ok(data) = serde_json::from_str::<serde_json::Value>(&data_json) else {
                    continue;
                };
                let stats = sessions.entry(session_id.clone()).or_insert_with(|| {
                    let mut s = SessionStats::new(AgentId::OpenCode, session_id.clone());
                    s.cwd = session_cwd.get(&session_id).cloned().flatten();
                    s.source_file = Some(db_path.clone());
                    s
                });
                stats.touch_ts(Some(ts));
                match data.get("role").and_then(|r| r.as_str()) {
                    Some("user") => stats.user_messages += 1,
                    Some("assistant") => stats.assistant_messages += 1,
                    _ => {}
                }
            }
        }

        // Tool calls live in the part table: data JSON {"type":"tool","tool":name,"state":{"input":...}}
        if let Ok(mut stmt) =
            conn.prepare("SELECT session_id, time_created, data FROM part WHERE time_created >= ?1")
            && let Ok(rows) = stmt.query_map(rusqlite::params![since.unwrap_or(0)], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, String>(2)?,
                ))
            })
        {
            for (session_id, ts, data_json) in rows.flatten() {
                let Ok(data) = serde_json::from_str::<serde_json::Value>(&data_json) else {
                    continue;
                };
                if data.get("type").and_then(|t| t.as_str()) != Some("tool") {
                    continue;
                }
                let Some(tool) = data.get("tool").and_then(|t| t.as_str()) else {
                    continue;
                };
                let stats = sessions.entry(session_id.clone()).or_insert_with(|| {
                    let mut s = SessionStats::new(AgentId::OpenCode, session_id.clone());
                    s.cwd = session_cwd.get(&session_id).cloned().flatten();
                    s.source_file = Some(db_path.clone());
                    s
                });
                stats.touch_ts(Some(ts));
                let empty = serde_json::Value::Null;
                let args = data
                    .get("state")
                    .and_then(|s| s.get("input"))
                    .unwrap_or(&empty);
                stats.record_tool(tool, args);
            }
        }

        sessions.into_values().collect()
    }
}

/// Per-session metadata: (model, cwd, time_created, time_updated)
type SessionMeta = (Option<String>, Option<String>, Option<i64>, Option<i64>);

/// Query individual messages within a time window and sum per-turn tokens per session.
/// Each message row has a `data` JSON column with a `tokens` object and `cost` field.
fn extract_opencode_window(
    conn: &rusqlite::Connection,
    since: i64,
    db_path: &std::path::Path,
) -> Vec<SessionCounts> {
    // Get session metadata for all sessions
    let mut session_meta: HashMap<String, SessionMeta> = HashMap::new();
    if let Ok(mut stmt) =
        conn.prepare("SELECT id, model, directory, time_created, time_updated FROM session")
        && let Ok(rows) = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, Option<String>>(1)?,
                r.get::<_, Option<String>>(2)?,
                r.get::<_, Option<i64>>(3)?,
                r.get::<_, Option<i64>>(4)?,
            ))
        })
    {
        for r in rows.flatten() {
            session_meta.insert(r.0, (r.1, r.2, r.3, r.4));
        }
    }

    // Query messages within the time window, sum tokens per session
    let sql = "SELECT session_id, time_created, data FROM message WHERE time_created >= ?1 ORDER BY time_created";
    let mut stmt = match conn.prepare(sql) {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!("opencode message query failed: {e}");
            return vec![];
        }
    };

    let rows = stmt
        .query_map(rusqlite::params![since], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .ok();

    let mut agg: HashMap<(String, String, String), WindowAgg> = HashMap::new();

    if let Some(iter) = rows {
        for r in iter.flatten() {
            let (session_id, time_created, data_json) = r;
            let Ok(data) = serde_json::from_str::<serde_json::Value>(&data_json) else {
                continue;
            };

            // Only assistant messages have tokens
            let role = data.get("role").and_then(|r| r.as_str()).unwrap_or("");
            if role != "assistant" {
                continue;
            }

            let tokens = match data.get("tokens") {
                Some(t) => t,
                None => continue,
            };

            let msg_cost = data.get("cost").and_then(|c| c.as_f64()).unwrap_or(0.0);

            // Aggregate by (session_id, providerID, modelID) so model
            // switches within a session produce separate rows.
            // ponytail: per-turn extraction avoids the cumulative-total problem
            // of the session table; key is (session, provider, model).
            let provider = data
                .get("providerID")
                .and_then(|p| p.as_str())
                .unwrap_or("—")
                .to_string();
            let model_id = data
                .get("modelID")
                .and_then(|m| m.as_str())
                .unwrap_or("—")
                .to_string();
            let key = (session_id.clone(), provider, model_id);

            let a = agg.entry(key).or_default();
            a.input += tokens.get("input").and_then(|v| v.as_u64()).unwrap_or(0);
            a.output += tokens.get("output").and_then(|v| v.as_u64()).unwrap_or(0);
            a.reasoning += tokens
                .get("reasoning")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            a.cache_read += tokens
                .get("cache")
                .and_then(|c| c.get("read"))
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            a.cache_write += tokens
                .get("cache")
                .and_then(|c| c.get("write"))
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            a.cost += msg_cost;
            a.msg_count += 1;
            if a.first_msg_ts.is_none() || Some(time_created) < a.first_msg_ts {
                a.first_msg_ts = Some(time_created);
            }
            if a.last_msg_ts.is_none() || Some(time_created) > a.last_msg_ts {
                a.last_msg_ts = Some(time_created);
            }
        }
    }

    agg.into_iter()
        .filter_map(|((session_id, provider, model_id), a)| {
            if a.msg_count == 0 {
                return None;
            }
            let (_, cwd, _, _) = session_meta.get(&session_id).cloned().unwrap_or_default();
            // Store model as JSON matching the session-table format so the
            // TUI render can parse providerID/id uniformly.
            let model_json = serde_json::json!({
                "id": model_id,
                "providerID": provider,
            })
            .to_string();
            Some(SessionCounts {
                agent: AgentId::OpenCode,
                session_id,
                model: Some(model_json),
                cwd,
                started_at_ms: a.first_msg_ts,
                last_seen_at_ms: a.last_msg_ts,
                counts: TokenCounts {
                    input: a.input,
                    output: a.output,
                    reasoning: a.reasoning,
                    cache_read: a.cache_read,
                    cache_write: a.cache_write,
                },
                cost: Some(a.cost),
                source_file: Some(db_path.to_path_buf()),
            })
        })
        .collect()
}

#[derive(Default)]
struct WindowAgg {
    input: u64,
    output: u64,
    reasoning: u64,
    cache_read: u64,
    cache_write: u64,
    cost: f64,
    msg_count: usize,
    first_msg_ts: Option<i64>,
    last_msg_ts: Option<i64>,
}
