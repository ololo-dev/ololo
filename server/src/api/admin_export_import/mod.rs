//! Admin-only project export/import as a single JSON file.

use crate::api::intervals::{ExportIntervals, ExportTaskIntervals};
use crate::api::settings::AdminUser;
use crate::state::AppState;
use arena_core::entities::{projects, tasks};
use arena_core::task_template::TestTemplate;
use axum::Json;
use axum::extract::{Path, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use chrono::Utc;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, QueryOrder, Set, TransactionTrait,
};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use uuid::Uuid;

const SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExportEnvelope {
    pub schema_version: u32,
    pub project: ExportProject,
    pub tasks: Vec<ExportTask>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExportProject {
    pub name: String,
    #[serde(default)]
    pub slug: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub category: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub cover_image_url: Option<String>,
    #[serde(default)]
    pub public: bool,
    #[serde(default)]
    pub archived_at: Option<String>,
    pub points: ExportPoints,
    pub intervals: ExportIntervals,
    /// Absent in export files written before the field existed → default 3600.
    #[serde(default = "default_export_session_duration_secs")]
    pub session_duration_secs: i64,
    /// Session-memory schema: JSON object of `key -> scalar default`.
    /// Absent in files written before the field existed → none.
    #[serde(default)]
    pub memory_schema: Option<serde_json::Value>,
    /// Whether the public project page lists the task arc up front.
    /// Quiz-shaped projects (Extreme Startup family) keep their ladder a
    /// surprise. Absent in files written before the field existed → shown.
    #[serde(default = "default_show_tasks")]
    pub show_tasks: bool,
    /// Campaign parts, in play order, by project slug. Non-empty makes this
    /// project a campaign parent: it carries no tasks and hosts no sessions,
    /// and each listed project becomes a part gated on the previous one.
    /// Absent/empty for every ordinary project, so old exports round-trip
    /// byte-identically.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parts: Vec<String>,
}

fn default_show_tasks() -> bool {
    true
}

fn default_export_session_duration_secs() -> i64 {
    3600
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExportPoints {
    pub value: i32,
    pub fail: i32,
    pub no_response: i32,
    pub completion_bonus: i32,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExportTask {
    pub ordinal: i32,
    pub title: String,
    pub content: String,
    pub test_template: TestTemplate,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub points: Option<ExportTaskPoints>,
    #[serde(default)]
    pub intervals: Option<ExportTaskIntervals>,
    /// Judges to attach to the task, in order: a bare slug, or
    /// `{slug, weight}` for open-ended panel weighting.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub judges: Vec<JudgeRef>,
    /// Open-ended evaluation contract
    /// (`arena_core::evaluation::EvaluationContract`). Absent = classic task.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evaluation: Option<serde_json::Value>,
}

/// One judge attachment in a task definition. The bare-string form is the
/// long-standing shape; the map form adds a panel weight. Serializes back to
/// the bare string when no weight is set, so pre-existing exports round-trip
/// byte-identically.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub enum JudgeRef {
    Slug(String),
    Weighted {
        slug: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        weight: Option<f64>,
    },
}

impl JudgeRef {
    pub fn slug(&self) -> &str {
        match self {
            JudgeRef::Slug(s) => s,
            JudgeRef::Weighted { slug, .. } => slug,
        }
    }

    pub fn weight(&self) -> Option<f64> {
        match self {
            JudgeRef::Slug(_) => None,
            JudgeRef::Weighted { weight, .. } => *weight,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[serde(deny_unknown_fields)]
pub struct ExportTaskPoints {
    #[serde(default)]
    pub value: Option<i32>,
    #[serde(default)]
    pub fail: Option<i32>,
    #[serde(default)]
    pub no_response: Option<i32>,
    #[serde(default)]
    pub completion_bonus: Option<i32>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ImportResponse {
    pub project_id: Uuid,
    pub name: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ReseedResponse {
    pub project_id: Uuid,
    pub name: String,
    pub tasks_updated: usize,
    pub tasks_inserted: usize,
    pub tasks_deleted: usize,
}

#[derive(Debug, thiserror::Error)]
pub enum ExportImportError {
    #[error("not_found")]
    NotFound,
    #[error("bad_request: {0}")]
    BadRequest(String),
    #[error("unsupported_schema")]
    UnsupportedSchema,
    #[error("bad_export_template: {0}")]
    BadExportTemplate(String),
    #[error("database error: {0}")]
    Db(#[from] sea_orm::DbErr),
}

crate::api::error::impl_api_error!(ExportImportError {
    Self::NotFound => (NOT_FOUND, "not_found"),
    Self::BadRequest(msg) => (BAD_REQUEST, "bad_request", "detail": msg),
    Self::UnsupportedSchema => (UNPROCESSABLE_ENTITY, "unsupported_schema"),
    Self::BadExportTemplate(msg) => (INTERNAL_SERVER_ERROR, "bad_export_template", "detail": msg),
    Self::Db(_) => (INTERNAL_SERVER_ERROR, "database_error"),
});

fn parse_tags(raw: &str, ctx: &str) -> Result<Vec<String>, ExportImportError> {
    serde_json::from_str::<Vec<String>>(raw)
        .map_err(|e| ExportImportError::BadExportTemplate(format!("tags parse ({ctx}): {e}")))
}

/// Validate the open-ended extras of a task definition: every `yaml probe`
/// fence (malformed fences are definition errors wherever they appear), the
/// evaluation contract against the DSL's section titles, and judge panel
/// weights. Classic tasks pass through with only the fence check.
pub(crate) fn validate_task_extras(task: &ExportTask) -> Result<(), String> {
    let sections = arena_core::task_template::parse_structured_markdown_tests(
        &task.test_template.command_template,
    );
    let mut titles = Vec::with_capacity(sections.len());
    for section in &sections {
        section
            .parsed_probe_config()
            .map_err(|e| format!("section {:?}: {e}", section.title))?;
        titles.push(section.title.clone());
    }
    if let Some(value) = &task.evaluation {
        let contract = arena_core::evaluation::EvaluationContract::from_json(value)?;
        arena_core::evaluation::validate_evaluation_contract(&contract, &titles)?;
    }
    for jref in &task.judges {
        if let Some(w) = jref.weight()
            && (!w.is_finite() || w <= 0.0)
        {
            return Err(format!(
                "judge {:?} weight must be a positive number",
                jref.slug()
            ));
        }
    }
    Ok(())
}

pub async fn export_project(
    _admin: AdminUser,
    State(state): State<AppState>,
    Path(project_id): Path<Uuid>,
) -> Result<Response, ExportImportError> {
    let project = projects::Entity::find_by_id(project_id)
        .one(&state.db)
        .await?
        .ok_or(ExportImportError::NotFound)?;

    let task_rows = tasks::Entity::find()
        .filter(tasks::Column::ProjectIdFk.eq(project_id))
        .order_by_asc(tasks::Column::Ordinal)
        .all(&state.db)
        .await?;

    let project_tags = parse_tags(&project.tags, "project")?;

    // Campaign parts, in play order. A slug-less child cannot be referenced
    // by an envelope, so it is dropped rather than exported unaddressably.
    let parts: Vec<String> = projects::Entity::find()
        .filter(projects::Column::ParentProjectIdFk.eq(project_id))
        .order_by_asc(projects::Column::PartOrdinal)
        .all(&state.db)
        .await?
        .into_iter()
        .filter_map(|p| p.slug)
        .collect();

    // Judge attachments per task, exported as slugs ordered by ordinal.
    let judge_slug_by_id: std::collections::HashMap<Uuid, String> =
        arena_core::entities::judges::Entity::find()
            .all(&state.db)
            .await?
            .into_iter()
            .map(|j| (j.id, j.slug))
            .collect();
    let mut judge_slugs_by_task: std::collections::HashMap<Uuid, Vec<(i32, String, Option<f64>)>> =
        std::collections::HashMap::new();
    for tj in arena_core::entities::task_judges::Entity::find()
        .all(&state.db)
        .await?
    {
        if let Some(slug) = judge_slug_by_id.get(&tj.judge_id) {
            judge_slugs_by_task.entry(tj.task_id).or_default().push((
                tj.ordinal,
                slug.clone(),
                tj.weight,
            ));
        }
    }

    let mut export_tasks = Vec::with_capacity(task_rows.len());
    for t in task_rows {
        let tpl: TestTemplate = serde_json::from_value(t.test_template.clone()).map_err(|e| {
            ExportImportError::BadExportTemplate(format!("task {} template: {e}", t.id))
        })?;
        let tags = parse_tags(&t.tags, &format!("task {}", t.id))?;
        let judges = {
            let mut list = judge_slugs_by_task.remove(&t.id).unwrap_or_default();
            list.sort_by_key(|(ord, _, _)| *ord);
            list.into_iter()
                .map(|(_, slug, weight)| match weight {
                    None => JudgeRef::Slug(slug),
                    Some(w) => JudgeRef::Weighted {
                        slug,
                        weight: Some(w),
                    },
                })
                .collect::<Vec<_>>()
        };
        export_tasks.push(ExportTask {
            ordinal: t.ordinal,
            title: t.title,
            content: t.content,
            test_template: tpl,
            tags,
            points: Some(ExportTaskPoints {
                value: Some(t.point_value),
                fail: Some(t.fail_points),
                no_response: Some(t.no_response_points),
                completion_bonus: Some(t.completion_bonus_points),
            }),
            intervals: Some(ExportTaskIntervals {
                deadline_secs: t.deadline_secs,
                min_interval_secs: t.min_interval_secs,
                interval_increment_secs: t.interval_increment_secs,
                max_interval_secs: t.max_interval_secs,
            }),
            judges,
            evaluation: t.evaluation.clone(),
        });
    }

    let envelope = ExportEnvelope {
        schema_version: SCHEMA_VERSION,
        project: ExportProject {
            name: project.name,
            slug: project.slug,
            description: Some(project.description),
            category: project.category,
            tags: project_tags,
            cover_image_url: project.cover_image_url,
            public: project.public,
            archived_at: project.archived_at.map(|dt| dt.to_rfc3339()),
            points: ExportPoints {
                value: project.default_value_points,
                fail: project.default_fail_points,
                no_response: project.default_no_response_points,
                completion_bonus: project.default_completion_bonus_points,
            },
            intervals: ExportIntervals {
                deadline_secs: project.default_deadline_secs,
                min_interval_secs: project.default_min_interval_secs,
                interval_increment_secs: project.default_interval_increment_secs,
                max_interval_secs: project.default_max_interval_secs,
            },
            session_duration_secs: project.default_session_duration_secs,
            memory_schema: project
                .memory_schema
                .as_deref()
                .and_then(|s| serde_json::from_str(s).ok()),
            show_tasks: project.show_tasks,
            parts,
        },
        tasks: export_tasks,
    };

    let filename = match &envelope.project.slug {
        Some(s) if !s.is_empty() => s.clone(),
        _ => envelope.project.name.replace(' ', "_"),
    };
    let disposition = format!("attachment; filename=\"{}.json\"", filename);

    let mut resp = Json(&envelope).into_response();
    resp.headers_mut()
        .insert(header::CONTENT_DISPOSITION, disposition.parse().unwrap());
    Ok(resp)
}

// ponytail: in-memory parse of the full import body (≤10MB via DefaultBodyLimit).
// Suitable for projects with up to ~10k tasks. Switch to a streaming parser
// (serde_json::from_reader over an async Read) if task count exceeds that ceiling.
pub async fn import_project(
    _admin: AdminUser,
    State(state): State<AppState>,
    Json(envelope): Json<ExportEnvelope>,
) -> Result<Response, ExportImportError> {
    if envelope.schema_version != SCHEMA_VERSION {
        return Err(ExportImportError::UnsupportedSchema);
    }

    let allowed = crate::api::projects::load_allowed_categories(&state.db).await?;

    crate::validation::tags::validate_tags(&envelope.project.tags)
        .map_err(|e| ExportImportError::BadRequest(e.to_string()))?;

    // Resolve slug: preserve only if valid + globally unique.
    let resolved_slug = match envelope.project.slug.as_deref() {
        Some(s) if !s.is_empty() => match crate::api::projects::validate_slug(s) {
            Ok(validated) => {
                let collision = projects::Entity::find()
                    .filter(projects::Column::Slug.eq(&validated))
                    .one(&state.db)
                    .await?;
                if collision.is_none() {
                    Some(validated)
                } else {
                    None
                }
            }
            Err(_) => None,
        },
        _ => None,
    };

    // Resolve category: keep only if in allowed list (permissive when None).
    let resolved_category = match envelope.project.category.as_deref() {
        Some(name) => match &allowed {
            Some(list) if list.iter().any(|a| a == name) => Some(name.to_string()),
            _ => None,
        },
        None => None,
    };

    // Per-task validation + duplicate-ordinal pre-check.
    let mut seen_ordinals: HashSet<i32> = HashSet::new();
    for task in &envelope.tasks {
        crate::api::project_tasks::validate_ordinal(task.ordinal)
            .map_err(|e| ExportImportError::BadRequest(e.to_string()))?;
        arena_core::validation::validate_template(&task.test_template)
            .map_err(|e| ExportImportError::BadRequest(e.to_string()))?;
        crate::validation::tags::validate_tags(&task.tags)
            .map_err(|e| ExportImportError::BadRequest(e.to_string()))?;
        validate_task_extras(task)
            .map_err(|e| ExportImportError::BadRequest(format!("task {}: {e}", task.ordinal)))?;
        if !seen_ordinals.insert(task.ordinal) {
            return Err(ExportImportError::BadRequest(format!(
                "duplicate ordinal: {}",
                task.ordinal
            )));
        }
    }

    // Resolve judge slugs to ids; unknown slugs are a client error.
    let judge_id_by_slug: std::collections::HashMap<String, Uuid> =
        arena_core::entities::judges::Entity::find()
            .all(&state.db)
            .await?
            .into_iter()
            .map(|j| (j.slug, j.id))
            .collect();
    for task in &envelope.tasks {
        for jref in &task.judges {
            let slug = jref.slug();
            if !judge_id_by_slug.contains_key(slug) {
                return Err(ExportImportError::BadRequest(format!(
                    "task {} references unknown judge slug '{slug}'",
                    task.ordinal
                )));
            }
        }
    }

    let project_id = insert_project_from_envelope(
        &state,
        &envelope,
        &judge_id_by_slug,
        _admin.id,
        resolved_slug,
        resolved_category,
        // Import intentionally lands the copy private and coverless; the
        // admin publishes it after review.
        false,
        None,
    )
    .await?;

    let resp = ImportResponse {
        project_id,
        name: envelope.project.name,
    };
    Ok((StatusCode::CREATED, Json(resp)).into_response())
}

/// Insert one task row from an envelope definition and attach its judges —
/// shared by project import (below) and boot seeding (`crate::seed`), the
/// two paths that materialize `ExportEnvelope` tasks into a fresh project.
/// Judge slugs must be validated beforehand; unknown ones are skipped.
/// Template serialization cannot fail here in practice: both callers
/// validate templates before opening the transaction.
pub(crate) async fn insert_task_with_judges(
    txn: &sea_orm::DatabaseTransaction,
    project_id: Uuid,
    task: &ExportTask,
    proj_pts: &ExportPoints,
    judge_id_by_slug: &std::collections::HashMap<String, Uuid>,
    now: chrono::DateTime<Utc>,
) -> Result<(), sea_orm::DbErr> {
    let task_id = Uuid::new_v4();
    let tpl_json = serde_json::to_value(&task.test_template)
        .map_err(|e| sea_orm::DbErr::Custom(format!("template serialize: {e}")))?;
    let task_tags_json = serde_json::to_string(&task.tags).unwrap_or_else(|_| "[]".to_string());
    let pts = task.points.clone().unwrap_or_default();
    let resolved_value = pts.value.unwrap_or(proj_pts.value).max(1);
    let resolved_fail = pts.fail.unwrap_or(proj_pts.fail);
    let resolved_no_response = pts.no_response.unwrap_or(proj_pts.no_response);
    let resolved_completion_bonus = pts.completion_bonus.unwrap_or(proj_pts.completion_bonus);
    let task_intervals = task.intervals.clone().unwrap_or_default();
    let task_am = tasks::ActiveModel {
        id: Set(task_id),
        project_id_fk: Set(project_id),
        ordinal: Set(task.ordinal),
        title: Set(task.title.clone()),
        content: Set(task.content.clone()),
        test_template: Set(tpl_json),
        tags: Set(task_tags_json),
        created_at: Set(now),
        point_value: Set(resolved_value),
        deadline_secs: Set(task_intervals.deadline_secs),
        min_interval_secs: Set(task_intervals.min_interval_secs),
        interval_increment_secs: Set(task_intervals.interval_increment_secs),
        max_interval_secs: Set(task_intervals.max_interval_secs),
        fail_points: Set(resolved_fail),
        no_response_points: Set(resolved_no_response),
        completion_bonus_points: Set(resolved_completion_bonus),
        evaluation: Set(task.evaluation.clone()),
    };
    tasks::Entity::insert(task_am).exec(txn).await?;

    for (idx, jref) in task.judges.iter().enumerate() {
        let Some(judge_id) = judge_id_by_slug.get(jref.slug()).copied() else {
            continue;
        };
        let tj_am = arena_core::entities::task_judges::ActiveModel {
            id: Set(Uuid::new_v4()),
            task_id: Set(task_id),
            judge_id: Set(judge_id),
            ordinal: Set(idx as i32),
            rating_scale_override: Set(None),
            weight: Set(jref.weight()),
            created_at: Set(now),
            updated_at: Set(now),
        };
        arena_core::entities::task_judges::Entity::insert(tj_am)
            .exec(txn)
            .await?;
    }
    Ok(())
}

/// Insert a brand-new project (and its tasks + judge attachments) from an
/// envelope, in one transaction. The caller decides slug/category/visibility
/// policy: import resolves them permissively, apply-seed preserves the
/// envelope's values. Judge slugs must already be validated against
/// `judge_id_by_slug`.
#[allow(clippy::too_many_arguments)]
async fn insert_project_from_envelope(
    state: &AppState,
    envelope: &ExportEnvelope,
    judge_id_by_slug: &std::collections::HashMap<String, Uuid>,
    owner_id: Uuid,
    slug: Option<String>,
    category: Option<String>,
    public: bool,
    cover_image_url: Option<String>,
) -> Result<Uuid, ExportImportError> {
    state
        .db
        .transaction::<_, Uuid, ExportImportError>(|txn| {
            let envelope = envelope.clone();
            let slug = slug.clone();
            let category = category.clone();
            let cover_image_url = cover_image_url.clone();
            let judge_id_by_slug = judge_id_by_slug.clone();
            Box::pin(async move {
                let project_id = Uuid::new_v4();
                let now = Utc::now();
                let proj_pts = envelope.project.points.clone();
                let proj_intervals = envelope.project.intervals.clone();
                let tags_json = serde_json::to_string(&envelope.project.tags)
                    .unwrap_or_else(|_| "[]".to_string());

                let project_am = projects::ActiveModel {
                    id: Set(project_id),
                    name: Set(envelope.project.name.clone()),
                    slug: Set(slug),
                    description: Set(envelope.project.description.clone().unwrap_or_default()),
                    category: Set(category),
                    tags: Set(tags_json),
                    cover_image_url: Set(cover_image_url),
                    owner_user_id_fk: Set(owner_id),
                    public: Set(public),
                    archived_at: Set(None),
                    created_at: Set(now),
                    updated_at: Set(now),
                    default_value_points: Set(proj_pts.value),
                    default_fail_points: Set(proj_pts.fail),
                    default_no_response_points: Set(proj_pts.no_response),
                    default_completion_bonus_points: Set(proj_pts.completion_bonus),
                    default_deadline_secs: Set(proj_intervals.deadline_secs),
                    default_session_duration_secs: Set(envelope.project.session_duration_secs),
                    idle_timeout_secs: Set(300),
                    default_min_interval_secs: Set(proj_intervals.min_interval_secs),
                    default_interval_increment_secs: Set(proj_intervals.interval_increment_secs),
                    default_max_interval_secs: Set(proj_intervals.max_interval_secs),
                    memory_schema: Set(envelope
                        .project
                        .memory_schema
                        .as_ref()
                        .map(|v| v.to_string())),
                    show_tasks: Set(envelope.project.show_tasks),
                    parent_project_id_fk: Set(None),
                    part_ordinal: Set(None),
                };
                projects::Entity::insert(project_am).exec(txn).await?;

                for task in &envelope.tasks {
                    insert_task_with_judges(
                        txn,
                        project_id,
                        task,
                        &proj_pts,
                        &judge_id_by_slug,
                        now,
                    )
                    .await?;
                }

                Ok(project_id)
            })
        })
        .await
        .map_err(|e| match e {
            sea_orm::TransactionError::Transaction(err) => err,
            sea_orm::TransactionError::Connection(db_err) => ExportImportError::Db(db_err),
        })
}

/// `POST /api/admin/projects/:id/reseed` — re-read the project's on-disk seed
/// definition (matched by slug in `ARENA_PROJECTS_DIR`) and update the project
/// in place: project fields are overwritten from the definition, tasks are
/// upserted by ordinal (update existing, insert new, delete removed — task
/// results survive via `SET NULL`), and judge attachments are replaced to
/// match. A category missing from the `categories` table is created.
pub async fn reseed_project(
    _admin: AdminUser,
    State(state): State<AppState>,
    Path(project_id): Path<Uuid>,
) -> Result<Response, ExportImportError> {
    let project = projects::Entity::find_by_id(project_id)
        .one(&state.db)
        .await?
        .ok_or(ExportImportError::NotFound)?;
    let slug = project
        .slug
        .clone()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            ExportImportError::BadRequest(
                "project has no slug; only seeded projects can be re-read".into(),
            )
        })?;

    let (source_path, envelope) = crate::seed::find_source_by_slug(&slug).ok_or_else(|| {
        ExportImportError::BadRequest(format!("no seed source with slug '{slug}' found on disk"))
    })?;
    if envelope.schema_version != SCHEMA_VERSION {
        return Err(ExportImportError::UnsupportedSchema);
    }

    let counts = apply_envelope_to_project(&state, project, &envelope).await?;

    tracing::info!(
        project_id = %project_id,
        source = %source_path.display(),
        updated = counts.0,
        inserted = counts.1,
        deleted = counts.2,
        "reseed: project updated from disk"
    );

    Ok(Json(ReseedResponse {
        project_id,
        name: envelope.project.name,
        tasks_updated: counts.0,
        tasks_inserted: counts.1,
        tasks_deleted: counts.2,
    })
    .into_response())
}

/// Update an existing project in place from a seed envelope: project fields
/// overwritten, tasks upserted by ordinal (update existing, insert new, delete
/// removed — task results survive via `SET NULL`), and judge attachments
/// replaced to match. A category missing from the `categories` table is
/// created. Returns `(updated, inserted, deleted)` task counts. Shared by
/// `/reseed` (envelope read from the server's own disk) and `/apply-seed`
/// (envelope pushed by the client).
async fn apply_envelope_to_project(
    state: &AppState,
    project: projects::Model,
    envelope: &ExportEnvelope,
) -> Result<(usize, usize, usize), ExportImportError> {
    if envelope.schema_version != SCHEMA_VERSION {
        return Err(ExportImportError::UnsupportedSchema);
    }
    let project_id = project.id;

    crate::validation::tags::validate_tags(&envelope.project.tags)
        .map_err(|e| ExportImportError::BadRequest(e.to_string()))?;
    validate_campaign_shape(envelope)?;

    // Per-task validation + duplicate-ordinal pre-check.
    let mut seen_ordinals: HashSet<i32> = HashSet::new();
    for task in &envelope.tasks {
        crate::api::project_tasks::validate_ordinal(task.ordinal)
            .map_err(|e| ExportImportError::BadRequest(e.to_string()))?;
        arena_core::validation::validate_template(&task.test_template)
            .map_err(|e| ExportImportError::BadRequest(e.to_string()))?;
        crate::validation::tags::validate_tags(&task.tags)
            .map_err(|e| ExportImportError::BadRequest(e.to_string()))?;
        validate_task_extras(task)
            .map_err(|e| ExportImportError::BadRequest(format!("task {}: {e}", task.ordinal)))?;
        if !seen_ordinals.insert(task.ordinal) {
            return Err(ExportImportError::BadRequest(format!(
                "duplicate ordinal: {}",
                task.ordinal
            )));
        }
    }

    // Resolve judge slugs to ids; unknown slugs are a definition error.
    let judge_id_by_slug: std::collections::HashMap<String, Uuid> =
        arena_core::entities::judges::Entity::find()
            .all(&state.db)
            .await?
            .into_iter()
            .map(|j| (j.slug, j.id))
            .collect();
    for task in &envelope.tasks {
        for jref in &task.judges {
            let slug = jref.slug();
            if !judge_id_by_slug.contains_key(slug) {
                return Err(ExportImportError::BadRequest(format!(
                    "task {} references unknown judge slug '{slug}'",
                    task.ordinal
                )));
            }
        }
    }

    // Category: find-or-create so a definition can introduce a new one.
    let resolved_category = match envelope.project.category.as_deref().map(str::trim) {
        Some(name) if !name.is_empty() && name.chars().count() <= 100 => {
            crate::api::categories::ensure_category(&state.db, name).await?;
            Some(name.to_string())
        }
        _ => None,
    };

    let counts =
        state
            .db
            .transaction::<_, (usize, usize, usize), ExportImportError>(|txn| {
                let envelope = envelope.clone();
                let project = project.clone();
                let resolved_category = resolved_category.clone();
                let judge_id_by_slug = judge_id_by_slug.clone();
                Box::pin(async move {
                    let now = Utc::now();
                    let proj_pts = envelope.project.points.clone();

                    let mut project_am: projects::ActiveModel = project.into();
                    project_am.name = Set(envelope.project.name.clone());
                    project_am.description =
                        Set(envelope.project.description.clone().unwrap_or_default());
                    project_am.category = Set(resolved_category);
                    project_am.tags = Set(serde_json::to_string(&envelope.project.tags)
                        .unwrap_or_else(|_| "[]".to_string()));
                    // Preserve an operator-set cover image across re-reads: seed
                    // definitions never carry one (it is managed in the UI), so only
                    // overwrite when the incoming definition explicitly provides it.
                    if let Some(url) = envelope.project.cover_image_url.clone() {
                        project_am.cover_image_url = Set(Some(url));
                    }
                    project_am.public = Set(envelope.project.public);
                    project_am.updated_at = Set(now);
                    project_am.default_value_points = Set(proj_pts.value);
                    project_am.default_fail_points = Set(proj_pts.fail);
                    project_am.default_no_response_points = Set(proj_pts.no_response);
                    project_am.default_completion_bonus_points = Set(proj_pts.completion_bonus);
                    project_am.default_deadline_secs =
                        Set(envelope.project.intervals.deadline_secs);
                    project_am.default_session_duration_secs =
                        Set(envelope.project.session_duration_secs);
                    project_am.default_min_interval_secs =
                        Set(envelope.project.intervals.min_interval_secs);
                    project_am.default_interval_increment_secs =
                        Set(envelope.project.intervals.interval_increment_secs);
                    project_am.default_max_interval_secs =
                        Set(envelope.project.intervals.max_interval_secs);
                    project_am.memory_schema = Set(envelope
                        .project
                        .memory_schema
                        .as_ref()
                        .map(|v| v.to_string()));
                    project_am.show_tasks = Set(envelope.project.show_tasks);
                    projects::Entity::update(project_am).exec(txn).await?;

                    let existing_tasks = tasks::Entity::find()
                        .filter(tasks::Column::ProjectIdFk.eq(project_id))
                        .all(txn)
                        .await?;
                    let existing_by_ordinal: std::collections::HashMap<i32, tasks::Model> =
                        existing_tasks.into_iter().map(|t| (t.ordinal, t)).collect();
                    let envelope_ordinals: HashSet<i32> =
                        envelope.tasks.iter().map(|t| t.ordinal).collect();

                    let (mut updated, mut inserted, mut deleted) = (0usize, 0usize, 0usize);

                    // Delete tasks that vanished from the definition. task_judges
                    // and tests/probes cascade; task_results/scheduler state go
                    // SET NULL, so history survives.
                    for (ordinal, task) in &existing_by_ordinal {
                        if !envelope_ordinals.contains(ordinal) {
                            tasks::Entity::delete_by_id(task.id).exec(txn).await?;
                            deleted += 1;
                        }
                    }

                    for task in &envelope.tasks {
                        let tpl_json = serde_json::to_value(&task.test_template)
                            .map_err(|e| ExportImportError::BadExportTemplate(e.to_string()))?;
                        let task_tags_json =
                            serde_json::to_string(&task.tags).unwrap_or_else(|_| "[]".to_string());
                        let pts = task.points.clone().unwrap_or_default();
                        let resolved_value = pts.value.unwrap_or(proj_pts.value).max(1);
                        let resolved_fail = pts.fail.unwrap_or(proj_pts.fail);
                        let resolved_no_response = pts.no_response.unwrap_or(proj_pts.no_response);
                        let resolved_completion_bonus =
                            pts.completion_bonus.unwrap_or(proj_pts.completion_bonus);
                        let task_intervals = task.intervals.clone().unwrap_or_default();

                        let task_id = match existing_by_ordinal.get(&task.ordinal) {
                            Some(existing) => {
                                let mut am: tasks::ActiveModel = existing.clone().into();
                                am.title = Set(task.title.clone());
                                am.content = Set(task.content.clone());
                                am.test_template = Set(tpl_json);
                                am.tags = Set(task_tags_json);
                                am.point_value = Set(resolved_value);
                                am.deadline_secs = Set(task_intervals.deadline_secs);
                                am.min_interval_secs = Set(task_intervals.min_interval_secs);
                                am.interval_increment_secs =
                                    Set(task_intervals.interval_increment_secs);
                                am.max_interval_secs = Set(task_intervals.max_interval_secs);
                                am.fail_points = Set(resolved_fail);
                                am.no_response_points = Set(resolved_no_response);
                                am.completion_bonus_points = Set(resolved_completion_bonus);
                                am.evaluation = Set(task.evaluation.clone());
                                tasks::Entity::update(am).exec(txn).await?;
                                updated += 1;
                                existing.id
                            }
                            None => {
                                let task_id = Uuid::new_v4();
                                let am = tasks::ActiveModel {
                                    id: Set(task_id),
                                    project_id_fk: Set(project_id),
                                    ordinal: Set(task.ordinal),
                                    title: Set(task.title.clone()),
                                    content: Set(task.content.clone()),
                                    test_template: Set(tpl_json),
                                    tags: Set(task_tags_json),
                                    created_at: Set(now),
                                    point_value: Set(resolved_value),
                                    deadline_secs: Set(task_intervals.deadline_secs),
                                    min_interval_secs: Set(task_intervals.min_interval_secs),
                                    interval_increment_secs: Set(
                                        task_intervals.interval_increment_secs
                                    ),
                                    max_interval_secs: Set(task_intervals.max_interval_secs),
                                    fail_points: Set(resolved_fail),
                                    no_response_points: Set(resolved_no_response),
                                    completion_bonus_points: Set(resolved_completion_bonus),
                                    evaluation: Set(task.evaluation.clone()),
                                };
                                tasks::Entity::insert(am).exec(txn).await?;
                                inserted += 1;
                                task_id
                            }
                        };

                        // Reconcile judge attachments with the definition,
                        // PRESERVING existing row ids: judge_results reference
                        // task_judge_id, so a recreated id orphans every
                        // verdict the task ever received — statuses regress to
                        // pending and the recovery sweep re-runs (and re-pays
                        // for) the whole panel after every reseed.
                        //
                        // (task_id, ordinal) is unique, so this runs in phases:
                        // detach first, then park every surviving row on a
                        // negative ordinal, then write the final order. A
                        // one-pass write inserts a new judge onto an ordinal a
                        // surviving row still holds and dies on the unique
                        // index (found the hard way: swapping a panel's judges
                        // 500'd every apply-seed).
                        let existing_tjs = arena_core::entities::task_judges::Entity::find()
                            .filter(arena_core::entities::task_judges::Column::TaskId.eq(task_id))
                            .all(txn)
                            .await?;
                        let named: std::collections::HashSet<Uuid> = task
                            .judges
                            .iter()
                            .filter_map(|jref| judge_id_by_slug.get(jref.slug()).copied())
                            .collect();
                        // Detach what the definition no longer names, freeing
                        // their ordinals.
                        for row in existing_tjs.iter().filter(|r| !named.contains(&r.judge_id)) {
                            arena_core::entities::task_judges::Entity::delete_by_id(row.id)
                                .exec(txn)
                                .await?;
                        }
                        // Park survivors out of the target range (final
                        // ordinals are >= 0, parking is negative and unique).
                        for (i, row) in existing_tjs
                            .iter()
                            .filter(|r| named.contains(&r.judge_id))
                            .enumerate()
                        {
                            let mut am: arena_core::entities::task_judges::ActiveModel =
                                row.clone().into();
                            am.ordinal = Set(-(i as i32) - 1);
                            am.update(txn).await?;
                        }
                        for (idx, jref) in task.judges.iter().enumerate() {
                            // Presence validated before the transaction.
                            let Some(judge_id) = judge_id_by_slug.get(jref.slug()).copied() else {
                                continue;
                            };
                            if let Some(row) = existing_tjs.iter().find(|r| r.judge_id == judge_id)
                            {
                                let mut am: arena_core::entities::task_judges::ActiveModel =
                                    row.clone().into();
                                am.ordinal = Set(idx as i32);
                                am.weight = Set(jref.weight());
                                // rating_scale_override is admin-set in the UI,
                                // not part of the seed — keep it.
                                am.updated_at = Set(now);
                                am.update(txn).await?;
                            } else {
                                let tj_am = arena_core::entities::task_judges::ActiveModel {
                                    id: Set(Uuid::new_v4()),
                                    task_id: Set(task_id),
                                    judge_id: Set(judge_id),
                                    ordinal: Set(idx as i32),
                                    rating_scale_override: Set(None),
                                    weight: Set(jref.weight()),
                                    created_at: Set(now),
                                    updated_at: Set(now),
                                };
                                arena_core::entities::task_judges::Entity::insert(tj_am)
                                    .exec(txn)
                                    .await?;
                            }
                        }
                    }

                    Ok((updated, inserted, deleted))
                })
            })
            .await
            .map_err(|e| match e {
                sea_orm::TransactionError::Transaction(err) => err,
                sea_orm::TransactionError::Connection(db_err) => ExportImportError::Db(db_err),
            })?;

    // Campaign membership lives on the children, so it is reconciled after
    // the project's own transaction: parts dropped from the list detach, the
    // remaining ones are renumbered in declaration order.
    apply_campaign_parts(&state.db, project_id, envelope).await?;

    // Tasks were just rewritten, and the seed's `judges:` lines do not carry
    // the session report — every project gets that centrally. Re-ensure it
    // here so a push cannot leave a project without its debrief.
    if let Err(e) = crate::seed::report_judge::ensure_report_judges(&state.db, project_id).await {
        tracing::warn!(project_id = %project_id, error = %e, "report judge: attach after apply-seed failed");
    }

    Ok(counts)
}

/// Reject the two shapes a campaign envelope must never have: a parent that
/// also ships tasks (they could never run — sessions on a parent are
/// refused), and a part list on a project that is already someone's part.
fn validate_campaign_shape(envelope: &ExportEnvelope) -> Result<(), ExportImportError> {
    if !envelope.project.parts.is_empty() && !envelope.tasks.is_empty() {
        return Err(ExportImportError::BadRequest(
            "a campaign parent declares `parts` and carries no tasks of its own".into(),
        ));
    }
    Ok(())
}

/// Link the envelope's declared parts to `project_id`, or unlink everything
/// when the envelope declares none. Rejections are 400s: an admin pushing a
/// seed wants to hear that a part slug is wrong, not to discover later that
/// the campaign silently lost a chapter.
async fn apply_campaign_parts(
    db: &sea_orm::DatabaseConnection,
    project_id: Uuid,
    envelope: &ExportEnvelope,
) -> Result<(), ExportImportError> {
    let slug = envelope.project.slug.clone().unwrap_or_default();
    crate::campaign_link::link_parts_strict(db, project_id, &slug, &envelope.project.parts)
        .await
        .map(|_| ())
        .map_err(|e| match e {
            crate::campaign_link::LinkError::Rejected(r) => {
                ExportImportError::BadRequest(r.to_string())
            }
            crate::campaign_link::LinkError::Db(db_err) => ExportImportError::Db(db_err),
        })
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ApplySeedResponse {
    pub project_id: Uuid,
    pub name: String,
    /// `true` when the project did not exist and was created.
    pub created: bool,
    pub tasks_updated: usize,
    pub tasks_inserted: usize,
    pub tasks_deleted: usize,
}

/// `POST /api/admin/projects/apply-seed` — upsert a project from a seed
/// envelope pushed by the client (the `push-seeds` command), matched by slug.
/// An existing project is updated in place exactly like `/reseed`; a missing
/// one is created with the envelope's slug, visibility and cover preserved and
/// its category ensured. Unlike `/import` (always creates a private copy,
/// dropping a colliding slug) this endpoint treats the envelope as the seed
/// source of truth, without requiring the file to exist on the server's disk.
pub async fn apply_seed(
    admin: AdminUser,
    State(state): State<AppState>,
    Json(envelope): Json<ExportEnvelope>,
) -> Result<Response, ExportImportError> {
    if envelope.schema_version != SCHEMA_VERSION {
        return Err(ExportImportError::UnsupportedSchema);
    }
    let raw_slug = envelope
        .project
        .slug
        .as_deref()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            ExportImportError::BadRequest(
                "seed envelope must carry a slug (it is the upsert key)".into(),
            )
        })?;
    let slug = crate::api::projects::validate_slug(raw_slug)
        .map_err(|e| ExportImportError::BadRequest(format!("invalid slug: {e}")))?;

    let existing = projects::Entity::find()
        .filter(projects::Column::Slug.eq(&slug))
        .one(&state.db)
        .await?;

    let (project_id, created, counts) = match existing {
        Some(project) => {
            let id = project.id;
            let counts = apply_envelope_to_project(&state, project, &envelope).await?;
            (id, false, counts)
        }
        None => {
            // Same pre-checks the update path runs inside apply_envelope_to_project.
            crate::validation::tags::validate_tags(&envelope.project.tags)
                .map_err(|e| ExportImportError::BadRequest(e.to_string()))?;
            validate_campaign_shape(&envelope)?;
            let mut seen_ordinals: HashSet<i32> = HashSet::new();
            for task in &envelope.tasks {
                crate::api::project_tasks::validate_ordinal(task.ordinal)
                    .map_err(|e| ExportImportError::BadRequest(e.to_string()))?;
                arena_core::validation::validate_template(&task.test_template)
                    .map_err(|e| ExportImportError::BadRequest(e.to_string()))?;
                crate::validation::tags::validate_tags(&task.tags)
                    .map_err(|e| ExportImportError::BadRequest(e.to_string()))?;
                validate_task_extras(task).map_err(|e| {
                    ExportImportError::BadRequest(format!("task {}: {e}", task.ordinal))
                })?;
                if !seen_ordinals.insert(task.ordinal) {
                    return Err(ExportImportError::BadRequest(format!(
                        "duplicate ordinal: {}",
                        task.ordinal
                    )));
                }
            }
            let judge_id_by_slug: std::collections::HashMap<String, Uuid> =
                arena_core::entities::judges::Entity::find()
                    .all(&state.db)
                    .await?
                    .into_iter()
                    .map(|j| (j.slug, j.id))
                    .collect();
            for task in &envelope.tasks {
                for jref in &task.judges {
                    let jslug = jref.slug();
                    if !judge_id_by_slug.contains_key(jslug) {
                        return Err(ExportImportError::BadRequest(format!(
                            "task {} references unknown judge slug '{jslug}'",
                            task.ordinal
                        )));
                    }
                }
            }
            // Category: find-or-create, matching boot-seed semantics.
            let resolved_category = match envelope.project.category.as_deref().map(str::trim) {
                Some(name) if !name.is_empty() && name.chars().count() <= 100 => {
                    crate::api::categories::ensure_category(&state.db, name).await?;
                    Some(name.to_string())
                }
                _ => None,
            };
            let inserted_tasks = envelope.tasks.len();
            let id = insert_project_from_envelope(
                &state,
                &envelope,
                &judge_id_by_slug,
                admin.id,
                Some(slug.clone()),
                resolved_category,
                envelope.project.public,
                envelope.project.cover_image_url.clone(),
            )
            .await?;
            apply_campaign_parts(&state.db, id, &envelope).await?;
            if let Err(e) = crate::seed::report_judge::ensure_report_judges(&state.db, id).await {
                tracing::warn!(project_id = %id, error = %e, "report judge: attach after apply-seed create failed");
            }
            (id, true, (0, inserted_tasks, 0))
        }
    };

    tracing::info!(
        project_id = %project_id,
        slug = %slug,
        created,
        updated = counts.0,
        inserted = counts.1,
        deleted = counts.2,
        "apply-seed: project upserted from pushed envelope"
    );

    Ok(Json(ApplySeedResponse {
        project_id,
        name: envelope.project.name,
        created,
        tasks_updated: counts.0,
        tasks_inserted: counts.1,
        tasks_deleted: counts.2,
    })
    .into_response())
}

#[cfg(test)]
mod tests;
