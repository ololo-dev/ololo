//! Admin CRUD for LLM model pools.
//!
//! A pool is a named, ordered set of `(provider, model)` candidates that an
//! assignment can point at instead of a single model (see
//! `arena_core::llm::resolve`). Members sharing a `priority` form one tier:
//! the tier splits the load between them via the pool's round-robin cursor,
//! and the next tier is reached only once the current one has been tried.
//!
//! A pool is edited as a whole — `PUT` replaces the member list rather than
//! patching individual rows. That keeps the tier layout atomic (no request
//! can leave a pool half-reordered) and matches how the admin UI edits one.

use crate::api::llm_admin::LlmAdminError;
use crate::api::settings::AdminUser;
use crate::state::AppState;
use arena_core::entities::{app_settings, llm_pool_members, llm_pools, llm_providers};
use arena_core::llm::resolve::{LLM_DEFAULT_KEY, LLM_OPERATIONS, llm_op_key};
use axum::extract::{Path, State};
use axum::{Json, http::StatusCode, response::IntoResponse, response::Response};
use chrono::Utc;
use sea_orm::{
    ActiveValue::Set, ColumnTrait, EntityTrait, QueryFilter, QueryOrder, TransactionTrait,
};
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Wire types
// ---------------------------------------------------------------------------

#[derive(Debug, serde::Serialize)]
pub struct PoolMemberResp {
    pub id: Uuid,
    pub provider_id: Uuid,
    pub model: String,
    pub priority: i32,
    pub enabled: bool,
}

