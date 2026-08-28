//! `/api/sessions/:session_id/tasks/*` HTTP handlers.
//!
//! GET endpoints (list / one) remain functional: they load the session,
//! derive the `project_id_fk`, and return tasks from the project's task
//! list. This lets existing agent observers and session UIs read tasks
//! via the session route without migration.
//!
//! Write endpoints (POST / PATCH / DELETE) return `410 Gone` with a
//! `moved_to` pointer to the new project-scoped route. Task creation
//! and mutation must now go through `POST /api/projects/:id/tasks`.
//! No auth check beyond JWT presence is performed on the 410 path —
//! just a session existence check to produce the correct project_id in
//! the redirect body.

use crate::auth::jwt::AccessClaims;
use crate::state::AppState;
use arena_core::entities::{players, sessions, tasks};
use arena_core::task_template::TestTemplate;
use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use sea_orm::{ColumnTrait, DatabaseConnection, DbErr, EntityTrait, QueryFilter, QueryOrder};
use serde::Serialize;
use uuid::Uuid;

// ─── Response shapes ───────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
struct TaskSummary {
    id: Uuid,
    /// tasks are now project-owned; field renamed from `session_id`.
    project_id: Uuid,
    ordinal: i32,
    title: String,
    description: String,
    test_template: TestTemplate,
    created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Serialize)]
struct TaskListResp {
    tasks: Vec<TaskSummary>,
}

// ─── Error type ────────────────────────────────────────────────────────────

#[derive(Debug)]
pub enum TaskError {
    NotFound,
    /// Internal data-shape error (corruption or serialiser bug). Maps to 500.
    DataCorruption,
    Db(DbErr),
}

impl From<DbErr> for TaskError {
    fn from(e: DbErr) -> Self {
        TaskError::Db(e)
    }
}

crate::api::error::impl_api_error!(TaskError {
    Self::NotFound => (NOT_FOUND, "not_found"),
    Self::DataCorruption | Self::Db(_) => (INTERNAL_SERVER_ERROR, "database_error"),
});

// ─── Helpers ───────────────────────────────────────────────────────────────

fn parse_user_id(claims: &AccessClaims) -> Result<Uuid, TaskError> {
    claims.user_id().map_err(|_| TaskError::NotFound)
}

fn template_from_json(v: serde_json::Value) -> Result<TestTemplate, TaskError> {
    serde_json::from_value(v).map_err(|_| TaskError::DataCorruption)
}

fn to_summary(m: tasks::Model) -> Result<TaskSummary, TaskError> {
    let tpl = template_from_json(m.test_template)?;
    Ok(TaskSummary {
        id: m.id,
        project_id: m.project_id_fk,
        ordinal: m.ordinal,
        title: m.title,
        description: m.content,
        test_template: tpl,
        created_at: m.created_at,
    })
}

/// Load session for visibility (owner OR member). Outsiders get 404.
async fn load_session_visible(
    db: &DatabaseConnection,
    id: Uuid,
    user_id: Uuid,
) -> Result<sessions::Model, TaskError> {
    let row = sessions::Entity::find_by_id(id)
        .one(db)
        .await?
        .ok_or(TaskError::NotFound)?;
    if row.owner_id_fk == Some(user_id) {
        return Ok(row);
    }
    let is_member = players::Entity::find()
        .filter(players::Column::SessionIdFk.eq(row.id))
        .filter(players::Column::UserIdFk.eq(user_id))
        .one(db)
        .await?
        .is_some();
    if is_member {
        Ok(row)
    } else {
        Err(TaskError::NotFound)
    }
}

/// Build a 410 Gone response pointing to the new project-scoped task route.
fn gone_response(project_id: Uuid) -> Response {
    (
        StatusCode::GONE,
        Json(serde_json::json!({
            "error": "route_gone",
            "moved_to": format!("/api/projects/{}/tasks", project_id),
            "project_id": project_id,
        })),
    )
        .into_response()
}

// ─── Handlers ──────────────────────────────────────────────────────────────

/// `GET /api/sessions/:session_id/tasks`
///
/// Still functional: returns tasks for the project this session belongs to,
/// ordered by ordinal ascending. Auth: owner or member.
pub async fn get_list(
    State(state): State<AppState>,
    claims: AccessClaims,
    Path(session_id): Path<Uuid>,
) -> Result<Response, TaskError> {
    let user_id = parse_user_id(&claims)?;
    let session = load_session_visible(&state.db, session_id, user_id).await?;
    let project_id = session.project_id_fk;

    let rows = tasks::Entity::find()
        .filter(tasks::Column::ProjectIdFk.eq(project_id))
        .order_by_asc(tasks::Column::Ordinal)
        .all(&state.db)
        .await?;
    let tasks_out: Vec<TaskSummary> = rows.into_iter().map(to_summary).collect::<Result<_, _>>()?;
    Ok(Json(TaskListResp { tasks: tasks_out }).into_response())
}

/// `GET /api/sessions/:session_id/tasks/:task_id`
///
/// Still functional. Auth: owner or member.
pub async fn get_one(
    State(state): State<AppState>,
    claims: AccessClaims,
    Path((session_id, task_id)): Path<(Uuid, Uuid)>,
) -> Result<Response, TaskError> {
    let user_id = parse_user_id(&claims)?;
    let session = load_session_visible(&state.db, session_id, user_id).await?;
    let project_id = session.project_id_fk;

    let row = tasks::Entity::find_by_id(task_id)
        .filter(tasks::Column::ProjectIdFk.eq(project_id))
        .one(&state.db)
        .await?
        .ok_or(TaskError::NotFound)?;
    Ok(Json(to_summary(row)?).into_response())
}

/// `POST /api/sessions/:session_id/tasks` → **410 Gone**
///
/// Route has moved to `POST /api/projects/:project_id/tasks`.
pub async fn post_create(
    State(state): State<AppState>,
    _claims: AccessClaims,
    Path(session_id): Path<Uuid>,
) -> Result<Response, TaskError> {
    let row = sessions::Entity::find_by_id(session_id)
        .one(&state.db)
        .await?
        .ok_or(TaskError::NotFound)?;
    Ok(gone_response(row.project_id_fk))
}

/// `PATCH /api/sessions/:session_id/tasks/:task_id` → **410 Gone**
pub async fn patch_one(
    State(state): State<AppState>,
    _claims: AccessClaims,
    Path((session_id, _task_id)): Path<(Uuid, Uuid)>,
) -> Result<Response, TaskError> {
    let row = sessions::Entity::find_by_id(session_id)
        .one(&state.db)
        .await?
        .ok_or(TaskError::NotFound)?;
    Ok(gone_response(row.project_id_fk))
}

/// `DELETE /api/sessions/:session_id/tasks/:task_id` → **410 Gone**
pub async fn delete_one(
    State(state): State<AppState>,
    _claims: AccessClaims,
    Path((session_id, _task_id)): Path<(Uuid, Uuid)>,
) -> Result<Response, TaskError> {
    let row = sessions::Entity::find_by_id(session_id)
        .one(&state.db)
        .await?
        .ok_or(TaskError::NotFound)?;
    Ok(gone_response(row.project_id_fk))
}
