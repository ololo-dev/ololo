use crate::api::intervals::{IntervalsReq, IntervalsResp};
use crate::api::points::{PointsRange, PointsReq, PointsResp};
use crate::auth::jwt::AccessClaims;
use arena_core::entities::{categories, players, projects, sessions, tasks};
use arena_core::session_status::SessionStatus;
use sea_orm::{
    ColumnTrait, Condition, DatabaseConnection, EntityTrait, PaginatorTrait, QueryFilter,
    QueryOrder, QuerySelect,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
#[derive(Debug, Deserialize)]
pub struct ListProjectsQuery {
    #[serde(default)]
    pub include_archived: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateProjectReq {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default = "default_true")]
    pub public: bool,
    #[serde(default)]
    pub category: Option<String>,
    #[serde(default)]
    pub tags: Option<Vec<String>>,
    #[serde(default)]
    pub cover_image_url: Option<String>,
    #[serde(default)]
    pub points: Option<PointsReq>,
    #[serde(default)]
    pub intervals: Option<IntervalsReq>,
    #[serde(default)]
    pub session_duration_secs: Option<i64>,
    /// Cancel a running session after this many seconds with no connected
    /// agents. 0 disables the idle sweep.
    #[serde(default)]
    pub idle_timeout_secs: Option<i32>,
    /// Session-memory schema: JSON object of `key -> default value`
    /// (scalars), e.g. `{"dev": "npm run dev", "port": 1234}`.
    #[serde(default)]
    pub memory_schema: Option<serde_json::Value>,
}

