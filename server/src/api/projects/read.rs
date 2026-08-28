use super::common::*;
use crate::auth::jwt::AccessClaims;
use crate::state::AppState;
use arena_core::entities::{
    categories, judges, players, projects, sessions, task_judges, tasks, users,
};
use axum::Json;
use axum::extract::{Path, Query, State};
use axum::response::{IntoResponse, Response};
use sea_orm::sea_query::Expr;
use sea_orm::{
    ColumnTrait, Condition, EntityTrait, JoinType, PaginatorTrait, QueryFilter, QueryOrder,
    QuerySelect, RelationTrait,
};
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use uuid::Uuid;
pub async fn get_list(
    State(state): State<AppState>,
    optional_claims: Option<AccessClaims>,
    Query(query): Query<ListProjectsQuery>,
) -> Result<Response, ProjectError> {
    // Resolve caller identity and admin status (DB lookup — no JWT claim for is_admin).
    let (caller_id, is_admin) = if let Some(ref claims) = optional_claims {
        if let Ok(user_id) = parse_user_id(claims) {
            let admin = crate::auth::is_user_admin(&state.db, user_id).await?;
            (Some(user_id), admin)
        } else {
            (None, false)
        }
    } else {
        (None, false)
    };

    let mut q = projects::Entity::find();

    // Admins see all projects; others see own + public.
    if !is_admin {
        let mut condition = Condition::any().add(projects::Column::Public.eq(true));
        if let Some(uid) = caller_id {
            condition = condition.add(projects::Column::OwnerUserIdFk.eq(uid));
        }
        q = q.filter(condition);
    }

    if !query.include_archived {
        q = q.filter(projects::Column::ArchivedAt.is_null());
    }
    let rows = q.all(&state.db).await?;

    // Batch-load task counts per project.
    let project_ids: Vec<Uuid> = rows.iter().map(|r| r.id).collect();
    let mut task_counts: std::collections::HashMap<Uuid, i64> = std::collections::HashMap::new();
    if !project_ids.is_empty() {
        let counts = tasks::Entity::find()
            .filter(tasks::Column::ProjectIdFk.is_in(project_ids))
            .select_only()
            .column(tasks::Column::ProjectIdFk)
            .column_as(tasks::Column::Id.count(), "count")
            .group_by(tasks::Column::ProjectIdFk)
            .into_tuple::<(Uuid, i64)>()
            .all(&state.db)
            .await?;
        for (pid, c) in counts {
            task_counts.insert(pid, c);
        }
    }

    // Batch-load played counters (finished sessions) per project — the
    // catalog card shows "N played" so a browsing player can tell a
    // battle-tested challenge from a fresh upload (audit UI-M3).
    let mut session_counts: HashMap<Uuid, i64> = HashMap::new();
    {
        let ids: Vec<Uuid> = rows.iter().map(|r| r.id).collect();
        if !ids.is_empty() {
            let counts = sessions::Entity::find()
                .filter(sessions::Column::ProjectIdFk.is_in(ids))
                .filter(sessions::Column::Status.eq("finished"))
                .select_only()
                .column(sessions::Column::ProjectIdFk)
                .column_as(sessions::Column::Id.count(), "count")
                .group_by(sessions::Column::ProjectIdFk)
                .into_tuple::<(Uuid, i64)>()
                .all(&state.db)
                .await?;
            for (pid, c) in counts {
                session_counts.insert(pid, c);
            }
        }
    }

    // Batch-load estimated judge reviews per project: total judges attached
    // across the project's tasks (each runs once per task per player, and
    // counts toward the monthly judge-run quota). Shown on cards and the
    // project page so a player knows the review cost before starting.
    let mut judge_counts: HashMap<Uuid, i64> = HashMap::new();
    {
        let ids: Vec<Uuid> = rows.iter().map(|r| r.id).collect();
        if !ids.is_empty() {
            let counts = task_judges::Entity::find()
                .join(JoinType::InnerJoin, task_judges::Relation::Task.def())
                .filter(tasks::Column::ProjectIdFk.is_in(ids))
                .select_only()
                .column(tasks::Column::ProjectIdFk)
                .column_as(task_judges::Column::Id.count(), "count")
                .group_by(tasks::Column::ProjectIdFk)
                .into_tuple::<(Uuid, i64)>()
                .all(&state.db)
                .await?;
            for (pid, c) in counts {
                judge_counts.insert(pid, c);
            }
        }
    }

    // Batch-load the signed-in caller's own played counter per project:
    // distinct finished sessions they joined as a player. Powers the per-user
    // "you've played this" card indicator. Skipped for anonymous callers.
    let mut user_session_counts: HashMap<Uuid, i64> = HashMap::new();
    if let Some(uid) = caller_id {
        let counts = players::Entity::find()
            .join(JoinType::InnerJoin, players::Relation::Session.def())
            .filter(players::Column::UserIdFk.eq(uid))
            .filter(sessions::Column::Status.eq("finished"))
            .select_only()
            .column(sessions::Column::ProjectIdFk)
            .expr_as(
                Expr::col((sessions::Entity, sessions::Column::Id)).count_distinct(),
                "cnt",
            )
            .group_by(sessions::Column::ProjectIdFk)
            .into_tuple::<(Uuid, i64)>()
            .all(&state.db)
            .await?;
        for (pid, c) in counts {
            user_session_counts.insert(pid, c);
        }
    }

    // Campaign context: how many parts each parent has (the catalog badge),
    // and the identity of every referenced parent (the "Part N of …" chip on
    // a child card). Parents are fetched by id rather than read off `rows`,
    // because a private parent can own public parts.
    let mut part_counts: HashMap<Uuid, i64> = HashMap::new();
    // Playing time of a campaign is its parts added up; its own duration is
    // a default nobody plays.
    let mut parts_durations: HashMap<Uuid, i64> = HashMap::new();
    {
        let ids: Vec<Uuid> = rows.iter().map(|r| r.id).collect();
        if !ids.is_empty() {
            let counts = projects::Entity::find()
                .filter(projects::Column::ParentProjectIdFk.is_in(ids))
                .select_only()
                .column(projects::Column::ParentProjectIdFk)
                .column_as(projects::Column::Id.count(), "count")
                // CAST: on Postgres SUM(bigint) yields NUMERIC, which does
                // not decode as i64 — SQLite returns INTEGER either way.
                .column_as(
                    projects::Column::DefaultSessionDurationSecs
                        .sum()
                        .cast_as(sea_orm::sea_query::Alias::new("BIGINT")),
                    "duration",
                )
                .group_by(projects::Column::ParentProjectIdFk)
                .into_tuple::<(Uuid, i64, Option<i64>)>()
                .all(&state.db)
                .await?;
            for (pid, c, duration) in counts {
                part_counts.insert(pid, c);
                if let Some(d) = duration {
                    parts_durations.insert(pid, d);
                }
            }
        }
    }
    let mut parent_identity: HashMap<Uuid, (Option<String>, String)> = HashMap::new();
    {
        let parent_ids: Vec<Uuid> = rows
            .iter()
            .filter_map(|r| r.parent_project_id_fk)
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();
        if !parent_ids.is_empty() {
            for p in projects::Entity::find()
                .filter(projects::Column::Id.is_in(parent_ids))
                .all(&state.db)
                .await?
            {
                parent_identity.insert(p.id, (p.slug, p.name));
            }
        }
    }

    let projects_out: Vec<ProjectSummary> = rows
        .into_iter()
        .map(|r| {
            let tc = *task_counts.get(&r.id).unwrap_or(&0);
            let sc = *session_counts.get(&r.id).unwrap_or(&0);
            let jc = *judge_counts.get(&r.id).unwrap_or(&0);
            let mut summary = to_summary_with_sessions(r, false, tc, None, Some(sc));
            summary.judge_review_count = Some(jc);
            summary.part_count = Some(*part_counts.get(&summary.id).unwrap_or(&0));
            summary.parts_duration_secs = parts_durations.get(&summary.id).copied();
            if let Some((slug, name)) = summary
                .parent_project_id
                .and_then(|pid| parent_identity.get(&pid))
            {
                summary.parent_project_slug = slug.clone();
                summary.parent_project_name = Some(name.clone());
            }
            // Present the per-user counter only to signed-in callers.
            if caller_id.is_some() {
                summary.user_session_count =
                    Some(*user_session_counts.get(&summary.id).unwrap_or(&0));
            }
            summary
        })
        .collect();
    Ok(Json(ProjectListResp {
        projects: projects_out,
    })
    .into_response())
}

