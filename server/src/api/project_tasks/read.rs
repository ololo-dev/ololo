//! Read handlers: `GET list`, `GET one`.

use crate::api::project_tasks::common::*;
use crate::auth::jwt::AccessClaims;
use crate::state::AppState;
use arena_core::entities::{task_results, tasks};
use axum::Json;
use axum::extract::{Path, State};
use axum::response::{IntoResponse, Response};
use sea_orm::{
    ColumnTrait, ConnectionTrait, EntityTrait, PaginatorTrait, QueryFilter, QueryOrder, Statement,
};
use uuid::Uuid;
pub async fn get_list(
    State(state): State<AppState>,
    claims: AccessClaims,
    Path(project_id): Path<Uuid>,
) -> Result<Response, ProjectTaskError> {
    let user_id = parse_user_id(&claims)?;
    let project = load_accessible(&state.db, project_id, user_id).await?;
    let proj_int = proj_intervals(&project);

    let rows = tasks::Entity::find()
        .filter(tasks::Column::ProjectIdFk.eq(project_id))
        .order_by_asc(tasks::Column::Ordinal)
        .all(&state.db)
        .await?;

    // Raw SQL for session_count: COUNT(DISTINCT session_id_fk via players).
    let backend = state.db.get_database_backend();
    let session_count_sql = match backend {
        sea_orm::DatabaseBackend::Postgres => concat!(
            "SELECT COUNT(DISTINCT p.session_id_fk) as cnt ",
            "FROM task_results tr ",
            "JOIN players p ON p.id = tr.player_id_fk ",
            "WHERE tr.task_id = $1"
        ),
        _ => concat!(
            "SELECT COUNT(DISTINCT p.session_id_fk) as cnt ",
            "FROM task_results tr ",
            "JOIN players p ON p.id = tr.player_id_fk ",
            "WHERE tr.task_id = ?"
        ),
    };

    let mut tasks_out: Vec<TaskListItem> = Vec::with_capacity(rows.len());
    for row in rows {
        let tpl = template_from_json(row.test_template.clone())?;
        let tags = parse_tags(&row.tags, row.id);

        // Direct count via task_id FK on task_results.
        let result_count = task_results::Entity::find()
            .filter(task_results::Column::TaskId.eq(row.id))
            .count(&state.db)
            .await? as i64;

        // Raw SQL session count.
        let stmt = Statement::from_sql_and_values(backend, session_count_sql, [row.id.into()]);
        let session_count: i64 = state
            .db
            .query_one(stmt)
            .await?
            .and_then(|r| r.try_get("", "cnt").ok())
            .unwrap_or(0);

        tasks_out.push(TaskListItem {
            id: row.id,
            project_id: row.project_id_fk,
            ordinal: row.ordinal,
            title: row.title.clone(),
            description: row.content.clone(),
            test_template: tpl,
            tags,
            created_at: row.created_at,
            result_count,
            session_count,
            points: PointsResp {
                value: row.point_value,
                fail: row.fail_points,
                no_response: row.no_response_points,
                completion_bonus: row.completion_bonus_points,
            },
            intervals: resolve_intervals(
                row.deadline_secs,
                row.min_interval_secs,
                row.interval_increment_secs,
                row.max_interval_secs,
                &proj_int,
            ),
        });
    }

    Ok(Json(TaskListResp { tasks: tasks_out }).into_response())
}

pub async fn get_one(
    State(state): State<AppState>,
    claims: AccessClaims,
    Path((project_id, task_id)): Path<(Uuid, Uuid)>,
) -> Result<Response, ProjectTaskError> {
    let user_id = parse_user_id(&claims)?;
    let project = load_accessible(&state.db, project_id, user_id).await?;
    let proj_int = proj_intervals(&project);

    let row = tasks::Entity::find_by_id(task_id)
        .filter(tasks::Column::ProjectIdFk.eq(project_id))
        .one(&state.db)
        .await?
        .ok_or(ProjectTaskError::NotFound)?;
    Ok(Json(to_summary(row, &proj_int)?).into_response())
}

/// A judge attached to a previewed task — name and face only, never the
/// prompt or rating machinery.
#[derive(serde::Serialize)]
pub struct TaskPreviewJudge {
    pub slug: String,
    pub name: String,
    /// The judge's public blurb — feeds the hover card on the task row.
    pub description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avatar_url: Option<String>,
}

