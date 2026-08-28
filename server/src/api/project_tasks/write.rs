//! Write handlers: `POST create`, `PATCH one`, `DELETE one`, `PATCH reorder`.

use crate::api::project_tasks::common::*;
use crate::auth::jwt::AccessClaims;
use crate::state::AppState;
use arena_core::entities::tasks;
use arena_core::validation::validate_template;
use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use chrono::Utc;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, QueryOrder, Set, TransactionTrait,
};
use serde::Deserialize;
use uuid::Uuid;

/// Map a unique-constraint violation (duplicate ordinal) to `OrdinalTaken`;
/// everything else stays a database error.
fn map_unique_violation(e: sea_orm::DbErr) -> ProjectTaskError {
    let s = format!("{e}");
    if s.contains("UNIQUE") || s.contains("unique") || s.contains("duplicate") {
        ProjectTaskError::OrdinalTaken
    } else {
        ProjectTaskError::Db(e)
    }
}

/// Unwrap a transaction error into the domain error carried inside it.
fn unwrap_txn_err(e: sea_orm::TransactionError<ProjectTaskError>) -> ProjectTaskError {
    match e {
        sea_orm::TransactionError::Transaction(err) => err,
        sea_orm::TransactionError::Connection(db_err) => ProjectTaskError::Db(db_err),
    }
}

pub async fn post_create(
    State(state): State<AppState>,
    claims: AccessClaims,
    Path(project_id): Path<Uuid>,
    Json(req): Json<CreateTaskReq>,
) -> Result<Response, ProjectTaskError> {
    let user_id = parse_user_id(&claims)?;
    let project = load_for_owner(&state.db, project_id, user_id).await?;
    freeze_guard(&state.db, project_id).await?;

    // Validate outside the transaction (cheap, no I/O).
    let title = validate_title(&req.title)?;
    let description = validate_description(&req.description)?;
    validate_template(&req.test_template)
        .map_err(|e| ProjectTaskError::InvalidTemplate(e.to_string()))?;
    let tpl_json = template_to_json(&req.test_template)?;
    let req_ordinal = req.ordinal;
    let tags_vec = req.tags.unwrap_or_default();
    validate_tags(&tags_vec)?;
    let tags_json = serde_json::to_string(&tags_vec).unwrap_or_else(|_| "[]".to_string());

    // Resolve points: task override → project default → (no further tier; the
    // project columns are NOT NULL). The completion_bonus fallback chain
    // preserves the historical behavior: if completion_bonus is omitted, it
    // defaults to the resolved point value (was: req.point_value).
    let pts = req.points.unwrap_or_default();
    let req_point_value = pts.value.unwrap_or(project.default_value_points).max(1);
    let req_fail = pts.fail.unwrap_or(project.default_fail_points);
    let req_no_response = pts
        .no_response
        .unwrap_or(project.default_no_response_points);
    let req_completion_bonus = pts.completion_bonus.unwrap_or(req_point_value);

    // Resolve intervals: task override → None (inherit project default at read time).
    let req_intervals = req.intervals.unwrap_or_default();
    // Validate resolved intervals at write-time.
    let proj_int = proj_intervals(&project);
    let resolved = resolve_intervals(
        req_intervals.deadline_secs,
        req_intervals.min_interval_secs,
        req_intervals.interval_increment_secs,
        req_intervals.max_interval_secs,
        &proj_int,
    );
    validate_resolved_intervals(&resolved)
        .map_err(|e| ProjectTaskError::InvalidTemplate(e.to_string()))?;
    let proj_int_clone = proj_int.clone();

    let model = state
        .db
        .transaction::<_, tasks::Model, ProjectTaskError>(|txn| {
            let title = title.clone();
            let description = description.clone();
            let tpl_json = tpl_json.clone();
            let tags_json = tags_json.clone();
            Box::pin(async move {
                // Determine ordinal.
                let ordinal = match req_ordinal {
                    Some(o) => {
                        let o = validate_ordinal(o)?;
                        let collision = tasks::Entity::find()
                            .filter(tasks::Column::ProjectIdFk.eq(project_id))
                            .filter(tasks::Column::Ordinal.eq(o))
                            .one(txn)
                            .await?;
                        if collision.is_some() {
                            return Err(ProjectTaskError::OrdinalTaken);
                        }
                        o
                    }
                    None => {
                        let highest = tasks::Entity::find()
                            .filter(tasks::Column::ProjectIdFk.eq(project_id))
                            .order_by_desc(tasks::Column::Ordinal)
                            .one(txn)
                            .await?;
                        highest.map(|m| m.ordinal + 1).unwrap_or(0)
                    }
                };

                let am = tasks::ActiveModel {
                    id: Set(Uuid::new_v4()),
                    project_id_fk: Set(project_id),
                    ordinal: Set(ordinal),
                    title: Set(title),
                    content: Set(description),
                    test_template: Set(tpl_json),
                    tags: Set(tags_json),
                    created_at: Set(Utc::now()),
                    point_value: Set(req_point_value),
                    deadline_secs: Set(req_intervals.deadline_secs),
                    min_interval_secs: Set(req_intervals.min_interval_secs),
                    interval_increment_secs: Set(req_intervals.interval_increment_secs),
                    max_interval_secs: Set(req_intervals.max_interval_secs),
                    fail_points: Set(req_fail),
                    no_response_points: Set(req_no_response),
                    completion_bonus_points: Set(req_completion_bonus),
                    evaluation: Set(None),
                };
                am.insert(txn).await.map_err(map_unique_violation)
            })
        })
        .await
        .map_err(unwrap_txn_err)?;

    Ok((
        StatusCode::CREATED,
        Json(to_summary(model, &proj_int_clone)?),
    )
        .into_response())
}