/// `POST /api/projects`
#[tracing::instrument(level = "info", skip_all)]
pub async fn get_one(
    State(state): State<AppState>,
    optional_claims: Option<AccessClaims>,
    Path(id): Path<Uuid>,
) -> Result<Response, ProjectError> {
    let row = match optional_claims.as_ref() {
        Some(claims) => {
            let user_id = parse_user_id(claims)?;
            let is_admin = crate::auth::is_user_admin(&state.db, user_id).await?;
            if is_admin {
                load_any(&state.db, id).await?
            } else {
                load_accessible(&state.db, id, user_id).await?
            }
        }
        None => {
            let row = projects::Entity::find_by_id(id)
                .one(&state.db)
                .await?
                .ok_or(ProjectError::NotFound)?;
            if !row.public {
                return Err(ProjectError::Forbidden);
            }
            row
        }
    };
    let active = project_has_active_sessions(&state.db, row.id).await?;
    let tc = tasks::Entity::find()
        .filter(tasks::Column::ProjectIdFk.eq(row.id))
        .count(&state.db)
        .await?;
    let range = compute_points_range(&state.db, row.id).await?;
    let jc = judge_review_count(&state.db, row.id).await?;
    let mut summary = to_summary(row, active, tc as i64, range);
    summary.judge_review_count = Some(jc);
    attach_campaign_context(&state.db, &mut summary).await?;
    Ok(Json(summary).into_response())
}

