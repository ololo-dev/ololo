use crate::auth::jwt::AccessClaims;
use arena_core::entities::{players, projects, sessions, tasks};
use arena_core::task_template::TestTemplate;

use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

const TITLE_MIN: usize = 1;
const TITLE_MAX: usize = 200;
const DESCRIPTION_MAX: usize = 4000;

// ─── Request / response shapes ─────────────────────────────────────────────

pub(crate) use crate::api::intervals::{
    ExportIntervals, resolve_intervals, validate_resolved_intervals,
};
pub use crate::api::intervals::{IntervalsReq, IntervalsResp};
pub use crate::api::points::{PointsReq, PointsResp};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateTaskReq {
    pub(crate) title: String,
    #[serde(default)]
    pub(crate) description: String,
    #[serde(default)]
    pub(crate) ordinal: Option<i32>,
    pub(crate) test_template: TestTemplate,
    #[serde(default)]
    pub(crate) tags: Option<Vec<String>>,
    #[serde(default)]
    pub(crate) points: Option<PointsReq>,
    #[serde(default)]
    pub(crate) intervals: Option<IntervalsReq>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PatchTaskReq {
    #[serde(default)]
    pub(crate) title: Option<String>,
    #[serde(default)]
    pub(crate) description: Option<String>,
    #[serde(default)]
    pub(crate) ordinal: Option<i32>,
    #[serde(default)]
    pub(crate) test_template: Option<TestTemplate>,
    #[serde(default)]
    pub(crate) tags: Option<Vec<String>>,
    #[serde(default)]
    pub(crate) points: Option<PointsReq>,
    #[serde(default)]
    pub(crate) intervals: Option<IntervalsReq>,
}

/// Returned by GET one — no aggregate counts.
#[derive(Debug, Serialize)]
pub(crate) struct TaskSummary {
    id: Uuid,
    project_id: Uuid,
    ordinal: i32,
    title: String,
    description: String,
    test_template: TestTemplate,
    tags: Vec<String>,
    created_at: chrono::DateTime<chrono::Utc>,
    points: PointsResp,
    intervals: IntervalsResp,
}

/// Returned by GET list — includes aggregate counts.
#[derive(Debug, Serialize)]
pub(crate) struct TaskListItem {
    pub(crate) id: Uuid,
    pub(crate) project_id: Uuid,
    pub(crate) ordinal: i32,
    pub(crate) title: String,
    pub(crate) description: String,
    pub(crate) test_template: TestTemplate,
    pub(crate) tags: Vec<String>,
    pub(crate) created_at: chrono::DateTime<chrono::Utc>,
    pub(crate) result_count: i64,
    pub(crate) session_count: i64,
    pub(crate) points: PointsResp,
    pub(crate) intervals: IntervalsResp,
}

#[derive(Debug, Serialize)]
pub(crate) struct TaskListResp {
    pub(crate) tasks: Vec<TaskListItem>,
}

// ─── Error type ────────────────────────────────────────────────────────────

#[derive(Debug, thiserror::Error)]
pub enum ProjectTaskError {
    #[error("project_frozen")]
    ProjectFrozen,
    #[error("not_found")]
    NotFound,
    #[error("forbidden")]
    Forbidden,
    #[error("invalid_title")]
    InvalidTitle,
    #[error("invalid_description")]
    InvalidDescription,
    #[error("invalid_tags")]
    InvalidTags(String),
    #[error("invalid_ordinal")]
    InvalidOrdinal,
    #[error("ordinal_taken")]
    OrdinalTaken,
    #[error("invalid_task_ids")]
    InvalidTaskIds,
    #[error("invalid_template")]
    InvalidTemplate(String),
    #[error("data corruption")]
    DataCorruption,
    #[error("database error: {0}")]
    Db(sea_orm::DbErr),
}

impl From<sea_orm::DbErr> for ProjectTaskError {
    fn from(e: sea_orm::DbErr) -> Self {
        ProjectTaskError::Db(e)
    }
}

crate::api::error::impl_api_error!(ProjectTaskError {
    Self::ProjectFrozen => (CONFLICT, "project_frozen"),
    Self::NotFound => (NOT_FOUND, "not_found"),
    Self::Forbidden => (FORBIDDEN, "forbidden"),
    Self::InvalidTitle => (UNPROCESSABLE_ENTITY, "invalid_title"),
    Self::InvalidDescription => (UNPROCESSABLE_ENTITY, "invalid_description"),
    Self::InvalidTags(msg) => (UNPROCESSABLE_ENTITY, "invalid_tags", "detail": msg),
    Self::InvalidOrdinal => (UNPROCESSABLE_ENTITY, "invalid_ordinal"),
    Self::OrdinalTaken => (CONFLICT, "ordinal_taken"),
    Self::InvalidTaskIds => (UNPROCESSABLE_ENTITY, "invalid_task_ids"),
    Self::InvalidTemplate(detail) => (UNPROCESSABLE_ENTITY, "invalid_template", "detail": detail),
    Self::DataCorruption | Self::Db(_) => (INTERNAL_SERVER_ERROR, "database_error"),
});

// ─── Helpers ───────────────────────────────────────────────────────────────

pub(crate) fn parse_user_id(claims: &AccessClaims) -> Result<Uuid, ProjectTaskError> {
    claims.user_id().map_err(|_| ProjectTaskError::Forbidden)
}

pub(crate) fn validate_title(raw: &str) -> Result<String, ProjectTaskError> {
    let trimmed = raw.trim().to_string();
    let len = trimmed.chars().count();
    if !(TITLE_MIN..=TITLE_MAX).contains(&len) {
        return Err(ProjectTaskError::InvalidTitle);
    }
    Ok(trimmed)
}

pub(crate) fn validate_description(raw: &str) -> Result<String, ProjectTaskError> {
    if raw.chars().count() > DESCRIPTION_MAX {
        return Err(ProjectTaskError::InvalidDescription);
    }
    Ok(raw.to_string())
}

pub(crate) fn validate_ordinal(o: i32) -> Result<i32, ProjectTaskError> {
    if o < 0 {
        Err(ProjectTaskError::InvalidOrdinal)
    } else {
        Ok(o)
    }
}

pub(crate) fn template_to_json(t: &TestTemplate) -> Result<serde_json::Value, ProjectTaskError> {
    serde_json::to_value(t).map_err(|_| ProjectTaskError::DataCorruption)
}

pub(crate) fn template_from_json(v: serde_json::Value) -> Result<TestTemplate, ProjectTaskError> {
    serde_json::from_value(v).map_err(|_| ProjectTaskError::DataCorruption)
}

/// Deserialize tags JSON text stored in the DB. Malformed JSON falls back to
/// an empty vec and emits a warning (FR-009).
pub(crate) fn parse_tags(raw: &str, task_id: Uuid) -> Vec<String> {
    serde_json::from_str::<Vec<String>>(raw).unwrap_or_else(|_| {
        tracing::warn!(
            task_id = %task_id,
            raw = %raw,
            "malformed tags JSON in DB; falling back to []"
        );
        vec![]
    })
}

/// Validate tags via the shared module, mapping errors to [`ProjectTaskError`].
pub(crate) fn validate_tags(tags: &[String]) -> Result<(), ProjectTaskError> {
    crate::validation::tags::validate_tags(tags)
        .map_err(|e| ProjectTaskError::InvalidTags(e.to_string()))
}

pub(crate) fn proj_intervals(p: &projects::Model) -> ExportIntervals {
    ExportIntervals {
        deadline_secs: p.default_deadline_secs,
        min_interval_secs: p.default_min_interval_secs,
        interval_increment_secs: p.default_interval_increment_secs,
        max_interval_secs: p.default_max_interval_secs,
    }
}

pub(crate) fn to_summary(
    m: tasks::Model,
    proj: &ExportIntervals,
) -> Result<TaskSummary, ProjectTaskError> {
    let tpl = template_from_json(m.test_template)?;
    let tags = parse_tags(&m.tags, m.id);
    let intervals = resolve_intervals(
        m.deadline_secs,
        m.min_interval_secs,
        m.interval_increment_secs,
        m.max_interval_secs,
        proj,
    );
    Ok(TaskSummary {
        id: m.id,
        project_id: m.project_id_fk,
        ordinal: m.ordinal,
        title: m.title,
        description: m.content,
        test_template: tpl,
        tags,
        created_at: m.created_at,
        points: PointsResp {
            value: m.point_value,
            fail: m.fail_points,
            no_response: m.no_response_points,
            completion_bonus: m.completion_bonus_points,
        },
        intervals,
    })
}

/// Returns `ProjectFrozen` if any session references this project.
/// Write operations (create/patch/delete/reorder) must call this after
/// confirming ownership.
pub(crate) async fn freeze_guard(
    db: &DatabaseConnection,
    project_id: Uuid,
) -> Result<(), ProjectTaskError> {
    let session_exists = sessions::Entity::find()
        .filter(sessions::Column::ProjectIdFk.eq(project_id))
        .one(db)
        .await?
        .is_some();
    if session_exists {
        return Err(ProjectTaskError::ProjectFrozen);
    }
    Ok(())
}

/// Load project requiring caller is the owner. Admins bypass the ownership
/// check. Non-owners (non-admins) → 403.
pub(crate) async fn load_for_owner(
    db: &DatabaseConnection,
    project_id: Uuid,
    user_id: Uuid,
) -> Result<projects::Model, ProjectTaskError> {
    let row = projects::Entity::find_by_id(project_id)
        .one(db)
        .await?
        .ok_or(ProjectTaskError::NotFound)?;
    let is_admin = crate::auth::is_user_admin(db, user_id).await?;
    if !is_admin && row.owner_user_id_fk != user_id {
        return Err(ProjectTaskError::Forbidden);
    }
    Ok(row)
}

/// Load project with full visibility (owner | public | session member | admin).
/// Project exists but no access → 403.
pub(crate) async fn load_accessible(
    db: &DatabaseConnection,
    project_id: Uuid,
    user_id: Uuid,
) -> Result<projects::Model, ProjectTaskError> {
    let row = projects::Entity::find_by_id(project_id)
        .one(db)
        .await?
        .ok_or(ProjectTaskError::NotFound)?;

    if row.owner_user_id_fk == user_id || row.public {
        return Ok(row);
    }

    let is_admin = crate::auth::is_user_admin(db, user_id).await?;
    if is_admin {
        return Ok(row);
    }

    let owns_session = sessions::Entity::find()
        .filter(sessions::Column::ProjectIdFk.eq(project_id))
        .filter(sessions::Column::OwnerIdFk.eq(user_id))
        .one(db)
        .await?;
    if owns_session.is_some() {
        return Ok(row);
    }

    let project_session_ids: Vec<Uuid> = sessions::Entity::find()
        .filter(sessions::Column::ProjectIdFk.eq(project_id))
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

    Err(ProjectTaskError::Forbidden)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_ordinal_rejects_negative() {
        assert!(validate_ordinal(-1).is_err());
        assert_eq!(validate_ordinal(0).unwrap(), 0);
        assert_eq!(validate_ordinal(42).unwrap(), 42);
    }

    #[test]
    fn validate_title_enforces_bounds() {
        assert!(validate_title("").is_err());
        assert!(validate_title("   ").is_err());
        let long = "x".repeat(TITLE_MAX + 1);
        assert!(validate_title(&long).is_err());
        assert_eq!(validate_title("  hi  ").unwrap(), "hi");
    }

    #[test]
    fn validate_description_caps_length() {
        let big = "y".repeat(DESCRIPTION_MAX + 1);
        assert!(validate_description(&big).is_err());
        assert_eq!(validate_description("ok").unwrap(), "ok");
    }
}
