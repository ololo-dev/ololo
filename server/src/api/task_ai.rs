//! Task-scoped AI handlers.
//!
//! `POST /api/projects/:project_id/beautify-task-description`
//! `POST /api/projects/:project_id/generate-task-tests`
//! `POST /api/projects/:project_id/tasks/:task_id/beautify-description`
//! `POST /api/projects/:project_id/tasks/:task_id/generate-tests`

use axum::Json;
use axum::extract::{Path, State};
use std::sync::OnceLock;
use std::time::Duration;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// System prompts (NFR-004: hardcoded; never interpolated with user content)
// ---------------------------------------------------------------------------

const BEAUTIFY_TASK_DESCRIPTION_SYSTEM: &str = "You are a technical writing assistant. Rewrite the given task description to be clear, concise, and precise for an AI coding agent. Preserve all technical details. Return only the improved description — no preamble, no explanation, no markdown fences.";

/// Returns the generate-task-tests system prompt (lazy singleton).
fn generate_task_tests_system() -> &'static str {
    static PROMPT: OnceLock<String> = OnceLock::new();
    PROMPT.get_or_init(|| {
        "You are a senior QA engineer. Given a task title and description, generate a markdown \
test template for project tasks.\n\
\n\
Return ONLY markdown content, with one or more tests, and each test MUST follow this exact shape:\n\
\n\
## <Test name>\n\
\n\
<Short description>\n\
\n\
```js fixtures\n\
<JavaScript expression or function body that returns an object, e.g. ({ a: 1, b: 2, c: \"x\" })>\n\
```\n\
\n\
```sh command\n\
<command template using placeholders from fixtures, e.g. echo {a}+{b}>\n\
```\n\
\n\
```js validation\n\
<JavaScript boolean expression using fixture vars and result, e.g. Number(a)+Number(b) === Number(result)>\n\
```\n\
\n\
Rules:\n\
1. VERIFICATION ONLY — never include setup, implementation, or code generation.\n\
2. Use relative paths for local files (e.g. ./README.md, ./src/main.rs).\n\
3. Use only standard tools: curl, grep, test, cat, ls, tail, head, jq.\n\
4. Code block info strings must be exactly: `js fixtures`, `sh command`, `js validation`.\n\
5. Fixtures block must return an object with variables (a, b, c...).\n\
6. Command must reference fixture variables via `{var}` placeholders.\n\
7. Validation must evaluate to boolean and reference `result` plus fixture vars.\n\
8. Include multiple tests when the task has multiple acceptance criteria.\n\
9. No outer fences, no preamble, no explanations."
            .to_string()
    })
}

// ---------------------------------------------------------------------------
// Request / Response types
// ---------------------------------------------------------------------------

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BeautifyTaskDescriptionRequest {
    pub description: String,
}

#[derive(Debug, serde::Serialize)]
pub struct BeautifyTaskDescriptionResponse {
    pub description: String,
}

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GenerateTaskTestsRequest {
    pub title: String,
    pub description: String,
}

#[derive(Debug, serde::Serialize)]
pub struct GenerateTaskTestsResponse {
    pub command_template: String,
    pub answer_template: String,
    pub fixtures: Vec<serde_json::Value>,
}