/// `PATCH /api/projects/:id`
///
/// Owner-only. Archived projects reject all mutations unless
/// `archived: false` is explicitly set (R-8).
#[tracing::instrument(level = "info", skip_all, fields(project_id = %slug))]
pub async fn get_by_slug(
    State(state): State<AppState>,
    optional_claims: Option<AccessClaims>,
    Path(slug): Path<String>,
) -> Result<Response, ProjectError> {
    let row = projects::Entity::find()
        .filter(projects::Column::Slug.eq(&slug))
        .one(&state.db)
        .await?
        .ok_or(ProjectError::NotFound)?;

    if !row.public {
        match optional_claims.as_ref() {
            Some(claims) => {
                let user_id = parse_user_id(claims)?;
                let is_admin = crate::auth::is_user_admin(&state.db, user_id).await?;
                if !is_admin && row.owner_user_id_fk != user_id {
                    return Err(ProjectError::NotFound);
                }
            }
            None => return Err(ProjectError::NotFound),
        }
    }

    let active = project_has_active_sessions(&state.db, row.id).await?;
    let tc = tasks::Entity::find()
        .filter(tasks::Column::ProjectIdFk.eq(row.id))
        .count(&state.db)
        .await?;
    let range = compute_points_range(&state.db, row.id).await?;
    let jc = judge_review_count(&state.db, row.id).await?;
    let mut summary = to_summary(row, active, tc as i64, range);
    summary.judge_review_count = Some(jc);
    attach_campaign_context(&state.db, &mut summary).await?;
    Ok(Json(summary).into_response())
}

/// `GET /api/projects/u/:user_id/by-slug/:slug`
///
/// Returns a project by owner + slug. Requires authentication as that owner.
/// Wrong owner returns 404 (slug enumeration protection — not 403).
pub async fn get_by_user_slug(
    State(state): State<AppState>,
    claims: AccessClaims,
    Path((user_id, slug)): Path<(Uuid, String)>,
) -> Result<Response, ProjectError> {
    let caller_id = parse_user_id(&claims)?;
    if caller_id != user_id {
        return Err(ProjectError::NotFound);
    }
    let row = projects::Entity::find()
        .filter(projects::Column::OwnerUserIdFk.eq(user_id))
        .filter(projects::Column::Slug.eq(&slug))
        .one(&state.db)
        .await?
        .ok_or(ProjectError::NotFound)?;
    let active = project_has_active_sessions(&state.db, row.id).await?;
    let tc = tasks::Entity::find()
        .filter(tasks::Column::ProjectIdFk.eq(row.id))
        .count(&state.db)
        .await?;
    let range = compute_points_range(&state.db, row.id).await?;
    let jc = judge_review_count(&state.db, row.id).await?;
    let mut summary = to_summary(row, active, tc as i64, range);
    summary.judge_review_count = Some(jc);
    attach_campaign_context(&state.db, &mut summary).await?;
    Ok(Json(summary).into_response())
}

