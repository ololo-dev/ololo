use super::common::*;
use crate::api::intervals::{
    ExportIntervals, IntervalsResp, resolve_intervals, validate_resolved_intervals,
};
use crate::api::settings::is_project_creation_allowed;
use crate::auth::jwt::AccessClaims;
use crate::state::AppState;
use arena_core::entities::{projects, sessions, tasks, users};
use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use chrono::Utc;
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter, Set};
use uuid::Uuid;
pub async fn post_create(
    State(state): State<AppState>,
    claims: AccessClaims,
    Json(req): Json<CreateProjectReq>,
) -> Result<Response, ProjectError> {
    let user_id = parse_user_id(&claims)?;
    let name = validate_name(&req.name)?;

    // FR-004–FR-009: gate non-admin users when allow_user_project_creation ≠ "true".
    // Admin check: DB lookup (is_admin column); no JWT claim for is_admin in this codebase.
    let user = users::Entity::find_by_id(user_id)
        .one(&state.db)
        .await?
        .ok_or(ProjectError::Forbidden)?;
    if !user.is_admin && !is_project_creation_allowed(&state.db).await? {
        return Err(ProjectError::CreationRestricted);
    }

    let description = req.description.unwrap_or_default();

    // Category validation (FR-011): permissive if key absent from app_settings.
    if let Some(ref cat) = req.category {
        let allowed = load_allowed_categories(&state.db).await?;
        validate_category(cat, &allowed)?;
    }

    // Tags validation (FR-012): max 10, each ≤50 chars, no duplicates.
    let tags_vec: Vec<String> = if let Some(ref tags) = req.tags {
        validate_tags(tags)?;
        tags.iter().map(|t| t.trim().to_string()).collect()
    } else {
        vec![]
    };
    let tags_json = serde_json::to_string(&tags_vec).unwrap_or_else(|_| "[]".to_string());

    // Cover image URL validation (FR-013).
    if let Some(ref url) = req.cover_image_url {
        validate_cover_image_url(url)?;
    }

    let now = Utc::now();
    let id = Uuid::new_v4();
    let pts = req.points.unwrap_or_default();
    let intv = req.intervals.unwrap_or_default();
    let resolved_intervals = resolve_intervals(
        intv.deadline_secs,
        intv.min_interval_secs,
        intv.interval_increment_secs,
        intv.max_interval_secs,
        &ExportIntervals {
            deadline_secs: 60,
            min_interval_secs: 5,
            interval_increment_secs: 5,
            max_interval_secs: 60,
        },
    );
    validate_resolved_intervals(&resolved_intervals).map_err(|_e| ProjectError::InvalidName)?;

    // Session duration validation: 60..=86400 inclusive, default 3600.
    let session_duration_secs = req.session_duration_secs.unwrap_or(3600);
    validate_session_duration_secs(session_duration_secs)?;
    let idle_timeout_secs = req.idle_timeout_secs.unwrap_or(300);
    validate_idle_timeout_secs(idle_timeout_secs)?;
    let memory_schema_json = match &req.memory_schema {
        Some(v) => Some(validate_memory_schema_json(v)?),
        None => None,
    };
    let am = projects::ActiveModel {
        id: Set(id),
        name: Set(name),
        slug: Set(None),
        description: Set(description),
        category: Set(req.category),
        tags: Set(tags_json),
        cover_image_url: Set(req.cover_image_url),
        owner_user_id_fk: Set(user_id),
        public: Set(if user.is_admin { req.public } else { true }),
        archived_at: Set(None),
        created_at: Set(now),
        updated_at: Set(now),
        default_value_points: Set(pts.value.unwrap_or(10)),
        default_fail_points: Set(pts.fail.unwrap_or(-5)),
        default_no_response_points: Set(pts.no_response.unwrap_or(-10)),
        default_completion_bonus_points: Set(pts.completion_bonus.unwrap_or(10)),
        default_deadline_secs: Set(resolved_intervals.deadline_secs),
        default_session_duration_secs: Set(session_duration_secs),
        idle_timeout_secs: Set(idle_timeout_secs),
        default_min_interval_secs: Set(resolved_intervals.min_interval_secs),
        default_interval_increment_secs: Set(resolved_intervals.interval_increment_secs),
        default_max_interval_secs: Set(resolved_intervals.max_interval_secs),
        memory_schema: Set(memory_schema_json),
        show_tasks: Set(true),
        parent_project_id_fk: Set(None),
        part_ordinal: Set(None),
    };
    let model = am.insert(&state.db).await?;
    Ok((StatusCode::CREATED, Json(to_summary(model, false, 0, None))).into_response())
}

