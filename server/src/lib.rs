//! Server library — exposes modules for integration tests and binaries.
//!
//! The actual binary entry point lives in `main.rs` and pulls the same
//! modules through `pub use` re-exports.

pub mod adaptation;
pub mod api;
pub mod auth;
pub mod campaign_link;
pub mod client_ip;
pub mod db;
pub mod email;
pub use arena_core::entities;
pub use arena_core::join_code;
pub mod git_store;
pub mod llm;
pub use arena_core::probe_engine;
pub use arena_core::protocol;
pub mod rate_limiter;
pub mod routes;
pub mod scoring;
pub mod seed;
pub mod state;
pub use arena_core::task_template;
pub use arena_core::util;
pub use arena_core::validation;
pub mod ws;
pub mod zmq_sub;

pub use state::{AppState, AuthConfig};

use axum::{
    Router,
    extract::DefaultBodyLimit,
    http::HeaderName,
    middleware,
    response::IntoResponse,
    routing::{get, patch, post},
};
use tower_http::{
    cors::CorsLayer,
    request_id::{MakeRequestUuid, PropagateRequestIdLayer, SetRequestIdLayer},
    trace::TraceLayer,
};

/// HTTP header used to carry a per-request trace ID.
/// Returned to callers as `X-Request-Id` so logs can be correlated with
/// client-side errors.
const REQUEST_ID_HEADER: &str = "x-request-id";

