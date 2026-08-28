use anyhow::Result;
use server::{AppState, AuthConfig, build_router};
use std::net::SocketAddr;
use std::sync::Arc;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[tokio::main]
async fn main() -> Result<()> {
    let _ = dotenvy::dotenv();
    init_tracing();

    // Docker 25+ leaves the container's soft nofile at the kernel default
    // (1024) — an evening of WS observers + keep-alive pools + git CGI put
    // the dev server over it and EMFILE took the whole API down. Use the
    // hard limit (hundreds of thousands) instead.
    if let Some((before, after)) = arena_core::rlimit::raise_nofile_limit() {
        tracing::info!(before, after, "nofile soft limit raised");
    }

    let port: u16 = std::env::var("SERVER_PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(8080);

    let db = server::db::connect().await?;
    let cfg = AuthConfig::from_env().map_err(|e| anyhow::anyhow!("auth config: {e}"))?;
    let (shutdown_tx, shutdown_rx) = tokio::sync::broadcast::channel::<()>(1);
    let state = AppState::new_with_shutdown(db.clone(), cfg, shutdown_rx);

    // Wire adaptation: channel + PassthroughAdaptationService (no LLM required
    // for local dev). Production wiring to LiveAdaptationService happens via
    // the same path once Ollama is stable.
    let (adaptation_tx, adaptation_rx) =
        tokio::sync::mpsc::channel::<server::adaptation::service::AdaptationRequest>(32);
    let adaptation_service: server::adaptation::service::AdaptationServiceHandle =
        if std::env::var("ARENA_ADAPTATION_MODE").ok().as_deref() == Some("live") {
            Arc::new(server::adaptation::live::LiveAdaptationService::from_env(
                db.clone(),
                state.llm_service.clone(),
                state.ollama_semaphore.clone(),
                state.adaptation_metrics.clone(),
                state.settings_encryption.clone(),
            ))
        } else {
            Arc::new(server::adaptation::passthrough::PassthroughAdaptationService::new(db.clone()))
        };
    let state = state
        .with_adaptation_tx(adaptation_tx.clone())
        .with_adaptation_service(adaptation_service.clone());

    // Hydrate session_registry for all non-terminal sessions so that WS
    // observer/dashboard connections work after a server restart.
    hydrate_session_registry(&state)
        .await
        .map_err(|e| anyhow::anyhow!("session registry hydration: {e}"))?;

    // Spawn the background adaptation loop. Processes AdaptationRequests
    // dispatched whenever sessions start or players advance.
    // The watch channel provides clean cancellation of in-flight backoff sleeps.
    let (_adapt_shutdown_tx, adapt_shutdown_rx) = tokio::sync::watch::channel(false);
    tokio::spawn(server::adaptation::loop_task::start_adaptation_loop(
        adaptation_rx,
        adaptation_service,
        state.admin_registry.clone(),
        state.adaptation_metrics.clone(),
        adapt_shutdown_rx,
    ));

    // Spawn background leaderboard refresh task. Broadcasts a LeaderboardUpdate
    // frame to every connected session observer and a ScoreRankUpdated frame to
    // every connected player every 5 seconds for all running sessions. This
    // ensures the session leaderboard page (and player pages) update even when
    // no player WS connection is open to drive the per-probe broadcast_leaderboard
    // call in db_poll_task.
    tokio::spawn(leaderboard_refresh_loop(
        db.clone(),
        state.session_registry.clone(),
        state.player_registry.clone(),
    ));

    // Attempt to initialize SES email service from app_settings credentials
    // (or AWS SDK default credential chain as fallback).
    let state = init_email_service(state, &db).await;

    // Discover game servers with ZMQ PUB URLs and spawn subscriber tasks.
    spawn_zmq_subscribers(&state, &db).await;

    // Seed judges first: project tasks may reference them by slug.
    if let Err(e) = server::seed::seed_judges(&db).await {
        tracing::warn!(error = %e, "seed: judges failed");
    }

    // LLM defaults: convert legacy ai_provider/ai_model settings into a
    // provider row + default assignment, or seed the Ollama default.
    server::seed::seed_llm_defaults(&db).await;

    // LLM telemetry retention: drop llm_requests rows older than 30 days.
    server::llm::purge_old_llm_telemetry(&db).await;

    // Seed projects from ARENA_PROJECTS_DIR (default ./projects) into the DB.
    // Skips projects whose slug already exists; log-and-continue per file.
    server::seed::seed_projects(&db).await;

    let app = build_router(state);

    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    tracing::info!("server listening on {addr}");
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(async move {
        tokio::signal::ctrl_c().await.ok();
        let _ = shutdown_tx.send(());
    })
    .await?;
    Ok(())
}

/// Populate `session_registry` from the database for all non-terminal sessions.
///
/// Called once at startup after `scheduler.resume_on_startup()` so that WS
/// observer connections (`/ws/s/:code/observe`) and dashboard connections
/// (`/ws/s/:code`) work correctly after a server restart without requiring
/// sessions to be recreated.
///
/// Known limitation: the lobby/running countdown timers are NOT restarted here.
/// Sessions restored via this path will not auto-advance or auto-finish until
/// a future restart-recovery improvement is implemented.
async fn hydrate_session_registry(state: &AppState) -> Result<()> {
    use arena_core::session_status::SessionStatus;
    use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
    use server::entities::{players, sessions, users};
    use server::protocol::MemberInfo;
    use server::state::{SessionCacheInner, SessionEntry};
    use std::collections::HashMap;
    use std::sync::Arc;
    use tokio::sync::broadcast;

    let active = sessions::Entity::find()
        .filter(sessions::Column::Status.ne(SessionStatus::Finished))
        .filter(sessions::Column::Status.ne(SessionStatus::Cancelled))
        .all(&state.db)
        .await
        .map_err(|e| anyhow::anyhow!("hydrate: session query: {e}"))?;

    for session in active {
        let jc = session.join_code.clone();
        // Skip entries already inserted (e.g. by concurrent request or test setup).
        if state.session_registry.contains_key(&jc) {
            continue;
        }

        let phase = if session.status == SessionStatus::Running {
            SessionStatus::Running
        } else {
            SessionStatus::Lobby
        };

        // Load non-revoked players for this session.
        let player_rows = players::Entity::find()
            .filter(players::Column::SessionIdFk.eq(session.id))
            .filter(players::Column::RevokedAt.is_null())
            .all(&state.db)
            .await
            .map_err(|e| anyhow::anyhow!("hydrate: player query for {jc}: {e}"))?;

        // Batch-load users so we can fill avatar_url and username in MemberInfo.
        let user_ids: Vec<uuid::Uuid> = player_rows.iter().filter_map(|p| p.user_id_fk).collect();
        let user_map: HashMap<uuid::Uuid, users::Model> = if !user_ids.is_empty() {
            users::Entity::find()
                .filter(users::Column::Id.is_in(user_ids))
                .all(&state.db)
                .await
                .map_err(|e| anyhow::anyhow!("hydrate: user query for {jc}: {e}"))?
                .into_iter()
                .map(|u| (u.id, u))
                .collect()
        } else {
            HashMap::new()
        };

        let participants: Vec<MemberInfo> = player_rows
            .iter()
            .map(|p| {
                let user = p.user_id_fk.and_then(|uid| user_map.get(&uid));
                MemberInfo {
                    user_id: p
                        .user_id_fk
                        .map(|uid| uid.to_string())
                        .unwrap_or_else(|| p.id.to_string()),
                    player_id: Some(p.id),
                    display_name: p.display_name.clone(),
                    joined_at: p.joined_at.to_rfc3339(),
                    avatar_url: user.and_then(|u| u.avatar_url.clone()),
                    fingerprint: p.fingerprint.clone(),
                    username: user.and_then(|u| u.username.clone()),
                    agent_display_name: arena_core::scoring::parse_agent_display_name(
                        p.metadata_json.as_deref(),
                    ),
                    completion_status: None,
                }
            })
            .collect();

        let leaderboard = server::scoring::compute_leaderboard(&state.db, session.id)
            .await
            .unwrap_or_default();

        let (tx, _) = broadcast::channel::<server::protocol::ArenaFrame>(256);
        state.session_registry.insert(
            jc.clone(),
            SessionEntry {
                tx,
                cache: Arc::new(std::sync::RwLock::new(SessionCacheInner {
                    session_id: session.id,
                    phase,
                    version: 1,
                    participants,
                    leaderboard,
                    started_at: session.started_at,
                })),
            },
        );
        tracing::info!(
            join_code = %jc,
            status = %session.status,
            "startup: hydrated session_registry entry"
        );
    }

    Ok(())
}

/// Read a single `app_settings` value, treating lookup errors as absent.
async fn read_app_setting(db: &sea_orm::DatabaseConnection, key: &str) -> Option<String> {
    use sea_orm::EntityTrait;
    server::entities::app_settings::Entity::find_by_id(key)
        .one(db)
        .await
        .ok()
        .flatten()
        .map(|m| m.value)
}

/// Initialize the transactional email service. The `email.provider` setting
/// (fallback: `EMAIL_PROVIDER` env var, default `ses`) selects between AWS
/// SES and Cloudflare Email Service; each provider reads its credentials
/// from `app_settings` first, then the environment.
async fn init_email_service(state: AppState, db: &sea_orm::DatabaseConnection) -> AppState {
    let provider = read_app_setting(db, "email.provider")
        .await
        .filter(|s| !s.is_empty())
        .or_else(|| {
            std::env::var("EMAIL_PROVIDER")
                .ok()
                .filter(|s| !s.is_empty())
        })
        .unwrap_or_else(|| "ses".to_string())
        .to_ascii_lowercase();

    let from_address = read_app_setting(db, "email.from_address")
        .await
        .filter(|s| !s.is_empty())
        .or_else(|| {
            std::env::var("EMAIL_FROM_ADDRESS")
                .ok()
                .filter(|s| !s.is_empty())
        });

    match provider.as_str() {
        "cloudflare" => init_cloudflare_email(state, db, from_address).await,
        "ses" => init_ses_email(state, db, from_address).await,
        other => {
            tracing::warn!(
                provider = other,
                "unknown email provider — email service disabled"
            );
            state
        }
    }
}

/// Cloudflare Email Service: needs an account id, an API token with the
/// "Email Sending: Edit" permission, and a from-address on a domain
/// onboarded for Email Sending.
async fn init_cloudflare_email(
    mut state: AppState,
    db: &sea_orm::DatabaseConnection,
    from_address: Option<String>,
) -> AppState {
    use server::email::cloudflare::CloudflareEmailService;
    use std::sync::Arc;

    let account_id = read_app_setting(db, "email.cloudflare_account_id")
        .await
        .filter(|s| !s.is_empty())
        .or_else(|| {
            std::env::var("CLOUDFLARE_ACCOUNT_ID")
                .ok()
                .filter(|s| !s.is_empty())
        });

    let api_token = read_app_setting(db, "email.cloudflare_api_token")
        .await
        .and_then(|enc| state.settings_encryption.decrypt(&enc).ok())
        .filter(|s| !s.is_empty())
        .or_else(|| {
            std::env::var("CLOUDFLARE_API_TOKEN")
                .ok()
                .filter(|s| !s.is_empty())
        });

    match (account_id, api_token, from_address) {
        (Some(account_id), Some(token), Some(from)) => {
            tracing::info!("Cloudflare email service initialized");
            state.email_service = Some(Arc::new(CloudflareEmailService::new(
                &account_id,
                token,
                from,
            )) as server::email::EmailServiceHandle);
        }
        (account_id, api_token, from_address) => {
            tracing::warn!(
                has_account_id = account_id.is_some(),
                has_api_token = api_token.is_some(),
                has_from_address = from_address.is_some(),
                "incomplete Cloudflare email configuration — email service disabled"
            );
        }
    }
    state
}

async fn init_ses_email(
    mut state: AppState,
    db: &sea_orm::DatabaseConnection,
    from_address: Option<String>,
) -> AppState {
    let access_key = read_app_setting(db, "email.access_key_id").await;
    let encrypted_secret = read_app_setting(db, "email.secret_access_key").await;
    let region = read_app_setting(db, "email.ses_region")
        .await
        .unwrap_or_else(|| {
            std::env::var("AWS_DEFAULT_REGION").unwrap_or_else(|_| "us-east-1".to_string())
        });

    let secret = encrypted_secret
        .as_deref()
        .and_then(|enc| state.settings_encryption.decrypt(enc).ok());

    let (key_id, secret_key) = match (access_key, secret) {
        (Some(k), Some(s)) if !k.is_empty() && !s.is_empty() => (k, s),
        _ => {
            tracing::warn!(
                "partial or absent SES credentials in app_settings — \
                 falling through to AWS SDK default credential chain"
            );
            match build_ses_client(None, &region, from_address.clone()).await {
                Some(handle) => {
                    state.email_service = Some(handle);
                }
                None => {
                    tracing::warn!(
                        "AWS SDK default credential chain also unavailable — \
                         email service disabled"
                    );
                }
            }
            return state;
        }
    };

    match build_ses_client(Some((&key_id, &secret_key)), &region, from_address).await {
        Some(handle) => {
            tracing::info!("SES email service initialized from app_settings credentials");
            state.email_service = Some(handle);
        }
        None => {
            tracing::error!("Failed to initialize SES client with app_settings credentials");
        }
    }
    state
}

/// Build the SES email service handle. With `creds` the app-settings key pair
/// is used; without, the AWS SDK default credential chain applies. Returns
/// `None` when no from-address is configured.
async fn build_ses_client(
    creds: Option<(&str, &str)>,
    region: &str,
    from_address: Option<String>,
) -> Option<server::email::EmailServiceHandle> {
    use aws_config::Region;
    use aws_credential_types::Credentials;
    use aws_sdk_sesv2::Client;
    use server::email::ses::SesEmailService;
    use std::sync::Arc;

    let from = from_address?;
    let mut loader = aws_config::from_env().region(Region::new(region.to_string()));
    if let Some((key_id, secret_key)) = creds {
        loader = loader.credentials_provider(Credentials::new(
            key_id,
            secret_key,
            None,
            None,
            "arena-app-settings",
        ));
    }
    let config = loader.load().await;
    let client = Client::new(&config);
    Some(Arc::new(SesEmailService::new(client, from)) as server::email::EmailServiceHandle)
}

/// Background task: periodically broadcasts leaderboard updates for all running
/// sessions in the registry.
///
/// Fires every 5 seconds. For each session with `cache.phase == SessionStatus::Running`:
///   - calls `compute_leaderboard` to read fresh scores from `task_results`
///   - sends `LeaderboardUpdate` to all connected session observers (session page)
///   - sends `ScoreRankUpdated` to all connected player channels (player page)
///
/// This runs independently of any WS connection so the session leaderboard
/// page updates even when no player has their individual player page open.
/// The per-probe call in `db_poll_task` remains for sub-second responsiveness
/// on the player page; this loop provides the fallback for all other views.
async fn leaderboard_refresh_loop(
    db: sea_orm::DatabaseConnection,
    session_registry: server::state::SessionRegistry,
    player_registry: server::state::PlayerRegistry,
) {
    use arena_core::session_status::SessionStatus;
    use server::scoring::broadcast_leaderboard;

    let mut interval = tokio::time::interval(std::time::Duration::from_secs(5));
    // Consume the initial immediate tick so the first real broadcast happens
    // 5 seconds after startup (hydration already loaded fresh leaderboard data).
    interval.tick().await;

    loop {
        interval.tick().await;

        // Snapshot active running sessions before any async work so we do not
        // hold DashMap shard locks across await points.
        let active: Vec<(String, uuid::Uuid)> = session_registry
            .iter()
            .filter_map(|e| {
                let cache = e.cache.read().ok()?;
                if cache.phase == SessionStatus::Running {
                    Some((e.key().clone(), cache.session_id))
                } else {
                    None
                }
            })
            .collect();

        for (join_code, session_id) in active {
            broadcast_leaderboard(
                &db,
                &session_registry,
                &player_registry,
                &join_code,
                session_id,
            )
            .await;
        }
    }
}

async fn spawn_zmq_subscribers(state: &AppState, db: &sea_orm::DatabaseConnection) {
    use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
    use server::entities::game_servers;
    use server::zmq_sub::spawn_subscriber_task;

    let servers = match game_servers::Entity::find()
        .filter(game_servers::Column::ZmqUrl.is_not_null())
        .filter(game_servers::Column::Status.eq("active"))
        .all(db)
        .await
    {
        Ok(s) => s,
        Err(e) => {
            tracing::error!("ZMQ subscriber discovery: DB query failed: {e}");
            return;
        }
    };

    for gs in servers {
        let zmq_url = match gs.zmq_url {
            Some(ref url) => url.clone(),
            None => continue,
        };
        let handle = spawn_subscriber_task(gs.id, zmq_url, state.clone());
        state.subscriber_tasks.insert(gs.id, handle);
        tracing::info!(game_server_id = %gs.id, "spawned ZMQ subscriber task");
    }

    tokio::spawn(zmq_discovery_loop(db.clone(), state.clone()));
}

async fn zmq_discovery_loop(db: sea_orm::DatabaseConnection, state: AppState) {
    use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
    use server::entities::game_servers;
    use server::zmq_sub::spawn_subscriber_task;

    let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(30));
    loop {
        interval.tick().await;
        let servers = match game_servers::Entity::find()
            .filter(game_servers::Column::ZmqUrl.is_not_null())
            .filter(game_servers::Column::Status.eq("active"))
            .all(&db)
            .await
        {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!("ZMQ discovery poll: DB query failed: {e}");
                continue;
            }
        };

        for gs in servers {
            let zmq_url = match gs.zmq_url {
                Some(ref url) => url.clone(),
                None => continue,
            };
            if !state.subscriber_tasks.contains_key(&gs.id) {
                let handle = spawn_subscriber_task(gs.id, zmq_url, state.clone());
                state.subscriber_tasks.insert(gs.id, handle);
                tracing::info!(game_server_id = %gs.id, "discovered new ZMQ game server, spawned subscriber task");
            }
        }

        let mut finished = Vec::new();
        for entry in state.subscriber_tasks.iter() {
            if entry.value().is_finished() {
                finished.push(*entry.key());
            }
        }
        for id in finished {
            state.subscriber_tasks.remove(&id);
            tracing::warn!(game_server_id = %id, "ZMQ subscriber task finished, removed from registry");
        }
    }
}

