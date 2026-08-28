use crate::auth::jwt::AccessClaims;
use crate::state::AppState;
use arena_core::join_code;
use axum::Json;
use axum::extract::Path;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use sea_orm::{ConnectionTrait, DbErr, Statement, TransactionTrait};
use uuid::Uuid;

use super::common::*;

#[tracing::instrument(level = "info", skip_all, fields(session_id = %id))]
pub async fn post_regenerate_code(
    State(state): State<AppState>,
    claims: AccessClaims,
    Path(id): Path<Uuid>,
) -> Result<Response, SessionError> {
    let user_id = parse_user_id(&claims)?;
    // Load and enforce owner-only (FR-JC-015).
    let row = load_for_owner(&state.db, id, user_id).await?;

    let current_code = row.join_code.clone();
    let new_code = join_code::generate();
    let new_code_clone = new_code.clone();

    // Optimistic lock within a transaction (FR-JC-017).
    let rows_affected = state
        .db
        .transaction::<_, u64, DbErr>(|txn| {
            Box::pin(async move {
                let backend = txn.get_database_backend();
                let sql = match backend {
                    sea_orm::DatabaseBackend::Postgres => {
                        "UPDATE sessions SET join_code = $1 WHERE id = $2 AND join_code = $3"
                    }
                    _ => "UPDATE sessions SET join_code = ? WHERE id = ? AND join_code = ?",
                };
                let result = txn
                    .execute(Statement::from_sql_and_values(
                        backend,
                        sql,
                        [
                            new_code_clone.clone().into(),
                            id.into(),
                            current_code.clone().into(),
                        ],
                    ))
                    .await?;
                Ok(result.rows_affected())
            })
        })
        .await
        .map_err(|e| match e {
            sea_orm::TransactionError::Transaction(db_err) => SessionError::Db(db_err),
            sea_orm::TransactionError::Connection(db_err) => SessionError::Db(db_err),
        })?;

    if rows_affected == 0 {
        return Err(SessionError::AlreadyRegenerated);
    }

    Ok((
        StatusCode::OK,
        Json(serde_json::json!({ "join_code": new_code })),
    )
        .into_response())
}