/// Construct the application router. Exposed for integration tests.
pub fn build_router(state: AppState) -> Router {
    let request_id_header = HeaderName::from_static(REQUEST_ID_HEADER);

    // Profile rate limiter: env-configurable RPS, default 20 per IP per second.
    let rps: usize = std::env::var("ARENA_PROFILE_RATE_LIMIT_RPS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(20);
    let profile_limiter = std::sync::Arc::new(crate::rate_limiter::ProfileRateLimiter::new(rps));

    let profile_routes: Router<AppState> = Router::new()
        .route(
            "/api/users/by-username/:username",
            get(api::users::get_by_username),
        )
        .route(
            "/api/users/by-username/:username/sessions",
            get(api::users::get_sessions_by_username),
        )
        .layer(middleware::from_fn(
            move |req: axum::extract::Request, next: middleware::Next| {
                let limiter = profile_limiter.clone();
                async move {
                    let peer = req
                        .extensions()
                        .get::<axum::extract::ConnectInfo<std::net::SocketAddr>>()
                        .map(|ci| ci.0.ip());
                    let ip = client_ip::client_ip_addr(req.headers(), peer)
                        .unwrap_or(std::net::IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED));
                    if !limiter.check_and_record(ip) {
                        return axum::http::StatusCode::TOO_MANY_REQUESTS.into_response();
                    }
                    next.run(req).await
                }
            },
        ));

    Router::new()
        .merge(profile_routes)
        .route("/health", get(routes::health))
        .route(
            "/api/public/turnstile",
            get(api::public::get_turnstile_config),
        )
        .route("/api/public/plans", get(api::public::get_public_plans))
        .route(
            "/api/public/active-sessions",
            get(api::public::get_active_sessions),
        )
        .route("/api/public/judges", get(api::public::get_public_judges))
        .route(
            "/api/sessions",
            get(api::sessions::get_list).post(api::sessions::post_create),
        )
        .route(
            "/api/sessions/by-code/:join_code",
            get(api::sessions::get_by_code),
        )
        .route(
            "/api/sessions/by-code/:join_code/campaign",
            get(api::sessions::get_campaign_by_code),
        )
        .route(
            "/api/sessions/:id",
            get(api::sessions::get_one)
                .patch(api::sessions::patch_one)
                .delete(api::sessions::delete_one),
        )
        .route("/api/sessions/:id/report", get(api::sessions::get_report))
        .route(
            "/api/sessions/:id/artifacts/:probe_id",
            get(api::sessions::get_session_artifact),
        )
        .route(
            "/api/sessions/:id/activity",
            get(api::sessions::get_activity),
        )
        .route(
            "/api/sessions/:id/player-stats",
            get(api::sessions::get_player_stats),
        )
        .route(
            "/api/sessions/:session_id/members",
            get(api::members::get_list).post(api::members::post_add),
        )
        .route(
            "/api/sessions/:session_id/members/:user_id",
            axum::routing::delete(api::members::delete_one),
        )
        .route(
            "/api/sessions/:session_id/regenerate-code",
            post(api::sessions::post_regenerate_code),
        )
        .route("/api/sessions/join", post(api::sessions::post_join))
        .route("/api/sessions/resolve", get(api::resolve::resolve_session))
        .route(
            "/api/sessions/:session_id/agents",
            get(api::agents::get_list).post(api::agents::post_register),
        )
        .route(
            "/api/sessions/:session_id/agents/:agent_id",
            get(api::agents::get_one).delete(api::agents::delete_one),
        )
        .route(
            "/api/sessions/:session_id/tasks",
            get(api::tasks::get_list).post(api::tasks::post_create),
        )
        .route(
            "/api/sessions/:session_id/tasks/:task_id",
            get(api::tasks::get_one)
                .patch(api::tasks::patch_one)
                .delete(api::tasks::delete_one),
        )
        .route(
            "/api/projects",
            get(api::projects::get_list).post(api::projects::post_create),
        )
        .route(
            "/api/projects/by-slug/:slug",
            get(api::projects::get_by_slug),
        )
        .route(
            "/api/projects/u/:user_id/by-slug/:slug",
            get(api::projects::get_by_user_slug),
        )
        .route(
            "/api/projects/categories",
            get(api::projects::get_categories),
        )
        .route("/api/categories", get(api::categories::get_list))
        .route("/api/admin/categories", post(api::categories::post_create))
        .route(
            "/api/admin/categories/reorder",
            patch(api::categories::patch_reorder),
        )
        .route(
            "/api/admin/categories/:id",
            patch(api::categories::patch_one).delete(api::categories::delete_one),
        )
        .route(
            "/api/projects/:project_id",
            get(api::projects::get_one)
                .patch(api::projects::patch_one)
                .delete(api::projects::delete_one),
        )
        .route(
            "/api/projects/:project_id/sessions",
            get(api::sessions::get_project_sessions),
        )
        .route(
            "/api/projects/:project_id/judges",
            get(api::projects::get_judges),
        )
        .route(
            "/api/projects/:project_id/parts",
            get(api::projects::get_parts),
        )
        .route(
            "/api/projects/:project_id/previous-part-source",
            get(api::campaign::get_previous_part_source),
        )
        .route(
            "/api/projects/:project_id/top-players",
            get(api::projects::get_top_players),
        )
        .route(
            "/api/projects/:project_id/tasks",
            get(api::project_tasks::get_list).post(api::project_tasks::post_create),
        )
        .route(
            "/api/projects/:project_id/tasks/preview",
            get(api::project_tasks::get_preview),
        )
        .route(
            "/api/projects/:project_id/tasks/reorder",
            patch(api::project_tasks::reorder),
        )
        .route(
            "/api/projects/:project_id/tasks/:task_id",
            get(api::project_tasks::get_one)
                .patch(api::project_tasks::patch_one)
                .delete(api::project_tasks::delete_one),
        )
        .route(
            "/api/projects/:project_id/beautify-description",
            post(api::project_ai::beautify_description),
        )
        .route(
            "/api/projects/:project_id/generate-tasks",
            post(api::project_ai::generate_tasks),
        )
        .route(
            "/api/projects/:project_id/tasks/ai/beautify",
            post(api::task_ai::project_beautify_task_description),
        )
        .route(
            "/api/projects/:project_id/tasks/ai/generate",
            post(api::task_ai::project_generate_task_tests),
        )
        .route(
            "/api/projects/:project_id/tasks/:task_id/ai/beautify",
            post(api::task_ai::task_beautify_description),
        )
        .route(
            "/api/projects/:project_id/tasks/:task_id/ai/generate",
            post(api::task_ai::task_generate_tests),
        )
        .route(
            "/api/projects/:project_id/tasks/:task_id/judges",
            get(api::task_judges::list).post(api::task_judges::attach),
        )
        .route(
            "/api/projects/:project_id/tasks/:task_id/judges/:judge_id",
            axum::routing::put(api::task_judges::update).delete(api::task_judges::detach),
        )
        .route("/auth/verify-email", get(api::email_auth::get_verify_email))
        .route(
            "/auth/verify-email/resend",
            post(api::email_auth::post_resend_verification),
        )
        .route(
            "/auth/forgot-password",
            post(api::email_auth::post_forgot_password),
        )
        .route(
            "/auth/reset-password",
            post(api::email_auth::post_reset_password),
        )
        .route(
            "/auth/magic-link/request",
            post(api::email_auth::post_magic_link_request),
        )
        .route(
            "/auth/magic-link/verify",
            get(api::email_auth::get_magic_link_verify),
        )
        .route("/auth/register", post(api::users::post_register))
        .route("/auth/login", post(api::users::post_login))
        .route("/auth/refresh", post(api::users::post_refresh))
        .route("/auth/logout", post(api::users::post_logout))
        .route("/auth/cli/post-auth", get(api::cli_auth::get_cli_post_auth))
        .route("/auth/cli/confirm", post(api::cli_auth::post_cli_confirm))
        .route(
            "/api/users/me",
            get(api::users::get_me).patch(api::users::patch_me),
        )
        .route(
            "/api/users/me/password",
            post(api::users::post_change_password),
        )
        .route(
            "/api/users/me/avatar-auth",
            get(api::users::get_avatar_auth),
        )
        .route("/api/users/me/pats", get(api::pats::get_list))
        .route(
            "/api/users/me/pats/:id",
            axum::routing::delete(api::pats::delete_one),
        )
        .route(
            "/api/admin/projects/import",
            post(api::admin_export_import::import_project)
                .layer(DefaultBodyLimit::max(10 * 1024 * 1024)),
        )
        .route(
            "/api/admin/projects/:id/export",
            get(api::admin_export_import::export_project),
        )
        .route(
            "/api/admin/projects/:id/reseed",
            post(api::admin_export_import::reseed_project),
        )
        .route(
            "/api/admin/projects/apply-seed",
            post(api::admin_export_import::apply_seed),
        )
        .route(
            "/api/admin/settings",
            get(api::settings::get_settings).put(api::settings::put_settings),
        )
        .route(
            "/api/admin/analytics/costs",
            get(api::analytics::get_costs_summary),
        )
        .route(
            "/api/admin/sessions/:id/costs",
            get(api::analytics::get_session_costs),
        )
        .route(
            "/api/admin/email-templates",
            get(api::email_templates::get_email_templates),
        )
        .route(
            "/api/admin/email-templates/:type",
            axum::routing::put(api::email_templates::put_email_template),
        )
        .route(
            "/api/admin/users",
            get(api::settings::get_admin_users).post(api::settings::post_admin_user),
        )
        .route(
            "/api/admin/users/:id",
            patch(api::settings::patch_admin_user).delete(api::settings::delete_admin_user),
        )
        .route("/api/admin/sessions", get(api::sessions::get_admin_list))
        .route("/api/admin/game-servers", get(api::game_servers::get_list))
        .route(
            "/api/admin/judges",
            get(api::judges::list).post(api::judges::create),
        )
        .route("/api/admin/judges/sync", post(api::judges::sync))
        .route("/api/admin/judges/usage", get(api::judges::usage))
        .route(
            "/api/admin/llm/providers",
            get(api::llm_admin::list_providers).post(api::llm_admin::create_provider),
        )
        .route(
            "/api/admin/llm/providers/:id",
            axum::routing::put(api::llm_admin::update_provider)
                .delete(api::llm_admin::delete_provider),
        )
        .route(
            "/api/admin/llm/providers/:id/models",
            get(api::llm_admin::list_provider_row_models),
        )
        .route(
            "/api/admin/llm/providers/:id/test",
            post(api::llm_admin::test_provider),
        )
        .route(
            "/api/admin/llm/assignments",
            get(api::llm_admin::get_assignments).put(api::llm_admin::put_assignments),
        )
        .route(
            "/api/admin/llm/pools",
            get(api::llm_pools::list_pools).post(api::llm_pools::create_pool),
        )
        .route(
            "/api/admin/llm/pools/:id",
            get(api::llm_pools::get_pool)
                .put(api::llm_pools::update_pool)
                .delete(api::llm_pools::delete_pool),
        )
        .route(
            "/api/admin/llm/telemetry",
            get(api::llm_admin::list_llm_telemetry),
        )
        .route(
            "/api/admin/llm/telemetry/:id",
            get(api::llm_admin::get_llm_telemetry),
        )
        .route(
            "/api/admin/judges/:id",
            get(api::judges::get_one)
                .put(api::judges::update)
                .delete(api::judges::delete),
        )
        .route(
            "/api/admin/game-servers/:id",
            patch(api::game_servers::patch_one),
        )
        .route(
            "/api/admin/sessions/:code/players/:player_id/judge-runs",
            axum::routing::post(api::judge_runs::post_run),
        )
        .route(
            "/api/admin/judge-results/:id",
            get(api::judge_runs::get_result_details),
        )
        .route(
            "/api/admin/sessions/:code/judge-usage",
            get(api::judge_runs::get_session_judge_usage),
        )
        .route(
            "/api/admin/sessions/:code/players/:player_id/tasks/:task_id/judge-log",
            get(api::judge_runs::get_player_task_judge_log),
        )
        .route(
            "/api/admin/sessions/:code/players/:player_id/event-log",
            get(api::judge_runs::get_player_event_log),
        )
        .route(
            "/api/sessions/:code/players/metadata",
            get(api::session_metadata::get_session_players_metadata),
        )
        .route(
            "/api/sessions/:code/players/:player_id",
            get(api::players::get_player_snapshot)
                .patch(api::session_players::patch_player_metadata)
                .layer(DefaultBodyLimit::max(4224)),
        )
        .route(
            "/api/sessions/:code/players/:player_id/history",
            get(api::players::get_player_history),
        )
        .route(
            "/api/sessions/:code/players/:player_id/memory",
            get(api::players::get_player_memory),
        )
        .route(
            "/api/sessions/:code/players/:player_id/artifacts/:probe_id",
            get(api::players::get_player_artifact),
        )
        .route(
            "/api/sessions/:code/players/:player_id/repo-file",
            get(api::players::get_player_repo_file),
        )
        .route(
            "/api/sessions/:code/players/:player_id/task-stats",
            get(api::task_stats::get_task_stats)
                .post(api::task_stats::post_task_stats)
                .layer(DefaultBodyLimit::max(256 * 1024)),
        )
        .route("/ws/cli-auth", get(ws::cli_auth::ws_cli_auth_handler))
        // git smart-HTTP bridge for per-player remote repos. Serves bare
        // repos under OLOLO_GIT_REPOS_DIR via `git http-backend` CGI. Auth
        // is PAT-based (Authorization: Bearer or X-API-Key).
        .route(
            "/git/:session_id/:player_id.git/*rest",
            axum::routing::any(ws::git_http_handler)
                .layer(DefaultBodyLimit::max(256 * 1024 * 1024)),
        )
        // observe route must precede :join_code to prevent wildcard capture
        .route("/ws/s/:join_code/observe", get(ws::ws_observe_handler))
        .route("/ws/s/:join_code", get(ws::session::ws_session_handler))
        .route(
            "/ws/project/:project_id/observe",
            get(ws::ws_project_observe_handler),
        )
        .route("/ws/landing/observe", get(ws::ws_landing_observe_handler))
        .route("/ws/admin", get(ws::ws_admin_handler))
        .route("/ws/admin/zmq", get(ws::ws_zmq_events_handler))
        .route("/ws/player/:player_id", get(ws::ws_player_handler))
        // Layer order: innermost first. Tower applies layers bottom-up so
        // the LAST layer added is outermost and processes requests first.
        //
        //  Request path (outermost → innermost):
        //    SetRequestId → PropagateRequestId → TraceLayer → CORS → origin_guard → handler
        //
        //  Response path (innermost → outermost):
        //    handler → origin_guard → CORS → TraceLayer → PropagateRequestId → SetRequestId
        //
        // SetRequestId generates the UUID and stamps it onto the request
        // headers so TraceLayer can read it when building the span.
        // PropagateRequestId copies it onto every outgoing response so
        // clients can correlate their requests with server-side log lines.
        .layer(middleware::from_fn_with_state(
            state.clone(),
            auth::middleware::origin_guard,
        ))
        .layer(CorsLayer::permissive())
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(|req: &axum::http::Request<_>| {
                    let request_id = req
                        .headers()
                        .get(REQUEST_ID_HEADER)
                        .and_then(|v| v.to_str().ok())
                        .unwrap_or("-");
                    tracing::info_span!(
                        "http",
                        method  = %req.method(),
                        path    = req.uri().path(),
                        request_id = %request_id,
                    )
                })
                .on_response(
                    |resp: &axum::http::Response<_>,
                     latency: std::time::Duration,
                     _span: &tracing::Span| {
                        tracing::info!(
                            status = resp.status().as_u16(),
                            latency_ms = latency.as_millis() as u64,
                            "response"
                        );
                    },
                )
                .on_failure(tower_http::trace::DefaultOnFailure::new().level(tracing::Level::WARN)),
        )
        .layer(PropagateRequestIdLayer::new(request_id_header.clone()))
        .layer(SetRequestIdLayer::new(request_id_header, MakeRequestUuid))
        .with_state(state)
}
