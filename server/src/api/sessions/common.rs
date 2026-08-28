use crate::auth::jwt::AccessClaims;
use arena_core::entities::{players, projects, sessions};
use arena_core::session_status::SessionStatus;
use axum::extract::ConnectInfo;
use axum::http::HeaderMap;
use sea_orm::{ColumnTrait, DatabaseConnection, DbErr, EntityTrait, QueryFilter};
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use uuid::Uuid;

const NAME_MIN: usize = 1;
const NAME_MAX: usize = 120;
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateSessionReq {
    pub name: String,
    pub project_id: uuid::Uuid,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PatchSessionReq {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub status: Option<SessionStatus>,
}

/// Request body for `POST /api/sessions/join`.
/// NOTE: The `code` field must NOT be logged (FR-JC-010). The
/// `#[tracing::instrument(skip_all)]` on `post_join` ensures the entire
/// request body — including `code` — is never captured in tracing spans.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JoinSessionReq {
    pub code: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct SessionSummary {
    pub id: String,
    pub name: String,
    pub status: String,
    pub owner_id: Option<Uuid>,
    pub project_id: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub join_code: Option<String>,
    /// Players in the session. Only the project listing fills this in (it
    /// drives the live cards there); other endpoints leave it at 0 rather
    /// than pay for a count nobody reads.
    pub player_count: u32,
    /// Winner's display name — only for sessions that have finished, and
    /// only from the project listing. `None` when nobody scored.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub best_player: Option<String>,
    /// The winner's points. Paired with `best_player`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub best_score: Option<i64>,
}

#[derive(Debug, Serialize)]
pub(crate) struct SessionListResp {
    pub sessions: Vec<SessionSummary>,
}

#[derive(Debug, Serialize)]
pub(crate) struct SessionReportTimelineItem {
    pub task_id: String,
    pub task_title: String,
    pub player_id: Uuid,
    pub player_display_name: String,
    #[serde(rename = "score")]
    pub point_delta: i32,
    pub answer: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Serialize)]
pub(crate) struct SessionReportResp {
    pub session_id: String,
    pub status: String,
    pub leaderboard: Vec<crate::protocol::LeaderboardEntry>,
    pub timeline: Vec<SessionReportTimelineItem>,
    pub activity_events: Vec<ActivityEventDto>,
    pub players: Vec<ReportPlayerDto>,
    /// Cumulative per-player score timeseries; `None` when the session never
    /// started or produced no scored events.
    pub score_history: Option<Vec<crate::protocol::ScoreHistorySample>>,
    /// Judge runs still owed after the session finished (0 for live
    /// sessions). The session is only *really* over — final scores, awards
    /// about to settle — when this reaches zero; the dashboard waits for
    /// that before celebrating.
    pub judges_pending: u64,
}

/// Per-player session statistics for the session page Statistics block.
#[derive(Debug, Serialize)]
pub(crate) struct SessionPlayerStatsDto {
    pub player_id: Uuid,
    pub user_id: Option<Uuid>,
    pub display_name: String,
    pub avatar_url: Option<String>,
    pub username: Option<String>,
    /// AI agent reported via player metadata (fallback when no stats rows).
    pub agent_display_name: Option<String>,
    /// 1-based leaderboard rank in this session (competition ranking).
    pub rank: u32,
    /// Final session score (probes + bonuses + judge deltas).
    pub game_points: i64,
    /// Probe results only (non-bonus task_results).
    pub probe_points: i64,
    /// Task completion bonuses.
    pub bonus_points: i64,
    /// Judge verdict deltas.
    pub judge_points: i64,
    /// Distinct completed tasks (completion bonus granted).
    pub solved_tasks: u64,
    /// Probes dispatched to this player.
    pub probes: u64,
    /// Distinct agents observed in reported stats (kebab ids, e.g. "claude").
    pub agents: Vec<String>,
    /// Distinct models observed in reported stats.
    pub models: Vec<String>,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cache_read_tokens: i64,
    pub cache_write_tokens: i64,
    pub reasoning_tokens: i64,
    pub cost: Option<f64>,
    pub tool_calls: i64,
}