#[derive(Debug, serde::Serialize)]
pub struct PoolResp {
    pub id: Uuid,
    pub name: String,
    pub description: String,
    /// Ordered by ascending `priority` — the order resolution walks them in.
    pub members: Vec<PoolMemberResp>,
    pub created_at: chrono::DateTime<Utc>,
    pub updated_at: chrono::DateTime<Utc>,
}

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PoolMemberReq {
    pub provider_id: Uuid,
    pub model: String,
    /// Lower is tried first; equal values form one load-splitting tier.
    #[serde(default)]
    pub priority: i32,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreatePoolReq {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub members: Vec<PoolMemberReq>,
}

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpdatePoolReq {
    /// Absent = unchanged.
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    /// Absent = members unchanged; present = the pool's members become
    /// exactly this list.
    #[serde(default)]
    pub members: Option<Vec<PoolMemberReq>>,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn normalized_name(raw: &str) -> Result<String, LlmAdminError> {
    let name = raw.trim().to_string();
    if name.is_empty() || name.chars().count() > 100 {
        return Err(LlmAdminError::InvalidName);
    }
    Ok(name)
}

/// Reject members naming a provider that does not exist or an empty model —
/// both would be silently dropped at resolve time, leaving an admin looking
/// at a pool that cannot explain why it never fires.
async fn validate_members(
    state: &AppState,
    members: &[PoolMemberReq],
) -> Result<(), LlmAdminError> {
    for m in members {
        if m.model.trim().is_empty() {
            return Err(LlmAdminError::UnknownProvider);
        }
        if llm_providers::Entity::find_by_id(m.provider_id)
            .one(&state.db)
            .await?
            .is_none()
        {
            return Err(LlmAdminError::UnknownProvider);
        }
    }
    Ok(())
}

async fn load_pool_resp(state: &AppState, id: Uuid) -> Result<PoolResp, LlmAdminError> {
    let pool = llm_pools::Entity::find_by_id(id)
        .one(&state.db)
        .await?
        .ok_or(LlmAdminError::NotFound)?;
    let members = llm_pool_members::Entity::find()
        .filter(llm_pool_members::Column::PoolIdFk.eq(id))
        .order_by_asc(llm_pool_members::Column::Priority)
        .order_by_asc(llm_pool_members::Column::CreatedAt)
        .all(&state.db)
        .await?;
    Ok(to_resp(pool, members))
}

fn to_resp(pool: llm_pools::Model, members: Vec<llm_pool_members::Model>) -> PoolResp {
    PoolResp {
        id: pool.id,
        name: pool.name,
        description: pool.description,
        members: members
            .into_iter()
            .map(|m| PoolMemberResp {
                id: m.id,
                provider_id: m.provider_id_fk,
                model: m.model,
                priority: m.priority,
                enabled: m.enabled,
            })
            .collect(),
        created_at: pool.created_at,
        updated_at: pool.updated_at,
    }
}

/// Replace a pool's members inside `txn`. The whole list is rewritten, so
/// member ids are not stable across an update.
async fn replace_members<C: sea_orm::ConnectionTrait>(
    txn: &C,
    pool_id: Uuid,
    members: Vec<PoolMemberReq>,
) -> Result<(), LlmAdminError> {
    llm_pool_members::Entity::delete_many()
        .filter(llm_pool_members::Column::PoolIdFk.eq(pool_id))
        .exec(txn)
        .await?;
    if members.is_empty() {
        return Ok(());
    }
    let now = Utc::now();
    let rows: Vec<llm_pool_members::ActiveModel> = members
        .into_iter()
        .map(|m| llm_pool_members::ActiveModel {
            id: Set(Uuid::new_v4()),
            pool_id_fk: Set(pool_id),
            provider_id_fk: Set(m.provider_id),
            model: Set(m.model.trim().to_string()),
            priority: Set(m.priority),
            enabled: Set(m.enabled),
            created_at: Set(now),
            updated_at: Set(now),
        })
        .collect();
    llm_pool_members::Entity::insert_many(rows)
        .exec(txn)
        .await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// `GET /api/admin/llm/pools`
pub async fn list_pools(
    _admin: AdminUser,
    State(state): State<AppState>,
) -> Result<Json<Vec<PoolResp>>, LlmAdminError> {
    let pools = llm_pools::Entity::find()
        .order_by_asc(llm_pools::Column::Name)
        .all(&state.db)
        .await?;
    // One members query for every pool rather than one per pool: the admin
    // page lists them all at once.
    let members = llm_pool_members::Entity::find()
        .order_by_asc(llm_pool_members::Column::Priority)
        .order_by_asc(llm_pool_members::Column::CreatedAt)
        .all(&state.db)
        .await?;
    let mut by_pool: std::collections::HashMap<Uuid, Vec<llm_pool_members::Model>> =
        std::collections::HashMap::new();
    for m in members {
        by_pool.entry(m.pool_id_fk).or_default().push(m);
    }
    Ok(Json(
        pools
            .into_iter()
            .map(|p| {
                let ms = by_pool.remove(&p.id).unwrap_or_default();
                to_resp(p, ms)
            })
            .collect(),
    ))
}

/// `GET /api/admin/llm/pools/:id`
pub async fn get_pool(
    _admin: AdminUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<PoolResp>, LlmAdminError> {
    Ok(Json(load_pool_resp(&state, id).await?))
}

/// `POST /api/admin/llm/pools`
pub async fn create_pool(
    _admin: AdminUser,
    State(state): State<AppState>,
    Json(req): Json<CreatePoolReq>,
) -> Result<Response, LlmAdminError> {
    let name = normalized_name(&req.name)?;
    validate_members(&state, &req.members).await?;
    if llm_pools::Entity::find()
        .filter(llm_pools::Column::Name.eq(name.clone()))
        .one(&state.db)
        .await?
        .is_some()
    {
        return Err(LlmAdminError::NameConflict);
    }

    let id = Uuid::new_v4();
    let now = Utc::now();
    let txn = state.db.begin().await?;
    llm_pools::Entity::insert(llm_pools::ActiveModel {
        id: Set(id),
        name: Set(name),
        description: Set(req.description.trim().to_string()),
        rr_cursor: Set(0),
        created_at: Set(now),
        updated_at: Set(now),
    })
    .exec(&txn)
    .await?;
    replace_members(&txn, id, req.members).await?;
    txn.commit().await?;

    Ok((StatusCode::CREATED, Json(load_pool_resp(&state, id).await?)).into_response())
}

/// `PUT /api/admin/llm/pools/:id`
pub async fn update_pool(
    _admin: AdminUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdatePoolReq>,
) -> Result<Json<PoolResp>, LlmAdminError> {
    let pool = llm_pools::Entity::find_by_id(id)
        .one(&state.db)
        .await?
        .ok_or(LlmAdminError::NotFound)?;

    let name = match &req.name {
        Some(raw) => {
            let name = normalized_name(raw)?;
            if name != pool.name
                && llm_pools::Entity::find()
                    .filter(llm_pools::Column::Name.eq(name.clone()))
                    .one(&state.db)
                    .await?
                    .is_some()
            {
                return Err(LlmAdminError::NameConflict);
            }
            name
        }
        None => pool.name.clone(),
    };
    if let Some(members) = &req.members {
        validate_members(&state, members).await?;
    }

    let txn = state.db.begin().await?;
    let mut am: llm_pools::ActiveModel = pool.clone().into();
    am.name = Set(name);
    if let Some(d) = &req.description {
        am.description = Set(d.trim().to_string());
    }
    am.updated_at = Set(Utc::now());
    <llm_pools::ActiveModel as sea_orm::ActiveModelTrait>::update(am, &txn).await?;
    if let Some(members) = req.members {
        replace_members(&txn, id, members).await?;
    }
    txn.commit().await?;

    Ok(Json(load_pool_resp(&state, id).await?))
}

/// `DELETE /api/admin/llm/pools/:id`
///
/// Members go with the pool (FK cascade). Assignments pointing at it are
/// deleted too, mirroring provider deletion: leaving them behind would keep
/// a dangling reference that silently resolves to nothing.
pub async fn delete_pool(
    _admin: AdminUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, LlmAdminError> {
    let pool = llm_pools::Entity::find_by_id(id)
        .one(&state.db)
        .await?
        .ok_or(LlmAdminError::NotFound)?;

    let mut keys: Vec<String> = vec![LLM_DEFAULT_KEY.to_string()];
    keys.extend(LLM_OPERATIONS.iter().map(|op| llm_op_key(op)));
    for key in keys {
        if let Some(setting) = app_settings::Entity::find_by_id(key.clone())
            .one(&state.db)
            .await?
            && let Some(a) = arena_core::llm::resolve::parse_assignment(&setting.value)
            && a.pool_id() == Some(id)
        {
            app_settings::Entity::delete_by_id(key)
                .exec(&state.db)
                .await?;
        }
    }

    llm_pools::Entity::delete_by_id(pool.id)
        .exec(&state.db)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}