/// Load a project by id, enforcing the same visibility rule as the public
/// project page: public projects are visible to anyone; private ones only to
/// the owner or an admin (otherwise 404, to avoid leaking existence).
async fn load_visible(
    state: &AppState,
    id: Uuid,
    optional_claims: Option<&AccessClaims>,
) -> Result<projects::Model, ProjectError> {
    let row = projects::Entity::find_by_id(id)
        .one(&state.db)
        .await?
        .ok_or(ProjectError::NotFound)?;
    if !row.public {
        match optional_claims {
            Some(claims) => {
                let user_id = parse_user_id(claims)?;
                let is_admin = crate::auth::is_user_admin(&state.db, user_id).await?;
                if !is_admin && row.owner_user_id_fk != user_id {
                    return Err(ProjectError::NotFound);
                }
            }
            None => return Err(ProjectError::NotFound),
        }
    }
    Ok(row)
}

/// A campaign part is a project, so it goes out as the same summary every
/// other listing uses — the campaign page then renders the very card the
/// catalog and the landing do — plus the caller's progression on it.
///
/// No `deny_unknown_fields` here: it is incompatible with `flatten`.
#[derive(Serialize)]
pub struct ProjectPartDto {
    #[serde(flatten)]
    pub project: ProjectSummary,
    /// Per-caller progression: `completed`, `in_progress`, `available` or
    /// `locked`.
    pub state: &'static str,
}

#[derive(Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectPartsResp {
    pub parts: Vec<ProjectPartDto>,
}

/// `GET /api/projects/:project_id/parts`
///
/// The campaign's parts in play order with the caller's progression state on
/// each. An ordinary project answers with an empty list, so the frontend can
/// ask unconditionally. Anonymous callers see the first part available and
/// the rest locked — the honest preview of what a sign-in buys.
pub async fn get_parts(
    State(state): State<AppState>,
    optional_claims: Option<AccessClaims>,
    Path(id): Path<Uuid>,
) -> Result<Response, ProjectError> {
    let project = load_visible(&state, id, optional_claims.as_ref()).await?;

    let children = projects::Entity::find()
        .filter(projects::Column::ParentProjectIdFk.eq(project.id))
        .order_by_asc(projects::Column::PartOrdinal)
        .all(&state.db)
        .await?;
    if children.is_empty() {
        return Ok(Json(ProjectPartsResp { parts: vec![] }).into_response());
    }

    let child_ids: Vec<Uuid> = children.iter().map(|c| c.id).collect();

    let mut task_counts: HashMap<Uuid, i64> = HashMap::new();
    for (pid, c) in tasks::Entity::find()
        .filter(tasks::Column::ProjectIdFk.is_in(child_ids.clone()))
        .select_only()
        .column(tasks::Column::ProjectIdFk)
        .column_as(tasks::Column::Id.count(), "count")
        .group_by(tasks::Column::ProjectIdFk)
        .into_tuple::<(Uuid, i64)>()
        .all(&state.db)
        .await?
    {
        task_counts.insert(pid, c);
    }

    // The rest of what the shared card shows, batched over the parts exactly
    // as the catalog batches it over the whole list.
    let mut session_counts: HashMap<Uuid, i64> = HashMap::new();
    for (pid, c) in sessions::Entity::find()
        .filter(sessions::Column::ProjectIdFk.is_in(child_ids.clone()))
        .filter(sessions::Column::Status.eq("finished"))
        .select_only()
        .column(sessions::Column::ProjectIdFk)
        .column_as(sessions::Column::Id.count(), "count")
        .group_by(sessions::Column::ProjectIdFk)
        .into_tuple::<(Uuid, i64)>()
        .all(&state.db)
        .await?
    {
        session_counts.insert(pid, c);
    }

    let mut judge_counts: HashMap<Uuid, i64> = HashMap::new();
    for (pid, c) in task_judges::Entity::find()
        .join(JoinType::InnerJoin, task_judges::Relation::Task.def())
        .filter(tasks::Column::ProjectIdFk.is_in(child_ids.clone()))
        .select_only()
        .column(tasks::Column::ProjectIdFk)
        .column_as(task_judges::Column::Id.count(), "count")
        .group_by(tasks::Column::ProjectIdFk)
        .into_tuple::<(Uuid, i64)>()
        .all(&state.db)
        .await?
    {
        judge_counts.insert(pid, c);
    }

    let caller_id = optional_claims
        .as_ref()
        .and_then(|claims| claims.user_id().ok());
    let (completed, live) = match caller_id {
        Some(uid) => (
            arena_core::campaign::user_completed_projects(&state.db, &child_ids, uid).await?,
            arena_core::campaign::user_live_projects(&state.db, &child_ids, uid).await?,
        ),
        None => Default::default(),
    };
    let states = arena_core::campaign::part_states(&child_ids, &completed, &live);

    // The caller's own play counter, the same per-user indicator the catalog
    // card carries. Skipped for anonymous callers, who have no history to show.
    let mut user_session_counts: HashMap<Uuid, i64> = HashMap::new();
    if let Some(uid) = caller_id {
        for (pid, c) in players::Entity::find()
            .join(JoinType::InnerJoin, players::Relation::Session.def())
            .filter(players::Column::UserIdFk.eq(uid))
            .filter(sessions::Column::ProjectIdFk.is_in(child_ids.clone()))
            .filter(sessions::Column::Status.eq("finished"))
            .select_only()
            .column(sessions::Column::ProjectIdFk)
            .expr_as(
                Expr::col((sessions::Entity, sessions::Column::Id)).count_distinct(),
                "cnt",
            )
            .group_by(sessions::Column::ProjectIdFk)
            .into_tuple::<(Uuid, i64)>()
            .all(&state.db)
            .await?
        {
            user_session_counts.insert(pid, c);
        }
    }

    let parent_slug = project.slug.clone();
    let parent_name = project.name.clone();
    let parts = children
        .into_iter()
        .zip(states)
        .map(|(c, part_state)| {
            let id = c.id;
            let mut summary = to_summary_with_sessions(
                c,
                false,
                *task_counts.get(&id).unwrap_or(&0),
                None,
                Some(*session_counts.get(&id).unwrap_or(&0)),
            );
            summary.judge_review_count = Some(*judge_counts.get(&id).unwrap_or(&0));
            summary.part_count = Some(0);
            summary.parent_project_slug = parent_slug.clone();
            summary.parent_project_name = Some(parent_name.clone());
            if caller_id.is_some() {
                summary.user_session_count = Some(*user_session_counts.get(&id).unwrap_or(&0));
            }
            ProjectPartDto {
                project: summary,
                state: part_state.as_str(),
            }
        })
        .collect();

    Ok(Json(ProjectPartsResp { parts }).into_response())
}