#[derive(Debug, Serialize)]
pub(crate) struct SessionPlayerStatsResp {
    pub total_tasks: u64,
    pub players: Vec<SessionPlayerStatsDto>,
}

/// Display info per session player for finished-session views, where no live
/// WS snapshot is available to supply avatars/usernames.
#[derive(Debug, Serialize)]
pub(crate) struct ReportPlayerDto {
    pub player_id: Uuid,
    /// Owning user; `None` for anonymous/unlinked players.
    pub user_id: Option<Uuid>,
    pub display_name: String,
    pub avatar_url: Option<String>,
    pub username: Option<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct ActivityEventDto {
    pub event_kind: String,
    pub player_id: Uuid,
    pub player_display_name: String,
    pub task_id: Uuid,
    pub task_ordinal: i32,
    pub task_title: String,
    pub judge_name: Option<String>,
    pub point_delta: Option<i32>,
    /// Criteria-judge verdicts: per-criterion sheet summary.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<serde_json::Value>,
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub version: i64,
}

#[derive(Debug, Serialize)]
pub(crate) struct ActivityListResp {
    pub events: Vec<ActivityEventDto>,
}

/// Internal error → HTTP response mapping. Kept local so this module does
/// not pollute `AuthError`.
#[derive(Debug)]
pub enum SessionError {
    NotFound,
    Forbidden,
    InvalidName,
    InvalidStatus,
    InvalidTransition,
    ReportNotReady,
    OwnerlessSession,
    JoinCodeExhausted,
    AlreadyRegenerated,
    /// FR-JC-006: session is in a terminal state.
    SessionClosed,
    /// FR-JC-008: caller is already a member or owner.
    AlreadyMember,
    /// FR-JC-005a: code format is invalid.
    InvalidCode,
    /// FR-JC-011: rate limit exceeded.
    RateLimited,
    /// Session already has `max_players_per_session` live players.
    SessionFull,
    /// Project does not exist.
    ProjectNotFound,
    /// Project is private and caller is not the owner.
    ProjectForbidden,
    /// Project is archived — cannot create sessions against it.
    ProjectArchived,
    /// The caller is already playing another live session — one at a time.
    ActiveSessionExists {
        join_code: String,
        project: String,
    },
    /// The project is a campaign parent: it is a table of contents, and the
    /// parts inside it are what you play.
    CampaignParent,
    /// A campaign part whose predecessor the caller has not completed.
    PartLocked {
        required_project: String,
        required_part_ordinal: i32,
    },
    Db(DbErr),
}

impl From<DbErr> for SessionError {
    fn from(e: DbErr) -> Self {
        SessionError::Db(e)
    }
}

crate::api::error::impl_api_error!(SessionError {
    Self::NotFound => (NOT_FOUND, "not_found"),
    Self::Forbidden | Self::OwnerlessSession => (FORBIDDEN, "forbidden"),
    Self::InvalidName => (UNPROCESSABLE_ENTITY, "invalid_name"),
    Self::InvalidStatus => (UNPROCESSABLE_ENTITY, "invalid_status"),
    Self::InvalidTransition => (CONFLICT, "invalid_transition"),
    Self::ReportNotReady => (CONFLICT, "report_not_ready"),
    Self::JoinCodeExhausted => (INTERNAL_SERVER_ERROR, "join_code_exhausted"),
    Self::AlreadyRegenerated => (CONFLICT, "already_regenerated"),
    Self::SessionClosed => (GONE, "session_closed"),
    Self::AlreadyMember => (CONFLICT, "already_member"),
    Self::InvalidCode => (UNPROCESSABLE_ENTITY, "invalid_code"),
    Self::RateLimited => (TOO_MANY_REQUESTS, "rate_limited"),
    Self::SessionFull => (CONFLICT, "session_full"),
    Self::ProjectNotFound => (NOT_FOUND, "project_not_found"),
    Self::ProjectForbidden => (FORBIDDEN, "project_forbidden"),
    Self::ProjectArchived => (CONFLICT, "project_archived"),
    Self::ActiveSessionExists { join_code, project } =>
        (CONFLICT, "active_session_exists", "active_join_code": join_code, "active_project": project),
    Self::CampaignParent => (CONFLICT, "campaign_project"),
    Self::PartLocked { required_project, required_part_ordinal } =>
        (CONFLICT, "part_locked", "required_project": required_project, "required_part_ordinal": required_part_ordinal),
    Self::Db(_) => (INTERNAL_SERVER_ERROR, "database_error"),
});
pub(crate) fn parse_user_id(claims: &AccessClaims) -> Result<Uuid, SessionError> {
    claims.user_id().map_err(|_| SessionError::Forbidden)
}

/// One live session per user: refuse when the caller already has a
/// non-revoked player row in a session that is still Lobby/Running/Paused.
/// Parallel play corrupts both runs — probes of browser projects share one
/// `agent-browser` instance per machine, and a single working directory is
/// the player's identity — so the platform enforces what the tooling
/// already assumes. `exempt` is the session the caller is (re)joining:
/// reconnecting to your own live session must always stay possible.
///
/// The error names the blocking session (join code + project) so the client
/// can say "finish or cancel THAT one" instead of a bare refusal.
pub(crate) async fn ensure_single_active_session(
    db: &DatabaseConnection,
    user_id: Uuid,
    exempt: Option<Uuid>,
) -> Result<(), SessionError> {
    use sea_orm::QuerySelect;
    let mut q = players::Entity::find()
        .find_also_related(sessions::Entity)
        .filter(players::Column::UserIdFk.eq(user_id))
        .filter(players::Column::RevokedAt.is_null())
        .filter(sessions::Column::Status.is_in([
            SessionStatus::Lobby,
            SessionStatus::Running,
            SessionStatus::Paused,
        ]));
    if let Some(exempt) = exempt {
        q = q.filter(players::Column::SessionIdFk.ne(exempt));
    }
    let Some((_, Some(blocking))) = q.limit(1).one(db).await? else {
        return Ok(());
    };
    let project = projects::Entity::find_by_id(blocking.project_id_fk)
        .one(db)
        .await?
        .map(|p| p.name)
        .unwrap_or_default();
    Err(SessionError::ActiveSessionExists {
        join_code: blocking.join_code,
        project,
    })
}

/// Campaign gating for session creation.
///
/// A campaign parent is a table of contents — it owns no tasks, so a session
/// on it would have nothing to run. A part opens only once the caller has a
/// completing run of the part before it (see
/// [`arena_core::campaign::user_completed_project`]), because the next part
/// continues the previous part's codebase.
///
/// The refusal names the part that has to be cleared first, so the CLI can
/// print "finish X" instead of a bare 409.
pub(crate) async fn ensure_campaign_unlocked(
    db: &DatabaseConnection,
    project: &projects::Model,
    user_id: Uuid,
) -> Result<(), SessionError> {
    let is_parent = projects::Entity::find()
        .filter(projects::Column::ParentProjectIdFk.eq(project.id))
        .one(db)
        .await?
        .is_some();
    if is_parent {
        return Err(SessionError::CampaignParent);
    }

    let (Some(parent_id), Some(ordinal)) = (project.parent_project_id_fk, project.part_ordinal)
    else {
        return Ok(());
    };
    if ordinal <= 0 {
        return Ok(());
    }

    let siblings = projects::Entity::find()
        .filter(projects::Column::ParentProjectIdFk.eq(parent_id))
        .all(db)
        .await?;
    let Some(previous) = arena_core::campaign::previous_part(&siblings, ordinal, |p| {
        p.part_ordinal.unwrap_or(i32::MIN)
    }) else {
        return Ok(());
    };

    if arena_core::campaign::user_completed_project(db, previous.id, user_id).await? {
        return Ok(());
    }
    Err(SessionError::PartLocked {
        required_project: previous
            .slug
            .clone()
            .unwrap_or_else(|| previous.name.clone()),
        required_part_ordinal: previous.part_ordinal.unwrap_or(0),
    })
}

pub(crate) fn validate_name(raw: &str) -> Result<String, SessionError> {
    let trimmed = raw.trim().to_string();
    let len = trimmed.chars().count();
    if !(NAME_MIN..=NAME_MAX).contains(&len) {
        return Err(SessionError::InvalidName);
    }
    Ok(trimmed)
}

pub(crate) fn to_summary(m: sessions::Model, join_code: Option<String>) -> SessionSummary {
    SessionSummary {
        id: m.id.to_string(),
        name: m.name,
        status: m.status.to_string(),
        owner_id: m.owner_id_fk,
        project_id: m.project_id_fk.to_string(),
        created_at: m.created_at,
        join_code,
        player_count: 0,
        best_player: None,
        best_score: None,
    }
}

/// Live player count for one session. Shared by the project listing (initial
/// render) and the ZMQ bridge (live updates) so both report the same number.
pub(crate) async fn count_players(db: &DatabaseConnection, session_id: Uuid) -> u32 {
    use sea_orm::PaginatorTrait;
    players::Entity::find()
        .filter(players::Column::SessionIdFk.eq(session_id))
        .filter(players::Column::RevokedAt.is_null())
        .count(db)
        .await
        .unwrap_or(0) as u32
}

/// Owner OR member visibility check. Returns `false` for outsiders.
pub(crate) async fn is_member_or_owner(
    db: &DatabaseConnection,
    session: &sessions::Model,
    user_id: Uuid,
) -> Result<bool, DbErr> {
    if session.owner_id_fk == Some(user_id) {
        return Ok(true);
    }
    let row = players::Entity::find()
        .filter(players::Column::SessionIdFk.eq(session.id))
        .filter(players::Column::UserIdFk.eq(user_id))
        .one(db)
        .await?;
    Ok(row.is_some())
}

/// Authorize read access to a session's public views (report / activity /
/// player-stats).
///
/// Sessions whose project is `public` are spectatable by anyone with the id or
/// join code — this preserves the finished-session spectator view at
/// `/s/:code`. Sessions of a non-public project are visible only to the owner
/// or a member; an anonymous or outsider request is rejected as `NotFound`, so
/// the join code cannot even confirm the session exists.
pub(crate) async fn authorize_session_view(
    db: &DatabaseConnection,
    session: &sessions::Model,
    claims: Option<&AccessClaims>,
) -> Result<(), SessionError> {
    let project = projects::Entity::find_by_id(session.project_id_fk)
        .one(db)
        .await?
        .ok_or(SessionError::NotFound)?;
    if project.public {
        return Ok(());
    }
    let Some(claims) = claims else {
        return Err(SessionError::NotFound);
    };
    let user_id = parse_user_id(claims)?;
    if !is_member_or_owner(db, session, user_id).await? {
        return Err(SessionError::NotFound);
    }
    Ok(())
}

/// Returns `true` when `e` represents a unique-constraint violation.
/// SeaORM surfaces these as `DbErr::Query(RuntimeErr::SqlxError(...))` where
/// the inner sqlx error is a database unique-constraint error.
pub(crate) fn is_unique_violation(e: &DbErr) -> bool {
    let msg = e.to_string().to_lowercase();
    msg.contains("unique") || msg.contains("duplicate")
}

/// Validate a transition request against the contract FR-009 lifecycle.
pub(crate) fn validate_transition(
    current: &SessionStatus,
    requested: &SessionStatus,
) -> Result<(), SessionError> {
    if current == requested {
        return Ok(());
    }
    let allowed = match current {
        SessionStatus::Lobby => {
            matches!(requested, SessionStatus::Running | SessionStatus::Cancelled)
        }
        SessionStatus::Running => matches!(
            requested,
            SessionStatus::Finished | SessionStatus::Cancelled | SessionStatus::Paused
        ),
        SessionStatus::Paused => {
            matches!(requested, SessionStatus::Running | SessionStatus::Cancelled)
        }
        // `finished` and `cancelled` are terminal (FR-009).
        SessionStatus::Finished | SessionStatus::Cancelled => false,
    };
    if allowed {
        Ok(())
    } else {
        Err(SessionError::InvalidTransition)
    }
}
pub(crate) fn fan_project_update(
    registry: &crate::state::ProjectRegistry,
    project_id: Uuid,
    frame: crate::protocol::ArenaFrame,
) {
    if let Some(tx) = registry.get(&project_id) {
        let _ = tx.send(frame);
    }
}

/// Broadcast a frame to the landing page's live-sessions observers — but only
/// when the session's project is public: a private project's join code is an
/// invite, and this channel publishes to anyone. The DB read is gated on
/// having subscribers, so idle instances pay nothing.
pub(crate) async fn fan_landing_update(
    state: &crate::state::AppState,
    project_id: Uuid,
    frame: crate::protocol::ArenaFrame,
) {
    if state.landing_tx.receiver_count() == 0 {
        return;
    }
    match projects::Entity::find_by_id(project_id)
        .one(&state.db)
        .await
    {
        Ok(Some(p)) if p.public => {
            let _ = state.landing_tx.send(frame);
        }
        _ => {}
    }
}

/// Broadcast a lifecycle frame to the session's own observer WS clients
/// (keyed by join_code in session_registry). The `/s/[code]` page subscribes
/// to this channel, so it learns about pause/resume/cancel transitions in
/// real time without a reload.
pub(crate) fn fan_session_update(
    registry: &crate::state::SessionRegistry,
    join_code: &str,
    frame: crate::protocol::ArenaFrame,
) {
    if let Some(entry) = registry.get(join_code) {
        let _ = entry.tx.send(frame);
    }
}
/// Load session and enforce visibility (owner OR member). Outsiders get
/// 404 (no leak of existence). Missing rows also yield 404.
pub(crate) async fn load_visible(
    db: &DatabaseConnection,
    id: Uuid,
    user_id: Uuid,
) -> Result<sessions::Model, SessionError> {
    let row = sessions::Entity::find_by_id(id)
        .one(db)
        .await?
        .ok_or(SessionError::NotFound)?;
    if !is_member_or_owner(db, &row, user_id).await? {
        return Err(SessionError::NotFound);
    }
    Ok(row)
}

/// Load session and require caller is the owner. Outsiders get 404
/// (no existence leak); members get 403.
pub(crate) async fn load_for_owner(
    db: &DatabaseConnection,
    id: Uuid,
    user_id: Uuid,
) -> Result<sessions::Model, SessionError> {
    let row = sessions::Entity::find_by_id(id)
        .one(db)
        .await?
        .ok_or(SessionError::NotFound)?;
    match row.owner_id_fk {
        Some(owner) if owner == user_id => Ok(row),
        Some(_) => {
            // Distinguish member vs outsider for status code accuracy.
            let is_member = players::Entity::find()
                .filter(players::Column::SessionIdFk.eq(row.id))
                .filter(players::Column::UserIdFk.eq(user_id))
                .one(db)
                .await?
                .is_some();
            if is_member {
                Err(SessionError::Forbidden)
            } else {
                Err(SessionError::NotFound)
            }
        }
        None => Err(SessionError::OwnerlessSession),
    }
}

/// Load session by PK with no ownership or membership check (admin paths
/// only). Missing → 404.
pub(crate) async fn load_any(
    db: &DatabaseConnection,
    id: Uuid,
) -> Result<sessions::Model, SessionError> {
    sessions::Entity::find_by_id(id)
        .one(db)
        .await?
        .ok_or(SessionError::NotFound)
}
pub(crate) fn validate_join_code(raw: &str) -> Result<String, SessionError> {
    let trimmed = raw.trim();
    if trimmed.len() != 6 {
        return Err(SessionError::InvalidCode);
    }
    if !trimmed.chars().all(|c| {
        c.is_ascii_alphabetic()
            || c == '2'
            || c == '3'
            || c == '4'
            || c == '5'
            || c == '6'
            || c == '7'
    }) {
        return Err(SessionError::InvalidCode);
    }
    Ok(trimmed.to_uppercase())
}

/// Extract the best available client IP from request headers or connection
/// info. See [`crate::client_ip`] for the header trust order.
pub(crate) fn extract_ip(
    headers: &HeaderMap,
    conn_info: Option<&ConnectInfo<SocketAddr>>,
) -> String {
    crate::client_ip::client_ip_str(headers, conn_info.map(|ci| ci.0.ip()))
}

#[cfg(test)]
mod tests;
