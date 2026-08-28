//! Campaign carry-over discovery for the CLI.
//!
//! Part N+1 of a campaign continues part N's codebase. The player's part-N
//! work already lives on the server as a per-(session, player) snapshot repo
//! served over git smart-HTTP, and their PAT already authorizes cloning it —
//! what the CLI cannot do on its own is find *which* repo, given only the
//! project it is about to start. This endpoint answers exactly that.
//!
//! PAT auth via `X-API-Key`, mirroring `/api/sessions/resolve`: the CLI has a
//! PAT in hand at this point in the flow, not a browser session.

use axum::{
    Json,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use serde::Serialize;
use uuid::Uuid;

use crate::state::AppState;
use arena_core::entities::projects;

#[derive(Serialize)]
pub struct PreviousPartSource {
    /// Slug of the part the workspace should be seeded from.
    pub prev_project_slug: Option<String>,
    pub prev_part_ordinal: i32,
    pub session_id: Uuid,
    pub player_id: Uuid,
    /// Path of the player's snapshot repo, to be appended to the server base
    /// URL: `/git/{session_id}/{player_id}.git`.
    pub git_remote_path: String,
}

#[derive(Serialize)]
struct ErrorBody {
    error: &'static str,
}

fn err(status: StatusCode, error: &'static str) -> Response {
    (status, Json(ErrorBody { error })).into_response()
}

/// `GET /api/projects/:project_id/previous-part-source`
///
/// Where to fetch the caller's work from for this campaign part:
/// the most recent session of the preceding part that the caller actually
/// completed. 404 with a reason when there is nothing to carry over — the
/// first part of a campaign, an ordinary project, or a predecessor the caller
/// never finished (in which case session creation would refuse anyway).
pub async fn get_previous_part_source(
    State(state): State<AppState>,
    Path(project_id): Path<Uuid>,
    headers: HeaderMap,
) -> Response {
    let Some(pat) = headers.get("X-API-Key").and_then(|v| v.to_str().ok()) else {
        return err(StatusCode::UNAUTHORIZED, "missing_api_key");
    };
    let Ok(claims) = crate::auth::pat::lookup_pat(&state.db, pat).await else {
        return err(StatusCode::UNAUTHORIZED, "invalid_api_key");
    };
    let Ok(user_id) = claims.sub.parse::<Uuid>() else {
        return err(StatusCode::UNAUTHORIZED, "invalid_user");
    };

    let project = match projects::Entity::find_by_id(project_id)
        .one(&state.db)
        .await
    {
        Ok(Some(p)) => p,
        Ok(None) => return err(StatusCode::NOT_FOUND, "project_not_found"),
        Err(e) => {
            tracing::error!(error = %e, "previous-part-source: project lookup failed");
            return err(StatusCode::INTERNAL_SERVER_ERROR, "database_error");
        }
    };

    let (Some(parent_id), Some(ordinal)) = (project.parent_project_id_fk, project.part_ordinal)
    else {
        return err(StatusCode::NOT_FOUND, "no_previous_part");
    };
    if ordinal <= 0 {
        return err(StatusCode::NOT_FOUND, "no_previous_part");
    }

    let siblings = match projects::Entity::find()
        .filter(projects::Column::ParentProjectIdFk.eq(parent_id))
        .all(&state.db)
        .await
    {
        Ok(rows) => rows,
        Err(e) => {
            tracing::error!(error = %e, "previous-part-source: sibling lookup failed");
            return err(StatusCode::INTERNAL_SERVER_ERROR, "database_error");
        }
    };
    let Some(previous) = arena_core::campaign::previous_part(&siblings, ordinal, |p| {
        p.part_ordinal.unwrap_or(i32::MIN)
    }) else {
        return err(StatusCode::NOT_FOUND, "no_previous_part");
    };

    match arena_core::campaign::latest_completing_session(&state.db, previous.id, user_id).await {
        Ok(Some((session_id, player_id))) => Json(PreviousPartSource {
            prev_project_slug: previous.slug.clone(),
            prev_part_ordinal: previous.part_ordinal.unwrap_or(0),
            session_id,
            player_id,
            git_remote_path: format!("/git/{session_id}/{player_id}.git"),
        })
        .into_response(),
        Ok(None) => err(StatusCode::NOT_FOUND, "not_completed"),
        Err(e) => {
            tracing::error!(error = %e, "previous-part-source: completion lookup failed");
            err(StatusCode::INTERNAL_SERVER_ERROR, "database_error")
        }
    }
}