// ---------------------------------------------------------------------------
// Error enum
// ---------------------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
pub enum TaskAiError {
    #[error("not_found")]
    ProjectNotFound,
    #[error("not_found")]
    TaskNotFound,
    #[error("forbidden")]
    Unauthorized,
    #[error("no_model_configured")]
    NoModelConfigured,
    #[error("invalid_model")]
    InvalidModel,
    #[error("ai_timeout")]
    AiTimeout,
    #[error("ai_error")]
    AiError,
    #[error("ai_empty_response")]
    AiEmptyResponse,
    #[error("ai_parse_error")]
    AiParseError,
    #[error("validation_error: {0}")]
    ValidationError(String),
    #[error("database error: {0}")]
    DbError(#[from] sea_orm::DbErr),
}

impl From<crate::llm::LlmError> for TaskAiError {
    fn from(e: crate::llm::LlmError) -> Self {
        match e {
            crate::llm::LlmError::Timeout => TaskAiError::AiTimeout,
            crate::llm::LlmError::AiError(_) => TaskAiError::AiError,
        }
    }
}

crate::api::error::impl_api_error!(TaskAiError {
    Self::ProjectNotFound | Self::TaskNotFound => (NOT_FOUND, "not_found"),
    Self::Unauthorized => (FORBIDDEN, "forbidden"),
    Self::NoModelConfigured => (UNPROCESSABLE_ENTITY, "no_model_configured"),
    Self::InvalidModel => (UNPROCESSABLE_ENTITY, "invalid_model"),
    Self::AiTimeout => (GATEWAY_TIMEOUT, "ai_timeout"),
    Self::AiError => (BAD_GATEWAY, "ai_error"),
    Self::AiEmptyResponse => (UNPROCESSABLE_ENTITY, "ai_empty_response"),
    Self::AiParseError => (UNPROCESSABLE_ENTITY, "ai_parse_error"),
    Self::ValidationError(_) => (BAD_REQUEST, "validation_error"),
    Self::DbError(_) => (INTERNAL_SERVER_ERROR, "database_error"),
});

// ---------------------------------------------------------------------------
// Input validation helpers
// ---------------------------------------------------------------------------

fn validate_description(raw: &str) -> Result<String, TaskAiError> {
    let trimmed = raw.trim().to_string();
    if trimmed.is_empty() {
        return Err(TaskAiError::ValidationError(
            "description_empty".to_string(),
        ));
    }
    if trimmed.chars().count() > 10_000 {
        return Err(TaskAiError::ValidationError(
            "description_too_long".to_string(),
        ));
    }
    Ok(trimmed)
}

fn validate_title(raw: &str) -> Result<String, TaskAiError> {
    let trimmed = raw.trim().to_string();
    if trimmed.is_empty() {
        return Err(TaskAiError::ValidationError("title_empty".to_string()));
    }
    if trimmed.chars().count() > 255 {
        return Err(TaskAiError::ValidationError("title_too_long".to_string()));
    }
    Ok(trimmed)
}

// ---------------------------------------------------------------------------
// Auth helper
// ---------------------------------------------------------------------------

fn parse_user_id(claims: &crate::auth::jwt::AccessClaims) -> Result<Uuid, TaskAiError> {
    claims.user_id().map_err(|_| TaskAiError::Unauthorized)
}

// ---------------------------------------------------------------------------
// DB helper: load project requiring ownership
// ---------------------------------------------------------------------------

async fn load_project_for_owner(
    db: &sea_orm::DatabaseConnection,
    project_id: Uuid,
    user_id: Uuid,
) -> Result<(), TaskAiError> {
    use arena_core::entities::projects;
    use sea_orm::EntityTrait;
    let row = projects::Entity::find_by_id(project_id)
        .one(db)
        .await?
        .ok_or(TaskAiError::ProjectNotFound)?;
    if row.owner_user_id_fk != user_id {
        return Err(TaskAiError::Unauthorized);
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// DB helper: load task requiring project membership (AC-007)
// ---------------------------------------------------------------------------

/// Single-query auth: verifies both task existence and project ownership in one
/// DB round-trip. Returns 404 for wrong project/task combination to prevent
/// task enumeration (AC-007).
async fn load_task_for_project(
    db: &sea_orm::DatabaseConnection,
    project_id: Uuid,
    task_id: Uuid,
    user_id: Uuid,
) -> Result<(), TaskAiError> {
    use arena_core::entities::{projects, tasks};
    use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};

    // Verify project ownership first
    let project = projects::Entity::find_by_id(project_id)
        .one(db)
        .await?
        .ok_or(TaskAiError::TaskNotFound)?;
    if project.owner_user_id_fk != user_id {
        // Return 404 (not 403) — prevents task enumeration (AC-007)
        return Err(TaskAiError::TaskNotFound);
    }

    // Single query: task must belong to this project
    tasks::Entity::find()
        .filter(tasks::Column::Id.eq(task_id))
        .filter(tasks::Column::ProjectIdFk.eq(project_id))
        .one(db)
        .await?
        .ok_or(TaskAiError::TaskNotFound)?;

    Ok(())
}

// ---------------------------------------------------------------------------
// Model resolution helper
// ---------------------------------------------------------------------------

/// Candidates for this operation, in failover order (one entry for a
/// single-model assignment, several when a pool is assigned).
async fn resolve_candidates(
    state: &crate::state::AppState,
) -> Result<Vec<arena_core::llm::ModelConfig>, TaskAiError> {
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
        return Err(TaskAiError::NoModelConfigured);
    }
    Ok(candidates)
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// Validate, run the beautify prompt, and shape the response. Callers are
/// responsible for authorization (project-owner vs task-in-project).
async fn beautify_description(
    state: &crate::state::AppState,
    req: &BeautifyTaskDescriptionRequest,
) -> Result<Json<BeautifyTaskDescriptionResponse>, TaskAiError> {
    let description = validate_description(&req.description)?;
    let candidates = resolve_candidates(state).await?;

    let result = crate::llm::complete_with_failover(
        state,
        &candidates,
        "task_ai",
        arena_core::llm::telemetry::LlmContext::default(),
        BEAUTIFY_TASK_DESCRIPTION_SYSTEM,
        &description,
        Duration::from_secs(60),
    )
    .await?;

    let trimmed = result.trim().to_string();
    if trimmed.is_empty() {
        return Err(TaskAiError::AiEmptyResponse);
    }

    Ok(Json(BeautifyTaskDescriptionResponse {
        description: trimmed,
    }))
}

/// Validate, run the test-generation prompt, and shape the response. Callers
/// are responsible for authorization (project-owner vs task-in-project).
async fn generate_task_tests(
    state: &crate::state::AppState,
    req: &GenerateTaskTestsRequest,
) -> Result<Json<GenerateTaskTestsResponse>, TaskAiError> {
    let title = validate_title(&req.title)?;
    let description = validate_description(&req.description)?;
    let candidates = resolve_candidates(state).await?;

    let user_content = format!(
        "Task title: {}\n\nTask description:\n{}",
        title, description
    );

    let raw = crate::llm::complete_with_failover(
        state,
        &candidates,
        "task_ai",
        arena_core::llm::telemetry::LlmContext::default(),
        generate_task_tests_system(),
        &user_content,
        Duration::from_secs(60),
    )
    .await?;

    if raw.trim().is_empty() {
        return Err(TaskAiError::AiEmptyResponse);
    }
    let command_template = raw.trim().to_string();

    Ok(Json(GenerateTaskTestsResponse {
        command_template,
        answer_template: String::new(),
        fixtures: Vec::new(),
    }))
}

/// `POST /api/projects/:project_id/beautify-task-description`
pub async fn project_beautify_task_description(
    State(state): State<crate::state::AppState>,
    claims: crate::auth::jwt::AccessClaims,
    Path(project_id): Path<Uuid>,
    Json(req): Json<BeautifyTaskDescriptionRequest>,
) -> Result<Json<BeautifyTaskDescriptionResponse>, TaskAiError> {
    let user_id = parse_user_id(&claims)?;
    load_project_for_owner(&state.db, project_id, user_id).await?;
    beautify_description(&state, &req).await
}

/// `POST /api/projects/:project_id/generate-task-tests`
pub async fn project_generate_task_tests(
    State(state): State<crate::state::AppState>,
    claims: crate::auth::jwt::AccessClaims,
    Path(project_id): Path<Uuid>,
    Json(req): Json<GenerateTaskTestsRequest>,
) -> Result<Json<GenerateTaskTestsResponse>, TaskAiError> {
    let user_id = parse_user_id(&claims)?;
    load_project_for_owner(&state.db, project_id, user_id).await?;
    generate_task_tests(&state, &req).await
}

/// `POST /api/projects/:project_id/tasks/:task_id/beautify-description`
pub async fn task_beautify_description(
    State(state): State<crate::state::AppState>,
    claims: crate::auth::jwt::AccessClaims,
    Path((project_id, task_id)): Path<(Uuid, Uuid)>,
    Json(req): Json<BeautifyTaskDescriptionRequest>,
) -> Result<Json<BeautifyTaskDescriptionResponse>, TaskAiError> {
    let user_id = parse_user_id(&claims)?;
    load_task_for_project(&state.db, project_id, task_id, user_id).await?;
    beautify_description(&state, &req).await
}

/// `POST /api/projects/:project_id/tasks/:task_id/generate-tests`
pub async fn task_generate_tests(
    State(state): State<crate::state::AppState>,
    claims: crate::auth::jwt::AccessClaims,
    Path((project_id, task_id)): Path<(Uuid, Uuid)>,
    Json(req): Json<GenerateTaskTestsRequest>,
) -> Result<Json<GenerateTaskTestsResponse>, TaskAiError> {
    let user_id = parse_user_id(&claims)?;
    load_task_for_project(&state.db, project_id, task_id, user_id).await?;
    generate_task_tests(&state, &req).await
}