#[derive(Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectJudgeDto {
    pub slug: String,
    pub name: String,
    pub description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avatar_url: Option<String>,
}

#[derive(Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectJudgesResp {
    pub judges: Vec<ProjectJudgeDto>,
}

/// `GET /api/projects/:project_id/judges`
///
/// Distinct judges attached across all of the project's tasks, ordered by
/// first appearance (task ordinal, then attachment ordinal). Public, subject
/// to the same visibility rule as the project page.
pub async fn get_judges(
    State(state): State<AppState>,
    optional_claims: Option<AccessClaims>,
    Path(id): Path<Uuid>,
) -> Result<Response, ProjectError> {
    let project = load_visible(&state, id, optional_claims.as_ref()).await?;

    // Task ordinal per task id, so judges list in the order tasks present them.
    let task_rows = tasks::Entity::find()
        .filter(tasks::Column::ProjectIdFk.eq(project.id))
        .all(&state.db)
        .await?;
    let ordinal_by_task: HashMap<Uuid, i32> = task_rows.iter().map(|t| (t.id, t.ordinal)).collect();
    if task_rows.is_empty() {
        return Ok(Json(ProjectJudgesResp { judges: vec![] }).into_response());
    }

    let task_ids: Vec<Uuid> = task_rows.iter().map(|t| t.id).collect();
    let mut attachments = task_judges::Entity::find()
        .filter(task_judges::Column::TaskId.is_in(task_ids))
        .all(&state.db)
        .await?;
    // Order by (task ordinal, attachment ordinal) for a stable list.
    attachments.sort_by_key(|tj| {
        (
            ordinal_by_task
                .get(&tj.task_id)
                .copied()
                .unwrap_or(i32::MAX),
            tj.ordinal,
        )
    });

    // Distinct judge ids preserving that order.
    let mut seen: HashSet<Uuid> = HashSet::new();
    let mut ordered_judge_ids: Vec<Uuid> = Vec::new();
    for tj in &attachments {
        if seen.insert(tj.judge_id) {
            ordered_judge_ids.push(tj.judge_id);
        }
    }
    if ordered_judge_ids.is_empty() {
        return Ok(Json(ProjectJudgesResp { judges: vec![] }).into_response());
    }

    let judge_by_id: HashMap<Uuid, judges::Model> = judges::Entity::find()
        .filter(judges::Column::Id.is_in(ordered_judge_ids.clone()))
        .all(&state.db)
        .await?
        .into_iter()
        .map(|j| (j.id, j))
        .collect();

    let judges_out: Vec<ProjectJudgeDto> = ordered_judge_ids
        .iter()
        .filter_map(|jid| judge_by_id.get(jid))
        .map(|j| ProjectJudgeDto {
            slug: j.slug.clone(),
            name: j.name.clone(),
            description: j.description.clone(),
            avatar_url: j.avatar_url.clone(),
        })
        .collect();

    Ok(Json(ProjectJudgesResp { judges: judges_out }).into_response())
}

