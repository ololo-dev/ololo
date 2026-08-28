//! On-disk per-player session event log (JSONL).
//!
//! Every observable activity of a player's session run is appended as one
//! JSON line:
//!
//! ```text
//! {ARENA_SESSION_LOG_DIR}/{session_id}/{player_id}.jsonl   player events
//! {ARENA_SESSION_LOG_DIR}/{session_id}/_session.jsonl      session-level events
//! ```
//!
//! Three feeds converge here. [`RecordingEventPublisher`] tees the ZMQ
//! fan-out stream (task starts, score changes, judge verdicts, presence,
//! status), direct calls from the probe/memory paths record what never
//! reaches the bus (probe dispatch and grading, memory extraction), and the
//! judge queue appends each run's full telemetry (prompts, events, tokens)
//! as a `judge_run` line. `SessionTimer` is dropped — a per-second tick
//! would drown the log in noise.
//!
//! Events that belong to the session rather than a player (status changes,
//! the roster, awards) land in `_session.jsonl`; [`read_player_log`] merges
//! them into the player's timeline at read time so every player log is a
//! self-contained record. The main server fetches it through
//! `GET /internal/session-logs/...` (it may not share this disk) and serves
//! it to admins for offline analysis.
//!
//! Each line is the event's own serialization plus two envelope fields:
//! `type` (the event kind) and `at` (UTC timestamp).
//!
//! Joins/leaves happen on the main server and never reach this process; the
//! `roster` line written when the session goes running carries the final
//! player list instead.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex};

use async_trait::async_trait;
use chrono::Utc;
use dashmap::DashMap;
use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter};
use uuid::Uuid;

use arena_core::entities::{players, sessions};
use arena_core::protocol::ZmqEvent;

use crate::zmq_pub::EventPublisher;

/// Base directory: `ARENA_SESSION_LOG_DIR`, default `./session-logs`.
pub fn base_dir() -> PathBuf {
    std::env::var("ARENA_SESSION_LOG_DIR")
        .ok()
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("./session-logs"))
}

/// `{base}/{session_id}/{player_id}.jsonl`
pub fn player_log_path(base: &Path, session_id: Uuid, player_id: Uuid) -> PathBuf {
    base.join(session_id.to_string())
        .join(format!("{player_id}.jsonl"))
}

/// `{base}/{session_id}/_session.jsonl` — events that belong to the session
/// rather than any one player. The leading underscore cannot collide with a
/// player file: player files are named by UUID.
pub fn session_log_path(base: &Path, session_id: Uuid) -> PathBuf {
    base.join(session_id.to_string()).join("_session.jsonl")
}

/// Append one event line to the player's log (or the session-level log when
/// `player_id` is `None`). `fields` must serialize to a JSON object; the
/// envelope adds `type` and `at`. Blocking IO runs on the blocking pool.
/// Failures are logged, never fatal — gameplay must not fail because the
/// log could not be written.
pub async fn record(
    base: PathBuf,
    session_id: Uuid,
    player_id: Option<Uuid>,
    kind: &str,
    fields: serde_json::Value,
) {
    let mut obj = match fields {
        serde_json::Value::Object(map) => map,
        other => {
            let mut map = serde_json::Map::new();
            map.insert("data".to_string(), other);
            map
        }
    };
    obj.insert(
        "type".to_string(),
        serde_json::Value::String(kind.to_string()),
    );
    obj.insert(
        "at".to_string(),
        serde_json::Value::String(Utc::now().to_rfc3339()),
    );
    let path = match player_id {
        Some(pid) => player_log_path(&base, session_id, pid),
        None => session_log_path(&base, session_id),
    };
    let res = tokio::task::spawn_blocking(move || {
        append_line_blocking(&path, &serde_json::Value::Object(obj))
    })
    .await;
    match res {
        Ok(Ok(())) => {}
        Ok(Err(e)) => tracing::warn!(error = %e, "session_log_store: write failed"),
        Err(e) => tracing::warn!(error = %e, "session_log_store: join failed"),
    }
}

/// Open append handles, reused across writes. Every probe dispatch and
/// grading result lands here, so per-append `create_dir_all` + open + close
/// was the dominant filesystem cost under load. Handles are dropped when the
/// session finishes ([`close_session_logs`]); the cap is a backstop for
/// sessions that never see a terminal event — clearing simply forces a
/// reopen, which O_APPEND makes safe at any time.
static OPEN_LOGS: LazyLock<DashMap<PathBuf, Arc<Mutex<std::fs::File>>>> =
    LazyLock::new(DashMap::new);
const OPEN_LOGS_CAP: usize = 1024;

/// Drop the cached handles for one session's log directory. Called when the
/// session reaches a terminal status; a late straggler write just reopens.
pub fn close_session_logs(base: &Path, session_id: Uuid) {
    let dir = base.join(session_id.to_string());
    OPEN_LOGS.retain(|path, _| !path.starts_with(&dir));
}

