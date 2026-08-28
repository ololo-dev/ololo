//! Task-attach judges API.

use crate::api::settings::AdminUser;
use crate::state::AppState;
use arena_core::entities::{judges, task_judges, tasks};
use arena_core::validation::judges::validate_rating_scale;
use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use chrono::{DateTime, Utc};
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, QueryOrder, Set};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AttachRequest {
    pub judge_id: Uuid,
    pub ordinal: i32,
    pub rating_scale_override: Option<serde_json::Value>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpdateTaskJudgeRequest {
    #[serde(default)]
    pub ordinal: Option<i32>,
    #[serde(default, deserialize_with = "deserialize_some")]
    pub rating_scale_override: Option<serde_json::Value>,
}

fn deserialize_some<'de, T, D>(d: D) -> Result<Option<T>, D::Error>
where
    T: Deserialize<'de>,
    D: serde::Deserializer<'de>,
{
    Deserialize::deserialize(d).map(Some)
}

#[derive(Debug, Serialize)]
pub struct TaskJudgeResponse {
    pub id: Uuid,
    pub task_id: Uuid,
    pub judge_id: Uuid,
    pub judge_slug: String,
    pub judge_name: String,
    pub ordinal: i32,
    pub rating_scale_override: Option<serde_json::Value>,
    pub effective_rating_scale: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(thiserror::Error, Debug)]
pub enum TaskJudgesError {
    #[error("not found")]
    NotFound,
    #[error("conflict: {0}")]
    Conflict(String),
    #[error("bad request: {0}")]
    BadRequest(String),
    #[error(transparent)]
    Db(#[from] sea_orm::DbErr),
}

crate::api::error::impl_api_error!(TaskJudgesError {
    Self::NotFound => (NOT_FOUND, "not_found"),
    Self::Conflict(_) => (CONFLICT, "conflict"),
    Self::BadRequest(_) => (BAD_REQUEST, "bad_request"),
    Self::Db(_) => (INTERNAL_SERVER_ERROR, "database_error"),
});

async fn load_task(
    db: &sea_orm::DatabaseConnection,
    project_id: Uuid,
    task_id: Uuid,
) -> Result<tasks::Model, TaskJudgesError> {
    tasks::Entity::find_by_id(task_id)
        .filter(tasks::Column::ProjectIdFk.eq(project_id))
        .one(db)
        .await?
        .ok_or(TaskJudgesError::NotFound)
}

fn build_response(row: task_judges::Model, judge: judges::Model) -> TaskJudgeResponse {
    let effective = row
        .rating_scale_override
        .clone()
        .unwrap_or_else(|| judge.rating_scale.clone());
    TaskJudgeResponse {
        id: row.id,
        task_id: row.task_id,
        judge_id: row.judge_id,
        judge_slug: judge.slug,
        judge_name: judge.name,
        ordinal: row.ordinal,
        rating_scale_override: row.rating_scale_override,
        effective_rating_scale: effective,
        created_at: row.created_at,
        updated_at: row.updated_at,
    }
}

pub async fn list(
    _admin: AdminUser,
    State(state): State<AppState>,
    Path((project_id, task_id)): Path<(Uuid, Uuid)>,
) -> Result<Json<Vec<TaskJudgeResponse>>, TaskJudgesError> {
    load_task(&state.db, project_id, task_id).await?;

    let rows = task_judges::Entity::find()
        .filter(task_judges::Column::TaskId.eq(task_id))
        .order_by_asc(task_judges::Column::Ordinal)
        .all(&state.db)
        .await?;

    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let judge = judges::Entity::find_by_id(row.judge_id)
            .one(&state.db)
            .await?
            .ok_or(TaskJudgesError::NotFound)?;
        out.push(build_response(row, judge));
    }
    Ok(Json(out))
}

pub async fn attach(
    _admin: AdminUser,
    State(state): State<AppState>,
    Path((project_id, task_id)): Path<(Uuid, Uuid)>,
    Json(body): Json<AttachRequest>,
) -> Result<(StatusCode, Json<TaskJudgeResponse>), TaskJudgesError> {
    load_task(&state.db, project_id, task_id).await?;

    let judge = judges::Entity::find_by_id(body.judge_id)
        .one(&state.db)
        .await?
        .ok_or(TaskJudgesError::NotFound)?;

    if let Some(scale) = body.rating_scale_override.as_ref()
        && !scale.is_null()
    {
        validate_rating_scale(scale).map_err(TaskJudgesError::BadRequest)?;
    }

    let dup_judge = task_judges::Entity::find()
        .filter(task_judges::Column::TaskId.eq(task_id))
        .filter(task_judges::Column::JudgeId.eq(body.judge_id))
        .one(&state.db)
        .await?;
    if dup_judge.is_some() {
        return Err(TaskJudgesError::Conflict(
            "judge already attached to task".into(),
        ));
    }

    let dup_ord = task_judges::Entity::find()
        .filter(task_judges::Column::TaskId.eq(task_id))
        .filter(task_judges::Column::Ordinal.eq(body.ordinal))
        .one(&state.db)
        .await?;
    if dup_ord.is_some() {
        return Err(TaskJudgesError::Conflict(
            "ordinal already in use for task".into(),
        ));
    }

    let now = Utc::now();
    let id = Uuid::new_v4();
    let am = task_judges::ActiveModel {
        id: Set(id),
        task_id: Set(task_id),
        judge_id: Set(body.judge_id),
        ordinal: Set(body.ordinal),
        rating_scale_override: Set(body.rating_scale_override),
        weight: Set(None),
        created_at: Set(now),
        updated_at: Set(now),
    };
    let row = am.insert(&state.db).await?;
    Ok((StatusCode::CREATED, Json(build_response(row, judge))))
}

pub async fn update(
    _admin: AdminUser,
    State(state): State<AppState>,
    Path((project_id, task_id, judge_id)): Path<(Uuid, Uuid, Uuid)>,
    Json(body): Json<UpdateTaskJudgeRequest>,
) -> Result<Json<TaskJudgeResponse>, TaskJudgesError> {
    load_task(&state.db, project_id, task_id).await?;

    let row = task_judges::Entity::find()
        .filter(task_judges::Column::TaskId.eq(task_id))
        .filter(task_judges::Column::JudgeId.eq(judge_id))
        .one(&state.db)
        .await?
        .ok_or(TaskJudgesError::NotFound)?;

    if let Some(scale) = body.rating_scale_override.as_ref()
        && !scale.is_null()
    {
        validate_rating_scale(scale).map_err(TaskJudgesError::BadRequest)?;
    }

    if let Some(ordinal) = body.ordinal
        && ordinal != row.ordinal
    {
        let dup = task_judges::Entity::find()
            .filter(task_judges::Column::TaskId.eq(task_id))
            .filter(task_judges::Column::Ordinal.eq(ordinal))
            .filter(task_judges::Column::JudgeId.ne(judge_id))
            .one(&state.db)
            .await?;
        if dup.is_some() {
            return Err(TaskJudgesError::Conflict(
                "ordinal already in use for task".into(),
            ));
        }
    }

    let mut am: task_judges::ActiveModel = row.into();
    if let Some(ordinal) = body.ordinal {
        am.ordinal = Set(ordinal);
    }
    if let Some(scale) = body.rating_scale_override {
        let scaled = if scale.is_null() { None } else { Some(scale) };
        am.rating_scale_override = Set(scaled);
    }
    am.updated_at = Set(Utc::now());
    let updated = am.update(&state.db).await?;

    let judge = judges::Entity::find_by_id(updated.judge_id)
        .one(&state.db)
        .await?
        .ok_or(TaskJudgesError::NotFound)?;
    Ok(Json(build_response(updated, judge)))
}

pub async fn detach(
    _admin: AdminUser,
    State(state): State<AppState>,
    Path((project_id, task_id, judge_id)): Path<(Uuid, Uuid, Uuid)>,
) -> Result<StatusCode, TaskJudgesError> {
    load_task(&state.db, project_id, task_id).await?;

    let row = task_judges::Entity::find()
        .filter(task_judges::Column::TaskId.eq(task_id))
        .filter(task_judges::Column::JudgeId.eq(judge_id))
        .one(&state.db)
        .await?
        .ok_or(TaskJudgesError::NotFound)?;

    task_judges::Entity::delete_by_id(row.id)
        .exec(&state.db)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}