#[derive(Serialize)]
#[serde(deny_unknown_fields)]
pub struct TopPlayerDto {
    pub rank: u64,
    pub user_id: Uuid,
    pub username: Option<String>,
    pub display_name: String,
    pub avatar_url: Option<String>,
    /// The player's **best** single-session score in this project — not a
    /// total. Summing rewarded playing often rather than playing well, and
    /// let a grinder outrank someone with a better run. Rows predating the
    /// score record count as 0.
    pub game_points: i64,
    pub sessions_played: u64,
    /// Best (lowest) placement the player achieved in any of the project's
    /// sessions; 1 = first place. `None` if never placed.
    pub best_placement: Option<i32>,
    /// Campaign boards only: how many of the campaign's parts this player has
    /// cleared. `None` on an ordinary project, where the idea has no meaning
    /// — a session count is not progress through anything.
    pub parts_completed: Option<u64>,
}

#[derive(Serialize)]
#[serde(deny_unknown_fields)]
pub struct TopPlayersResp {
    /// Every season the project has ever been played in.
    pub players: Vec<TopPlayerDto>,
    /// The same board restricted to the current season. Separate rather than
    /// a query flag: the page shows both, and the split costs no extra query.
    pub season_players: Vec<TopPlayerDto>,
    /// Start of the current season (RFC3339) — labels the seasonal board.
    pub season_start: String,
    /// Campaign boards only: how many parts there are to clear, so a row's
    /// `parts_completed` can be read as progress rather than a bare number.
    pub parts_total: Option<u64>,
}

struct TopPlayerAgg {
    /// `None` until the first scored session is seen. Not seeded to 0:
    /// judges can take a whole session below zero, and a 0 floor would
    /// report a score the player never achieved.
    best_game_points: Option<i64>,
    sessions_played: u64,
    best_placement: Option<i32>,
}

/// One account's result in one finished session, derived from the scoring
/// tables (`session_standings`) rather than any award snapshot — the board
/// works the same on deployments without the global ladder.
struct ScoredRow {
    user_id: Uuid,
    session_id: Uuid,
    game_points: i64,
    placement: i32,
    finished_at: chrono::DateTime<chrono::Utc>,
}

/// Keep each player's best single-session score, plus their best placement
/// as a per-row MIN, and rank by it (user_id as final tiebreak keeps ranks
/// stable). Capped at 20 entries.
/// Campaign board: a player's score is the sum, over the campaign's parts, of
/// their best run in each. Replaying one part cannot beat clearing several,
/// and the per-part rule stays the same one an ordinary project uses.
fn rank_campaign_top_players<'a>(
    rows: impl Iterator<Item = &'a ScoredRow>,
    project_by_session: &HashMap<Uuid, Uuid>,
) -> Vec<(Uuid, TopPlayerAgg)> {
    let mut best_per_part: HashMap<(Uuid, Uuid), i64> = HashMap::new();
    let mut totals: HashMap<Uuid, TopPlayerAgg> = HashMap::new();

    for a in rows {
        let Some(part_id) = project_by_session.get(&a.session_id) else {
            continue;
        };
        let points = a.game_points;
        best_per_part
            .entry((a.user_id, *part_id))
            .and_modify(|best| *best = (*best).max(points))
            .or_insert(points);

        let e = totals.entry(a.user_id).or_insert(TopPlayerAgg {
            best_game_points: None,
            sessions_played: 0,
            best_placement: None,
        });
        e.sessions_played += 1;
        e.best_placement = Some(match e.best_placement {
            Some(p) => p.min(a.placement),
            None => a.placement,
        });
    }

    for ((user_id, _part), best) in best_per_part {
        if let Some(e) = totals.get_mut(&user_id) {
            e.best_game_points = Some(e.best_game_points.unwrap_or(0) + best);
        }
    }

    let mut ranked: Vec<(Uuid, TopPlayerAgg)> = totals.into_iter().collect();
    ranked.sort_by(|a, b| {
        b.1.best_game_points
            .cmp(&a.1.best_game_points)
            .then_with(|| a.0.cmp(&b.0))
    });
    ranked.truncate(20);
    ranked
}