/// `GET /api/projects/:id`
///
/// Unauthenticated callers may access public projects.
/// Private projects require auth (403 on mismatch — AD-7).
pub async fn patch_one(
    State(state): State<AppState>,
    claims: AccessClaims,
    Path(id): Path<Uuid>,
    Json(req): Json<PatchProjectReq>,
) -> Result<Response, ProjectError> {
    let user_id = parse_user_id(&claims)?;
    let is_admin = crate::auth::is_user_admin(&state.db, user_id).await?;
    let row = if is_admin {
        load_any(&state.db, id).await?
    } else {
        load_for_owner(&state.db, id, user_id).await?
    };

    // Archived guard: allow only explicit unarchive (archived: false).
    if row.archived_at.is_some() && req.archived != Some(false) {
        return Err(ProjectError::ProjectArchived);
    }

    let new_name = match &req.name {
        Some(n) => Some(validate_name(n)?),
        None => None,
    };

    // Validate and permission-check slug change (admin-only).
    // new_slug_change: None = no change, Some(None) = clear, Some(Some(s)) = set.
    let new_slug_change: Option<Option<String>> = match &req.slug {
        None => None,
        Some(s) if s.is_empty() => {
            // Empty string = clear slug. Admin check required.
            if !is_admin {
                return Err(ProjectError::Forbidden);
            }
            Some(None)
        }
        Some(s) => {
            let validated = validate_slug(s)?;
            if !is_admin {
                return Err(ProjectError::Forbidden);
            }
            if row.slug.as_deref() != Some(validated.as_str())
                && slug_exists(&state.db, row.owner_user_id_fk, &validated, Some(row.id)).await?
            {
                return Err(ProjectError::SlugConflict);
            }
            Some(Some(validated))
        }
    };

    if new_name.is_none()
        && req.public.is_none()
        && req.archived.is_none()
        && req.description.is_none()
        && new_slug_change.is_none()
        && req.category.is_none()
        && req.tags.is_none()
        && req.cover_image_url.is_none()
        && req.clear_cover_image.is_none()
        && req.points.is_none()
        && req.intervals.is_none()
        && req.session_duration_secs.is_none()
        && req.idle_timeout_secs.is_none()
        && req.memory_schema.is_none()
    {
        let tc = tasks::Entity::find()
            .filter(tasks::Column::ProjectIdFk.eq(row.id))
            .count(&state.db)
            .await?;
        let range = compute_points_range(&state.db, row.id).await?;
        return Ok(Json(to_summary(row, false, tc as i64, range)).into_response());
    }

    let now = Utc::now();
    let cur_deadline_secs = row.default_deadline_secs;
    let cur_min_interval_secs = row.default_min_interval_secs;
    let cur_interval_increment_secs = row.default_interval_increment_secs;
    let cur_max_interval_secs = row.default_max_interval_secs;
    let mut am: projects::ActiveModel = row.into();
    if let Some(n) = new_name {
        am.name = Set(n);
    }
    if let Some(p) = req.public {
        am.public = Set(p);
    }
    if let Some(d) = req.description {
        am.description = Set(d);
    }
    match new_slug_change {
        Some(Some(s)) => am.slug = Set(Some(s)),
        Some(None) => am.slug = Set(None),
        None => {}
    }
    match req.archived {
        Some(true) => am.archived_at = Set(Some(now)),
        Some(false) => am.archived_at = Set(None),
        None => {}
    }

    // Category validation and update (FR-011).
    if let Some(ref cat) = req.category {
        let allowed = load_allowed_categories(&state.db).await?;
        validate_category(cat, &allowed)?;
        am.category = Set(Some(cat.clone()));
    }

    // Tags validation and update (FR-012).
    if let Some(ref tags) = req.tags {
        validate_tags(tags)?;
        let trimmed: Vec<String> = tags.iter().map(|t| t.trim().to_string()).collect();
        let tags_json = serde_json::to_string(&trimmed).unwrap_or_else(|_| "[]".to_string());
        am.tags = Set(tags_json);
    }

    // Cover image update (FR-009): clear_cover_image: true takes precedence.
    if req.clear_cover_image == Some(true) {
        am.cover_image_url = Set(None);
    } else if let Some(ref url) = req.cover_image_url {
        validate_cover_image_url(url)?;
        am.cover_image_url = Set(Some(url.clone()));
    }

    // Points defaults update: only `Some` fields of the inner PointsReq.
    if let Some(pts) = req.points {
        if let Some(v) = pts.value {
            am.default_value_points = Set(v);
        }
        if let Some(f) = pts.fail {
            am.default_fail_points = Set(f);
        }
        if let Some(nr) = pts.no_response {
            am.default_no_response_points = Set(nr);
        }
        if let Some(cb) = pts.completion_bonus {
            am.default_completion_bonus_points = Set(cb);
        }
    }

    // Intervals defaults update: only Some fields of the inner IntervalsReq.
    if let Some(intv) = req.intervals {
        let cur_deadline = intv.deadline_secs.unwrap_or(cur_deadline_secs);
        let cur_min = intv.min_interval_secs.unwrap_or(cur_min_interval_secs);
        let cur_inc = intv
            .interval_increment_secs
            .unwrap_or(cur_interval_increment_secs);
        let cur_max = intv.max_interval_secs.unwrap_or(cur_max_interval_secs);
        let resolved = IntervalsResp {
            deadline_secs: cur_deadline,
            min_interval_secs: cur_min,
            interval_increment_secs: cur_inc,
            max_interval_secs: cur_max,
        };
        validate_resolved_intervals(&resolved).map_err(|_e| ProjectError::InvalidName)?;
        if let Some(d) = intv.deadline_secs {
            am.default_deadline_secs = Set(d);
        }
        if let Some(mn) = intv.min_interval_secs {
            am.default_min_interval_secs = Set(mn);
        }
        if let Some(inc) = intv.interval_increment_secs {
            am.default_interval_increment_secs = Set(inc);
        }
        if let Some(mx) = intv.max_interval_secs {
            am.default_max_interval_secs = Set(mx);
        }
    }

    // Session duration update: validated against arena-core bounds.
    if let Some(it) = req.idle_timeout_secs {
        validate_idle_timeout_secs(it)?;
        am.idle_timeout_secs = Set(it);
    }
    if let Some(sd) = req.session_duration_secs {
        validate_session_duration_secs(sd)?;
        am.default_session_duration_secs = Set(sd);
    }

    // Memory schema: absent = unchanged, explicit null = clear, object = set.
    match req.memory_schema {
        None => {}
        Some(None) => am.memory_schema = Set(None),
        Some(Some(ref v)) => am.memory_schema = Set(Some(validate_memory_schema_json(v)?)),
    }

    am.updated_at = Set(now);
    let updated = am.update(&state.db).await?;
    let tc = tasks::Entity::find()
        .filter(tasks::Column::ProjectIdFk.eq(updated.id))
        .count(&state.db)
        .await?;
    let range = compute_points_range(&state.db, updated.id).await?;
    Ok(Json(to_summary(updated, false, tc as i64, range)).into_response())
}

