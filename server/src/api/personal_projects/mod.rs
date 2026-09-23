//! `/api/personal-projects` — a user's own work as a playable project.
//!
//! The user describes the work and, optionally, lists the tasks it splits
//! into (the navigation map); the server turns that into an ordinary
//! private project — one open-ended, judged task per map entry, or one for
//! the whole description — that its owner starts with `ololo start <slug>`
//! inside their own repository. The request is kept beside the project
//! (`personal_projects.spec`) so the owner can edit it until the first
//! session freezes the tasks, and duplicate it after.
//!
//! Routes:
//! - `GET  /api/personal-projects/options` — judges, limits, session bounds
//! - `POST /api/personal-projects` — create
//! - `POST /api/personal-projects/suggest-tasks` — draft a navigation map
//! - `GET  /api/personal-projects/:project_id` — the project and its spec
//! - `PUT  /api/personal-projects/:project_id` — rebuild before first play

mod build;
mod suggest;

pub use suggest::suggest_tasks;

use arena_core::entities::{personal_projects, projects, sessions, tasks, users};
use arena_core::personal::{self, PersonalSpec};
use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use chrono::Utc;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, PaginatorTrait, QueryFilter,
    Set, TransactionTrait,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::api::admin_export_import::{ExportPoints, insert_task_with_judges};
use crate::api::projects::{
    KIND_PERSONAL, ProjectSummary, compute_points_range, judge_review_count,
    project_has_active_sessions, to_summary,
};
use crate::auth::jwt::AccessClaims;
use crate::state::AppState;

// ─── Wire types ────────────────────────────────────────────────────────────

