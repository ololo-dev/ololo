pub mod analysis;
pub mod api;
pub mod artifacts;
pub mod heartbeat;
pub mod idle_sweep;
pub mod judge_exec;
pub mod judge_log_store;
pub mod judge_queue;
pub mod judge_registrar;
pub mod probe_exec;
pub mod probe_scheduler;
pub mod recovery;
pub mod session_log_store;
pub mod session_memory;
pub mod similarity;
pub mod state;
#[cfg(test)]
pub(crate) mod test_fixtures;
pub mod ws;
pub mod zmq_pub;

use axum::Router;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use state::GameServerState;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;

pub fn build_router(state: GameServerState) -> Router {
    // `/internal/*` shares the game server's public listener with the player
    // WS, so it is service-to-service authenticated with a shared token
    // derived from JWT_SIGNING_KEY (see arena_core::auth::internal_api_token).
    let internal = Router::new()
        .route(
            "/internal/judge-run",
            axum::routing::post(api::judge_run::post_run),
        )
        .route(
            "/internal/judge-logs/:session_id/:player_id/:task_id",
            axum::routing::get(api::judge_logs::get_task_log),
        )
        .route(
            "/internal/session-logs/:session_id/:player_id",
            axum::routing::get(api::session_logs::get_player_log),
        )
        .route_layer(axum::middleware::from_fn_with_state(
            state.clone(),
            require_internal_auth,
        ));

    Router::new()
        .route(
            "/ws/player/agent/:join_code",
            axum::routing::get(ws::player_agent::ws_player_agent_handler),
        )
        .route(
            "/ws/s/:join_code",
            axum::routing::get(ws::session_lifecycle::ws_cli_session_handler),
        )
        .route("/health", axum::routing::get(health))
        .merge(internal)
        .fallback(fallback_handler)
        .layer(
            tower::ServiceBuilder::new()
                .layer(axum::middleware::from_fn(log_request))
                .layer(TraceLayer::new_for_http())
                .layer(CorsLayer::permissive()),
        )
        .with_state(state)
}

/// Reject any `/internal/*` request that does not carry a valid
/// `X-Internal-Auth` token (derived from the shared `JWT_SIGNING_KEY`).
async fn require_internal_auth(
    axum::extract::State(state): axum::extract::State<GameServerState>,
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    let presented = req
        .headers()
        .get(arena_core::auth::INTERNAL_API_HEADER)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if arena_core::auth::verify_internal_api_token(state.jwt_signing_secret.as_slice(), presented) {
        next.run(req).await
    } else {
        tracing::warn!(path = %req.uri().path(), "rejected /internal request: bad or missing X-Internal-Auth");
        StatusCode::UNAUTHORIZED.into_response()
    }
}

async fn fallback_handler(uri: axum::http::Uri) -> (StatusCode, String) {
    tracing::warn!(path = %uri, "fallback: no route matched");
    (StatusCode::NOT_FOUND, format!("no route: {}", uri))
}

async fn log_request(
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    let method = req.method().clone();
    let path = req.uri().path().to_string();
    let response = next.run(req).await;
    let status = response.status();
    tracing::info!(%method, %path, %status, "HTTP request");
    response
}

async fn health() -> &'static str {
    "ok"
}
