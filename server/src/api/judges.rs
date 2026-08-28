//! Admin judges CRUD API.

use crate::api::settings::AdminUser;
use crate::state::AppState;
use arena_core::entities::{judges, task_judges};
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
pub struct CreateJudgeRequest {
    pub slug: String,
    pub name: String,
    pub description: String,
    pub prompt: String,
    pub rating_scale: serde_json::Value,
    /// `"llm"` (default) or `"execution"`.
    #[serde(default = "default_judge_kind")]
    pub kind: String,
    /// `"task"` (default) or `"session"`.
    #[serde(default = "default_judge_scope")]
    pub scope: String,
    /// Optional per-judge model override (provider row + model id). The two
    /// travel together — a model without a provider is rejected.
    #[serde(default)]
    pub llm_provider_id: Option<Uuid>,
    #[serde(default)]
    pub llm_model: Option<String>,
    /// Optional per-judge pool override. May be combined with the pair
    /// above; `llm_source_order` then decides which leads.
    #[serde(default)]
    pub llm_pool_id: Option<Uuid>,
    /// `"pool_first"` (default) or `"model_first"`.
    #[serde(default = "default_source_order")]
    pub llm_source_order: String,
    /// Optional avatar (https URL on the configured image host).
    #[serde(default)]
    pub avatar_url: Option<String>,
    /// Repo paths this judge's git tools must not open, e.g. `[".ololo/"]`.
    #[serde(default)]
    pub ignore_paths: Option<Vec<String>>,
}

/// Canonical JSON for the `judges.ignore_paths` column, or `None` when the
/// judge sees everything. Prefixes only: the tools match by prefix, so a glob
/// would quietly never match and the judge would keep paying for the tree it
/// meant to skip.
fn encode_ignore_paths(paths: Option<Vec<String>>) -> Result<Option<String>, JudgesError> {
    let Some(list) = paths else { return Ok(None) };
    let cleaned: Vec<String> = list
        .into_iter()
        .map(|p| p.trim().to_string())
        .filter(|p| !p.is_empty())
        .collect();
    if let Some(bad) = cleaned
        .iter()
        .find(|p| p.contains('*') || p.starts_with('/'))
    {
        return Err(JudgesError::BadRequest(format!(
            "ignore_paths: '{bad}' must be a repo-relative path prefix, not a glob or absolute path"
        )));
    }
    if cleaned.is_empty() {
        return Ok(None);
    }
    serde_json::to_string(&cleaned)
        .map(Some)
        .map_err(|e| JudgesError::BadRequest(format!("ignore_paths: {e}")))
}

fn default_source_order() -> String {
    arena_core::llm::resolve::SOURCE_ORDER_POOL_FIRST.to_string()
}

fn default_judge_kind() -> String {
    "llm".to_string()
}