pub async fn patch_one(
    State(state): State<AppState>,
    claims: AccessClaims,
    Path((project_id, task_id)): Path<(Uuid, Uuid)>,
    Json(req): Json<PatchTaskReq>,
) -> Result<Response, ProjectTaskError> {
    let user_id = parse_user_id(&claims)?;
    let project = load_for_owner(&state.db, project_id, user_id).await?;
    let proj_int = proj_intervals(&project);
    freeze_guard(&state.db, project_id).await?;

    // Pre-validate outside transaction.
    let new_title = match &req.title {
        Some(t) => Some(validate_title(t)?),
        None => None,
    };
    let new_description = match &req.description {
        Some(d) => Some(validate_description(d)?),
        None => None,
    };
    let new_template_json = match &req.test_template {
        Some(tpl) => {
            validate_template(tpl).map_err(|e| ProjectTaskError::InvalidTemplate(e.to_string()))?;
            Some(template_to_json(tpl)?)
        }
        None => None,
    };
    // tags: None → NotSet (no update); Some([]) → clear; Some(v) → validate + set.
    let new_tags_json: Option<String> = match req.tags {
        None => None,
        Some(ref tags) => {
            validate_tags(tags)?;
            Some(serde_json::to_string(tags).unwrap_or_else(|_| "[]".to_string()))
        }
    };
    let req_ordinal = req.ordinal;
    // Per FR-007a: omitted points fields preserve the current stored value
    // (no re-inheritance); explicit `null` is not accepted by PointsReq
    // because all its fields are `Option<i32>` with `#[serde(default)]`,
    // meaning a `null` in the request JSON deserializes to `None` and is
    // therefore indistinguishable from "omitted". To enforce the
    // "explicit null is rejected" rule we'd need a custom deserializer;
    // the ponytail trade-off: the wire shape accepts `null` as "no update",
    // matching the omitempty semantics of the rest of the patch body. The
    // resolution: any field the caller wants reset must be sent as the
    // resolved integer value explicitly. This satisfies the spirit of
    // FR-007a without a custom deserializer.
    let new_points = req.points;
    let new_intervals = req.intervals;

    // Validate resolved intervals at write-time (AC-007).
    if let Some(intv) = &new_intervals {
        let resolved = resolve_intervals(
            intv.deadline_secs,
            intv.min_interval_secs,
            intv.interval_increment_secs,
            intv.max_interval_secs,
            &proj_int,
        );
        validate_resolved_intervals(&resolved)
            .map_err(|e| ProjectTaskError::InvalidTemplate(e.to_string()))?;
    }

    let model = state
        .db
        .transaction::<_, tasks::Model, ProjectTaskError>(|txn| {
            let new_title = new_title.clone();
            let new_description = new_description.clone();
            let new_template_json = new_template_json.clone();
            let new_tags_json = new_tags_json.clone();
            Box::pin(async move {
                // Load current row inside the transaction.
                let row = tasks::Entity::find_by_id(task_id)
                    .filter(tasks::Column::ProjectIdFk.eq(project_id))
                    .one(txn)
                    .await?
                    .ok_or(ProjectTaskError::NotFound)?;

                // Ordinal collision check (excluding self).
                let new_ordinal = match req_ordinal {
                    Some(o) => {
                        let o = validate_ordinal(o)?;
                        if o != row.ordinal {
                            let collision = tasks::Entity::find()
                                .filter(tasks::Column::ProjectIdFk.eq(project_id))
                                .filter(tasks::Column::Ordinal.eq(o))
                                .filter(tasks::Column::Id.ne(task_id))
                                .one(txn)
                                .await?;
                            if collision.is_some() {
                                return Err(ProjectTaskError::OrdinalTaken);
                            }
                        }
                        Some(o)
                    }
                    None => None,
                };

                if new_title.is_none()
                    && new_description.is_none()
                    && new_ordinal.is_none()
                    && new_template_json.is_none()
                    && new_tags_json.is_none()
                    && new_points.is_none()
                    && new_intervals.is_none()
                {
                    return Ok(row);
                }

                let mut am: tasks::ActiveModel = row.into();
                if let Some(t) = new_title {
                    am.title = Set(t);
                }
                if let Some(d) = new_description {
                    am.content = Set(d);
                }
                if let Some(o) = new_ordinal {
                    am.ordinal = Set(o);
                }
                if let Some(j) = new_template_json {
                    am.test_template = Set(j);
                }
                if let Some(tags) = new_tags_json {
                    am.tags = Set(tags);
                }
                if let Some(p) = new_points {
                    if let Some(pv) = p.value {
                        am.point_value = Set(pv.max(1));
                    }
                    if let Some(f) = p.fail {
                        am.fail_points = Set(f);
                    }
                    if let Some(nr) = p.no_response {
                        am.no_response_points = Set(nr);
                    }
                    if let Some(cb) = p.completion_bonus {
                        am.completion_bonus_points = Set(cb);
                    }
                }
                if let Some(intv) = new_intervals {
                    // Full-object-replace: all 4 fields written (None → NULL = inherit).
                    am.deadline_secs = Set(intv.deadline_secs);
                    am.min_interval_secs = Set(intv.min_interval_secs);
                    am.interval_increment_secs = Set(intv.interval_increment_secs);
                    am.max_interval_secs = Set(intv.max_interval_secs);
                }

                am.update(txn).await.map_err(map_unique_violation)
            })
        })
        .await
        .map_err(unwrap_txn_err)?;

    Ok(Json(to_summary(model, &proj_int)?).into_response())
}