fn rank_top_players<'a>(rows: impl Iterator<Item = &'a ScoredRow>) -> Vec<(Uuid, TopPlayerAgg)> {
    let mut by_user: HashMap<Uuid, TopPlayerAgg> = HashMap::new();
    for a in rows {
        let e = by_user.entry(a.user_id).or_insert(TopPlayerAgg {
            best_game_points: None,
            sessions_played: 0,
            best_placement: None,
        });
        let points = a.game_points;
        e.best_game_points = Some(match e.best_game_points {
            Some(p) => p.max(points),
            None => points,
        });
        e.sessions_played += 1;
        e.best_placement = Some(match e.best_placement {
            Some(p) => p.min(a.placement),
            None => a.placement,
        });
    }

    let mut ranked: Vec<(Uuid, TopPlayerAgg)> = by_user.into_iter().collect();
    ranked.sort_by(|a, b| {
        b.1.best_game_points
            .cmp(&a.1.best_game_points)
            .then_with(|| a.0.cmp(&b.0))
    });
    ranked.truncate(20);
    ranked
}

fn top_player_dtos(
    ranked: &[(Uuid, TopPlayerAgg)],
    user_by_id: &HashMap<Uuid, users::Model>,
    // Campaign boards pass how many parts each player cleared; an ordinary
    // project passes `None`, and the field stays absent from every row.
    parts_by_user: Option<&HashMap<Uuid, HashSet<Uuid>>>,
) -> Vec<TopPlayerDto> {
    ranked
        .iter()
        .enumerate()
        .filter_map(|(i, (uid, agg))| {
            let user = user_by_id.get(uid)?;
            Some(TopPlayerDto {
                rank: i as u64 + 1,
                user_id: *uid,
                username: user.username.clone(),
                display_name: user.display_name.clone(),
                avatar_url: user.avatar_url.clone(),
                game_points: agg.best_game_points.unwrap_or(0),
                sessions_played: agg.sessions_played,
                best_placement: agg.best_placement,
                parts_completed: parts_by_user
                    .map(|m| m.get(uid).map(|s| s.len() as u64).unwrap_or(0)),
            })
        })
        .collect()
}