/// One row of the public task-arc preview: the brief a player reads before
/// committing to a session — never the machinery that grades it.
#[derive(serde::Serialize)]
pub struct TaskPreviewItem {
    pub ordinal: i32,
    pub title: String,
    /// The task's markdown brief (`content` in the seed).
    pub description: String,
    /// `point_value`: for a classic task the award per passing check; for an
    /// open-ended task the point budget the judge panel splits.
    pub points: i32,
    /// Open-ended tasks carry an `evaluation` contract — checks are
    /// measurements and the points come from the judge panel.
    pub open_ended: bool,
    /// Extra points for completing every check of the task.
    pub completion_bonus: i32,
    /// Judges attached to this task, in attachment order.
    pub judges: Vec<TaskPreviewJudge>,
}

#[derive(serde::Serialize)]
pub struct TaskPreviewResp {
    pub tasks: Vec<TaskPreviewItem>,
}

/// `GET /api/projects/:project_id/tasks/preview` — the task arc for the
/// public project page (audit UI-H2), auth optional.
///
/// Deliberately a separate endpoint from [`get_list`]: that one is the
/// authoring surface and ships the full `test_template` (fixtures,
/// validations, expected answers) to owners and admins. The preview carries
/// only what a player may see before playing — ordinal, title, brief,
/// points — and only when the project chose to show its ladder: quiz-shaped
/// projects (Extreme Startup family) set `show_tasks: false` because the
/// task list IS the answer sheet. Owners and admins see the preview
/// regardless (it is their own project).
pub async fn get_preview(
    State(state): State<AppState>,
    claims: Option<AccessClaims>,
    Path(project_id): Path<Uuid>,
) -> Result<Response, ProjectTaskError> {
    use arena_core::entities::projects;

    let project = projects::Entity::find_by_id(project_id)
        .one(&state.db)
        .await?
        .ok_or(ProjectTaskError::NotFound)?;

    let caller = claims.as_ref().and_then(|c| c.user_id().ok());
    let privileged = match caller {
        Some(user_id) => {
            project.owner_user_id_fk == user_id
                || crate::auth::is_user_admin(&state.db, user_id).await?
        }
        None => false,
    };
    if !privileged {
        // A hidden ladder and a private project both answer NotFound — the
        // preview must not confirm what it refuses to show.
        if !project.public || !project.show_tasks {
            return Err(ProjectTaskError::NotFound);
        }
    }

    let rows = tasks::Entity::find()
        .filter(tasks::Column::ProjectIdFk.eq(project_id))
        .order_by_asc(tasks::Column::Ordinal)
        .all(&state.db)
        .await?;

    // Batch-load judge attachments for the whole arc: the preview names who
    // grades each task (audit follow-up to UI-H2) without an N+1.
    let task_ids: Vec<Uuid> = rows.iter().map(|t| t.id).collect();
    let mut judges_by_task: std::collections::HashMap<Uuid, Vec<TaskPreviewJudge>> =
        std::collections::HashMap::new();
    if !task_ids.is_empty() {
        use arena_core::entities::{judges, task_judges};
        let mut attachments = task_judges::Entity::find()
            .filter(task_judges::Column::TaskId.is_in(task_ids))
            .all(&state.db)
            .await?;
        attachments.sort_by_key(|tj| tj.ordinal);
        let judge_ids: Vec<Uuid> = attachments.iter().map(|tj| tj.judge_id).collect();
        let judge_by_id: std::collections::HashMap<Uuid, judges::Model> = judges::Entity::find()
            .filter(judges::Column::Id.is_in(judge_ids))
            .all(&state.db)
            .await?
            .into_iter()
            .map(|j| (j.id, j))
            .collect();
        for tj in attachments {
            if let Some(j) = judge_by_id.get(&tj.judge_id) {
                judges_by_task
                    .entry(tj.task_id)
                    .or_default()
                    .push(TaskPreviewJudge {
                        slug: j.slug.clone(),
                        name: j.name.clone(),
                        description: j.description.clone(),
                        avatar_url: j.avatar_url.clone(),
                    });
            }
        }
    }

    let tasks_out: Vec<TaskPreviewItem> = rows
        .into_iter()
        .map(|row| TaskPreviewItem {
            ordinal: row.ordinal,
            title: row.title,
            points: row.point_value,
            open_ended: row.evaluation.is_some(),
            completion_bonus: row.completion_bonus_points,
            judges: judges_by_task.remove(&row.id).unwrap_or_default(),
            description: row.content,
        })
        .collect();

    Ok(Json(TaskPreviewResp { tasks: tasks_out }).into_response())
}