pub(crate) fn default_true() -> bool {
    true
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PatchProjectReq {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub public: Option<bool>,
    /// `true` = archive now; `false` = unarchive. Absent = no change.
    #[serde(default)]
    pub archived: Option<bool>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub slug: Option<String>,
    #[serde(default)]
    pub category: Option<String>,
    #[serde(default)]
    pub tags: Option<Vec<String>>,
    #[serde(default)]
    pub cover_image_url: Option<String>,
    /// When `true`, sets `cover_image_url` to NULL (mirrors `archived` pattern).
    /// Takes precedence over `cover_image_url` in the same request.
    #[serde(default)]
    pub clear_cover_image: Option<bool>,
    #[serde(default)]
    pub points: Option<PointsReq>,
    #[serde(default)]
    pub intervals: Option<IntervalsReq>,
    #[serde(default)]
    pub session_duration_secs: Option<i64>,
    /// Cancel a running session after this many seconds with no connected
    /// agents. 0 disables the idle sweep.
    #[serde(default)]
    pub idle_timeout_secs: Option<i32>,
    /// Session-memory schema (JSON object of scalar defaults). Explicit
    /// `null` clears it; absent leaves it unchanged.
    #[serde(default, deserialize_with = "deserialize_some")]
    pub memory_schema: Option<Option<serde_json::Value>>,
}

/// Validate a memory-schema JSON value (object of scalar defaults, capped
/// key/value sizes) and return its canonical JSON string for storage.
pub(crate) fn validate_memory_schema_json(v: &serde_json::Value) -> Result<String, ProjectError> {
    arena_core::memory::parse_memory_schema(v)
        .map_err(|e| ProjectError::InvalidMemorySchema(e.to_string()))?;
    Ok(v.to_string())
}

/// Distinguish "absent" (outer `None`) from explicit `null` (inner `None`)
/// on PATCH bodies (mirrors `task_judges::deserialize_some`).
fn deserialize_some<'de, T, D>(d: D) -> Result<Option<T>, D::Error>
where
    T: serde::Deserialize<'de>,
    D: serde::Deserializer<'de>,
{
    T::deserialize(d).map(Some)
}

#[derive(Debug, Serialize)]
pub struct ProjectSummary {
    pub id: Uuid,
    pub name: String,
    pub slug: Option<String>,
    pub description: String,
    pub category: Option<String>,
    pub tags: Vec<String>,
    pub cover_image_url: Option<String>,
    pub owner_user_id: Uuid,
    pub public: bool,
    pub archived_at: Option<chrono::DateTime<chrono::Utc>>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
    pub has_active_sessions: bool,
    pub task_count: i64,
    pub points: PointsResp,
    pub points_range: Option<PointsRange>,
    pub intervals: IntervalsResp,
    pub session_duration_secs: i64,
    pub idle_timeout_secs: i32,
    pub memory_schema: Option<serde_json::Value>,
    /// Whether the public project page lists the task arc up front.
    pub show_tasks: bool,
    /// Finished sessions ever played on this project — the catalog's "played"
    /// counter. Only the list endpoint computes it (batched); detail/mutation
    /// responses omit it rather than pay an extra count per request.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_count: Option<i64>,
    /// Estimated judge reviews a full session triggers: the total number of
    /// judges attached across all of this project's tasks (each runs once per
    /// task per player, and counts toward the monthly judge-run quota).
    /// Computed on the list and detail endpoints; `None` on mutation
    /// responses that don't pay the extra count.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub judge_review_count: Option<i64>,
    /// Sessions the *authenticated caller* has played on this project — the
    /// per-user "you've played this" indicator on the catalog card. Only the
    /// list endpoint sets it, and only when a user is signed in; `None`
    /// otherwise (anonymous callers, detail/mutation responses).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_session_count: Option<i64>,
    /// Campaign membership, straight off the row: the campaign this project
    /// is a part of, and its position in it. Both `None` for standalone
    /// projects and for campaign parents.
    pub parent_project_id: Option<Uuid>,
    pub part_ordinal: Option<i32>,
    /// How many parts this project has as a campaign parent. Set by the
    /// endpoints that pay for the count; `None` elsewhere. Non-`None` and
    /// non-zero is what makes a project render as a campaign.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub part_count: Option<i64>,
    /// Playing time of a whole campaign: its parts' session durations added
    /// up. A campaign's own `session_duration_secs` is meaningless — it
    /// hosts no sessions — so this is what a listing should show as "how
    /// long is this". `None` for anything that is not a campaign.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parts_duration_secs: Option<i64>,
    /// Identity of the campaign this project belongs to, for the "Part N of
    /// …" breadcrumb. Set alongside `parent_project_id` by endpoints that
    /// resolve it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_project_slug: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_project_name: Option<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct ProjectListResp {
    pub projects: Vec<ProjectSummary>,
}

// ─── Error type ────────────────────────────────────────────────────────────

#[derive(Debug, thiserror::Error)]
pub enum ProjectError {
    #[error("not_found")]
    NotFound,
    #[error("forbidden")]
    Forbidden,
    #[error("invalid_name")]
    InvalidName,
    #[error("project_archived")]
    ProjectArchived,
    #[error("project_in_use")]
    ProjectInUse { session_count: i64 },
    #[error("slug_conflict")]
    SlugConflict,
    #[error("invalid_slug")]
    InvalidSlug,
    #[error("invalid_category")]
    InvalidCategory,
    #[error("invalid_tags")]
    InvalidTags,
    #[error("invalid_cover_image_url")]
    InvalidCoverImageUrl,
    #[error("invalid_session_duration")]
    InvalidSessionDuration,
    #[error("invalid_idle_timeout")]
    InvalidIdleTimeout,
    #[error("invalid_memory_schema: {0}")]
    InvalidMemorySchema(String),
    #[error("project creation is currently restricted to administrators")]
    CreationRestricted,
    #[error("database error: {0}")]
    Db(#[from] sea_orm::DbErr),
}

crate::api::error::impl_api_error!(ProjectError {
    Self::NotFound => (NOT_FOUND, "not_found"),
    Self::Forbidden => (FORBIDDEN, "forbidden"),
    Self::InvalidName => (UNPROCESSABLE_ENTITY, "invalid_name"),
    Self::ProjectArchived => (CONFLICT, "project_archived"),
    Self::ProjectInUse { session_count } => (
        CONFLICT,
        "project_in_use",
        "session_count": session_count,
    ),
    Self::SlugConflict => (CONFLICT, "slug_conflict"),
    Self::InvalidSlug => (UNPROCESSABLE_ENTITY, "invalid_slug"),
    Self::InvalidCategory => (UNPROCESSABLE_ENTITY, "invalid_category"),
    Self::InvalidTags => (UNPROCESSABLE_ENTITY, "invalid_tags"),
    Self::InvalidCoverImageUrl => (UNPROCESSABLE_ENTITY, "invalid_cover_image_url"),
    Self::InvalidSessionDuration => (UNPROCESSABLE_ENTITY, "invalid_session_duration"),
    Self::InvalidIdleTimeout => (UNPROCESSABLE_ENTITY, "invalid_idle_timeout"),
    Self::InvalidMemorySchema(_) => (UNPROCESSABLE_ENTITY, "invalid_memory_schema"),
    Self::CreationRestricted => (
        FORBIDDEN,
        "project creation is currently restricted to administrators",
    ),
    Self::Db(_) => (INTERNAL_SERVER_ERROR, "database_error"),
});

// ─── Helpers ───────────────────────────────────────────────────────────────

pub(crate) fn parse_user_id(claims: &AccessClaims) -> Result<Uuid, ProjectError> {
    claims.user_id().map_err(|_| ProjectError::Forbidden)
}

pub(crate) fn validate_name(raw: &str) -> Result<String, ProjectError> {
    let trimmed = raw.trim().to_string();
    let len = trimmed.chars().count();
    if !(NAME_MIN..=NAME_MAX).contains(&len) {
        return Err(ProjectError::InvalidName);
    }
    Ok(trimmed)
}

/// Check whether (owner, slug) already exists. Returns true if conflict.
pub(crate) async fn slug_exists(
    db: &DatabaseConnection,
    owner_id: Uuid,
    slug: &str,
    exclude_id: Option<Uuid>,
) -> Result<bool, sea_orm::DbErr> {
    let mut q = projects::Entity::find()
        .filter(projects::Column::OwnerUserIdFk.eq(owner_id))
        .filter(projects::Column::Slug.eq(slug));
    if let Some(ex) = exclude_id {
        q = q.filter(projects::Column::Id.ne(ex));
    }
    Ok(q.one(db).await?.is_some())
}

/// Validate a user-provided slug. Returns the slug as-is if valid.
/// Rule: `^[a-z0-9][a-z0-9-]*[a-z0-9]$` OR single char `^[a-z0-9]$`, max 64 chars.
pub(crate) fn validate_slug(slug: &str) -> Result<String, ProjectError> {
    let len = slug.len();
    if len == 0 || len > 64 {
        return Err(ProjectError::InvalidSlug);
    }
    let mut chars = slug.chars().peekable();
    for c in &mut chars {
        if !(c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-') {
            return Err(ProjectError::InvalidSlug);
        }
    }
    // Must not contain consecutive dashes
    if slug.contains("--") {
        return Err(ProjectError::InvalidSlug);
    }
    // Must start and end with alphanumeric
    let first = slug.chars().next().unwrap();
    let last = slug.chars().last().unwrap();
    if !(first.is_ascii_lowercase() || first.is_ascii_digit()) {
        return Err(ProjectError::InvalidSlug);
    }
    if !(last.is_ascii_lowercase() || last.is_ascii_digit()) {
        return Err(ProjectError::InvalidSlug);
    }
    Ok(slug.to_string())
}

/// Load the allowed category names from the `categories` table.
/// Returns `None` if the table is empty — callers use this as the permissive
/// fallback: any string is accepted when no categories are defined.
pub(crate) async fn load_allowed_categories(
    db: &DatabaseConnection,
) -> Result<Option<Vec<String>>, sea_orm::DbErr> {
    let rows = categories::Entity::find()
        .order_by_asc(categories::Column::Id)
        .all(db)
        .await?;
    if rows.is_empty() {
        Ok(None) // permissive fallback
    } else {
        Ok(Some(rows.into_iter().map(|r| r.name).collect()))
    }
}

/// Validate a category string against the allowed list (FR-011).
/// If `allowed` is `None` (key absent from app_settings), validation is skipped —
/// any string is accepted. Values stored under the permissive path are preserved
/// on read even after the key is restored.
pub(crate) fn validate_category(
    category: &str,
    allowed: &Option<Vec<String>>,
) -> Result<(), ProjectError> {
    match allowed {
        None => Ok(()), // permissive fallback — key absent
        Some(list) => {
            if list.iter().any(|a| a == category) {
                Ok(())
            } else {
                Err(ProjectError::InvalidCategory)
            }
        }
    }
}

/// Validate a tag list — delegates to the shared `validation::tags` module and
/// maps any error to `ProjectError::InvalidTags`.
pub(crate) fn validate_tags(tags: &[String]) -> Result<(), ProjectError> {
    crate::validation::tags::validate_tags(tags).map_err(|_| ProjectError::InvalidTags)
}

/// Validate a cover image URL (FR-013): must start with `https://` and be ≤2048 chars.
pub(crate) fn validate_cover_image_url(url: &str) -> Result<(), ProjectError> {
    if !url.starts_with("https://") || url.len() > 2048 {
        return Err(ProjectError::InvalidCoverImageUrl);
    }
    Ok(())
}

/// Validate a session duration — delegates to the shared arena-core validator
/// (60..=86400 inclusive) and maps any error to `ProjectError::InvalidSessionDuration`.
pub(crate) fn validate_session_duration_secs(secs: i64) -> Result<(), ProjectError> {
    let unsigned = u64::try_from(secs).map_err(|_| ProjectError::InvalidSessionDuration)?;
    crate::validation::validate_session_duration(unsigned)
        .map_err(|_| ProjectError::InvalidSessionDuration)
}

/// Validate an idle timeout: 0 (disabled) or 30s..=24h. Sub-30s thresholds
/// would race normal reconnects.
pub(crate) fn validate_idle_timeout_secs(secs: i32) -> Result<(), ProjectError> {
    if secs == 0 || (30..=86_400).contains(&secs) {
        Ok(())
    } else {
        Err(ProjectError::InvalidIdleTimeout)
    }
}

pub fn to_summary(
    m: projects::Model,
    has_active_sessions: bool,
    task_count: i64,
    points_range: Option<PointsRange>,
) -> ProjectSummary {
    to_summary_with_sessions(m, has_active_sessions, task_count, points_range, None)
}

pub fn to_summary_with_sessions(
    m: projects::Model,
    has_active_sessions: bool,
    task_count: i64,
    points_range: Option<PointsRange>,
    session_count: Option<i64>,
) -> ProjectSummary {
    let tags: Vec<String> = serde_json::from_str(&m.tags).unwrap_or_else(|_| {
        tracing::warn!(project_id = %m.id, "failed to parse tags JSON; returning empty vec");
        vec![]
    });
    ProjectSummary {
        id: m.id,
        name: m.name,
        slug: m.slug,
        description: m.description,
        category: m.category,
        tags,
        cover_image_url: m.cover_image_url,
        owner_user_id: m.owner_user_id_fk,
        public: m.public,
        archived_at: m.archived_at,
        created_at: m.created_at,
        updated_at: m.updated_at,
        has_active_sessions,
        task_count,
        points: PointsResp {
            value: m.default_value_points,
            fail: m.default_fail_points,
            no_response: m.default_no_response_points,
            completion_bonus: m.default_completion_bonus_points,
        },
        points_range,
        intervals: IntervalsResp {
            deadline_secs: m.default_deadline_secs,
            min_interval_secs: m.default_min_interval_secs,
            interval_increment_secs: m.default_interval_increment_secs,
            max_interval_secs: m.default_max_interval_secs,
        },
        session_duration_secs: m.default_session_duration_secs,
        idle_timeout_secs: m.idle_timeout_secs,
        show_tasks: m.show_tasks,
        memory_schema: m
            .memory_schema
            .as_deref()
            .and_then(|s| serde_json::from_str(s).ok()),
        session_count,
        judge_review_count: None,
        user_session_count: None,
        parent_project_id: m.parent_project_id_fk,
        part_ordinal: m.part_ordinal,
        part_count: None,
        parts_duration_secs: None,
        parent_project_slug: None,
        parent_project_name: None,
    }
}

/// Fill the campaign fields an endpoint chose to pay for: how many parts this
/// project has, and who its campaign parent is. Kept as one call so no
/// endpoint sets half of the pair.
pub async fn attach_campaign_context(
    db: &sea_orm::DatabaseConnection,
    summary: &mut ProjectSummary,
) -> Result<(), sea_orm::DbErr> {
    use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};

    let parts = projects::Entity::find()
        .filter(projects::Column::ParentProjectIdFk.eq(summary.id))
        .all(db)
        .await?;
    summary.part_count = Some(parts.len() as i64);
    if !parts.is_empty() {
        summary.parts_duration_secs =
            Some(parts.iter().map(|p| p.default_session_duration_secs).sum());
    }

    if let Some(parent_id) = summary.parent_project_id
        && let Some(parent) = projects::Entity::find_by_id(parent_id).one(db).await?
    {
        summary.parent_project_slug = parent.slug;
        summary.parent_project_name = Some(parent.name);
    }
    Ok(())
}

/// Check whether the project has any active sessions (lobby or running).
pub(crate) async fn project_has_active_sessions(
    db: &DatabaseConnection,
    project_id: Uuid,
) -> Result<bool, sea_orm::DbErr> {
    let count = sessions::Entity::find()
        .filter(sessions::Column::ProjectIdFk.eq(project_id))
        .filter(
            Condition::any()
                .add(sessions::Column::Status.eq(SessionStatus::Lobby))
                .add(sessions::Column::Status.eq(SessionStatus::Running)),
        )
        .count(db)
        .await?;
    Ok(count > 0)
}

/// Compute the min/max of `tasks.point_value` for a project. Returns `None`
/// when the project has no tasks. Used by single-project reads to populate
/// `ProjectSummary.points_range`; the list endpoint skips this (N+1 avoidance).
pub(crate) async fn compute_points_range(
    db: &DatabaseConnection,
    project_id: Uuid,
) -> Result<Option<PointsRange>, sea_orm::DbErr> {
    use sea_orm::sea_query::Expr;
    let row = tasks::Entity::find()
        .filter(tasks::Column::ProjectIdFk.eq(project_id))
        .select_only()
        .column_as(Expr::col(tasks::Column::PointValue).min(), "min_pv")
        .column_as(Expr::col(tasks::Column::PointValue).max(), "max_pv")
        .into_tuple::<(Option<i32>, Option<i32>)>()
        .one(db)
        .await?;
    Ok(row.and_then(|(min, max)| match (min, max) {
        (Some(lo), Some(hi)) => Some(PointsRange { min: lo, max: hi }),
        _ => None,
    }))
}

/// Load project and require caller is the owner.
/// Non-owners → 403. Missing → 404.
pub(crate) async fn load_for_owner(
    db: &DatabaseConnection,
    id: Uuid,
    user_id: Uuid,
) -> Result<projects::Model, ProjectError> {
    let row = projects::Entity::find_by_id(id)
        .one(db)
        .await?
        .ok_or(ProjectError::NotFound)?;
    if row.owner_user_id_fk != user_id {
        return Err(ProjectError::Forbidden);
    }
    Ok(row)
}

/// Load project with full visibility check (AD-7):
/// owner | public=true | owns or is member of any session in project.
/// Project exists but no access → 403 (not 404).
pub async fn load_accessible(
    db: &DatabaseConnection,
    id: Uuid,
    user_id: Uuid,
) -> Result<projects::Model, ProjectError> {
    let row = projects::Entity::find_by_id(id)
        .one(db)
        .await?
        .ok_or(ProjectError::NotFound)?;

    if row.owner_user_id_fk == user_id || row.public {
        return Ok(row);
    }

    // Check if user owns any session in this project.
    let owns_session = sessions::Entity::find()
        .filter(sessions::Column::ProjectIdFk.eq(id))
        .filter(sessions::Column::OwnerIdFk.eq(user_id))
        .one(db)
        .await?;
    if owns_session.is_some() {
        return Ok(row);
    }

    // Check if user is a member of any session in this project.
    let project_session_ids: Vec<Uuid> = sessions::Entity::find()
        .filter(sessions::Column::ProjectIdFk.eq(id))
        .all(db)
        .await?
        .into_iter()
        .map(|s| s.id)
        .collect();
    if !project_session_ids.is_empty() {
        let member = players::Entity::find()
            .filter(players::Column::SessionIdFk.is_in(project_session_ids))
            .filter(players::Column::UserIdFk.eq(user_id))
            .one(db)
            .await?;
        if member.is_some() {
            return Ok(row);
        }
    }

    Err(ProjectError::Forbidden)
}

/// Load project by PK with no visibility or ownership check (admin paths only).
/// Missing → 404.
pub(crate) async fn load_any(
    db: &DatabaseConnection,
    id: Uuid,
) -> Result<projects::Model, ProjectError> {
    projects::Entity::find_by_id(id)
        .one(db)
        .await?
        .ok_or(ProjectError::NotFound)
}

#[cfg(test)]
mod tests;
const NAME_MIN: usize = 1;
const NAME_MAX: usize = 200;