fn default_judge_scope() -> String {
    arena_core::judging::JUDGE_SCOPE_TASK.to_string()
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpdateJudgeRequest {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub prompt: Option<String>,
    #[serde(default)]
    pub rating_scale: Option<serde_json::Value>,
    /// Per-judge model override. Absent = unchanged; explicit `null` clears.
    #[serde(default, deserialize_with = "deserialize_some")]
    pub llm_provider_id: Option<Option<Uuid>>,
    #[serde(default, deserialize_with = "deserialize_some")]
    pub llm_model: Option<Option<String>>,
    /// Per-judge pool override. Absent = unchanged; explicit `null` clears.
    #[serde(default, deserialize_with = "deserialize_some")]
    pub llm_pool_id: Option<Option<Uuid>>,
    /// Absent = unchanged. `"pool_first"` or `"model_first"`.
    #[serde(default)]
    pub llm_source_order: Option<String>,
    /// Evidence sections. Absent = unchanged; explicit `null` clears (the
    /// judge reverts to the whole snapshot).
    #[serde(default, deserialize_with = "deserialize_some")]
    pub needs: Option<Option<Vec<String>>>,
    /// Criteria keys. Absent = unchanged; explicit `null` clears.
    #[serde(default, deserialize_with = "deserialize_some")]
    pub criteria: Option<Option<Vec<String>>>,
    /// Interactive budget. Absent = unchanged; explicit `null` clears.
    #[serde(default, deserialize_with = "deserialize_some")]
    pub max_interactive: Option<Option<i32>>,
    /// Repo paths this judge's git tools must not open. Absent = unchanged;
    /// explicit `null` clears (the judge sees the whole snapshot again).
    #[serde(default, deserialize_with = "deserialize_some")]
    pub ignore_paths: Option<Option<Vec<String>>>,
    /// Avatar URL. Absent = unchanged; explicit `null` clears.
    #[serde(default, deserialize_with = "deserialize_some")]
    pub avatar_url: Option<Option<String>>,
}

fn deserialize_some<'de, T, D>(d: D) -> Result<Option<T>, D::Error>
where
    T: serde::Deserialize<'de>,
    D: serde::Deserializer<'de>,
{
    T::deserialize(d).map(Some)
}

/// Normalize a model id, treating whitespace-only as absent.
fn clean_model(model: Option<String>) -> Option<String> {
    model.filter(|m| !m.trim().is_empty())
}

/// The provider and the model are one override, so half of one is a
/// mistake, not a partial setting: a model alone used to be grafted onto
/// whichever provider the chain resolved, which made the effective model
/// depend on unrelated settings.
fn validate_model_pair(provider: Option<Uuid>, model: Option<&str>) -> Result<(), JudgesError> {
    match (provider, model) {
        (Some(_), None) => Err(JudgesError::BadRequest(
            "llm_model is required when llm_provider_id is set".to_string(),
        )),
        (None, Some(_)) => Err(JudgesError::BadRequest(
            "llm_provider_id is required when llm_model is set".to_string(),
        )),
        _ => Ok(()),
    }
}

fn validate_source_order(order: &str) -> Result<(), JudgesError> {
    use arena_core::llm::resolve::{SOURCE_ORDER_MODEL_FIRST, SOURCE_ORDER_POOL_FIRST};
    if order == SOURCE_ORDER_POOL_FIRST || order == SOURCE_ORDER_MODEL_FIRST {
        return Ok(());
    }
    Err(JudgesError::BadRequest(format!(
        "llm_source_order must be {SOURCE_ORDER_POOL_FIRST} or {SOURCE_ORDER_MODEL_FIRST}"
    )))
}

/// Reject a pool id that does not exist — an override pointing at nothing
/// silently falls through to the operation assignment, which looks like the
/// override was ignored.
async fn validate_pool(state: &AppState, pool_id: Option<Uuid>) -> Result<(), JudgesError> {
    let Some(id) = pool_id else { return Ok(()) };
    let exists = arena_core::entities::llm_pools::Entity::find_by_id(id)
        .one(&state.db)
        .await?
        .is_some();
    if exists {
        return Ok(());
    }
    Err(JudgesError::BadRequest("unknown llm_pool_id".to_string()))
}

#[derive(Debug, Serialize)]
pub struct JudgeResponse {
    pub id: Uuid,
    pub slug: String,
    pub name: String,
    pub description: String,
    pub prompt: String,
    pub rating_scale: serde_json::Value,
    pub kind: String,
    pub scope: String,
    /// `"tools"` (agentic loop) or `"dossier"` (single completion against a
    /// server-built evidence pack). Read-only: it comes from the judge's seed
    /// file, which is also where the pack's contents are described.
    pub evidence_mode: String,
    /// The evidence sections the judge declared it needs, or `null` when it
    /// never declared and gets the whole snapshot.
    pub evidence_needs: Option<Vec<String>>,
    /// Open-ended criteria keys this judge scores, or `null` for a
    /// single-score judge.
    pub criteria: Option<Vec<String>>,
    /// How many interactive probes this judge may register per task.
    pub max_interactive: Option<i32>,
    /// Repo paths this judge's git tools must not open, e.g. `[".ololo/"]`.
    #[serde(default)]
    pub ignore_paths: Option<Vec<String>>,
    pub llm_provider_id: Option<Uuid>,
    pub llm_model: Option<String>,
    pub llm_pool_id: Option<Uuid>,
    /// Which override half leads when both a pool and a model are set.
    pub llm_source_order: String,
    pub avatar_url: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl From<judges::Model> for JudgeResponse {
    fn from(m: judges::Model) -> Self {
        JudgeResponse {
            id: m.id,
            slug: m.slug,
            name: m.name,
            description: m.description,
            prompt: m.prompt,
            rating_scale: m.rating_scale,
            kind: m.kind,
            scope: m.scope,
            evidence_mode: m.evidence_mode,
            evidence_needs: m
                .evidence_needs
                .as_deref()
                .and_then(|raw| serde_json::from_str::<Vec<String>>(raw).ok()),
            criteria: m
                .criteria
                .as_deref()
                .and_then(|raw| serde_json::from_str::<Vec<String>>(raw).ok()),
            max_interactive: m.max_interactive,
            ignore_paths: m
                .ignore_paths
                .as_deref()
                .and_then(|raw| serde_json::from_str(raw).ok()),
            llm_provider_id: m.llm_provider_id_fk,
            llm_model: m.llm_model,
            llm_pool_id: m.llm_pool_id_fk,
            llm_source_order: m.llm_source_order,
            avatar_url: m.avatar_url,
            created_at: m.created_at,
            updated_at: m.updated_at,
        }
    }
}

/// Same rule as user avatars: https, and pinned to the configured image
/// host when ImageKit is set up — an arbitrary URL rendered on every
/// session page is a tracking pixel anyone with admin could plant.
fn validate_judge_avatar_url(url_str: &str, state: &AppState) -> Result<(), JudgesError> {
    let parsed = url::Url::parse(url_str)
        .map_err(|_| JudgesError::BadRequest("avatar_url is not a valid URL".to_string()))?;
    if parsed.scheme() != "https" {
        return Err(JudgesError::BadRequest(
            "avatar_url must be https".to_string(),
        ));
    }
    if let Some(ik) = &state.imagekit {
        let endpoint = url::Url::parse(&ik.url_endpoint)
            .map_err(|_| JudgesError::BadRequest("image host misconfigured".to_string()))?;
        if parsed.host_str() != endpoint.host_str() {
            return Err(JudgesError::BadRequest(
                "avatar_url must point at the configured image host".to_string(),
            ));
        }
    }
    Ok(())
}

#[derive(thiserror::Error, Debug)]
pub enum JudgesError {
    #[error("not found")]
    NotFound,
    #[error("conflict: {0}")]
    Conflict(String),
    #[error("bad request: {0}")]
    BadRequest(String),
    #[error(transparent)]
    Db(#[from] sea_orm::DbErr),
}

crate::api::error::impl_api_error!(JudgesError {
    Self::NotFound => (NOT_FOUND, "not_found"),
    Self::Conflict(_) => (CONFLICT, "conflict"),
    Self::BadRequest(_) => (BAD_REQUEST, "bad_request"),
    Self::Db(_) => (INTERNAL_SERVER_ERROR, "database_error"),
});

fn validate_slug(slug: &str) -> Result<String, JudgesError> {
    let len = slug.len();
    if len == 0 || len > 64 {
        return Err(JudgesError::BadRequest("invalid slug".into()));
    }
    for c in slug.chars() {
        if !(c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-') {
            return Err(JudgesError::BadRequest("invalid slug".into()));
        }
    }
    if slug.contains("--") {
        return Err(JudgesError::BadRequest("invalid slug".into()));
    }
    let first = slug.chars().next().unwrap();
    let last = slug.chars().last().unwrap();
    if !(first.is_ascii_lowercase() || first.is_ascii_digit())
        || !(last.is_ascii_lowercase() || last.is_ascii_digit())
    {
        return Err(JudgesError::BadRequest("invalid slug".into()));
    }
    Ok(slug.to_string())
}

pub async fn list(
    _admin: AdminUser,
    State(state): State<AppState>,
) -> Result<Json<Vec<JudgeResponse>>, JudgesError> {
    let rows = judges::Entity::find()
        .order_by_asc(judges::Column::Slug)
        .all(&state.db)
        .await?;
    Ok(Json(rows.into_iter().map(JudgeResponse::from).collect()))
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SyncJudgesResponse {
    pub inserted: usize,
    pub updated: usize,
    pub skipped: usize,
}

/// `POST /api/admin/judges/sync` — re-run the boot judge seed against the
/// server's on-disk `judges/*.md` definitions (`ARENA_JUDGES_DIR`, default
/// `./judges`): new files insert, changed files refresh the DB row, unchanged
/// and unparsable files are skipped. The markdown files are the source of
/// truth, same as on restart — admin-UI edits to a seeded judge are overwritten
/// when its file differs.
pub async fn sync(
    _admin: AdminUser,
    State(state): State<AppState>,
) -> Result<Json<SyncJudgesResponse>, JudgesError> {
    let dir = std::env::var("ARENA_JUDGES_DIR").unwrap_or_else(|_| "./judges".to_string());
    let (inserted, updated, skipped) =
        crate::seed::judges::seed_judges_with_dir(&state.db, std::path::Path::new(&dir))
            .await
            .map_err(|e| JudgesError::BadRequest(format!("judge sync failed: {e}")))?;
    tracing::info!(inserted, updated, skipped, dir = %dir, "admin judge sync from disk");
    Ok(Json(SyncJudgesResponse {
        inserted,
        updated,
        skipped,
    }))
}

pub async fn get_one(
    _admin: AdminUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<JudgeResponse>, JudgesError> {
    let row = judges::Entity::find_by_id(id)
        .one(&state.db)
        .await?
        .ok_or(JudgesError::NotFound)?;
    Ok(Json(JudgeResponse::from(row)))
}

/// Reject a judge whose own program does not parse. The prompt is authored
/// content either way, but a broken program fails every run of that judge
/// rather than reading badly to a model.
fn validate_decide_program(prompt: &str) -> Result<(), JudgesError> {
    let programs = arena_core::judging::programs::split_programs(prompt).1;
    for (fence, program) in [("decide", &programs.decide), ("review", &programs.review)] {
        if let Some(program) = program {
            arena_core::judging::programs::validate_program(program).map_err(|e| {
                JudgesError::BadRequest(format!("`js {fence}` program does not parse: {e}"))
            })?;
        }
    }
    Ok(())
}

pub async fn create(
    _admin: AdminUser,
    State(state): State<AppState>,
    Json(body): Json<CreateJudgeRequest>,
) -> Result<(StatusCode, Json<JudgeResponse>), JudgesError> {
    let slug = validate_slug(&body.slug)?;
    validate_rating_scale(&body.rating_scale).map_err(JudgesError::BadRequest)?;
    validate_decide_program(&body.prompt)?;
    if body.kind != arena_core::judging::JUDGE_KIND_LLM
        && body.kind != arena_core::judging::JUDGE_KIND_EXECUTION
        && body.kind != arena_core::judging::JUDGE_KIND_REPORT
    {
        return Err(JudgesError::BadRequest(format!(
            "unknown judge kind '{}'",
            body.kind
        )));
    }
    if body.scope != arena_core::judging::JUDGE_SCOPE_TASK
        && body.scope != arena_core::judging::JUDGE_SCOPE_SESSION
    {
        return Err(JudgesError::BadRequest(format!(
            "unknown judge scope '{}'",
            body.scope
        )));
    }
    // An execution judge re-runs one task's probes against that task's commit;
    // there is no session-wide equivalent.
    if body.scope == arena_core::judging::JUDGE_SCOPE_SESSION && body.kind == "execution" {
        return Err(JudgesError::BadRequest(
            "session-scoped execution judges are not supported".into(),
        ));
    }

    let existing = judges::Entity::find()
        .filter(judges::Column::Slug.eq(slug.clone()))
        .one(&state.db)
        .await?;
    if existing.is_some() {
        return Err(JudgesError::Conflict("slug already exists".into()));
    }

    let model = clean_model(body.llm_model);
    validate_model_pair(body.llm_provider_id, model.as_deref())?;
    validate_source_order(&body.llm_source_order)?;
    validate_pool(&state, body.llm_pool_id).await?;

    let now = Utc::now();
    let avatar = match body.avatar_url.as_deref().map(str::trim) {
        Some(url) if !url.is_empty() => {
            validate_judge_avatar_url(url, &state)?;
            Some(url.to_string())
        }
        _ => None,
    };

    let id = Uuid::new_v4();
    let am = judges::ActiveModel {
        id: Set(id),
        slug: Set(slug),
        name: Set(body.name),
        description: Set(body.description),
        prompt: Set(body.prompt),
        rating_scale: Set(body.rating_scale),
        kind: Set(body.kind),
        scope: Set(body.scope),
        // Admin-created judges investigate with tools; the dossier mode is
        // wired from a seed file, which is where the evidence pack it needs
        // is described.
        evidence_mode: Set(arena_core::judging::EVIDENCE_MODE_TOOLS.to_string()),
        // Likewise: what a judge needs to see is declared in its seed file,
        // beside the programs that read it. Undeclared here means the whole
        // snapshot.
        evidence_needs: Set(None),
        llm_provider_id_fk: Set(body.llm_provider_id),
        llm_model: Set(model),
        llm_pool_id_fk: Set(body.llm_pool_id),
        llm_source_order: Set(body.llm_source_order),
        criteria: Set(None),
        probes_config: Set(None),
        max_interactive: Set(None),
        avatar_url: Set(avatar),
        ignore_paths: Set(encode_ignore_paths(body.ignore_paths.clone())?),
        created_at: Set(now),
        updated_at: Set(now),
    };
    let model = am.insert(&state.db).await?;
    Ok((StatusCode::CREATED, Json(JudgeResponse::from(model))))
}

pub async fn update(
    _admin: AdminUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(body): Json<UpdateJudgeRequest>,
) -> Result<Json<JudgeResponse>, JudgesError> {
    let row = judges::Entity::find_by_id(id)
        .one(&state.db)
        .await?
        .ok_or(JudgesError::NotFound)?;

    if let Some(ref scale) = body.rating_scale {
        validate_rating_scale(scale).map_err(JudgesError::BadRequest)?;
    }

    // Validate the state the patch *lands on*, not the patch alone: clearing
    // just the provider on a judge that still has a model would otherwise
    // slip through and leave an unusable half-override.
    let eff_provider = body.llm_provider_id.unwrap_or(row.llm_provider_id_fk);
    let eff_model = match body.llm_model.clone() {
        Some(m) => clean_model(m),
        None => row.llm_model.clone(),
    };
    validate_model_pair(eff_provider, eff_model.as_deref())?;
    if let Some(order) = &body.llm_source_order {
        validate_source_order(order)?;
    }
    if let Some(pool) = body.llm_pool_id {
        validate_pool(&state, pool).await?;
    }

    let mut am: judges::ActiveModel = row.into();
    if let Some(name) = body.name {
        am.name = Set(name);
    }
    if let Some(description) = body.description {
        am.description = Set(description);
    }
    if let Some(prompt) = body.prompt {
        validate_decide_program(&prompt)?;
        am.prompt = Set(prompt);
    }
    if let Some(scale) = body.rating_scale {
        am.rating_scale = Set(scale);
    }
    if let Some(pid) = body.llm_provider_id {
        am.llm_provider_id_fk = Set(pid);
    }
    if let Some(model) = body.llm_model {
        am.llm_model = Set(clean_model(model));
    }
    if let Some(pool) = body.llm_pool_id {
        am.llm_pool_id_fk = Set(pool);
    }
    if let Some(order) = body.llm_source_order {
        am.llm_source_order = Set(order);
    }
    if let Some(needs) = body.needs {
        am.evidence_needs = Set(match needs {
            None => None,
            Some(sections) => Some(
                arena_core::judging::evidence::EvidenceNeeds::parse(&sections)
                    .map_err(|e| JudgesError::BadRequest(format!("needs: {e}")))?
                    .to_column(),
            ),
        });
    }
    if let Some(criteria) = body.criteria {
        am.criteria = Set(match criteria {
            None => None,
            Some(keys) => {
                if keys.iter().any(|k| k.trim().is_empty()) {
                    return Err(JudgesError::BadRequest(
                        "criteria keys must be non-empty".to_string(),
                    ));
                }
                Some(serde_json::to_string(&keys).unwrap_or_else(|_| "[]".to_string()))
            }
        });
    }
    if let Some(ignore_paths) = body.ignore_paths.clone() {
        am.ignore_paths = Set(encode_ignore_paths(ignore_paths)?);
    }
    if let Some(max_interactive) = body.max_interactive {
        if max_interactive.is_some_and(|n| n < 0) {
            return Err(JudgesError::BadRequest(
                "max_interactive must be >= 0".to_string(),
            ));
        }
        am.max_interactive = Set(max_interactive);
    }
    if let Some(avatar) = body.avatar_url {
        let cleaned = avatar
            .map(|u| u.trim().to_string())
            .filter(|u| !u.is_empty());
        if let Some(url) = &cleaned {
            validate_judge_avatar_url(url, &state)?;
        }
        am.avatar_url = Set(cleaned);
    }
    am.updated_at = Set(Utc::now());
    let updated = am.update(&state.db).await?;
    Ok(Json(JudgeResponse::from(updated)))
}

pub async fn delete(
    _admin: AdminUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, JudgesError> {
    let row = judges::Entity::find_by_id(id)
        .one(&state.db)
        .await?
        .ok_or(JudgesError::NotFound)?;

    let referenced = task_judges::Entity::find()
        .filter(task_judges::Column::JudgeId.eq(id))
        .one(&state.db)
        .await?;
    if referenced.is_some() {
        return Err(JudgesError::Conflict(
            "judge is referenced by task_judges".into(),
        ));
    }

    judges::Entity::delete_by_id(row.id).exec(&state.db).await?;
    Ok(StatusCode::NO_CONTENT)
}

// ─────────────────────────── where a judge is used ───────────────────────────

/// One task a judge is attached to, with the project it belongs to.
#[derive(Debug, Serialize, Deserialize)]
pub struct JudgeAttachment {
    pub project_id: Uuid,
    pub project_name: String,
    pub project_slug: Option<String>,
    /// True for a campaign part, so the page can group parts under their
    /// parent instead of listing six near-identical projects.
    pub parent_project_id: Option<Uuid>,
    pub task_id: Uuid,
    pub task_ordinal: i32,
    pub task_title: String,
}

/// What a judge has actually done, across every session it ever ran in.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct JudgeStats {
    /// Verdicts that landed — `judge_results` rows with status `scored`.
    pub verdicts: i64,
    /// Runs that ended without a verdict (provider error, bad output …).
    pub failed_runs: i64,
    /// Points this judge moved, summed over its verdicts. Negative for the
    /// penalty judges, which is the honest number for them.
    pub points_total: i64,
    /// Points given and taken, kept apart: a judge that hands out 400 and
    /// takes 380 is a different animal from one that quietly moved 20.
    pub points_awarded: i64,
    pub points_withdrawn: i64,
    /// Distinct sessions and players the judge has verdicted in.
    pub sessions: i64,
    pub players: i64,
    /// When it last produced a verdict.
    pub last_verdict_at: Option<DateTime<Utc>>,
}

/// A judge's attachments and its record, for the admin page.
#[derive(Debug, Serialize, Deserialize)]
pub struct JudgeUsage {
    pub judge_id: Uuid,
    pub attachments: Vec<JudgeAttachment>,
    pub stats: JudgeStats,
}

/// `GET /api/admin/judges/usage` — for every judge: the tasks it is attached
/// to and what it has done.
///
/// Separate from `GET /api/admin/judges` on purpose: the judge list is loaded
/// by the project editor and the settings layout too, and neither needs these
/// aggregates. The page that shows them asks for them.
pub async fn usage(
    _admin: AdminUser,
    State(state): State<AppState>,
) -> Result<Json<Vec<JudgeUsage>>, JudgesError> {
    use arena_core::entities::{judge_results, projects, task_judges, tasks};
    use sea_orm::QuerySelect;
    use std::collections::HashMap;

    let judge_rows = judges::Entity::find()
        .order_by_asc(judges::Column::Slug)
        .all(&state.db)
        .await?;

    // Attachments: task_judges → tasks → projects, assembled in three reads
    // rather than a join so the shapes stay obvious and portable.
    let links = task_judges::Entity::find().all(&state.db).await?;
    let task_ids: Vec<Uuid> = links.iter().map(|l| l.task_id).collect();
    let task_by_id: HashMap<Uuid, tasks::Model> = if task_ids.is_empty() {
        HashMap::new()
    } else {
        tasks::Entity::find()
            .filter(tasks::Column::Id.is_in(task_ids))
            .all(&state.db)
            .await?
            .into_iter()
            .map(|t| (t.id, t))
            .collect()
    };
    let project_ids: Vec<Uuid> = task_by_id.values().map(|t| t.project_id_fk).collect();
    let project_by_id: HashMap<Uuid, projects::Model> = if project_ids.is_empty() {
        HashMap::new()
    } else {
        projects::Entity::find()
            .filter(projects::Column::Id.is_in(project_ids))
            .all(&state.db)
            .await?
            .into_iter()
            .map(|p| (p.id, p))
            .collect()
    };

    let mut attachments: HashMap<Uuid, Vec<JudgeAttachment>> = HashMap::new();
    for link in links {
        let Some(task) = task_by_id.get(&link.task_id) else {
            continue;
        };
        let Some(project) = project_by_id.get(&task.project_id_fk) else {
            continue;
        };
        attachments
            .entry(link.judge_id)
            .or_default()
            .push(JudgeAttachment {
                project_id: project.id,
                project_name: project.name.clone(),
                project_slug: project.slug.clone(),
                parent_project_id: project.parent_project_id_fk,
                task_id: task.id,
                task_ordinal: task.ordinal,
                task_title: task.title.clone(),
            });
    }
    for list in attachments.values_mut() {
        list.sort_by(|a, b| {
            a.project_name
                .cmp(&b.project_name)
                .then(a.task_ordinal.cmp(&b.task_ordinal))
        });
    }

    // The record. `judge_results` knows its judge only through
    // `task_judge_id`, so map the links once and fold the rows in Rust: the
    // alternative is a three-table join with a portable SUM, and this table
    // is small enough that clarity wins.
    let judge_of_link: HashMap<Uuid, Uuid> = task_judges::Entity::find()
        .select_only()
        .column(task_judges::Column::Id)
        .column(task_judges::Column::JudgeId)
        .into_tuple::<(Uuid, Uuid)>()
        .all(&state.db)
        .await?
        .into_iter()
        .collect();

    #[derive(Default)]
    struct Fold {
        stats: JudgeStats,
        sessions: std::collections::HashSet<Uuid>,
        players: std::collections::HashSet<Uuid>,
    }
    let mut folded: HashMap<Uuid, Fold> = HashMap::new();
    let results = judge_results::Entity::find()
        .select_only()
        .column(judge_results::Column::TaskJudgeId)
        .column(judge_results::Column::SessionIdFk)
        .column(judge_results::Column::PlayerIdFk)
        .column(judge_results::Column::PointDelta)
        .column(judge_results::Column::Status)
        .column(judge_results::Column::CreatedAt)
        .into_tuple::<(Uuid, Uuid, Uuid, i32, String, DateTime<Utc>)>()
        .all(&state.db)
        .await?;
    for (task_judge_id, session_id, player_id, point_delta, status, created_at) in results {
        let Some(judge_id) = judge_of_link.get(&task_judge_id) else {
            continue;
        };
        let fold = folded.entry(*judge_id).or_default();
        match status.as_str() {
            "scored" => {
                fold.stats.verdicts += 1;
                fold.stats.points_total += i64::from(point_delta);
                if point_delta >= 0 {
                    fold.stats.points_awarded += i64::from(point_delta);
                } else {
                    fold.stats.points_withdrawn += i64::from(-point_delta);
                }
                fold.sessions.insert(session_id);
                fold.players.insert(player_id);
                fold.stats.last_verdict_at = Some(
                    fold.stats
                        .last_verdict_at
                        .map_or(created_at, |seen| seen.max(created_at)),
                );
            }
            // "running"/"waiting" rows are in flight, not history.
            "failed" => fold.stats.failed_runs += 1,
            _ => {}
        }
    }

    Ok(Json(
        judge_rows
            .into_iter()
            .map(|judge| {
                let fold = folded.remove(&judge.id).unwrap_or_default();
                let mut stats = fold.stats;
                stats.sessions = fold.sessions.len() as i64;
                stats.players = fold.players.len() as i64;
                JudgeUsage {
                    judge_id: judge.id,
                    attachments: attachments.remove(&judge.id).unwrap_or_default(),
                    stats,
                }
            })
            .collect(),
    ))
}