/// `DELETE /api/projects/:id`
///
/// Owner-only. RESTRICT: if any session (any status) references this project
/// the delete is rejected with 409 `project_in_use` + session_count (R-5).
pub async fn delete_one(
    State(state): State<AppState>,
    claims: AccessClaims,
    Path(id): Path<Uuid>,
) -> Result<Response, ProjectError> {
    let user_id = parse_user_id(&claims)?;
    let is_admin = crate::auth::is_user_admin(&state.db, user_id).await?;
    let row = if is_admin {
        load_any(&state.db, id).await?
    } else {
        load_for_owner(&state.db, id, user_id).await?
    };

    let blocking = sessions::Entity::find()
        .filter(sessions::Column::ProjectIdFk.eq(row.id))
        .all(&state.db)
        .await?;
    if !blocking.is_empty() {
        return Err(ProjectError::ProjectInUse {
            session_count: blocking.len() as i64,
        });
    }

    // Detach any campaign parts first: `parent_project_id_fk` has no DB-level
    // FK (SQLite cannot add one via ALTER TABLE), so nothing else would clear
    // the pointer and the parts would keep claiming a campaign that is gone.
    crate::campaign_link::unlink_children(&state.db, row.id).await?;

    // No sessions reference this project — safe to delete.
    // Tasks cascade via ON DELETE CASCADE (m20260504_000002).
    projects::Entity::delete_by_id(row.id)
        .exec(&state.db)
        .await?;

    Ok(StatusCode::NO_CONTENT.into_response())
}
