//! Admin judge-run proxy API + judge-result run details.

use crate::api::settings::AdminUser;
use crate::state::AppState;
use arena_core::entities::{game_servers, judge_results, judges, players, sessions, task_judges};
use axum::Json;
use axum::extract::{Path, State};
use axum::response::IntoResponse;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use uuid::Uuid;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JudgeRunReq {
    pub task_id: Uuid,
    pub judge_id: Uuid,
    /// Re-run even when a verdict already exists. Session-scoped judges skip
    /// a pass whose every verdict is already written — a cost guard aimed at
    /// the recovery sweeps — so without this an admin re-run of, say, the
    /// session report returns the old text and looks like it worked.
    #[serde(default)]
    pub force: bool,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JudgeRunResp {
    pub judge_result_id: Uuid,
    pub rating: f64,
    pub feedback: String,
    pub point_delta: i32,
    pub raw_output: String,
    pub model: String,
}

#[derive(Debug, Serialize)]
struct ProxyBody {
    session_id: Uuid,
    player_id: Uuid,
    task_id: Uuid,
    judge_id: Uuid,
    force: bool,
}

#[derive(thiserror::Error, Debug)]
pub enum JudgeRunError {
    #[error("task judge not found")]
    TaskJudgeNotFound,
    #[error("event log not found")]
    EventLogNotFound,
    #[error("validation error: {0}")]
    Validation(String),
    #[error("game server unavailable")]
    GameServerUnavailable,
    #[error("game server timeout")]
    GameServerTimeout,
    #[error("game server error: {0}")]
    GameServerError(String),
    /// The game server ran (or refused) the judge and reported why. Mapped to
    /// 422, NOT 502: Cloudflare replaces origin 502/504 bodies with its own
    /// error page, which hid the actual reason ("player repo empty") from
    /// every API caller behind the proxy.
    #[error("judge run failed: {0}")]
    JudgeFailed(String),
    #[error(transparent)]
    Db(#[from] sea_orm::DbErr),
}

crate::api::error::impl_api_error!(JudgeRunError {
    Self::TaskJudgeNotFound => (NOT_FOUND, "task_judge_not_found"),
    Self::EventLogNotFound => (NOT_FOUND, "event_log_not_found"),
    Self::Validation(m) => (UNPROCESSABLE_ENTITY, "validation_error", "message": m),
    Self::GameServerUnavailable => (SERVICE_UNAVAILABLE, "game_server_unavailable"),
    Self::GameServerTimeout => (GATEWAY_TIMEOUT, "game_server_timeout"),
    Self::GameServerError(m) => (BAD_GATEWAY, "game_server_error", "message": m),
    Self::JudgeFailed(m) => (UNPROCESSABLE_ENTITY, "judge_failed", "message": m),
    Self::Db(_) => (INTERNAL_SERVER_ERROR, "database_error"),
});

/// HTTP base URL for internal calls to a game server. `game_servers.url`
/// holds the *WebSocket* advertise URL (`GAME_SERVER_ADVERTISE_URL`, e.g.
/// `wss://ved.ololo.dev`) — reqwest rejects `ws(s)://` outright, so map the
/// scheme to its HTTP counterpart. Trailing slashes are trimmed so callers
/// can append `/internal/...` directly.
/// Shared, connection-pooled HTTP client for internal game-server calls.
/// Building a fresh `reqwest::Client` per request threw away the connection
/// pool and re-did TLS every time; per-request timeouts are set on the request
/// builder instead.
fn shared_client() -> &'static reqwest::Client {
    static CLIENT: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
    CLIENT.get_or_init(reqwest::Client::new)
}

fn game_server_http_base(advertise_url: &str) -> String {
    let base = advertise_url.trim_end_matches('/');
    if let Some(rest) = base.strip_prefix("wss://") {
        format!("https://{rest}")
    } else if let Some(rest) = base.strip_prefix("ws://") {
        format!("http://{rest}")
    } else {
        base.to_string()
    }
}

pub async fn post_run(
    _admin: AdminUser,
    State(state): State<AppState>,
    Path((session_code, player_id)): Path<(String, Uuid)>,
    Json(req): Json<JudgeRunReq>,
) -> Result<Json<JudgeRunResp>, JudgeRunError> {
    let session = match Uuid::parse_str(&session_code) {
        Ok(id) => sessions::Entity::find_by_id(id)
            .one(&state.db)
            .await?
            .ok_or(JudgeRunError::TaskJudgeNotFound)?,
        Err(_) => sessions::Entity::find()
            .filter(sessions::Column::JoinCode.eq(session_code.to_uppercase()))
            .one(&state.db)
            .await?
            .ok_or(JudgeRunError::TaskJudgeNotFound)?,
    };
    let session_id = session.id;

    let _tj = task_judges::Entity::find()
        .filter(task_judges::Column::TaskId.eq(req.task_id))
        .filter(task_judges::Column::JudgeId.eq(req.judge_id))
        .one(&state.db)
        .await?
        .ok_or(JudgeRunError::TaskJudgeNotFound)?;

    let game_server_id = session
        .game_server_id
        .ok_or(JudgeRunError::GameServerUnavailable)?;
    let gs = game_servers::Entity::find_by_id(game_server_id)
        .one(&state.db)
        .await?
        .ok_or(JudgeRunError::GameServerUnavailable)?;
    let base_url = gs.url;

    let body = ProxyBody {
        session_id,
        player_id,
        task_id: req.task_id,
        judge_id: req.judge_id,
        force: req.force,
    };

    let url = format!("{}/internal/judge-run", game_server_http_base(&base_url));
    let resp = shared_client()
        .post(&url)
        .timeout(Duration::from_secs(200))
        .header(
            arena_core::auth::INTERNAL_API_HEADER,
            arena_core::auth::internal_api_token(&state.jwt_signing_secret),
        )
        .json(&body)
        .send()
        .await
        .map_err(|e| {
            if e.is_timeout() {
                JudgeRunError::GameServerTimeout
            } else {
                JudgeRunError::GameServerError(e.to_string())
            }
        })?;

    let status = resp.status();
    if !status.is_success() {
        let text = resp.text().await.unwrap_or_default();
        return Err(JudgeRunError::JudgeFailed(if text.is_empty() {
            format!("game server responded {status}")
        } else {
            text
        }));
    }

    let parsed: JudgeRunResp = resp
        .json()
        .await
        .map_err(|e| JudgeRunError::GameServerError(e.to_string()))?;

    Ok(Json(parsed))
}

/// Full judge-run detail for the admin judges tab: model + provider, token
/// usage, wall-clock duration, the chronological run log (LLM turns + tool
/// calls), the raw verdict output, and the UNREDACTED error. Admin-only —
/// public payloads carry a generic failure message instead.
#[derive(Debug, Serialize)]
pub struct JudgeResultDetailsResp {
    pub id: Uuid,
    pub task_judge_id: Uuid,
    pub judge_slug: Option<String>,
    pub judge_name: Option<String>,
    pub status: String,
    pub model: String,
    pub provider: String,
    pub rating: serde_json::Value,
    pub point_delta: i32,
    pub feedback: String,
    pub raw_output: String,
    pub duration_ms: Option<i64>,
    pub tokens_input: Option<i64>,
    pub tokens_output: Option<i64>,
    pub tokens_cache_read: Option<i64>,
    pub tokens_cache_write: Option<i64>,
    /// Chronological `JudgeLogEvent` array (see arena-core judging).
    pub run_log: Option<serde_json::Value>,
    /// The `Evidence` snapshot this verdict was reached from. Absent on
    /// legacy rows and on runs that failed before gathering it.
    pub evidence: Option<serde_json::Value>,
    /// `judge_fingerprint` of the definition that ran — the judge's prompt is
    /// re-seeded over the same slug on every boot, so the slug alone does not
    /// identify what was asked.
    pub judge_fingerprint: Option<String>,
    pub error: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

pub async fn get_result_details(
    _admin: AdminUser,
    State(state): State<AppState>,
    Path(judge_result_id): Path<Uuid>,
) -> Result<Json<JudgeResultDetailsResp>, JudgeRunError> {
    let row = judge_results::Entity::find_by_id(judge_result_id)
        .one(&state.db)
        .await?
        .ok_or(JudgeRunError::TaskJudgeNotFound)?;
    // Judge identity for the header (best-effort).
    let tj = task_judges::Entity::find_by_id(row.task_judge_id)
        .one(&state.db)
        .await?;
    let judge = match &tj {
        Some(tj) => {
            judges::Entity::find_by_id(tj.judge_id)
                .one(&state.db)
                .await?
        }
        None => None,
    };
    // The event log lives on the game-server's disk
    // ({join_code}/{username}/{task_id}.json) — proxy it in. Legacy rows
    // predating the file store still carry run_log in the DB column.
    let mut run_log = row.run_log.clone();
    let mut entry = None;
    if let (Some(tj), Some(judge)) = (&tj, &judge) {
        entry = fetch_run_entry_from_game_server(
            &state,
            row.session_id_fk,
            row.player_id_fk,
            tj.task_id,
            &judge.slug,
        )
        .await;
    }
    // Legacy rows predating the file store carry their events in the DB
    // column; for everyone else the store is the source.
    if run_log.is_none() {
        run_log = entry.as_ref().and_then(|e| e.get("events").cloned());
    }
    Ok(Json(JudgeResultDetailsResp {
        id: row.id,
        task_judge_id: row.task_judge_id,
        judge_slug: judge.as_ref().map(|j| j.slug.clone()),
        judge_name: judge.as_ref().map(|j| j.name.clone()),
        status: row.status,
        model: row.model,
        provider: row.provider,
        rating: row.rating,
        point_delta: row.point_delta,
        feedback: row.feedback,
        raw_output: row.raw_output,
        duration_ms: row.duration_ms,
        tokens_input: row.tokens_input,
        tokens_output: row.tokens_output,
        tokens_cache_read: row.tokens_cache_read,
        tokens_cache_write: row.tokens_cache_write,
        run_log,
        evidence: entry
            .as_ref()
            .and_then(|e| e.get("evidence").cloned())
            .filter(|v| !v.is_null()),
        judge_fingerprint: entry
            .as_ref()
            .and_then(|e| e.get("judge_fingerprint"))
            .and_then(|v| v.as_str())
            .map(str::to_string),
        error: row.error,
        created_at: row.created_at,
        updated_at: row.updated_at,
    }))
}

/// Fetch this judge's run entry from the game-server's on-disk store
/// (`GET /internal/judge-logs/:session_id/:player_id/:task_id`, an object
/// keyed by judge slug): the event log, the evidence snapshot, and the
/// fingerprint of the definition that ran. Best-effort: any failure (no game
/// server, file absent, network) degrades to `None` — the details endpoint
/// still serves the DB metadata.
async fn fetch_run_entry_from_game_server(
    state: &AppState,
    session_id: Uuid,
    player_id: Uuid,
    task_id: Uuid,
    judge_slug: &str,
) -> Option<serde_json::Value> {
    let session = sessions::Entity::find_by_id(session_id)
        .one(&state.db)
        .await
        .ok()??;
    let gs = game_servers::Entity::find_by_id(session.game_server_id?)
        .one(&state.db)
        .await
        .ok()??;
    let url = format!(
        "{}/internal/judge-logs/{session_id}/{player_id}/{task_id}",
        game_server_http_base(&gs.url)
    );
    let doc: serde_json::Value = shared_client()
        .get(&url)
        .timeout(Duration::from_secs(5))
        .header(
            arena_core::auth::INTERNAL_API_HEADER,
            arena_core::auth::internal_api_token(&state.jwt_signing_secret),
        )
        .send()
        .await
        .ok()?
        .json()
        .await
        .ok()?;
    doc.get(judge_slug).cloned()
}

/// Judge LLM usage aggregated per player for one session — the cost view:
/// how many tokens the operator's judge model spent evaluating each
/// player's submissions (all judges, all tasks, retries included).
#[derive(Debug, Default, Serialize)]
pub struct JudgeUsageEntry {
    pub player_id: Option<Uuid>,
    pub display_name: String,
    pub runs: u64,
    pub scored: u64,
    pub failed: u64,
    pub tokens_input: i64,
    pub tokens_output: i64,
    pub tokens_cache_read: i64,
    pub tokens_cache_write: i64,
}

#[derive(Debug, Serialize)]
pub struct JudgeUsageResp {
    pub players: Vec<JudgeUsageEntry>,
    pub totals: JudgeUsageEntry,
}

/// `GET /api/admin/sessions/:code/judge-usage` — per-player judge token
/// spend for the session. Admin-only.
pub async fn get_session_judge_usage(
    _admin: AdminUser,
    State(state): State<AppState>,
    Path(session_code): Path<String>,
) -> Result<Json<JudgeUsageResp>, JudgeRunError> {
    let session = match Uuid::parse_str(&session_code) {
        Ok(id) => sessions::Entity::find_by_id(id)
            .one(&state.db)
            .await?
            .ok_or(JudgeRunError::TaskJudgeNotFound)?,
        Err(_) => sessions::Entity::find()
            .filter(sessions::Column::JoinCode.eq(session_code.to_uppercase()))
            .one(&state.db)
            .await?
            .ok_or(JudgeRunError::TaskJudgeNotFound)?,
    };
    let rows = judge_results::Entity::find()
        .filter(judge_results::Column::SessionIdFk.eq(session.id))
        .all(&state.db)
        .await?;
    let player_rows = players::Entity::find()
        .filter(players::Column::SessionIdFk.eq(session.id))
        .all(&state.db)
        .await?;
    let name_by_id: std::collections::HashMap<Uuid, String> = player_rows
        .iter()
        .map(|p| (p.id, p.display_name.clone()))
        .collect();

    let mut by_player: std::collections::HashMap<Uuid, JudgeUsageEntry> =
        std::collections::HashMap::new();
    let mut totals = JudgeUsageEntry {
        display_name: "total".to_string(),
        ..Default::default()
    };
    for r in &rows {
        let e = by_player
            .entry(r.player_id_fk)
            .or_insert_with(|| JudgeUsageEntry {
                player_id: Some(r.player_id_fk),
                display_name: name_by_id
                    .get(&r.player_id_fk)
                    .cloned()
                    .unwrap_or_else(|| r.player_id_fk.to_string()),
                ..Default::default()
            });
        for t in [e, &mut totals] {
            t.runs += 1;
            match r.status.as_str() {
                "scored" => t.scored += 1,
                "failed" => t.failed += 1,
                _ => {}
            }
            t.tokens_input += r.tokens_input.unwrap_or(0);
            t.tokens_output += r.tokens_output.unwrap_or(0);
            t.tokens_cache_read += r.tokens_cache_read.unwrap_or(0);
            t.tokens_cache_write += r.tokens_cache_write.unwrap_or(0);
        }
    }
    let mut players_out: Vec<JudgeUsageEntry> = by_player.into_values().collect();
    players_out.sort_by(|a, b| {
        (b.tokens_input + b.tokens_output)
            .cmp(&(a.tokens_input + a.tokens_output))
            .then_with(|| a.display_name.cmp(&b.display_name))
    });
    Ok(Json(JudgeUsageResp {
        players: players_out,
        totals,
    }))
}

/// `GET /api/admin/sessions/:code/players/:player_id/tasks/:task_id/judge-log`
/// — the raw on-disk judge log file for one player+task (JSON object keyed
/// by judge slug, every judge's full telemetry). Proxied from the game
/// server that owns the session. Admin-only.
pub async fn get_player_task_judge_log(
    _admin: AdminUser,
    State(state): State<AppState>,
    Path((session_code, player_id, task_id)): Path<(String, Uuid, Uuid)>,
) -> Result<Json<serde_json::Value>, JudgeRunError> {
    let session = match Uuid::parse_str(&session_code) {
        Ok(id) => sessions::Entity::find_by_id(id)
            .one(&state.db)
            .await?
            .ok_or(JudgeRunError::TaskJudgeNotFound)?,
        Err(_) => sessions::Entity::find()
            .filter(sessions::Column::JoinCode.eq(session_code.to_uppercase()))
            .one(&state.db)
            .await?
            .ok_or(JudgeRunError::TaskJudgeNotFound)?,
    };
    let gs = game_servers::Entity::find_by_id(
        session
            .game_server_id
            .ok_or(JudgeRunError::GameServerUnavailable)?,
    )
    .one(&state.db)
    .await?
    .ok_or(JudgeRunError::GameServerUnavailable)?;

    let url = format!(
        "{}/internal/judge-logs/{}/{player_id}/{task_id}",
        game_server_http_base(&gs.url),
        session.id
    );
    let resp = shared_client()
        .get(&url)
        .timeout(Duration::from_secs(10))
        .header(
            arena_core::auth::INTERNAL_API_HEADER,
            arena_core::auth::internal_api_token(&state.jwt_signing_secret),
        )
        .send()
        .await
        .map_err(|e| {
            if e.is_timeout() {
                JudgeRunError::GameServerTimeout
            } else {
                JudgeRunError::GameServerError(e.to_string())
            }
        })?;
    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        return Err(JudgeRunError::TaskJudgeNotFound);
    }
    if !resp.status().is_success() {
        return Err(JudgeRunError::GameServerError(
            resp.text().await.unwrap_or_default(),
        ));
    }
    let doc: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| JudgeRunError::GameServerError(e.to_string()))?;
    Ok(Json(doc))
}

