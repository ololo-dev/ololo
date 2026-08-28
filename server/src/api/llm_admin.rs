//! Admin CRUD for LLM providers and operation-model assignments.
//!
//! Providers live in `llm_providers` (multiple rows; keys encrypted at
//! rest, never returned). Assignments live in `app_settings` as JSON
//! (`llm_default`, `llm_op_<operation>`); resolution happens per call in
//! `arena_core::llm::resolve`, so changes apply without restarts.

use crate::api::settings::AdminUser;
use crate::state::AppState;
use arena_core::entities::{app_settings, judges, llm_pools, llm_providers, llm_requests};
use arena_core::llm::resolve::{
    LLM_DEFAULT_KEY, LLM_OPERATIONS, LlmAssignment, llm_op_key, registry_id_for_kind,
};
use axum::extract::{Path, Query, State};
use axum::{Json, http::StatusCode, response::IntoResponse, response::Response};
use chrono::Utc;
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter,
    QueryOrder, QuerySelect,
};
use uuid::Uuid;

#[derive(Debug, thiserror::Error)]
pub enum LlmAdminError {
    #[error("not_found")]
    NotFound,
    #[error("invalid_kind")]
    InvalidKind,
    #[error("invalid_name")]
    InvalidName,
    #[error("name_conflict")]
    NameConflict,
    #[error("base_url_required")]
    BaseUrlRequired,
    #[error("invalid_operation")]
    InvalidOperation,
    #[error("unknown_provider")]
    UnknownProvider,
    #[error("unknown_pool")]
    UnknownPool,
    #[error("model_required")]
    ModelRequired,
    #[error("provider_unusable")]
    ProviderUnusable,
    #[error("database error")]
    Db(#[from] sea_orm::DbErr),
}

crate::api::error::impl_api_error!(LlmAdminError {
    Self::NotFound => (NOT_FOUND, "not_found"),
    Self::InvalidKind => (UNPROCESSABLE_ENTITY, "invalid_kind"),
    Self::InvalidName => (UNPROCESSABLE_ENTITY, "invalid_name"),
    Self::NameConflict => (CONFLICT, "name_conflict"),
    Self::BaseUrlRequired => (UNPROCESSABLE_ENTITY, "base_url_required"),
    Self::InvalidOperation => (UNPROCESSABLE_ENTITY, "invalid_operation"),
    Self::UnknownProvider => (UNPROCESSABLE_ENTITY, "unknown_provider"),
    Self::UnknownPool => (UNPROCESSABLE_ENTITY, "unknown_pool"),
    Self::ModelRequired => (UNPROCESSABLE_ENTITY, "model_required"),
    Self::ProviderUnusable => (UNPROCESSABLE_ENTITY, "provider_unusable"),
    Self::Db(_) => (INTERNAL_SERVER_ERROR, "database_error"),
});

#[derive(Debug, serde::Serialize)]
pub struct ProviderResp {
    pub id: Uuid,
    pub name: String,
    pub kind: String,
    pub base_url: Option<String>,
    /// Whether an API key is stored; the key itself never leaves the server.
    pub has_api_key: bool,
    pub enabled: bool,
    pub catalog_id: Option<String>,
    pub created_at: chrono::DateTime<Utc>,
    pub updated_at: chrono::DateTime<Utc>,
}

fn to_resp(m: llm_providers::Model) -> ProviderResp {
    ProviderResp {
        id: m.id,
        name: m.name,
        kind: m.kind,
        base_url: m.base_url,
        has_api_key: m.api_key_enc.as_deref().is_some_and(|s| !s.is_empty()),
        enabled: m.enabled,
        catalog_id: m.catalog_id,
        created_at: m.created_at,
        updated_at: m.updated_at,
    }
}

fn validate_kind(kind: &str) -> Result<(), LlmAdminError> {
    registry_id_for_kind(kind)
        .map(|_| ())
        .ok_or(LlmAdminError::InvalidKind)
}

fn validate_base_url(kind: &str, base_url: Option<&str>) -> Result<(), LlmAdminError> {
    // openai_compatible has no default endpoint — a base URL is mandatory.
    if kind == "openai_compatible" && base_url.map(str::trim).is_none_or(str::is_empty) {
        return Err(LlmAdminError::BaseUrlRequired);
    }
    Ok(())
}

fn normalized_name(name: &str) -> Result<String, LlmAdminError> {
    let n = name.trim();
    if n.is_empty() || n.len() > 100 {
        return Err(LlmAdminError::InvalidName);
    }
    Ok(n.to_string())
}

/// `GET /api/admin/llm/providers`
pub async fn list_providers(
    _admin: AdminUser,
    State(state): State<AppState>,
) -> Result<Json<Vec<ProviderResp>>, LlmAdminError> {
    let rows = llm_providers::Entity::find()
        .order_by_asc(llm_providers::Column::Name)
        .all(&state.db)
        .await?;
    Ok(Json(rows.into_iter().map(to_resp).collect()))
}

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateProviderReq {
    pub name: String,
    pub kind: String,
    #[serde(default)]
    pub base_url: Option<String>,
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub catalog_id: Option<String>,
}

fn default_true() -> bool {
    true
}

/// `POST /api/admin/llm/providers`
pub async fn create_provider(
    _admin: AdminUser,
    State(state): State<AppState>,
    Json(req): Json<CreateProviderReq>,
) -> Result<Response, LlmAdminError> {
    validate_kind(&req.kind)?;
    validate_base_url(&req.kind, req.base_url.as_deref())?;
    let name = normalized_name(&req.name)?;
    let exists = llm_providers::Entity::find()
        .filter(llm_providers::Column::Name.eq(name.clone()))
        .one(&state.db)
        .await?;
    if exists.is_some() {
        return Err(LlmAdminError::NameConflict);
    }
    let now = Utc::now();
    let am = llm_providers::ActiveModel {
        id: Set(Uuid::new_v4()),
        name: Set(name),
        kind: Set(req.kind),
        base_url: Set(req.base_url.filter(|s| !s.trim().is_empty())),
        api_key_enc: Set(req
            .api_key
            .filter(|s| !s.trim().is_empty())
            .map(|k| state.settings_encryption.encrypt(&k))),
        enabled: Set(req.enabled),
        catalog_id: Set(req.catalog_id.filter(|s| !s.trim().is_empty())),
        created_at: Set(now),
        updated_at: Set(now),
    };
    let model = am.insert(&state.db).await?;
    Ok((StatusCode::CREATED, Json(to_resp(model))).into_response())
}

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpdateProviderReq {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub base_url: Option<String>,
    /// New API key; absent keeps the stored one.
    #[serde(default)]
    pub api_key: Option<String>,
    /// `true` removes the stored key (takes precedence over `api_key`).
    #[serde(default)]
    pub clear_api_key: Option<bool>,
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub catalog_id: Option<String>,
}

/// `PUT /api/admin/llm/providers/:id`
pub async fn update_provider(
    _admin: AdminUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateProviderReq>,
) -> Result<Json<ProviderResp>, LlmAdminError> {
    let row = llm_providers::Entity::find_by_id(id)
        .one(&state.db)
        .await?
        .ok_or(LlmAdminError::NotFound)?;
    let kind = req.kind.clone().unwrap_or_else(|| row.kind.clone());
    validate_kind(&kind)?;
    let base_url = match &req.base_url {
        Some(u) => Some(u.trim().to_string()).filter(|s| !s.is_empty()),
        None => row.base_url.clone(),
    };
    validate_base_url(&kind, base_url.as_deref())?;

    let mut am: llm_providers::ActiveModel = row.clone().into();
    if let Some(name) = &req.name {
        let name = normalized_name(name)?;
        if name != row.name {
            let exists = llm_providers::Entity::find()
                .filter(llm_providers::Column::Name.eq(name.clone()))
                .one(&state.db)
                .await?;
            if exists.is_some() {
                return Err(LlmAdminError::NameConflict);
            }
            am.name = Set(name);
        }
    }
    am.kind = Set(kind);
    if req.base_url.is_some() {
        am.base_url = Set(base_url);
    }
    if req.clear_api_key == Some(true) {
        am.api_key_enc = Set(None);
    } else if let Some(key) = req.api_key.filter(|s| !s.trim().is_empty()) {
        am.api_key_enc = Set(Some(state.settings_encryption.encrypt(&key)));
    }
    if let Some(enabled) = req.enabled {
        am.enabled = Set(enabled);
    }
    if let Some(cid) = &req.catalog_id {
        am.catalog_id = Set(Some(cid.clone()).filter(|s| !s.trim().is_empty()));
    }
    am.updated_at = Set(Utc::now());
    let updated = am.update(&state.db).await?;
    Ok(Json(to_resp(updated)))
}

/// `DELETE /api/admin/llm/providers/:id` — also clears assignments and
/// judge overrides referencing the provider (no DB-level FK; see entity doc).
pub async fn delete_provider(
    _admin: AdminUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, LlmAdminError> {
    let row = llm_providers::Entity::find_by_id(id)
        .one(&state.db)
        .await?
        .ok_or(LlmAdminError::NotFound)?;

    // Clear judge overrides pointing at this provider.
    judges::Entity::update_many()
        .col_expr(
            judges::Column::LlmProviderIdFk,
            sea_orm::prelude::Expr::value(Option::<Uuid>::None),
        )
        .col_expr(
            judges::Column::LlmModel,
            sea_orm::prelude::Expr::value(Option::<String>::None),
        )
        .filter(judges::Column::LlmProviderIdFk.eq(id))
        .exec(&state.db)
        .await?;

    // Clear assignments pointing at this provider.
    let mut keys: Vec<String> = vec![LLM_DEFAULT_KEY.to_string()];
    keys.extend(LLM_OPERATIONS.iter().map(|op| llm_op_key(op)));
    for key in keys {
        if let Some(setting) = app_settings::Entity::find_by_id(key.clone())
            .one(&state.db)
            .await?
            && let Some(a) = arena_core::llm::resolve::parse_assignment(&setting.value)
            && a.provider_id() == Some(id)
        {
            app_settings::Entity::delete_by_id(key)
                .exec(&state.db)
                .await?;
        }
    }

    llm_providers::Entity::delete_by_id(row.id)
        .exec(&state.db)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// `GET /api/admin/llm/providers/:id/models` — live model list from the
/// provider's endpoint using its stored credentials. Unreachable → `[]`.
pub async fn list_provider_row_models(
    _admin: AdminUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Vec<String>>, LlmAdminError> {
    let row = llm_providers::Entity::find_by_id(id)
        .one(&state.db)
        .await?
        .ok_or(LlmAdminError::NotFound)?;
    let api_key = row
        .api_key_enc
        .as_deref()
        .filter(|s| !s.is_empty())
        .and_then(|s| state.settings_encryption.decrypt(s).ok());
    let models = crate::api::settings::list_models_for(
        &state,
        &row.kind,
        row.base_url.as_deref(),
        api_key.as_deref(),
    )
    .await;
    Ok(Json(models))
}

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TestProviderReq {
    pub model: String,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct TestProviderResp {
    pub ok: bool,
    pub model: String,
    pub latency_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// `POST /api/admin/llm/providers/:id/test` — one real completion through
/// the stored credentials.
///
/// Model *listing* is not proof of life: a provider happily lists models
/// with an empty balance or a key scoped away from completions, and the
/// failure then surfaces mid-session in a judge run. This calls the paid
/// endpoint, so what the admin sees is what a judge would get.
pub async fn test_provider(
    _admin: AdminUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(req): Json<TestProviderReq>,
) -> Result<Json<TestProviderResp>, LlmAdminError> {
    let row = llm_providers::Entity::find_by_id(id)
        .one(&state.db)
        .await?
        .ok_or(LlmAdminError::NotFound)?;
    let model = req.model.trim().to_string();
    if model.is_empty() {
        return Err(LlmAdminError::ModelRequired);
    }
    let Some(cfg) = arena_core::llm::resolve::model_config_from_provider(
        &row,
        &model,
        &state.settings_encryption,
    ) else {
        return Err(LlmAdminError::ProviderUnusable);
    };

    const TEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(45);
    let started = std::time::Instant::now();
    let outcome = tokio::time::timeout(
        TEST_TIMEOUT,
        cfg.complete(
            "You are a connectivity probe for an admin settings page.",
            "Reply with the single word: ok",
        ),
    )
    .await;
    let latency_ms = started.elapsed().as_millis() as u64;

    let resp = match outcome {
        Ok(Ok(text)) => TestProviderResp {
            ok: true,
            model,
            latency_ms,
            output: Some(text.chars().take(300).collect()),
            error: None,
        },
        // The provider's own words, unredacted: this page is admin-only and
        // "Insufficient balance" is exactly what the admin needs to read.
        Ok(Err(e)) => TestProviderResp {
            ok: false,
            model,
            latency_ms,
            output: None,
            error: Some(e.to_string()),
        },
        Err(_) => TestProviderResp {
            ok: false,
            model,
            latency_ms,
            output: None,
            error: Some(format!("no response within {}s", TEST_TIMEOUT.as_secs())),
        },
    };
    Ok(Json(resp))
}

// ---------------------------------------------------------------------------
// Assignments
// ---------------------------------------------------------------------------

#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct AssignmentsResp {
    /// Fallback for every operation without its own assignment.
    pub default: Option<LlmAssignment>,
    /// Keyed by operation name (see `LLM_OPERATIONS`).
    pub operations: std::collections::BTreeMap<String, LlmAssignment>,
}

/// `GET /api/admin/llm/assignments`
pub async fn get_assignments(
    _admin: AdminUser,
    State(state): State<AppState>,
) -> Result<Json<AssignmentsResp>, LlmAdminError> {
    let mut resp = AssignmentsResp::default();
    if let Some(row) = app_settings::Entity::find_by_id(LLM_DEFAULT_KEY.to_string())
        .one(&state.db)
        .await?
    {
        resp.default = arena_core::llm::resolve::parse_assignment(&row.value);
    }
    for op in LLM_OPERATIONS {
        if let Some(row) = app_settings::Entity::find_by_id(llm_op_key(op))
            .one(&state.db)
            .await?
            && let Some(a) = arena_core::llm::resolve::parse_assignment(&row.value)
        {
            resp.operations.insert(op.to_string(), a);
        }
    }
    Ok(Json(resp))
}

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PutAssignmentsReq {
    /// Absent = unchanged; `null` = clear; object = set.
    #[serde(default, deserialize_with = "deserialize_some")]
    pub default: Option<Option<LlmAssignment>>,
    /// Per-operation updates; `null` value clears that operation.
    #[serde(default)]
    pub operations: std::collections::BTreeMap<String, Option<LlmAssignment>>,
}

fn deserialize_some<'de, T, D>(d: D) -> Result<Option<T>, D::Error>
where
    T: serde::Deserialize<'de>,
    D: serde::Deserializer<'de>,
{
    T::deserialize(d).map(Some)
}

async fn provider_exists(state: &AppState, id: Uuid) -> Result<bool, LlmAdminError> {
    Ok(llm_providers::Entity::find_by_id(id)
        .one(&state.db)
        .await?
        .is_some())
}

async fn pool_exists(state: &AppState, id: Uuid) -> Result<bool, LlmAdminError> {
    Ok(llm_pools::Entity::find_by_id(id)
        .one(&state.db)
        .await?
        .is_some())
}

/// Reject an assignment that points at something that does not exist.
///
/// A pool is accepted even when it currently has no usable members: pools
/// are edited independently of the assignments that reference them, and an
/// empty pool already degrades safely (resolution treats it as inert and
/// falls through to the default assignment).
async fn validate_assignment(state: &AppState, a: &LlmAssignment) -> Result<(), LlmAdminError> {
    match a {
        LlmAssignment::Single(s) => {
            if s.model.trim().is_empty() || !provider_exists(state, s.provider_id).await? {
                return Err(LlmAdminError::UnknownProvider);
            }
        }
        LlmAssignment::Pool(p) => {
            if !pool_exists(state, p.pool_id).await? {
                return Err(LlmAdminError::UnknownPool);
            }
        }
    }
    Ok(())
}

async fn upsert_setting(state: &AppState, key: String, value: String) -> Result<(), LlmAdminError> {
    use sea_orm::sea_query::OnConflict;
    let am = app_settings::ActiveModel {
        key: Set(key),
        value: Set(value),
    };
    app_settings::Entity::insert(am)
        .on_conflict(
            OnConflict::column(app_settings::Column::Key)
                .update_column(app_settings::Column::Value)
                .to_owned(),
        )
        .exec(&state.db)
        .await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Unified LLM request telemetry (`llm_requests`)
// ---------------------------------------------------------------------------

/// Row DTO for `llm_requests` — all columns (admin-only surface).
#[derive(Debug, serde::Serialize)]
pub struct LlmRequestResp {
    pub id: Uuid,
    pub operation: String,
    /// Registry id — "custom" for every `openai_compatible` endpoint.
    pub provider: String,
    /// Name of the configured provider row, when the call went through one.
    /// This is what distinguishes two `custom` endpoints; prefer it for display.
    pub provider_name: Option<String>,
    pub model: String,
    pub status: String,
    pub error: Option<String>,
    pub tokens_input: i64,
    pub tokens_output: i64,
    pub tokens_cache_read: i64,
    pub tokens_cache_write: i64,
    pub duration_ms: i64,
    pub session_id: Option<Uuid>,
    pub player_id: Option<Uuid>,
    pub task_id: Option<Uuid>,
    /// Resolved labels for the ids above, so the UI shows what a human
    /// recognises instead of a uuid. All optional: telemetry outlives the
    /// rows it points at, and an anonymous player has no account.
    pub session_code: Option<String>,
    pub player_name: Option<String>,
    /// Account username, when the player was signed in — the only one of
    /// these that can be linked (`/u/{username}`).
    pub player_username: Option<String>,
    pub task_title: Option<String>,
    pub judge_slug: Option<String>,
    pub detail_json: Option<String>,
    /// Per-turn trace (JSON array of judge log events). Populated only on
    /// the single-row read — a list of 50 traces would be megabytes.
    pub events_json: Option<String>,
    pub created_at: chrono::DateTime<Utc>,
    /// USD from the `llm_model_prices` map; `null` when the model is unpriced.
    pub cost: Option<f64>,
}

/// List-shaped DTO: everything except the (potentially ~0.5 MB) trace.
fn telemetry_to_resp(m: llm_requests::Model) -> LlmRequestResp {
    LlmRequestResp {
        events_json: None,
        ..telemetry_to_detail_resp(m)
    }
}

/// Detail-shaped DTO including the per-turn trace. Context labels are left
/// empty here; [`resolve_context_labels`] fills them for a whole page at once.
fn telemetry_to_detail_resp(m: llm_requests::Model) -> LlmRequestResp {
    LlmRequestResp {
        id: m.id,
        operation: m.operation,
        provider: m.provider,
        provider_name: m.provider_name,
        model: m.model,
        status: m.status,
        error: m.error,
        tokens_input: m.tokens_input,
        tokens_output: m.tokens_output,
        tokens_cache_read: m.tokens_cache_read,
        tokens_cache_write: m.tokens_cache_write,
        duration_ms: m.duration_ms,
        session_id: m.session_id,
        player_id: m.player_id,
        task_id: m.task_id,
        session_code: None,
        player_name: None,
        player_username: None,
        task_title: None,
        judge_slug: m.judge_slug,
        detail_json: m.detail_json,
        events_json: m.events_json,
        created_at: m.created_at,
        cost: None,
    }
}

/// Price each response row from the stored model-price map.
fn apply_costs(
    rows: &mut [LlmRequestResp],
    prices: &std::collections::HashMap<String, crate::api::analytics::ModelPrice>,
) {
    for r in rows {
        r.cost = prices.get(&r.model).map(|p| {
            p.cost(
                r.tokens_input,
                r.tokens_output,
                r.tokens_cache_read,
                r.tokens_cache_write,
            )
        });
    }
}

/// Resolve the session / player / task ids on `rows` into names.
///
/// Three-ish queries for the whole page rather than per row, and every
/// lookup is best-effort: `llm_requests` has no foreign keys precisely so
/// telemetry survives the rows it references, which means a missing target
/// is normal and must leave the label empty rather than fail the request.
async fn resolve_context_labels(state: &AppState, rows: &mut [LlmRequestResp]) {
    use arena_core::entities::{players, sessions, tasks, users};

    fn unique<T: Ord + Copy>(ids: impl Iterator<Item = T>) -> Vec<T> {
        let mut v: Vec<T> = ids.collect();
        v.sort_unstable();
        v.dedup();
        v
    }

    let session_ids = unique(rows.iter().filter_map(|r| r.session_id));
    let player_ids = unique(rows.iter().filter_map(|r| r.player_id));
    let task_ids = unique(rows.iter().filter_map(|r| r.task_id));

    let mut codes: std::collections::HashMap<Uuid, String> = Default::default();
    if !session_ids.is_empty()
        && let Ok(found) = sessions::Entity::find()
            .filter(sessions::Column::Id.is_in(session_ids))
            .all(&state.db)
            .await
    {
        codes = found.into_iter().map(|s| (s.id, s.join_code)).collect();
    }

    // display_name lives on the player row; the username needed for a profile
    // link lives on the account behind it, which anonymous players lack.
    let mut player_rows: std::collections::HashMap<Uuid, players::Model> = Default::default();
    if !player_ids.is_empty()
        && let Ok(found) = players::Entity::find()
            .filter(players::Column::Id.is_in(player_ids))
            .all(&state.db)
            .await
    {
        player_rows = found.into_iter().map(|p| (p.id, p)).collect();
    }
    let mut usernames: std::collections::HashMap<Uuid, String> = Default::default();
    let user_ids = unique(player_rows.values().filter_map(|p| p.user_id_fk));
    if !user_ids.is_empty()
        && let Ok(found) = users::Entity::find()
            .filter(users::Column::Id.is_in(user_ids))
            .all(&state.db)
            .await
    {
        usernames = found
            .into_iter()
            .filter_map(|u| u.username.map(|n| (u.id, n)))
            .collect();
    }

    let mut titles: std::collections::HashMap<Uuid, String> = Default::default();
    if !task_ids.is_empty()
        && let Ok(found) = tasks::Entity::find()
            .filter(tasks::Column::Id.is_in(task_ids))
            .all(&state.db)
            .await
    {
        titles = found.into_iter().map(|t| (t.id, t.title)).collect();
    }

    for row in rows.iter_mut() {
        row.session_code = row.session_id.and_then(|id| codes.get(&id).cloned());
        row.task_title = row.task_id.and_then(|id| titles.get(&id).cloned());
        if let Some(p) = row.player_id.and_then(|id| player_rows.get(&id)) {
            row.player_name = Some(p.display_name.clone());
            row.player_username = p.user_id_fk.and_then(|uid| usernames.get(&uid).cloned());
        }
    }
}

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TelemetryListQuery {
    #[serde(default)]
    pub operation: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub limit: Option<u64>,
    #[serde(default)]
    pub offset: Option<u64>,
}

#[derive(Debug, serde::Serialize)]
pub struct TelemetryListResp {
    pub items: Vec<LlmRequestResp>,
    /// Total matching rows (before limit/offset).
    pub total: u64,
}

/// `GET /api/admin/llm/telemetry?operation=&status=&limit=&offset=`
///
/// Newest first (`created_at DESC`); limit defaults to 50, capped at 200.
pub async fn list_llm_telemetry(
    _admin: AdminUser,
    State(state): State<AppState>,
    Query(q): Query<TelemetryListQuery>,
) -> Result<Json<TelemetryListResp>, LlmAdminError> {
    let limit = q.limit.unwrap_or(50).min(200);
    let offset = q.offset.unwrap_or(0);

    let mut find = llm_requests::Entity::find();
    if let Some(op) = q
        .operation
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        find = find.filter(llm_requests::Column::Operation.eq(op));
    }
    if let Some(st) = q.status.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        find = find.filter(llm_requests::Column::Status.eq(st));
    }

    let total = find.clone().count(&state.db).await?;
    let items = find
        .order_by_desc(llm_requests::Column::CreatedAt)
        // Deterministic tiebreak for rows created within the same instant.
        .order_by_desc(llm_requests::Column::Id)
        .offset(offset)
        .limit(limit)
        .all(&state.db)
        .await?
        .into_iter()
        .map(telemetry_to_resp)
        .collect();

    let mut items: Vec<LlmRequestResp> = items;
    resolve_context_labels(&state, &mut items).await;
    let prices = crate::api::analytics::load_prices(&state.db).await;
    apply_costs(&mut items, &prices);

    Ok(Json(TelemetryListResp { items, total }))
}

/// `GET /api/admin/llm/telemetry/:id`
pub async fn get_llm_telemetry(
    _admin: AdminUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<LlmRequestResp>, LlmAdminError> {
    let row = llm_requests::Entity::find_by_id(id)
        .one(&state.db)
        .await?
        .ok_or(LlmAdminError::NotFound)?;
    let mut one = [telemetry_to_detail_resp(row)];
    resolve_context_labels(&state, &mut one).await;
    let prices = crate::api::analytics::load_prices(&state.db).await;
    apply_costs(&mut one, &prices);
    let [resp] = one;
    Ok(Json(resp))
}

/// `PUT /api/admin/llm/assignments`
pub async fn put_assignments(
    _admin: AdminUser,
    State(state): State<AppState>,
    Json(req): Json<PutAssignmentsReq>,
) -> Result<Json<AssignmentsResp>, LlmAdminError> {
    for op in req.operations.keys() {
        if !LLM_OPERATIONS.contains(&op.as_str()) {
            return Err(LlmAdminError::InvalidOperation);
        }
    }
    let mut writes: Vec<(String, Option<LlmAssignment>)> = Vec::new();
    if let Some(default) = req.default {
        writes.push((LLM_DEFAULT_KEY.to_string(), default));
    }
    for (op, a) in req.operations {
        writes.push((llm_op_key(&op), a));
    }
    for (key, assignment) in writes {
        match assignment {
            Some(a) => {
                validate_assignment(&state, &a).await?;
                let value = serde_json::to_string(&a).expect("assignment serializes");
                upsert_setting(&state, key, value).await?;
            }
            None => {
                app_settings::Entity::delete_by_id(key)
                    .exec(&state.db)
                    .await?;
            }
        }
    }
    get_assignments(_admin, State(state)).await
}
