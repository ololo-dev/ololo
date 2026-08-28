use crate::auth::AuthError;
use crate::state::AppState;
use arena_core::entities::{
    judge_results, players, projects, sessions, task_agent_stats, task_results, users,
};
use axum::Json;
use axum::extract::{Path, Query, State};
use sea_orm::{ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter, QueryOrder, QuerySelect};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Serialize)]
#[serde(deny_unknown_fields)]
pub struct PublicUserDto {
    pub username: String,
    pub display_name: String,
    pub avatar_url: Option<String>,
    /// Arena Points this season (0 if unranked, `None` when the ladder is
    /// not part of this deployment).
    pub arena_points: Option<i64>,
    /// Position on the current-season ladder; `None` when no awards yet.
    pub leaderboard_rank: Option<u64>,
    /// Display skill rating; `None` until the user finishes a rated session.
    pub rating: Option<i64>,
    /// Account creation timestamp (RFC 3339).
    pub joined_at: String,
}

#[derive(Deserialize)]
pub struct SessionsQuery {
    pub page: Option<u64>,
    pub per_page: Option<u64>,
}

#[derive(Serialize)]
pub struct PublicSessionEntry {
    pub session_id: String,
    pub name: String,
    pub session_datetime: String,
    pub participant_count: u64,
    pub status: String,
    pub join_code: String,
    pub project_id: uuid::Uuid,
    pub project_name: String,
    pub project_slug: Option<String>,
    /// Arena Points this user earned in this session; `None` until awarded
    /// (session unfinished, or predates the leaderboard).
    pub arena_points: Option<i64>,
    /// Raw in-game score: `SUM(point_delta)` across the user's task and
    /// judge results in this session. Can be negative.
    pub game_points: i64,
    /// Final place on this session's leaderboard (from the award row);
    /// `None` until awarded.
    pub placement: Option<i32>,
    /// Coding agent the player used, e.g. "claude" / "opencode". Prefers the
    /// agents actually observed in client-reported stats, falling back to the
    /// agent declared in the player's join metadata. `None` when neither.
    pub agent: Option<String>,
    /// Models observed in client-reported stats, if any.
    pub models: Vec<String>,
}

#[derive(Serialize)]
pub struct PublicSessionsResponse {
    pub sessions: Vec<PublicSessionEntry>,
    pub total: u64,
    pub page: u64,
    pub per_page: u64,
}

/// `GET /api/users/by-username/:username` — public profile lookup (unauthenticated).
#[tracing::instrument(level = "info", skip_all, fields(username = %username))]
pub async fn get_by_username(
    State(state): State<AppState>,
    Path(username): Path<String>,
) -> Result<Json<PublicUserDto>, AuthError> {
    let user = users::Entity::find()
        .filter(users::Column::Username.eq(&username))
        .one(&state.db)
        .await?
        .ok_or(AuthError::UserNotFound)?;
    let uname = user.username.ok_or(AuthError::UserNotFound)?;

    Ok(Json(PublicUserDto {
        username: uname,
        display_name: user.display_name,
        avatar_url: user.avatar_url,
        arena_points: None,
        leaderboard_rank: None,
        rating: None,
        joined_at: user.created_at.to_rfc3339(),
    }))
}