fn append_line_blocking(path: &Path, line: &serde_json::Value) -> std::io::Result<()> {
    let mut buf = serde_json::to_vec(line)?;
    buf.push(b'\n');
    let handle = match OPEN_LOGS.get(path) {
        Some(h) => Arc::clone(&h),
        None => {
            if OPEN_LOGS.len() >= OPEN_LOGS_CAP {
                OPEN_LOGS.clear();
            }
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let file = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)?;
            // On a lost race the freshly opened handle is dropped in favour
            // of the one already in the map — both are O_APPEND, either works.
            Arc::clone(
                &OPEN_LOGS
                    .entry(path.to_path_buf())
                    .or_insert_with(|| Arc::new(Mutex::new(file))),
            )
        }
    };
    let mut file = handle.lock().unwrap_or_else(|e| e.into_inner());
    file.write_all(&buf)
}

/// A player's full timeline: their own events merged with the session-level
/// ones, ordered by the `at` stamp. `None` when the player has no recorded
/// events at all (session-level lines alone do not make a player log).
pub async fn read_player_log(base: PathBuf, session_id: Uuid, player_id: Uuid) -> Option<String> {
    let player_path = player_log_path(&base, session_id, player_id);
    let session_path = session_log_path(&base, session_id);
    tokio::task::spawn_blocking(move || {
        let player_lines = std::fs::read_to_string(&player_path).ok()?;
        let session_lines = std::fs::read_to_string(&session_path).unwrap_or_default();
        let mut lines: Vec<&str> = player_lines
            .lines()
            .chain(session_lines.lines())
            .filter(|l| !l.trim().is_empty())
            .collect();
        // Both files are internally ordered; the merge key is the envelope's
        // rfc3339 `at`. Parsed, not compared as text: to_rfc3339 emits
        // variable-width fractional seconds, which breaks a string sort.
        lines.sort_by_cached_key(|l| {
            serde_json::from_str::<serde_json::Value>(l)
                .ok()
                .and_then(|v| {
                    v.get("at")?
                        .as_str()?
                        .parse::<chrono::DateTime<chrono::Utc>>()
                        .ok()
                })
                .unwrap_or(chrono::DateTime::<chrono::Utc>::MIN_UTC)
        });
        let mut out = lines.join("\n");
        out.push('\n');
        Some(out)
    })
    .await
    .ok()?
}

/// [`EventPublisher`] tee: forwards to the wrapped publisher, then appends
/// the event to the owning player's log file (session-level events go to
/// `_session.jsonl`). Resolves `join_code` → `session_id` through the
/// in-process session registry first, the database once per unknown code
/// after that.
pub struct RecordingEventPublisher {
    inner: Arc<dyn EventPublisher>,
    db: DatabaseConnection,
    registry: crate::state::SessionRegistry,
    base: PathBuf,
    code_to_session: DashMap<String, Uuid>,
}

impl RecordingEventPublisher {
    pub fn new(
        inner: Arc<dyn EventPublisher>,
        db: DatabaseConnection,
        registry: crate::state::SessionRegistry,
        base: PathBuf,
    ) -> Self {
        Self {
            inner,
            db,
            registry,
            base,
            code_to_session: DashMap::new(),
        }
    }

    async fn resolve_session_id(&self, join_code: &str) -> Option<Uuid> {
        if let Some(id) = self.code_to_session.get(join_code) {
            return Some(*id);
        }
        let id = match self.registry.get(join_code) {
            Some(entry) => Some(entry.cache.read().ok()?.session_id),
            None => sessions::Entity::find()
                .filter(sessions::Column::JoinCode.eq(join_code))
                .one(&self.db)
                .await
                .ok()
                .flatten()
                .map(|s| s.id),
        }?;
        self.code_to_session.insert(join_code.to_string(), id);
        Some(id)
    }

    /// The final player list, written when the session goes running —
    /// join/leave events happen on the main server and never reach this
    /// process, so the roster snapshot stands in for them.
    async fn record_roster(&self, session_id: Uuid) {
        let Ok(rows) = players::Entity::find()
            .filter(players::Column::SessionIdFk.eq(session_id))
            .all(&self.db)
            .await
        else {
            return;
        };
        let roster: Vec<serde_json::Value> = rows
            .iter()
            .map(|p| {
                serde_json::json!({
                    "player_id": p.id,
                    "display_name": p.display_name,
                    "user_id": p.user_id_fk,
                    "joined_at": p.joined_at,
                })
            })
            .collect();
        record(
            self.base.clone(),
            session_id,
            None,
            "roster",
            serde_json::json!({ "players": roster }),
        )
        .await;
    }
}