/// `GET /api/admin/sessions/:code/players/:player_id/event-log` — the
/// player's full event timeline (JSONL, one event per line: session status
/// changes, task starts, probes, score changes, judge runs with their full
/// telemetry, memory extraction). Proxied from the game server that owns the
/// session and served as a downloadable NDJSON file. Admin-only.
pub async fn get_player_event_log(
    _admin: AdminUser,
    State(state): State<AppState>,
    Path((session_code, player_id)): Path<(String, Uuid)>,
) -> Result<axum::response::Response, JudgeRunError> {
    let session = match Uuid::parse_str(&session_code) {
        Ok(id) => sessions::Entity::find_by_id(id)
            .one(&state.db)
            .await?
            .ok_or(JudgeRunError::EventLogNotFound)?,
        Err(_) => sessions::Entity::find()
            .filter(sessions::Column::JoinCode.eq(session_code.to_uppercase()))
            .one(&state.db)
            .await?
            .ok_or(JudgeRunError::EventLogNotFound)?,
    };
    let gs = game_servers::Entity::find_by_id(
        session
            .game_server_id
            .ok_or(JudgeRunError::GameServerUnavailable)?,
    )
    .one(&state.db)
    .await?
    .ok_or(JudgeRunError::GameServerUnavailable)?;

    let url = format!(
        "{}/internal/session-logs/{}/{player_id}",
        game_server_http_base(&gs.url),
        session.id
    );
    let resp = shared_client()
        .get(&url)
        .timeout(Duration::from_secs(30))
        .header(
            arena_core::auth::INTERNAL_API_HEADER,
            arena_core::auth::internal_api_token(&state.jwt_signing_secret),
        )
        .send()
        .await
        .map_err(|e| {
            if e.is_timeout() {
                JudgeRunError::GameServerTimeout
            } else {
                JudgeRunError::GameServerError(e.to_string())
            }
        })?;
    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        return Err(JudgeRunError::EventLogNotFound);
    }
    if !resp.status().is_success() {
        return Err(JudgeRunError::GameServerError(
            resp.text().await.unwrap_or_default(),
        ));
    }
    let body = resp
        .text()
        .await
        .map_err(|e| JudgeRunError::GameServerError(e.to_string()))?;
    let filename = format!(
        "session-{}-player-{player_id}-events.jsonl",
        session.join_code
    );
    Ok((
        [
            (
                axum::http::header::CONTENT_TYPE,
                "application/x-ndjson".to_string(),
            ),
            (
                axum::http::header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"{filename}\""),
            ),
        ],
        body,
    )
        .into_response())
}

#[cfg(test)]
mod game_server_http_base_tests {
    use super::game_server_http_base;

    #[test]
    fn wss_maps_to_https() {
        // Prod shape: GAME_SERVER_ADVERTISE_URL is the wss:// client URL.
        assert_eq!(
            game_server_http_base("wss://ved.ololo.dev"),
            "https://ved.ololo.dev"
        );
        assert_eq!(
            game_server_http_base("wss://ved.ololo.dev/"),
            "https://ved.ololo.dev"
        );
    }

    #[test]
    fn ws_maps_to_http() {
        // Dev default: ws://localhost:PORT.
        assert_eq!(
            game_server_http_base("ws://localhost:9000"),
            "http://localhost:9000"
        );
    }

    #[test]
    fn http_urls_pass_through_with_trailing_slash_trimmed() {
        assert_eq!(game_server_http_base("http://gs:9000/"), "http://gs:9000");
        assert_eq!(
            game_server_http_base("https://gs.example.com"),
            "https://gs.example.com"
        );
    }
}
