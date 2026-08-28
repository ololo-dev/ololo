//! `/api/categories/*` and `/api/admin/categories/*` handlers.
//!
//! Routes (mounted by `lib.rs`):
//!   GET    /api/categories                        — list all (public, no auth)
//!   POST   /api/admin/categories                  — create a category (admin only)
//!   PATCH  /api/admin/categories/reorder          — set display order (admin only)
//!   PATCH  /api/admin/categories/:id              — rename a category (admin only)
//!   DELETE /api/admin/categories/:id              — delete a category (admin only)
//!
//! Note: `projects.category` stores the category **name**, not a foreign key,
//! so a rename must rewrite every project carrying the old name — otherwise
//! those projects fall out of the category sidebar. `patch_one` does both
//! inside one transaction.

use crate::api::settings::AdminUser;
use crate::state::AppState;
use arena_core::entities::{categories, projects};
use axum::Json;
use axum::extract::{Path, State};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect, Set,
    TransactionTrait,
};
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// DTOs
// ---------------------------------------------------------------------------

#[derive(Serialize)]
pub struct CategoryDto {
    pub id: i32,
    pub name: String,
    /// Projects currently carrying this category name. Lets the admin UI warn
    /// before a rename or delete that touches live projects. `0` on responses
    /// that don't compute it (create/rename).
    #[serde(default)]
    pub project_count: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateCategoryBody {
    pub name: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpdateCategoryBody {
    pub name: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReorderCategoriesBody {
    pub ids: Vec<i32>,
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
pub enum CategoryError {
    #[error("not found")]
    NotFound,
    #[error("name already exists")]
    Duplicate,
    #[error("name must be 1–100 non-blank characters")]
    InvalidName,
    #[error("database error")]
    Database,
}

crate::api::error::impl_api_error!(CategoryError {
    Self::NotFound => (NOT_FOUND, "not_found"),
    Self::Duplicate => (CONFLICT, "duplicate"),
    Self::InvalidName => (UNPROCESSABLE_ENTITY, "invalid_name"),
    Self::Database => (INTERNAL_SERVER_ERROR, "database_error"),
});

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Find-or-create a category by name, appending it at the end of the ordinal
/// sequence. Used by seeding/reseeding so a project definition can introduce
/// a new category. A concurrent duplicate insert is treated as success.
pub(crate) async fn ensure_category(
    db: &sea_orm::DatabaseConnection,
    name: &str,
) -> Result<(), sea_orm::DbErr> {
    use sea_orm::{ColumnTrait, QueryFilter};

    let exists = categories::Entity::find()
        .filter(categories::Column::Name.eq(name))
        .one(db)
        .await?
        .is_some();
    if exists {
        return Ok(());
    }

    let last = categories::Entity::find()
        .order_by_desc(categories::Column::Ordinal)
        .one(db)
        .await?;
    let next_ordinal = last.map_or(0, |m| m.ordinal + 1);

    match (categories::ActiveModel {
        name: Set(name.to_string()),
        ordinal: Set(next_ordinal),
        ..Default::default()
    }
    .insert(db)
    .await)
    {
        Ok(_) => Ok(()),
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("UNIQUE") || msg.contains("unique") || msg.contains("duplicate") {
                Ok(())
            } else {
                Err(e)
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// `GET /api/categories` — list all categories ordered by `ordinal`.
pub async fn get_list(
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, CategoryError> {
    let rows = categories::Entity::find()
        .order_by_asc(categories::Column::Ordinal)
        .all(&state.db)
        .await
        .map_err(|_| CategoryError::Database)?;

    // Usage counts: `projects.category` holds the name, so tally in memory
    // (the admin list is small) rather than a per-row COUNT query.
    let used: Vec<Option<String>> = projects::Entity::find()
        .select_only()
        .column(projects::Column::Category)
        .into_tuple()
        .all(&state.db)
        .await
        .unwrap_or_default();
    let mut counts: std::collections::HashMap<String, u32> = std::collections::HashMap::new();
    for name in used.into_iter().flatten() {
        *counts.entry(name).or_insert(0) += 1;
    }

    let dtos: Vec<CategoryDto> = rows
        .into_iter()
        .map(|r| CategoryDto {
            project_count: counts.get(&r.name).copied().unwrap_or(0),
            id: r.id,
            name: r.name,
        })
        .collect();

    Ok(Json(serde_json::json!({ "categories": dtos })))
}

/// `PATCH /api/admin/categories/:id` — rename a category (admin only).
///
/// Cascades the new name to every project that carries the old one, in the
/// same transaction: `projects.category` is a name string, not an FK, so
/// renaming without the rewrite would orphan those projects.
pub async fn patch_one(
    _admin: AdminUser,
    State(state): State<AppState>,
    Path(id): Path<i32>,
    Json(body): Json<UpdateCategoryBody>,
) -> Result<Json<serde_json::Value>, CategoryError> {
    let name = body.name.trim().to_string();
    if name.is_empty() || name.chars().count() > 100 {
        return Err(CategoryError::InvalidName);
    }

    let existing = categories::Entity::find_by_id(id)
        .one(&state.db)
        .await
        .map_err(|_| CategoryError::Database)?
        .ok_or(CategoryError::NotFound)?;
    let old_name = existing.name.clone();

    // Unchanged (including case/whitespace-only edits) — nothing to do.
    if old_name == name {
        return Ok(Json(serde_json::json!({
            "category": CategoryDto { id, name, project_count: 0 },
            "projects_updated": 0,
        })));
    }

    // Reject a collision with a *different* category up front; the unique
    // index is still the backstop under concurrent renames.
    let clash = categories::Entity::find()
        .filter(categories::Column::Name.eq(name.clone()))
        .one(&state.db)
        .await
        .map_err(|_| CategoryError::Database)?;
    if clash.is_some_and(|c| c.id != id) {
        return Err(CategoryError::Duplicate);
    }

    let txn = state
        .db
        .begin()
        .await
        .map_err(|_| CategoryError::Database)?;

    let mut am: categories::ActiveModel = existing.into();
    am.name = Set(name.clone());
    am.update(&txn).await.map_err(|e| {
        let msg = e.to_string();
        if msg.contains("UNIQUE") || msg.contains("unique") || msg.contains("duplicate") {
            CategoryError::Duplicate
        } else {
            CategoryError::Database
        }
    })?;

    let updated = projects::Entity::update_many()
        .col_expr(
            projects::Column::Category,
            sea_orm::sea_query::Expr::value(name.clone()),
        )
        .filter(projects::Column::Category.eq(old_name))
        .exec(&txn)
        .await
        .map_err(|_| CategoryError::Database)?;

    txn.commit().await.map_err(|_| CategoryError::Database)?;

    Ok(Json(serde_json::json!({
        "category": CategoryDto { id, name, project_count: updated.rows_affected as u32 },
        "projects_updated": updated.rows_affected,
    })))
}

/// `POST /api/admin/categories` — create a category (admin only).
///
/// The new category is appended at the end of the current ordinal sequence.
pub async fn post_create(
    _admin: AdminUser,
    State(state): State<AppState>,
    Json(body): Json<CreateCategoryBody>,
) -> Result<Json<serde_json::Value>, CategoryError> {
    let name = body.name.trim().to_string();
    if name.is_empty() || name.chars().count() > 100 {
        return Err(CategoryError::InvalidName);
    }

    // Append after the current last item by ordinal.
    let last = categories::Entity::find()
        .order_by_desc(categories::Column::Ordinal)
        .one(&state.db)
        .await
        .map_err(|_| CategoryError::Database)?;
    let next_ordinal = last.map_or(0, |m| m.ordinal + 1);

    let model = categories::ActiveModel {
        name: Set(name.clone()),
        ordinal: Set(next_ordinal),
        ..Default::default()
    }
    .insert(&state.db)
    .await
    .map_err(|e| {
        // sea-orm surfaces unique constraint violations as DbErr::Query
        let msg = e.to_string();
        if msg.contains("UNIQUE") || msg.contains("unique") || msg.contains("duplicate") {
            CategoryError::Duplicate
        } else {
            CategoryError::Database
        }
    })?;

    Ok(Json(serde_json::json!({
        "category": CategoryDto { id: model.id, name: model.name, project_count: 0 }
    })))
}

/// `PATCH /api/admin/categories/reorder` — set display order (admin only).
///
/// Body: `{ "ids": [id, id, ...] }` — full ordered list of category IDs.
/// Each category's `ordinal` is set to its 0-based position in the list.
pub async fn patch_reorder(
    _admin: AdminUser,
    State(state): State<AppState>,
    Json(body): Json<ReorderCategoriesBody>,
) -> Result<Json<serde_json::Value>, CategoryError> {
    let txn = state
        .db
        .begin()
        .await
        .map_err(|_| CategoryError::Database)?;

    for (ordinal, id) in body.ids.iter().enumerate() {
        let row = categories::Entity::find_by_id(*id)
            .one(&txn)
            .await
            .map_err(|_| CategoryError::Database)?
            .ok_or(CategoryError::NotFound)?;

        let mut active: categories::ActiveModel = row.into();
        active.ordinal = Set(ordinal as i32);
        active
            .update(&txn)
            .await
            .map_err(|_| CategoryError::Database)?;
    }

    txn.commit().await.map_err(|_| CategoryError::Database)?;

    Ok(Json(serde_json::json!({ "ok": true })))
}

/// `DELETE /api/admin/categories/:id` — delete a category (admin only).
pub async fn delete_one(
    _admin: AdminUser,
    State(state): State<AppState>,
    Path(id): Path<i32>,
) -> Result<Json<serde_json::Value>, CategoryError> {
    let result = categories::Entity::delete_by_id(id)
        .exec(&state.db)
        .await
        .map_err(|_| CategoryError::Database)?;

    if result.rows_affected == 0 {
        return Err(CategoryError::NotFound);
    }

    Ok(Json(serde_json::json!({ "ok": true })))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::StatusCode;
    use axum::response::IntoResponse;

    #[test]
    fn category_error_not_found_is_404() {
        let resp = CategoryError::NotFound.into_response();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[test]
    fn category_error_duplicate_is_409() {
        let resp = CategoryError::Duplicate.into_response();
        assert_eq!(resp.status(), StatusCode::CONFLICT);
    }

    #[test]
    fn category_error_invalid_name_is_422() {
        let resp = CategoryError::InvalidName.into_response();
        assert_eq!(resp.status(), StatusCode::UNPROCESSABLE_ENTITY);
    }
}