/// `GET /api/projects/:project_id/top-players`
///
/// Best players for the project, twice over: all time, and the current
/// season alone. A season resets on the 1st, so the all-time board is what
/// keeps a project page from reading as abandoned early in a month, while
/// the seasonal one is what is still up for grabs. Public, subject to
/// project visibility.
pub async fn get_top_players(
    State(state): State<AppState>,
    optional_claims: Option<AccessClaims>,
    Path(id): Path<Uuid>,
) -> Result<Response, ProjectError> {
    let project = load_visible(&state, id, optional_claims.as_ref()).await?;
    let season_from = arena_core::seasons::season_start(chrono::Utc::now());

    // A campaign hosts no sessions of its own — its board is the sum of its
    // parts. Scoring by best-run-per-part and adding those up keeps the
    // per-project rule ("your best run counts") while rewarding a player for
    // getting further through the campaign, and stops a replayed part one
    // from out-scoring an honest run of all five.
    let part_ids: Vec<Uuid> = projects::Entity::find()
        .filter(projects::Column::ParentProjectIdFk.eq(project.id))
        .select_only()
        .column(projects::Column::Id)
        .into_tuple::<Uuid>()
        .all(&state.db)
        .await?;
    let is_campaign = !part_ids.is_empty();
    let parts_total = is_campaign.then_some(part_ids.len() as u64);
    let scored_part_ids = part_ids;
    let scored_project_ids = if is_campaign {
        scored_part_ids.clone()
    } else {
        vec![project.id]
    };

    // Only finished sessions count: a live session's score is still moving,
    // and the board freezing mid-run would leak partial results.
    let session_rows: Vec<(Uuid, Uuid, Option<chrono::DateTime<chrono::Utc>>)> =
        sessions::Entity::find()
            .filter(sessions::Column::ProjectIdFk.is_in(scored_project_ids))
            .filter(sessions::Column::FinishedAt.is_not_null())
            .select_only()
            .column(sessions::Column::Id)
            .column(sessions::Column::ProjectIdFk)
            .column(sessions::Column::FinishedAt)
            .into_tuple()
            .all(&state.db)
            .await?;
    let project_by_session: HashMap<Uuid, Uuid> = session_rows
        .iter()
        .map(|(sid, pid, _)| (*sid, *pid))
        .collect();
    let finished_by_session: HashMap<Uuid, chrono::DateTime<chrono::Utc>> = session_rows
        .iter()
        .filter_map(|(sid, _, fin)| fin.map(|f| (*sid, f)))
        .collect();
    let session_ids: Vec<Uuid> = session_rows.into_iter().map(|(sid, _, _)| sid).collect();
    if session_ids.is_empty() {
        return Ok(Json(TopPlayersResp {
            players: vec![],
            season_players: vec![],
            season_start: season_from.to_rfc3339(),
            parts_total,
        })
        .into_response());
    }

    // Standings re-derived from the scoring tables; anonymous players hold
    // their placement but only account-backed rows reach the board.
    let standings = arena_core::scoring::session_standings(&state.db, &session_ids).await?;
    let mut scored_rows: Vec<ScoredRow> = Vec::new();
    for (sid, rows) in &standings {
        let Some(finished_at) = finished_by_session.get(sid).copied() else {
            continue;
        };
        for r in rows {
            if let Some(user_id) = r.user_id {
                scored_rows.push(ScoredRow {
                    user_id,
                    session_id: *sid,
                    game_points: r.total_points,
                    placement: r.placement,
                    finished_at,
                });
            }
        }
    }

    let rank = |rows: Vec<&ScoredRow>| {
        if is_campaign {
            rank_campaign_top_players(rows.into_iter(), &project_by_session)
        } else {
            rank_top_players(rows.into_iter())
        }
    };
    let ranked = rank(scored_rows.iter().collect());
    let season_ranked = rank(
        scored_rows
            .iter()
            .filter(|a| a.finished_at >= season_from)
            .collect(),
    );

    // One lookup covers both boards; the seasonal one is a subset of users
    // on the all-time board only until the cap truncates it, so union them.
    let user_ids: Vec<Uuid> = ranked
        .iter()
        .chain(season_ranked.iter())
        .map(|(uid, _)| *uid)
        .collect();
    let user_by_id: HashMap<Uuid, users::Model> = users::Entity::find()
        .filter(users::Column::Id.is_in(user_ids.clone()))
        .all(&state.db)
        .await?
        .into_iter()
        .map(|u| (u.id, u))
        .collect();

    // How far each of these people got through the campaign — two queries for
    // the whole board. A part cleared counts once however often it was
    // replayed, which is what "3 of 5 parts" has to mean.
    let parts_by_user = if is_campaign {
        Some(
            arena_core::campaign::completed_projects_for_users(
                &state.db,
                &scored_part_ids,
                &user_ids,
            )
            .await?,
        )
    } else {
        None
    };

    Ok(Json(TopPlayersResp {
        players: top_player_dtos(&ranked, &user_by_id, parts_by_user.as_ref()),
        season_players: top_player_dtos(&season_ranked, &user_by_id, parts_by_user.as_ref()),
        season_start: season_from.to_rfc3339(),
        parts_total,
    })
    .into_response())
}

/// `GET /api/projects/categories`
///
/// Returns the list of available project categories from app_settings.
/// Returns an empty array if the `project_categories` key is absent or unparseable.
/// Public — no auth required.
pub async fn get_categories(
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, ProjectError> {
    // Honor the admin-set order (Settings → Categories drag-reorder writes
    // `ordinal`); the id tiebreak keeps never-reordered categories stable.
    // The landing groups and the catalog sidebar both read this list, so the
    // order chosen in settings flows straight through.
    let rows = categories::Entity::find()
        .order_by_asc(categories::Column::Ordinal)
        .order_by_asc(categories::Column::Id)
        .all(&state.db)
        .await?;

    let names: Vec<String> = rows.into_iter().map(|r| r.name).collect();
    Ok(Json(serde_json::json!({ "categories": names })))
}

/// Estimated judge reviews for one project: total judges attached across all
/// its tasks. Each runs once per task per player and counts toward the
/// monthly judge-run quota, so the number is the review cost of a full run.
async fn judge_review_count(
    db: &sea_orm::DatabaseConnection,
    project_id: Uuid,
) -> Result<i64, sea_orm::DbErr> {
    Ok(task_judges::Entity::find()
        .join(JoinType::InnerJoin, task_judges::Relation::Task.def())
        .filter(tasks::Column::ProjectIdFk.eq(project_id))
        .count(db)
        .await? as i64)
}
