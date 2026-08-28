use anyhow::Result;
use dashmap::DashMap;
use game_server::heartbeat;
use game_server::recovery;
use game_server::state::GameServerState;
use game_server::zmq_pub::{EventPublisher, NoopEventPublisher, ZmqEventPublisher};
use jsonwebtoken::{DecodingKey, EncodingKey};
use std::sync::Arc;
use uuid::Uuid;

#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::dotenv().ok();

    let mut filter = tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
        tracing_subscriber::EnvFilter::new("game_server=debug,tower_http=info,info")
    });
    // rig-core's `execute_tool` spans record full tool results (entire player
    // files) at INFO — cap the crate at WARN unless RUST_LOG explicitly
    // mentions rig (deliberate opt-in for LLM debugging).
    if !std::env::var("RUST_LOG")
        .map(|v| v.contains("rig"))
        .unwrap_or(false)
    {
        filter = filter.add_directive("rig=warn".parse().expect("valid directive"));
    }
    tracing_subscriber::fmt().with_env_filter(filter).init();

    // Same 1024-soft-nofile default as the main server (Docker 25+ stopped
    // raising it); probes, WS agents and judge runs all hold descriptors.
    if let Some((before, after)) = arena_core::rlimit::raise_nofile_limit() {
        tracing::info!(before, after, "nofile soft limit raised");
    }

    let port: u16 = std::env::var("GAME_SERVER_PORT")
        .unwrap_or_else(|_| "8081".to_string())
        .parse()?;

    let url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "sqlite://./arena.db?mode=rwc".to_string());
    let db = migration::connect_and_migrate(&url).await?;

    let server_id: Uuid = std::env::var("GAME_SERVER_ID")
        .ok()
        .filter(|s| !s.is_empty())
        .map(|s| s.parse())
        .transpose()?
        .unwrap_or_else(Uuid::new_v4);

    let advertise_url = std::env::var("GAME_SERVER_ADVERTISE_URL")
        .unwrap_or_else(|_| format!("ws://localhost:{}", port));

    let capacity: i32 = std::env::var("GAME_SERVER_CAPACITY")
        .ok()
        .map(|s| s.parse())
        .transpose()?
        .unwrap_or(64);

    let jwt_signing_key = std::env::var("JWT_SIGNING_KEY")
        .map_err(|_| anyhow::anyhow!("JWT_SIGNING_KEY missing"))?
        .into_bytes();

    let jwt_encoding_key = Arc::new(EncodingKey::from_secret(&jwt_signing_key));
    let jwt_decoding_key = Arc::new(DecodingKey::from_secret(&jwt_signing_key));
    let jwt_signing_secret = Arc::new(jwt_signing_key);

    let lobby_timer_secs: u64 = std::env::var("ARENA_LOBBY_TIMER_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .filter(|n: &u64| *n > 0)
        .unwrap_or(60);

    let judge_max_concurrent: usize = std::env::var("ARENA_JUDGE_MAX_CONCURRENT")
        .ok()
        .and_then(|v| v.parse().ok())
        .filter(|n: &usize| *n > 0)
        .unwrap_or(game_server::judge_queue::DEFAULT_JUDGE_MAX_CONCURRENT as usize);

    // Judge/memory models resolve per run from the admin-configured
    // `llm_providers` + assignments (see `GameServerState::resolve_llm`);
    // the encryption handle decrypts `llm_providers.api_key_enc`.
    let settings_encryption =
        arena_core::settings_encryption::SettingsEncryption::new(jwt_signing_secret.as_slice());

    let zmq_bind_addr = std::env::var("GAME_SERVER_ZMQ_BIND_ADDR")
        .unwrap_or_else(|_| "tcp://127.0.0.1:6000".to_string());

    let zmq_publisher: Arc<dyn EventPublisher> = match ZmqEventPublisher::bind(&zmq_bind_addr).await
    {
        Ok(pub_socket) => {
            tracing::info!("ZMQ PUB bound on {}", zmq_bind_addr);
            Arc::new(pub_socket)
        }
        Err(e) => {
            tracing::warn!(
                "ZMQ PUB bind failed on {}: {}. Falling back to no-op publisher.",
                zmq_bind_addr,
                e
            );
            Arc::new(NoopEventPublisher)
        }
    };

    // Tee every published event into the per-session JSONL log (admins pull
    // it via the main server's /api/admin/sessions/:code/event-log).
    let session_registry: game_server::state::SessionRegistry = Arc::new(DashMap::new());
    let event_publisher: Arc<dyn EventPublisher> = Arc::new(
        game_server::session_log_store::RecordingEventPublisher::new(
            zmq_publisher,
            db.clone(),
            session_registry.clone(),
            game_server::session_log_store::base_dir(),
        ),
    );

    let state = GameServerState {
        db: db.clone(),
        server_id,
        advertise_url,
        jwt_encoding_key,
        jwt_decoding_key,
        jwt_signing_secret,
        session_registry,
        player_agent_registry: Arc::new(DashMap::new()),
        lobby_timer_secs,
        event_publisher,
        judge_semaphore: Arc::new(tokio::sync::Semaphore::new(judge_max_concurrent)),
        settings_encryption: Arc::new(settings_encryption),
    };

    heartbeat::register(
        &db,
        server_id,
        &state.advertise_url,
        capacity,
        Some(&zmq_bind_addr),
    )
    .await?;

    let hb_db = db.clone();
    let hb_server_id = server_id;
    let hb_handle = tokio::spawn(async move {
        heartbeat::run_heartbeat(hb_db, hb_server_id).await;
    });

    let rec_state = state.clone();
    let rec_handle = tokio::spawn(async move {
        if let Err(e) = recovery::resume_on_startup(rec_state).await {
            tracing::error!("resume_on_startup failed: {}", e);
        }
    });

    // Deploys/crashes during the post-expiry judge-settle window lose the
    // in-memory settle task; sweep recently finished sessions and re-announce.
    let settle_state = state.clone();
    tokio::spawn(async move {
        if let Err(e) = recovery::settle_unsettled_finished_sessions(settle_state).await {
            tracing::error!("settle recovery sweep failed: {}", e);
        }
    });

    // Judge runs are scheduled only in memory, so a restart inside the
    // commit-wait window loses them with nothing to re-drive them. Sweep for
    // pairs that never produced a row. Runs AFTER the settle sweep so a
    // session that only lacked its event is settled first.
    let judge_state = state.clone();
    tokio::spawn(async move {
        if let Err(e) = recovery::enqueue_missed_judge_runs(judge_state.clone()).await {
            tracing::error!("missed-judge recovery sweep failed: {}", e);
        }
        // After a restart nothing is running in-process, so every `running`
        // row we own is an orphan — zero age floor.
        if let Err(e) =
            recovery::requeue_orphaned_judge_runs(judge_state.clone(), std::time::Duration::ZERO)
                .await
        {
            tracing::error!("orphaned-judge recovery sweep failed: {}", e);
        }
        // Runs the sweeps above can no longer reach (session outside the
        // recovery window, or bound to a dead server id) will never happen —
        // settle them as failed so their sessions stop reading as "judges
        // scoring" forever.
        if let Err(e) = recovery::fail_abandoned_judge_runs(judge_state).await {
            tracing::error!("abandoned-judge reaper failed: {}", e);
        }
    });

    // The startup sweep fires once. A judge that hangs or dies mid-run AFTER
    // boot has nothing to re-drive it and stays `pending` forever (session
    // WJGI66: golf-verify on the interrupted last task wedged for half an hour,
    // freezing completion). Reconcile on a timer so it self-heals in minutes.
    // The 5-minute age floor keeps the sweep from racing a judge that is
    // legitimately still running.
    let periodic_judge_state = state.clone();
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(std::time::Duration::from_secs(120));
        ticker.tick().await; // consume the immediate first tick — startup sweep already ran
        loop {
            ticker.tick().await;
            if let Err(e) = recovery::enqueue_missed_judge_runs_older_than(
                periodic_judge_state.clone(),
                std::time::Duration::from_secs(300),
            )
            .await
            {
                tracing::error!("periodic missed-judge sweep failed: {}", e);
            }
            // Cancelled sessions are outside the missed-run sweep; runs that
            // already started there still must settle. The same 5-minute
            // floor keeps a legitimately-running judge out of reach.
            if let Err(e) = recovery::requeue_orphaned_judge_runs(
                periodic_judge_state.clone(),
                std::time::Duration::from_secs(300),
            )
            .await
            {
                tracing::error!("orphaned-judge sweep failed: {}", e);
            }
        }
    });

    // Runs lost outside the recovery window never self-heal via the sweeps
    // above; reap them into terminal failed rows hourly (startup already ran
    // one pass) so a long-lived process converges too, not just fresh boots.
    let reaper_state = state.clone();
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(std::time::Duration::from_secs(3600));
        ticker.tick().await; // startup pass already ran
        loop {
            ticker.tick().await;
            if let Err(e) = recovery::fail_abandoned_judge_runs(reaper_state.clone()).await {
                tracing::error!("abandoned-judge reaper failed: {}", e);
            }
        }
    });

    // No agent socket survives a restart, but the presence flags in the DB
    // still say whatever the previous process wrote. Reset them before the
    // idle sweep starts, or a crash with players connected would freeze
    // "Live" forever and blind the sweep.
    match game_server::ws::player_agent::presence::reset_agent_presence_on_startup(&db, server_id)
        .await
    {
        Ok(0) => {}
        Ok(n) => tracing::info!(reset = n, "startup: cleared stale agent presence"),
        Err(e) => tracing::error!(error = %e, "startup: presence reset failed"),
    }

    // Cancel running sessions that have had no connected agents longer than
    // their project's idle_timeout_secs.
    let idle_state = state.clone();
    tokio::spawn(async move {
        game_server::idle_sweep::run_idle_sweep(idle_state).await;
    });

    // Server-side probes of open-ended tasks: interval/start schedules run
    // off this ticker (participant-side probes ride the agent loop).
    let probe_state = state.clone();
    tokio::spawn(async move {
        game_server::probe_scheduler::run_probe_scheduler(probe_state).await;
    });

    // Execution judges need a sandbox that can actually execute. A `bwrap` that
    // is installed but blocked (no permission to create user namespaces) fails
    // silently per-probe, so surface it once at boot instead of discovering it
    // from a player's score.
    tokio::spawn(async {
        let backend = arena_core::sandbox::detect_backend();
        let Ok(probe_dir) = tempfile::tempdir() else {
            return;
        };
        match arena_core::sandbox::self_check(backend, probe_dir.path()).await {
            Ok(()) => tracing::info!(backend = ?backend, "execution-judge sandbox is functional"),
            Err(why) => tracing::error!(
                backend = ?backend, reason = %why,
                "execution-judge sandbox is NOT functional — execution judges will refuse to \
                 score (no player is penalized, but golf results go unverified)"
            ),
        }
    });

    let (shutdown_tx, _) = tokio::sync::broadcast::channel(1);
    let app = game_server::build_router(state.clone());

    let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{}", port)).await?;
    tracing::info!("game-server listening on 0.0.0.0:{}", port);
    tracing::info!(
        "server_id={}, advertise_url={}",
        server_id,
        state.advertise_url
    );

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    let _ = shutdown_tx.send(());
    hb_handle.abort();
    rec_handle.abort();
    let _ = shutdown_tx;

    Ok(())
}

async fn shutdown_signal() {
    tokio::signal::ctrl_c()
        .await
        .expect("failed to install CTRL+C handler");
}