/// The player a published event belongs to; `None` for session-level events.
fn player_id_of(event: &ZmqEvent) -> Option<Uuid> {
    match event {
        ZmqEvent::SessionTimer { .. }
        | ZmqEvent::SessionStatus { .. }
        | ZmqEvent::SessionAwarded { .. }
        | ZmqEvent::SessionSettled { .. } => None,
        ZmqEvent::PlayerJoin { player_id, .. }
        | ZmqEvent::PlayerLeave { player_id, .. }
        | ZmqEvent::AgentPresence { player_id, .. }
        | ZmqEvent::ScoreChange { player_id, .. }
        | ZmqEvent::TaskStarted { player_id, .. }
        | ZmqEvent::JudgeStarted { player_id, .. }
        | ZmqEvent::JudgeFailed { player_id, .. }
        | ZmqEvent::JudgeScored { player_id, .. }
        | ZmqEvent::ProbeFinished { player_id, .. }
        | ZmqEvent::ProbeRegistered { player_id, .. }
        | ZmqEvent::ArtifactReceived { player_id, .. }
        | ZmqEvent::ArtifactAwaited { player_id, .. }
        | ZmqEvent::EvaluationReady { player_id, .. }
        | ZmqEvent::SessionReportReady { player_id, .. } => Some(*player_id),
    }
}

#[async_trait]
impl EventPublisher for RecordingEventPublisher {
    async fn publish(&self, event: &ZmqEvent) {
        self.inner.publish(event).await;

        if matches!(event, ZmqEvent::SessionTimer { .. }) {
            return;
        }
        let Some(session_id) = self.resolve_session_id(event.join_code()).await else {
            tracing::warn!(join_code = %event.join_code(), "session_log_store: unknown join code, event dropped");
            return;
        };
        let fields = match serde_json::to_value(event) {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(error = %e, "session_log_store: serialize failed");
                return;
            }
        };
        // The tagged enum already carries `type`; `record` overwrites it
        // with the same value, so pass the kind through explicitly.
        let kind = match fields.get("type").and_then(|v| v.as_str()) {
            Some(k) => k.to_string(),
            None => "unknown".to_string(),
        };
        record(
            self.base.clone(),
            session_id,
            player_id_of(event),
            &kind,
            fields,
        )
        .await;

        match event {
            ZmqEvent::SessionStatus { status, .. } if status == "running" => {
                self.record_roster(session_id).await;
            }
            // The session is over: the cached mapping and open file handles
            // will not be needed again; dropping them keeps both maps from
            // growing forever.
            ZmqEvent::SessionStatus {
                status, join_code, ..
            } if status == "finished" || status == "cancelled" => {
                self.code_to_session.remove(join_code);
                close_session_logs(&self.base, session_id);
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn player_log_merges_session_level_events_in_time_order() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path().to_path_buf();
        let session_id = Uuid::new_v4();
        let player = Uuid::new_v4();

        record(
            base.clone(),
            session_id,
            None,
            "session_status",
            serde_json::json!({"status": "running"}),
        )
        .await;
        record(
            base.clone(),
            session_id,
            Some(player),
            "probe_dispatched",
            serde_json::json!({"probe_id": "p1"}),
        )
        .await;
        record(
            base.clone(),
            session_id,
            Some(player),
            "judge_run",
            serde_json::json!({"judge_slug": "task-anti-cheat", "events": [{"kind": "llm"}]}),
        )
        .await;

        let raw = read_player_log(base.clone(), session_id, player)
            .await
            .expect("player log readable");
        let lines: Vec<serde_json::Value> = raw
            .lines()
            .map(|l| serde_json::from_str(l).expect("valid json line"))
            .collect();
        assert_eq!(lines.len(), 3);
        // The session-level line was recorded first, so the merge puts it first.
        assert_eq!(lines[0]["type"], "session_status");
        assert_eq!(lines[1]["type"], "probe_dispatched");
        assert_eq!(lines[2]["type"], "judge_run");
        assert_eq!(lines[2]["events"][0]["kind"], "llm");
        assert!(lines.iter().all(|l| l["at"].is_string()));
    }

    #[tokio::test]
    async fn another_players_events_stay_out_of_the_log() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path().to_path_buf();
        let session_id = Uuid::new_v4();
        let (a, b) = (Uuid::new_v4(), Uuid::new_v4());

        record(
            base.clone(),
            session_id,
            Some(a),
            "task_started",
            serde_json::json!({"ordinal": 0}),
        )
        .await;
        record(
            base.clone(),
            session_id,
            Some(b),
            "task_started",
            serde_json::json!({"ordinal": 5}),
        )
        .await;

        let raw = read_player_log(base.clone(), session_id, a)
            .await
            .expect("readable");
        assert_eq!(raw.lines().count(), 1);
        assert!(raw.contains("\"ordinal\":0"));
    }

    #[tokio::test]
    async fn read_missing_player_returns_none() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path().to_path_buf();
        let session_id = Uuid::new_v4();
        // Session-level lines alone are not a player log.
        record(
            base.clone(),
            session_id,
            None,
            "session_status",
            serde_json::json!({}),
        )
        .await;
        assert!(
            read_player_log(base, session_id, Uuid::new_v4())
                .await
                .is_none()
        );
    }
}