/// Create/replace body. Everything but the description is optional: no
/// name is derived from the description, no tasks make one task, no judges
/// get the default panel, no duration gets two hours.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PersonalProjectReq {
    #[serde(default)]
    pub name: Option<String>,
    pub description: String,
    #[serde(default)]
    pub tasks: Vec<PersonalTaskReq>,
    #[serde(default)]
    pub judges: Option<Vec<String>>,
    #[serde(default)]
    pub session_duration_secs: Option<i64>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PersonalTaskReq {
    pub title: String,
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct OptionsResp {
    /// Whether the caller may create a personal project at all.
    pub creation_allowed: bool,
    pub judges: Vec<JudgeOption>,
    pub limits: Limits,
    pub session: SessionBounds,
    /// Whether "Suggest tasks" has a model to ask.
    pub suggest_available: bool,
    /// Points one task pays, for the form's "what a session costs" line.
    pub task_points: i32,
}

#[derive(Debug, Serialize)]
pub struct JudgeOption {
    pub slug: String,
    pub name: String,
    pub description: String,
    pub criteria: Vec<String>,
    pub avatar_url: Option<String>,
    /// On the panel of a project that does not choose.
    pub default: bool,
}

#[derive(Debug, Serialize)]
pub struct Limits {
    pub max_tasks: usize,
    pub max_judges: usize,
    pub max_name_chars: usize,
    pub max_description_chars: usize,
    pub max_task_title_chars: usize,
    pub max_task_description_chars: usize,
}

#[derive(Debug, Serialize)]
pub struct SessionBounds {
    pub min_secs: i64,
    pub max_secs: i64,
    pub default_secs: i64,
}

#[derive(Debug, Serialize)]
pub struct PersonalProjectResp {
    pub project: ProjectSummary,
    pub spec: PersonalSpec,
    /// Sessions ever started on it; its tasks are frozen from the first.
    pub session_count: u64,
    /// Whether `PUT` may still rebuild the tasks.
    pub editable: bool,
}

// ─── Errors ────────────────────────────────────────────────────────────────

#[derive(Debug, thiserror::Error)]
pub enum PersonalProjectError {
    #[error("not_found")]
    NotFound,
    #[error("forbidden")]
    Forbidden,
    #[error("project creation is currently restricted to administrators")]
    CreationRestricted,
    #[error("invalid {field}: {detail}")]
    Invalid { field: &'static str, detail: String },
    /// A session exists: the tasks are what that session played.
    #[error("project_frozen")]
    ProjectFrozen,
    #[error("no_judges_available")]
    NoJudgesAvailable,
    #[error("database error: {0}")]
    Db(#[from] sea_orm::DbErr),
}

crate::api::error::impl_api_error!(PersonalProjectError {
    Self::NotFound => (NOT_FOUND, "not_found"),
    Self::Forbidden => (FORBIDDEN, "forbidden"),
    Self::CreationRestricted => (
        FORBIDDEN,
        "project creation is currently restricted to administrators",
    ),
    Self::Invalid { field, detail } => (
        UNPROCESSABLE_ENTITY,
        "invalid_personal_project",
        "field": field,
        "detail": detail,
    ),
    Self::ProjectFrozen => (CONFLICT, "project_frozen"),
    Self::NoJudgesAvailable => (UNPROCESSABLE_ENTITY, "no_judges_available"),
    Self::Db(_) => (INTERNAL_SERVER_ERROR, "database_error"),
});

impl From<sea_orm::TransactionError<PersonalProjectError>> for PersonalProjectError {
    fn from(e: sea_orm::TransactionError<PersonalProjectError>) -> Self {
        match e {
            sea_orm::TransactionError::Transaction(err) => err,
            sea_orm::TransactionError::Connection(db) => PersonalProjectError::Db(db),
        }
    }
}

// ─── Helpers ───────────────────────────────────────────────────────────────

/// The caller, refused unless they may create projects on this instance.
async fn creator(
    db: &DatabaseConnection,
    claims: &AccessClaims,
) -> Result<users::Model, PersonalProjectError> {
    let user_id = claims
        .user_id()
        .map_err(|_| PersonalProjectError::Forbidden)?;
    let user = users::Entity::find_by_id(user_id)
        .one(db)
        .await?
        .ok_or(PersonalProjectError::Forbidden)?;
    if !user.is_admin && !crate::api::settings::is_project_creation_allowed(db).await? {
        return Err(PersonalProjectError::CreationRestricted);
    }
    Ok(user)
}

/// The personal project `id` with its spec; 404 for anything else,
/// including someone else's (existence is not leaked).
async fn load_personal(
    db: &DatabaseConnection,
    id: Uuid,
    caller: Uuid,
    allow_admin: bool,
) -> Result<(projects::Model, personal_projects::Model), PersonalProjectError> {
    let project = projects::Entity::find_by_id(id)
        .one(db)
        .await?
        .ok_or(PersonalProjectError::NotFound)?;
    let marker = personal_projects::Entity::find_by_id(id)
        .one(db)
        .await?
        .ok_or(PersonalProjectError::NotFound)?;
    if project.owner_user_id_fk != caller
        && !(allow_admin && crate::auth::is_user_admin(db, caller).await?)
    {
        return Err(PersonalProjectError::NotFound);
    }
    Ok((project, marker))
}

async fn summary(
    db: &DatabaseConnection,
    row: projects::Model,
) -> Result<ProjectSummary, sea_orm::DbErr> {
    let active = project_has_active_sessions(db, row.id).await?;
    let task_count = tasks::Entity::find()
        .filter(tasks::Column::ProjectIdFk.eq(row.id))
        .count(db)
        .await?;
    let range = compute_points_range(db, row.id).await?;
    let reviews = judge_review_count(db, row.id).await?;
    let mut out = to_summary(row, active, task_count as i64, range);
    out.judge_review_count = Some(reviews);
    out.kind = KIND_PERSONAL;
    Ok(out)
}

/// Project-level defaults of a personal project: its tasks carry their own
/// points; the intervals pace the one-a-minute done-check.
fn project_points() -> ExportPoints {
    ExportPoints {
        value: 10,
        fail: -5,
        no_response: -10,
        completion_bonus: personal::TASK_COMPLETION_BONUS,
        health: personal::TASK_HEALTH_POINTS,
    }
}

/// Insert the tasks of `spec` (judges attached) into `project_id`.
async fn insert_tasks(
    txn: &sea_orm::DatabaseTransaction,
    project_id: Uuid,
    spec: &PersonalSpec,
    eligible: &[arena_core::entities::judges::Model],
    now: chrono::DateTime<Utc>,
) -> Result<(), PersonalProjectError> {
    let panel = build::panel(spec, eligible);
    let judge_ids: std::collections::HashMap<String, Uuid> =
        eligible.iter().map(|j| (j.slug.clone(), j.id)).collect();
    let points = project_points();
    for task in personal::build_tasks(spec, &panel) {
        let export = build::export_task(task, &panel);
        crate::api::admin_export_import::validate_task_extras(&export).map_err(|detail| {
            PersonalProjectError::Invalid {
                field: "tasks",
                detail,
            }
        })?;
        insert_task_with_judges(txn, project_id, &export, &points, &judge_ids, now).await?;
    }
    Ok(())
}

/// The session report rides on the first task of every project; the boot
/// pass would add it on the next restart, a player finishing before then
/// would get no report. Best-effort: a failure costs the report, not the
/// project.
async fn attach_report_judge(db: &DatabaseConnection, project_id: Uuid) {
    if let Err(e) = crate::seed::report_judge::ensure_report_judges(db, project_id).await {
        tracing::warn!(project_id = %project_id, error = %e, "personal project: report judge not attached");
    }
}

// ─── Handlers ──────────────────────────────────────────────────────────────

/// `GET /api/personal-projects/options`
pub async fn get_options(
    State(state): State<AppState>,
    claims: AccessClaims,
) -> Result<Json<OptionsResp>, PersonalProjectError> {
    let creation_allowed = match creator(&state.db, &claims).await {
        Ok(_) => true,
        Err(PersonalProjectError::CreationRestricted) => false,
        Err(e) => return Err(e),
    };
    let eligible = build::eligible_judges(&state.db).await?;
    let defaults = build::default_panel(&eligible);
    let judges = eligible
        .iter()
        .map(|j| JudgeOption {
            slug: j.slug.clone(),
            name: j.name.clone(),
            description: j.description.clone(),
            criteria: build::criteria_of(j),
            avatar_url: j.avatar_url.clone(),
            default: defaults.contains(&j.slug),
        })
        .collect();
    let suggest_available = !crate::llm::resolve_candidates_for_operation(
        &state.db,
        &state.settings_encryption,
        "project_ai",
    )
    .await
    .is_empty();
    Ok(Json(OptionsResp {
        creation_allowed,
        judges,
        limits: Limits {
            max_tasks: personal::MAX_TASKS,
            max_judges: personal::MAX_JUDGES,
            max_name_chars: personal::MAX_NAME_CHARS,
            max_description_chars: personal::MAX_DESCRIPTION_CHARS,
            max_task_title_chars: personal::MAX_TASK_TITLE_CHARS,
            max_task_description_chars: personal::MAX_TASK_DESCRIPTION_CHARS,
        },
        session: SessionBounds {
            min_secs: personal::MIN_SESSION_SECS,
            max_secs: personal::MAX_SESSION_SECS,
            default_secs: personal::DEFAULT_SESSION_SECS,
        },
        suggest_available,
        task_points: personal::TASK_POINTS,
    }))
}

/// `POST /api/personal-projects`
#[tracing::instrument(level = "info", skip_all)]
pub async fn post_create(
    State(state): State<AppState>,
    claims: AccessClaims,
    Json(req): Json<PersonalProjectReq>,
) -> Result<Response, PersonalProjectError> {
    let user = creator(&state.db, &claims).await?;
    let eligible = build::eligible_judges(&state.db).await?;
    if eligible.is_empty() {
        return Err(PersonalProjectError::NoJudgesAvailable);
    }
    let spec = build::validate(&req, &eligible)?;
    let slug = build::unique_slug(&state.db, &spec.name).await?;
    let spec_json = serde_json::to_value(&spec).expect("a spec always serializes");

    let project_id = Uuid::new_v4();
    let owner = user.id;
    state
        .db
        .transaction::<_, (), PersonalProjectError>(|txn| {
            let spec = spec.clone();
            let eligible = eligible.clone();
            Box::pin(async move {
                let now = Utc::now();
                let points = project_points();
                projects::ActiveModel {
                    id: Set(project_id),
                    name: Set(spec.name.clone()),
                    slug: Set(Some(slug)),
                    description: Set(spec.description.clone()),
                    category: Set(None),
                    tags: Set("[]".to_string()),
                    cover_image_url: Set(None),
                    owner_user_id_fk: Set(owner),
                    // Private for good: the generic editor refuses to
                    // publish a personal project.
                    public: Set(false),
                    archived_at: Set(None),
                    created_at: Set(now),
                    updated_at: Set(now),
                    default_value_points: Set(points.value),
                    default_fail_points: Set(points.fail),
                    default_no_response_points: Set(points.no_response),
                    default_health_points: Set(points.health),
                    default_completion_bonus_points: Set(points.completion_bonus),
                    default_deadline_secs: Set(90),
                    default_session_duration_secs: Set(spec.session_duration_secs),
                    idle_timeout_secs: Set(personal::IDLE_TIMEOUT_SECS),
                    default_min_interval_secs: Set(20),
                    default_interval_increment_secs: Set(20),
                    default_max_interval_secs: Set(120),
                    memory_schema: Set(None),
                    show_tasks: Set(true),
                    parent_project_id_fk: Set(None),
                    part_ordinal: Set(None),
                }
                .insert(txn)
                .await?;
                insert_tasks(txn, project_id, &spec, &eligible, now).await?;
                personal_projects::ActiveModel {
                    project_id_fk: Set(project_id),
                    spec: Set(spec_json),
                    created_at: Set(now),
                    updated_at: Set(now),
                }
                .insert(txn)
                .await?;
                Ok(())
            })
        })
        .await?;
    attach_report_judge(&state.db, project_id).await;

    let row = projects::Entity::find_by_id(project_id)
        .one(&state.db)
        .await?
        .ok_or(PersonalProjectError::NotFound)?;
    tracing::info!(project_id = %project_id, owner = %owner, tasks = spec.tasks.len(), "personal project created");
    Ok((StatusCode::CREATED, Json(summary(&state.db, row).await?)).into_response())
}

/// `GET /api/personal-projects/:project_id` — owner or admin.
pub async fn get_one(
    State(state): State<AppState>,
    claims: AccessClaims,
    Path(project_id): Path<Uuid>,
) -> Result<Json<PersonalProjectResp>, PersonalProjectError> {
    let caller = claims
        .user_id()
        .map_err(|_| PersonalProjectError::Forbidden)?;
    let (project, marker) = load_personal(&state.db, project_id, caller, true).await?;
    let spec: PersonalSpec = serde_json::from_value(marker.spec).map_err(|e| {
        sea_orm::DbErr::Custom(format!("personal project {project_id}: bad spec: {e}"))
    })?;
    let session_count = sessions::Entity::find()
        .filter(sessions::Column::ProjectIdFk.eq(project_id))
        .count(&state.db)
        .await?;
    Ok(Json(PersonalProjectResp {
        project: summary(&state.db, project).await?,
        spec,
        session_count,
        editable: session_count == 0,
    }))
}

/// `PUT /api/personal-projects/:project_id` — owner only, until the first
/// session. Rebuilds every task from the new request; the slug stays, so
/// the `ololo start` line the owner copied keeps working.
#[tracing::instrument(level = "info", skip_all, fields(project_id = %project_id))]
pub async fn put_one(
    State(state): State<AppState>,
    claims: AccessClaims,
    Path(project_id): Path<Uuid>,
    Json(req): Json<PersonalProjectReq>,
) -> Result<Response, PersonalProjectError> {
    // Editing what one already owns is not creating: no creation gate.
    let caller = claims
        .user_id()
        .map_err(|_| PersonalProjectError::Forbidden)?;
    let (project, _) = load_personal(&state.db, project_id, caller, false).await?;
    if project.archived_at.is_some() {
        return Err(PersonalProjectError::Invalid {
            field: "project",
            detail: "unarchive the project before editing it".into(),
        });
    }
    let eligible = build::eligible_judges(&state.db).await?;
    if eligible.is_empty() {
        return Err(PersonalProjectError::NoJudgesAvailable);
    }
    let spec = build::validate(&req, &eligible)?;
    let spec_json = serde_json::to_value(&spec).expect("a spec always serializes");

    state
        .db
        .transaction::<_, (), PersonalProjectError>(|txn| {
            let spec = spec.clone();
            let eligible = eligible.clone();
            Box::pin(async move {
                // Checked inside the transaction: a session created between
                // the load and here must not find its tasks replaced.
                let played = sessions::Entity::find()
                    .filter(sessions::Column::ProjectIdFk.eq(project_id))
                    .count(txn)
                    .await?;
                if played > 0 {
                    return Err(PersonalProjectError::ProjectFrozen);
                }
                let now = Utc::now();
                let row = projects::Entity::find_by_id(project_id)
                    .one(txn)
                    .await?
                    .ok_or(PersonalProjectError::NotFound)?;
                let mut am: projects::ActiveModel = row.into();
                am.name = Set(spec.name.clone());
                am.description = Set(spec.description.clone());
                am.default_session_duration_secs = Set(spec.session_duration_secs);
                am.updated_at = Set(now);
                am.update(txn).await?;
                // Judge attachments go with their tasks (ON DELETE CASCADE).
                tasks::Entity::delete_many()
                    .filter(tasks::Column::ProjectIdFk.eq(project_id))
                    .exec(txn)
                    .await?;
                insert_tasks(txn, project_id, &spec, &eligible, now).await?;
                let marker = personal_projects::Entity::find_by_id(project_id)
                    .one(txn)
                    .await?
                    .ok_or(PersonalProjectError::NotFound)?;
                let mut am: personal_projects::ActiveModel = marker.into();
                am.spec = Set(spec_json);
                am.updated_at = Set(now);
                am.update(txn).await?;
                Ok(())
            })
        })
        .await?;
    attach_report_judge(&state.db, project_id).await;

    let row = projects::Entity::find_by_id(project_id)
        .one(&state.db)
        .await?
        .ok_or(PersonalProjectError::NotFound)?;
    Ok(Json(summary(&state.db, row).await?).into_response())
}