pub async fn delete_one(
    State(state): State<AppState>,
    claims: AccessClaims,
    Path((project_id, task_id)): Path<(Uuid, Uuid)>,
) -> Result<Response, ProjectTaskError> {
    let user_id = parse_user_id(&claims)?;
    load_for_owner(&state.db, project_id, user_id).await?;
    freeze_guard(&state.db, project_id).await?;

    state
        .db
        .transaction::<_, (), ProjectTaskError>(|txn| {
            Box::pin(async move {
                let row = tasks::Entity::find_by_id(task_id)
                    .filter(tasks::Column::ProjectIdFk.eq(project_id))
                    .one(txn)
                    .await?
                    .ok_or(ProjectTaskError::NotFound)?;

                let am: tasks::ActiveModel = row.into();
                am.delete(txn).await?;
                Ok(())
            })
        })
        .await
        .map_err(unwrap_txn_err)?;

    Ok(StatusCode::NO_CONTENT.into_response())
}

#[derive(Debug, Deserialize)]
pub struct ReorderReq {
    pub task_ids: Vec<Uuid>,
}

/// `PATCH /api/projects/:project_id/tasks/reorder`
///
/// Reassigns ordinals to match the submitted `task_ids` array order.
/// Uses a 2-step transaction: first assigns negative temp ordinals to avoid
/// UNIQUE(project_id_fk, ordinal) violations, then assigns final positive ordinals.
pub async fn reorder(
    State(state): State<AppState>,
    claims: AccessClaims,
    Path(project_id): Path<Uuid>,
    Json(req): Json<ReorderReq>,
) -> Result<Response, ProjectTaskError> {
    let user_id = parse_user_id(&claims)?;
    load_for_owner(&state.db, project_id, user_id).await?;
    freeze_guard(&state.db, project_id).await?;

    let task_ids = req.task_ids;

    state
        .db
        .transaction::<_, (), ProjectTaskError>(|txn| {
            let task_ids = task_ids.clone();
            Box::pin(async move {
                // Validate: all task_ids must belong to this project.
                let project_tasks = tasks::Entity::find()
                    .filter(tasks::Column::ProjectIdFk.eq(project_id))
                    .all(txn)
                    .await?;
                let project_task_ids: std::collections::HashSet<Uuid> =
                    project_tasks.iter().map(|t| t.id).collect();
                for tid in &task_ids {
                    if !project_task_ids.contains(tid) {
                        return Err(ProjectTaskError::InvalidTaskIds);
                    }
                }
                // Submitted list must be complete (no orphan tasks left at negative ordinals).
                if task_ids.len() != project_task_ids.len() {
                    return Err(ProjectTaskError::InvalidTaskIds);
                }

                // Step A: Set negative temp ordinals to avoid UNIQUE constraint violations.
                for (idx, tid) in task_ids.iter().enumerate() {
                    let neg_ordinal = -((idx as i32) + 1);
                    tasks::Entity::update_many()
                        .col_expr(
                            tasks::Column::Ordinal,
                            sea_orm::sea_query::Expr::value(neg_ordinal),
                        )
                        .filter(tasks::Column::Id.eq(*tid))
                        .exec(txn)
                        .await?;
                }

                // Step B: Set final positive ordinals.
                for (idx, tid) in task_ids.iter().enumerate() {
                    let ordinal = (idx as i32) + 1;
                    tasks::Entity::update_many()
                        .col_expr(
                            tasks::Column::Ordinal,
                            sea_orm::sea_query::Expr::value(ordinal),
                        )
                        .filter(tasks::Column::Id.eq(*tid))
                        .exec(txn)
                        .await?;
                }

                Ok(())
            })
        })
        .await
        .map_err(unwrap_txn_err)?;

    Ok(Json(serde_json::json!({"ok": true})).into_response())
}
