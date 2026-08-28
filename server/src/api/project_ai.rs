//! `/api/projects/:id/beautify-description` and `/api/projects/:id/generate-tasks` handlers.
//!
//! FR-001 (beautify-description endpoint), FR-011 (generate-tasks endpoint).

use axum::Json;
use axum::extract::{Path, State};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Request / Response types
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BeautifyRequest {
    pub description: String,
}

#[derive(Debug, Serialize)]
pub struct BeautifyResponse {
    pub description: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GenerateTasksRequest {
    pub description: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct TaskDraft {
    pub title: String,
    pub description: String,
    #[serde(default)]
    pub command_template: String,
    #[serde(default)]
    pub tags: Option<Vec<String>>,
}

#[derive(Debug, Serialize)]
pub struct GenerateTasksResponse {
    pub tasks: Vec<TaskDraft>,
}

// ---------------------------------------------------------------------------
// Error enum
// ---------------------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
pub enum ProjectAiError {
    #[error("not_found")]
    NotFound,
    #[error("forbidden")]
    Forbidden,
    #[error("no_model_configured")]
    NoModelConfigured,
    #[error("description_empty")]
    DescriptionEmpty,
    #[error("description_too_long")]
    DescriptionTooLong,
    #[error("too_many_tasks")]
    TooManyTasks,
    #[error("ai_timeout")]
    AiTimeout,
    #[error("ai_error")]
    AiError,
    #[error("ai_parse_error")]
    AiParseError,
    #[error("database error: {0}")]
    Db(#[from] sea_orm::DbErr),
}

impl From<crate::llm::LlmError> for ProjectAiError {
    fn from(e: crate::llm::LlmError) -> Self {
        match e {
            crate::llm::LlmError::Timeout => ProjectAiError::AiTimeout,
            crate::llm::LlmError::AiError(_) => ProjectAiError::AiError,
        }
    }
}

crate::api::error::impl_api_error!(ProjectAiError {
    Self::NotFound => (NOT_FOUND, "not_found"),
    Self::Forbidden => (FORBIDDEN, "forbidden"),
    Self::NoModelConfigured => (UNPROCESSABLE_ENTITY, "no_model_configured"),
    Self::DescriptionEmpty => (BAD_REQUEST, "description_empty"),
    Self::DescriptionTooLong => (BAD_REQUEST, "description_too_long"),
    Self::TooManyTasks => (BAD_REQUEST, "too_many_tasks"),
    Self::AiTimeout => (GATEWAY_TIMEOUT, "ai_timeout"),
    Self::AiError => (BAD_GATEWAY, "ai_error"),
    Self::AiParseError => (BAD_GATEWAY, "ai_parse_error"),
    Self::Db(_) => (INTERNAL_SERVER_ERROR, "database_error"),
});

// ---------------------------------------------------------------------------
// Input validation helper (FR-005 / FR-015)
// ---------------------------------------------------------------------------

/// Validate a description string per FR-005 / FR-015.
/// Returns the trimmed string or an error.
pub fn validate_description(raw: &str) -> Result<String, ProjectAiError> {
    let trimmed = raw.trim().to_string();
    if trimmed.is_empty() {
        return Err(ProjectAiError::DescriptionEmpty);
    }
    if trimmed.chars().count() > 10_000 {
        return Err(ProjectAiError::DescriptionTooLong);
    }
    Ok(trimmed)
}

// ---------------------------------------------------------------------------
// Auth helper
// ---------------------------------------------------------------------------

fn parse_user_id(claims: &crate::auth::jwt::AccessClaims) -> Result<Uuid, ProjectAiError> {
    claims.user_id().map_err(|_| ProjectAiError::Forbidden)
}

// ---------------------------------------------------------------------------
// DB helper: load project requiring ownership (FR-003 / FR-013)
// ---------------------------------------------------------------------------

async fn load_project_for_owner(
    db: &sea_orm::DatabaseConnection,
    project_id: Uuid,
    user_id: Uuid,
) -> Result<(), ProjectAiError> {
    use arena_core::entities::projects;
    use sea_orm::EntityTrait;
    let row = projects::Entity::find_by_id(project_id)
        .one(db)
        .await?
        .ok_or(ProjectAiError::NotFound)?;
    if row.owner_user_id_fk != user_id {
        return Err(ProjectAiError::Forbidden);
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Model resolution helper (FR-004 / FR-014)
// ---------------------------------------------------------------------------

/// Candidates for this operation, in failover order (one entry for a
/// single-model assignment, several when a pool is assigned).
async fn resolve_candidates(
    state: &crate::state::AppState,
) -> Result<Vec<arena_core::llm::ModelConfig>, ProjectAiError> {
    // Admin-configured assignment chain (`llm_op_project_ai` → `llm_default`).
    // The admin picked the model from the provider's own list, so no live
    // membership check is needed; nothing configured → 422.
    let candidates = crate::llm::resolve_candidates_for_operation(
        &state.db,
        &state.settings_encryption,
        "project_ai",
    )
    .await;
    if candidates.is_empty() {
        return Err(ProjectAiError::NoModelConfigured);
    }
    Ok(candidates)
}

// ---------------------------------------------------------------------------
// System prompts (NFR-004: hardcoded; never interpolated with user content)
// ---------------------------------------------------------------------------

const BEAUTIFY_SYSTEM: &str = "\
You are a technical writing assistant for software projects. \
Rewrite the following project description to be clear, concise, and professional \
as a developer-oriented specification. \
Return only the rewritten description — no explanation, no preamble, no markdown code fences.";

const GENERATE_TASKS_SYSTEM: &str = "\
You are a software project planning assistant. \
Generate a concise task list from the provided project description.\n\
Rules:\n\
- The FIRST task MUST be an environment-setup task (e.g. \"Set up development environment\").\n\
- The LAST task MUST be a delivery task (e.g. \"Final review and release\").\n\
- Include focused implementation tasks between first and last.\n\
- Each task has exactly three fields:\n\
  - \"title\": 5-10 words, imperative sentence\n\
  - \"description\": ONE sentence (max 20 words) stating what to implement\n\
  - \"tags\": array of 1-5 short label strings categorizing the task (e.g. \"backend\", \"testing\", \"frontend\")\n\
- Do NOT include command_template, code blocks, or markdown in the fields.\n\
- Do NOT duplicate tasks listed under existing tasks.\n\
Return ONLY a valid JSON array — \
no explanation, no preamble, no markdown code fence wrapping the array.\n\
Example: [{\"title\":\"...\",\"description\":\"...\",\"tags\":[\"setup\"]}]";

// ---------------------------------------------------------------------------
// JSON parsing helper for generate-tasks (FR-019)
// ---------------------------------------------------------------------------

fn parse_tasks(raw: &str) -> Result<Vec<TaskDraft>, ProjectAiError> {
    let s = raw.trim();
    // Strip optional markdown code fence (```json ... ``` or ``` ... ```)
    let json_str = if let Some(rest) = s.strip_prefix("```json") {
        rest.trim_start_matches('\n').trim_end_matches("```").trim()
    } else if let Some(rest) = s.strip_prefix("```") {
        rest.trim_start_matches('\n').trim_end_matches("```").trim()
    } else {
        s
    };
    serde_json::from_str::<Vec<TaskDraft>>(json_str).map_err(|_| ProjectAiError::AiParseError)
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// `POST /api/projects/:project_id/beautify-description`
///
/// FR-001..FR-010.
pub async fn beautify_description(
    State(state): State<crate::state::AppState>,
    claims: crate::auth::jwt::AccessClaims,
    Path(project_id): Path<Uuid>,
    Json(req): Json<BeautifyRequest>,
) -> Result<Json<BeautifyResponse>, ProjectAiError> {
    let user_id = parse_user_id(&claims)?;
    load_project_for_owner(&state.db, project_id, user_id).await?;

    let description = validate_description(&req.description)?;
    let candidates = resolve_candidates(&state).await?;

    let result = crate::llm::complete_with_failover(
        &state,
        &candidates,
        "project_ai",
        arena_core::llm::telemetry::LlmContext::default(),
        BEAUTIFY_SYSTEM,
        &description,
        Duration::from_secs(60),
    )
    .await?;

    Ok(Json(BeautifyResponse {
        description: result.trim().to_string(),
    }))
}

/// `POST /api/projects/:project_id/generate-tasks`
///
/// FR-011..FR-023.
pub async fn generate_tasks(
    State(state): State<crate::state::AppState>,
    claims: crate::auth::jwt::AccessClaims,
    Path(project_id): Path<Uuid>,
    Json(req): Json<GenerateTasksRequest>,
) -> Result<Json<GenerateTasksResponse>, ProjectAiError> {
    use arena_core::entities::tasks;
    use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};

    let user_id = parse_user_id(&claims)?;
    load_project_for_owner(&state.db, project_id, user_id).await?;

    let description = validate_description(&req.description)?;
    let candidates = resolve_candidates(&state).await?;

    // FR-016: Query existing tasks for context to avoid duplication.
    let existing = tasks::Entity::find()
        .filter(tasks::Column::ProjectIdFk.eq(project_id))
        .all(&state.db)
        .await?;

    // FR-017: User role carries description + existing task titles only.
    // Only titles are needed for deduplication; sending full descriptions
    // inflates the prompt token count and increases inference time.
    // No user-provided content is interpolated into the system instruction.
    let existing_titles: Vec<&str> = existing.iter().map(|t| t.title.as_str()).collect();
    let titles_list = if existing_titles.is_empty() {
        "none".to_string()
    } else {
        existing_titles
            .iter()
            .map(|t| format!("- {t}"))
            .collect::<Vec<_>>()
            .join("\n")
    };

    let user_content = format!(
        "Project description:\n{description}\n\n\
         Existing tasks (do not duplicate):\n{titles_list}\n\n\
         Generate a complete task list for this project."
    );

    let raw = crate::llm::complete_with_failover(
        &state,
        &candidates,
        "project_ai",
        arena_core::llm::telemetry::LlmContext::default(),
        GENERATE_TASKS_SYSTEM,
        &user_content,
        Duration::from_secs(180),
    )
    .await?;

    // FR-019: Parse the AI response into Vec<TaskDraft>.
    let mut tasks_vec = parse_tasks(&raw)?;

    // FR-020: Cap at 20 tasks.
    if tasks_vec.len() > 20 {
        return Err(ProjectAiError::TooManyTasks);
    }

    // FR-012: Validate AI-returned tags per shared rules; silently fall back
    // to [] on failure (advisory guidance only — no error returned to client).
    for draft in &mut tasks_vec {
        if let Some(tags) = &draft.tags
            && crate::validation::tags::validate_tags(tags).is_err()
        {
            tracing::warn!(
                title = %draft.title,
                "AI-generated tags failed validation; falling back to []"
            );
            draft.tags = Some(vec![]);
        }
    }

    Ok(Json(GenerateTasksResponse { tasks: tasks_vec }))
}
