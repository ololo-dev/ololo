//! Admin-wide session registry.
//!
//! `GET /api/sessions` deliberately answers with the caller's own sessions —
//! the ones they own or joined. An admin looking for a stuck session has no
//! way to reach it there, because being an admin does not make them a member
//! of anything. This is the one listing scoped to the whole instance, so it
//! is paginated and filterable rather than returning every row ever created.

use crate::api::sessions::common::SessionError;
use crate::api::settings::common::AdminUser;
use crate::state::AppState;
use arena_core::entities::{players, projects, sessions, users};
use arena_core::session_status::SessionStatus;
use axum::Json;
use axum::extract::{Query, State};
use sea_orm::sea_query::{Expr, Func};
use sea_orm::{
    ColumnTrait, Condition, EntityTrait, PaginatorTrait, QueryFilter, QueryOrder, QuerySelect,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

#[derive(Debug, Deserialize)]
pub struct AdminSessionsQuery {
    pub page: Option<u64>,
    pub per_page: Option<u64>,
    /// One of the `SessionStatus` names; anything else is rejected rather
    /// than silently ignored, so a typo does not read as "no filter".
    pub status: Option<String>,
    pub project_id: Option<Uuid>,
    /// Matches a join code or a session name, case-insensitively.
    pub q: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct AdminSessionEntry {
    pub id: Uuid,
    pub join_code: String,
    pub name: String,
    pub status: String,
    pub project_id: Uuid,
    pub project_name: Option<String>,
    pub project_slug: Option<String>,
    pub owner_id: Option<Uuid>,
    pub owner_display_name: Option<String>,
    pub owner_username: Option<String>,
    pub player_count: u64,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub started_at: Option<chrono::DateTime<chrono::Utc>>,
    pub finished_at: Option<chrono::DateTime<chrono::Utc>>,
    /// `"user"` or `"idle_timeout"` for cancelled sessions; the admin list is
    /// where "why did this end?" is actually asked.
    pub cancel_reason: Option<String>,
    pub cancelled_by: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct AdminSessionsResp {
    pub sessions: Vec<AdminSessionEntry>,
    pub total: u64,
    pub page: u64,
    pub per_page: u64,
}

/// `GET /api/admin/sessions` — every session on the instance, newest first.
pub async fn get_admin_list(
    _admin: AdminUser,
    State(state): State<AppState>,
    Query(params): Query<AdminSessionsQuery>,
) -> Result<Json<AdminSessionsResp>, SessionError> {
    let page = params.page.unwrap_or(1).max(1);
    let per_page = params.per_page.unwrap_or(25).clamp(1, 100);

    let mut cond = Condition::all();
    if let Some(raw) = params
        .status
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        let status: SessionStatus = raw.parse().map_err(|_| SessionError::InvalidStatus)?;
        cond = cond.add(sessions::Column::Status.eq(status));
    }
    if let Some(project_id) = params.project_id {
        cond = cond.add(sessions::Column::ProjectIdFk.eq(project_id));
    }
    if let Some(q) = params.q.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        // A join code is the thing an admin has in hand — it is what a player
        // reports when something goes wrong — so it is matched as well as the
        // name. Both sides are lowered rather than left to `LIKE`, whose
        // case-sensitivity differs between SQLite and PostgreSQL: an admin
        // typing a code in lower case must not get an empty list on one
        // backend and a hit on the other.
        let needle = format!("%{}%", q.to_lowercase());
        cond = cond.add(
            Condition::any()
                .add(
                    Expr::expr(Func::lower(Expr::col(sessions::Column::JoinCode)))
                        .like(needle.clone()),
                )
                .add(Expr::expr(Func::lower(Expr::col(sessions::Column::Name))).like(needle)),
        );
    }

    let query = sessions::Entity::find().filter(cond);
    let total = query.clone().count(&state.db).await?;
    let rows = query
        .order_by_desc(sessions::Column::CreatedAt)
        .paginate(&state.db, per_page)
        .fetch_page(page - 1)
        .await?;

    // Project, owner and player count are resolved in one query each for the
    // whole page rather than per row: this list is the only place in the app
    // that can show sessions from every project at once, so a per-row lookup
    // would be a page-sized burst of queries on every filter change.
    let project_ids: Vec<Uuid> = rows.iter().map(|s| s.project_id_fk).collect();
    let projects_by_id: HashMap<Uuid, projects::Model> = if project_ids.is_empty() {
        HashMap::new()
    } else {
        projects::Entity::find()
            .filter(projects::Column::Id.is_in(project_ids))
            .all(&state.db)
            .await?
            .into_iter()
            .map(|p| (p.id, p))
            .collect()
    };

    let owner_ids: Vec<Uuid> = rows.iter().filter_map(|s| s.owner_id_fk).collect();
    let owners_by_id: HashMap<Uuid, users::Model> = if owner_ids.is_empty() {
        HashMap::new()
    } else {
        users::Entity::find()
            .filter(users::Column::Id.is_in(owner_ids))
            .all(&state.db)
            .await?
            .into_iter()
            .map(|u| (u.id, u))
            .collect()
    };

    let session_ids: Vec<Uuid> = rows.iter().map(|s| s.id).collect();
    let mut players_by_session: HashMap<Uuid, u64> = HashMap::new();
    if !session_ids.is_empty() {
        let counts: Vec<(Uuid, i64)> = players::Entity::find()
            .select_only()
            .column(players::Column::SessionIdFk)
            .column_as(players::Column::Id.count(), "n")
            .filter(players::Column::SessionIdFk.is_in(session_ids))
            .filter(players::Column::RevokedAt.is_null())
            .group_by(players::Column::SessionIdFk)
            .into_tuple()
            .all(&state.db)
            .await?;
        for (session_id, n) in counts {
            players_by_session.insert(session_id, n.max(0) as u64);
        }
    }

    let entries = rows
        .into_iter()
        .map(|s| {
            let project = projects_by_id.get(&s.project_id_fk);
            let owner = s.owner_id_fk.and_then(|id| owners_by_id.get(&id));
            AdminSessionEntry {
                player_count: players_by_session.get(&s.id).copied().unwrap_or(0),
                project_name: project.map(|p| p.name.clone()),
                project_slug: project.and_then(|p| p.slug.clone()),
                owner_display_name: owner.map(|u| u.display_name.clone()),
                owner_username: owner.and_then(|u| u.username.clone()),
                id: s.id,
                join_code: s.join_code,
                name: s.name,
                status: s.status.to_string(),
                project_id: s.project_id_fk,
                owner_id: s.owner_id_fk,
                created_at: s.created_at,
                started_at: s.started_at,
                finished_at: s.finished_at,
                cancel_reason: s.cancel_reason,
                cancelled_by: s.cancelled_by,
            }
        })
        .collect();

    Ok(Json(AdminSessionsResp {
        sessions: entries,
        total,
        page,
        per_page,
    }))
}