/// Format is controlled by the `ARENA_LOG_FORMAT` env var:
/// - `json` → structured JSON (one object per line; good for log aggregators)
/// - anything else → human-readable text (default)
///
/// Filter level defaults to `server=debug,tower_http=info,info` when
/// `RUST_LOG` is not set, so the server is informative out of the box
/// without drowning in tower noise.
fn init_tracing() {
    const DEFAULT_FILTER: &str = "server=debug,tower_http=info,info";
    let mut filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(DEFAULT_FILTER));
    // rig-core's LLM spans record full prompts/tool results at INFO — cap the
    // crate at WARN unless RUST_LOG explicitly mentions rig (opt-in).
    if !std::env::var("RUST_LOG")
        .map(|v| v.contains("rig"))
        .unwrap_or(false)
    {
        filter = filter.add_directive("rig=warn".parse().expect("valid directive"));
    }

    let json_format = std::env::var("ARENA_LOG_FORMAT")
        .map(|v| v.eq_ignore_ascii_case("json"))
        .unwrap_or(false);

    if json_format {
        tracing_subscriber::registry()
            .with(filter)
            .with(tracing_subscriber::fmt::layer().json())
            .init();
    } else {
        tracing_subscriber::registry()
            .with(filter)
            .with(tracing_subscriber::fmt::layer())
            .init();
    }
}
