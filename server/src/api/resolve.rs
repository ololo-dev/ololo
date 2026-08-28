use axum::{
    Json,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::state::AppState;
use arena_core::session_status::SessionStatus;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResolveQuery {
    pub join_code: String,
}

#[derive(Serialize)]
pub struct ResolveResponse {
    pub game_server_url: String,
    pub session_id: Uuid,
    pub player_id: Uuid,
}

#[derive(Serialize)]
pub struct ResolveError {
    pub error: String,
}

pub async fn resolve_session(
    State(state): State<AppState>,
    axum::extract::Query(query): axum::extract::Query<ResolveQuery>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let pat = match headers.get("X-API-Key").and_then(|v| v.to_str().ok()) {
        Some(v) => v.to_string(),
        None => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(ResolveError {
                    error: "missing_api_key".to_string(),
                }),
            )
                .into_response();
        }
    };

    let claims = match crate::auth::pat::lookup_pat(&state.db, &pat).await {
        Ok(c) => c,
        Err(_) => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(ResolveError {
                    error: "invalid_api_key".to_string(),
                }),
            )
                .into_response();
        }
    };

    let user_id: Uuid = match claims.sub.parse() {
        Ok(u) => u,
        Err(_) => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(ResolveError {
                    error: "invalid_user".to_string(),
                }),
            )
                .into_response();
        }
    };

    // Join codes are stored uppercase and every other lookup normalises before
    // comparing. This one did not, so a lowercase code 404'd here — and the CLI
    // treats a 404 as "no resolve payload" and connects WITHOUT a player_id,
    // which the game server then had to guess at. That guess is how two players
    // ended up sharing one row in session VKIBCB.
    let join_code = query.join_code.trim().to_uppercase();
    let session = match arena_core::entities::sessions::Entity::find()
        .filter(arena_core::entities::sessions::Column::JoinCode.eq(&join_code))
        .one(&state.db)
        .await
    {
        Ok(Some(s)) => s,
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(ResolveError {
                    error: "session_not_found".to_string(),
                }),
            )
                .into_response();
        }
        Err(e) => {
            tracing::error!("DB error finding session: {}", e);
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ResolveError {
                    error: "internal_error".to_string(),
                }),
            )
                .into_response();
        }
    };

    if session.status == SessionStatus::Finished || session.status == SessionStatus::Cancelled {
        return (
            StatusCode::GONE,
            Json(ResolveError {
                error: "session_ended".to_string(),
            }),
        )
            .into_response();
    }

    let player = arena_core::entities::players::Entity::find()
        .filter(arena_core::entities::players::Column::SessionIdFk.eq(session.id))
        .filter(arena_core::entities::players::Column::UserIdFk.eq(user_id))
        .filter(arena_core::entities::players::Column::RevokedAt.is_null())
        .one(&state.db)
        .await;

    let player = match player {
        Ok(Some(p)) => p,
        Ok(None) => {
            return (
                StatusCode::FORBIDDEN,
                Json(ResolveError {
                    error: "not_a_member".to_string(),
                }),
            )
                .into_response();
        }
        Err(e) => {
            tracing::error!("DB error finding player: {}", e);
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ResolveError {
                    error: "internal_error".to_string(),
                }),
            )
                .into_response();
        }
    };

    let game_server_id = match session.game_server_id {
        Some(id) => id,
        None => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(ResolveError {
                    error: "no_game_server".to_string(),
                }),
            )
                .into_response();
        }
    };

    let gs = match arena_core::entities::game_servers::Entity::find_by_id(game_server_id)
        .one(&state.db)
        .await
    {
        Ok(Some(gs)) => gs,
        Ok(None) => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(ResolveError {
                    error: "no_game_server".to_string(),
                }),
            )
                .into_response();
        }
        Err(e) => {
            tracing::error!("DB error finding game server: {}", e);
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ResolveError {
                    error: "internal_error".to_string(),
                }),
            )
                .into_response();
        }
    };

    if gs.status != "active" || gs.last_heartbeat < crate::api::game_servers::stale_cutoff() {
        // Either administratively disabled, or heartbeats have stopped (server
        // presumed dead) — do not hand clients a stale/dead WS URL.
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ResolveError {
                error: "no_game_server".to_string(),
            }),
        )
            .into_response();
    }

    (
        StatusCode::OK,
        Json(ResolveResponse {
            game_server_url: gs.url,
            session_id: session.id,
            player_id: player.id,
        }),
    )
        .into_response()
}