/// `GET /api/users/by-username/:username/sessions` — paginated public sessions.
#[tracing::instrument(level = "info", skip_all, fields(username = %username))]
pub async fn get_sessions_by_username(
    State(state): State<AppState>,
    Path(username): Path<String>,
    Query(params): Query<SessionsQuery>,
) -> Result<Json<PublicSessionsResponse>, AuthError> {
    let user = users::Entity::find()
        .filter(users::Column::Username.eq(&username))
        .one(&state.db)
        .await?
        .ok_or(AuthError::UserNotFound)?;

    let page = params.page.unwrap_or(1).max(1);
    let per_page = params.per_page.unwrap_or(20).clamp(1, 50);

    let total = players::Entity::find()
        .filter(players::Column::UserIdFk.eq(user.id))
        .count(&state.db)
        .await?;

    let members = players::Entity::find()
        .filter(players::Column::UserIdFk.eq(user.id))
        .order_by_desc(players::Column::JoinedAt)
        .paginate(&state.db, per_page)
        .fetch_page(page - 1)
        .await?;

    let session_ids: Vec<Uuid> = members.iter().map(|m| m.session_id_fk).collect();
    // Where this user finished in each of those sessions, re-derived from the
    // scoring tables under the leaderboard sort rule.
    let placement_by_session: std::collections::HashMap<Uuid, i32> =
        arena_core::scoring::session_standings(&state.db, &session_ids)
            .await?
            .into_iter()
            .filter_map(|(sid, rows)| {
                rows.into_iter()
                    .find(|r| r.user_id == Some(user.id))
                    .map(|r| (sid, r.placement))
            })
            .collect();

    // Raw in-game score per player row: task + judge point deltas, summed
    // in SQL and merged here (player ids are globally unique, so no session
    // qualifier is needed).
    let member_ids: Vec<Uuid> = members.iter().map(|m| m.id).collect();
    let stat_member_ids = member_ids.clone();
    let mut score_by_player: std::collections::HashMap<Uuid, i64> =
        std::collections::HashMap::new();
    let task_sums: Vec<(Uuid, i64)> = task_results::Entity::find()
        .select_only()
        .column(task_results::Column::PlayerIdFk)
        .column_as(task_results::Column::PointDelta.sum(), "points")
        .filter(task_results::Column::PlayerIdFk.is_in(member_ids.clone()))
        .group_by(task_results::Column::PlayerIdFk)
        .into_tuple()
        .all(&state.db)
        .await?;
    let judge_sums: Vec<(Uuid, i64)> = judge_results::Entity::find()
        .select_only()
        .column(judge_results::Column::PlayerIdFk)
        .column_as(judge_results::Column::PointDelta.sum(), "points")
        .filter(judge_results::Column::PlayerIdFk.is_in(member_ids))
        .group_by(judge_results::Column::PlayerIdFk)
        .into_tuple()
        .all(&state.db)
        .await?;
    for (player_id, points) in task_sums.into_iter().chain(judge_sums) {
        *score_by_player.entry(player_id).or_insert(0) += points;
    }

    // Agents/models per player from client-reported task stats (batched, so
    // the per-session loop below stays free of extra queries).
    let mut agents_by_player: std::collections::HashMap<
        Uuid,
        (
            std::collections::BTreeSet<String>,
            std::collections::BTreeSet<String>,
        ),
    > = std::collections::HashMap::new();
    let stat_rows = task_agent_stats::Entity::find()
        .filter(task_agent_stats::Column::PlayerIdFk.is_in(stat_member_ids))
        .all(&state.db)
        .await
        .unwrap_or_default();
    for row in &stat_rows {
        if let Ok(sessions_stats) =
            serde_json::from_str::<Vec<arena_core::protocol::AgentSessionStats>>(&row.agents_json)
        {
            let entry = agents_by_player.entry(row.player_id_fk).or_default();
            for st in sessions_stats {
                entry.0.insert(st.agent);
                if let Some(m) = st.model {
                    entry.1.insert(m);
                }
            }
        }
    }

    let mut session_entries = Vec::with_capacity(members.len());
    for member in members {
        if let Some(session) = sessions::Entity::find_by_id(member.session_id_fk)
            .one(&state.db)
            .await?
        {
            let participant_count = players::Entity::find()
                .filter(players::Column::SessionIdFk.eq(session.id))
                .count(&state.db)
                .await?;
            let project = projects::Entity::find_by_id(session.project_id_fk)
                .one(&state.db)
                .await?;
            session_entries.push(PublicSessionEntry {
                session_id: session.id.to_string(),
                name: session.name,
                session_datetime: member.joined_at.to_rfc3339(),
                participant_count,
                status: session.status.to_string(),
                join_code: session.join_code,
                project_id: session.project_id_fk,
                project_name: project.as_ref().map(|p| p.name.clone()).unwrap_or_default(),
                project_slug: project.and_then(|p| p.slug),
                arena_points: None,
                game_points: score_by_player.get(&member.id).copied().unwrap_or(0),
                placement: placement_by_session.get(&session.id).copied(),
                agent: agents_by_player
                    .get(&member.id)
                    .filter(|(a, _)| !a.is_empty())
                    .map(|(a, _)| a.iter().cloned().collect::<Vec<_>>().join(", "))
                    .or_else(|| {
                        arena_core::scoring::parse_agent_display_name(
                            member.metadata_json.as_deref(),
                        )
                    }),
                models: agents_by_player
                    .get(&member.id)
                    .map(|(_, m)| m.iter().cloned().collect())
                    .unwrap_or_default(),
            });
        }
    }

    Ok(Json(PublicSessionsResponse {
        sessions: session_entries,
        total,
        page,
        per_page,
    }))
}
