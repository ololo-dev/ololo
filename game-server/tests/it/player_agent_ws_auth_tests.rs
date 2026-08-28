//! SEC-H1 regression: the player-agent WS upgrade must authenticate the
//! caller with a PAT (`X-API-Key`) and verify the PAT's user owns the player
//! row it drives — knowing the 6-char join code (shareable, guessable) must
//! not be enough to become another player's agent.

use std::sync::Arc;

use arena_core::entities::{cli_tokens, game_servers, players, projects, sessions, users};
use arena_core::session_status::SessionStatus;
use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use chrono::Utc;
use dashmap::DashMap;
use game_server::state::GameServerState;
use game_server::zmq_pub::NoopEventPublisher;
use jsonwebtoken::{DecodingKey, EncodingKey};
use migration::MigratorTrait;
use sea_orm::{ActiveModelTrait, DatabaseConnection, Set};
use tokio::sync::Semaphore;
use tower::ServiceExt;
use uuid::Uuid;

async fn setup_db() -> DatabaseConnection {
    let db = sea_orm::Database::connect("sqlite::memory:")
        .await
        .expect("connect");
    migration::Migrator::up(&db, None).await.expect("migrate");
    db
}

fn test_state(db: DatabaseConnection) -> GameServerState {
    let secret = b"test-secret-32-bytes-or-more-xxxxxxx".to_vec();
    GameServerState {
        db,
        server_id: Uuid::new_v4(),
        advertise_url: "ws://localhost:8081".to_string(),
        jwt_encoding_key: Arc::new(EncodingKey::from_secret(&secret)),
        jwt_decoding_key: Arc::new(DecodingKey::from_secret(&secret)),
        jwt_signing_secret: Arc::new(secret),
        session_registry: Arc::new(DashMap::new()),
        player_agent_registry: Arc::new(DashMap::new()),
        lobby_timer_secs: 60,
        event_publisher: Arc::new(NoopEventPublisher),
        judge_semaphore: Arc::new(Semaphore::new(3)),
        settings_encryption: std::sync::Arc::new(
            arena_core::settings_encryption::SettingsEncryption::new(
                b"test-secret-key-for-settings-enc",
            ),
        ),
    }
}

async fn insert_user(db: &DatabaseConnection) -> Uuid {
    users::ActiveModel {
        id: Set(Uuid::new_v4()),
        email: Set(format!("u{}@example.com", Uuid::new_v4())),
        password_hash: Set(None),
        display_name: Set("tester".to_string()),
        created_at: Set(Utc::now()),
        updated_at: Set(Utc::now()),
        is_admin: Set(false),
        avatar_url: Set(None),
        email_verified: Set(false),
        username: Set(None),
        plan: Set(arena_core::quota::PLAN_PREMIUM.to_string()),
        judge_run_limit: Set(None),
        judge_run_credits: Set(0),
    }
    .insert(db)
    .await
    .expect("insert user")
    .id
}

async fn insert_pat(db: &DatabaseConnection, user_id: Uuid, token: &str) {
    cli_tokens::ActiveModel {
        id: Set(Uuid::new_v4()),
        token_hash: Set(arena_core::auth::hash_pat(token)),
        user_id: Set(user_id),
        created_at: Set(Utc::now().into()),
        expires_at: Set((Utc::now() + chrono::Duration::hours(1)).into()),
    }
    .insert(db)
    .await
    .expect("insert cli token");
}

async fn insert_player(db: &DatabaseConnection, session_id: Uuid, user_id: Uuid) -> Uuid {
    players::ActiveModel {
        id: Set(Uuid::new_v4()),
        session_id_fk: Set(session_id),
        user_id_fk: Set(Some(user_id)),
        display_name: Set("player".to_string()),
        fingerprint: Set(None),
        metadata_json: Set(None),
        joined_at: Set(Utc::now()),
        reconnected_at: Set(None),
        revoked_at: Set(None),
        agent_connected: Set(false),
        agent_last_seen_at: Set(None),
    }
    .insert(db)
    .await
    .expect("insert player")
    .id
}

/// Seed owner + project + lobby session bound to `state.server_id`.
/// Returns (session_id, owner_user_id).
async fn seed_session(state: &GameServerState, join_code: &str) -> (Uuid, Uuid) {
    let db = &state.db;
    let owner = insert_user(db).await;

    game_servers::ActiveModel {
        id: Set(state.server_id),
        url: Set(state.advertise_url.clone()),
        zmq_url: Set(None),
        display_name: Set(None),
        capacity: Set(10),
        active_sessions: Set(0),
        status: Set("active".to_string()),
        last_heartbeat: Set(Utc::now()),
        created_at: Set(Utc::now()),
        updated_at: Set(Utc::now()),
    }
    .insert(db)
    .await
    .expect("insert game server");

    let project_id = projects::ActiveModel {
        id: Set(Uuid::new_v4()),
        name: Set("proj".to_string()),
        slug: Set(None),
        description: Set(String::new()),
        category: Set(None),
        tags: Set(String::new()),
        cover_image_url: Set(None),
        owner_user_id_fk: Set(owner),
        public: Set(true),
        archived_at: Set(None),
        created_at: Set(Utc::now()),
        updated_at: Set(Utc::now()),
        default_value_points: Set(10),
        default_fail_points: Set(-5),
        default_no_response_points: Set(-10),
        default_completion_bonus_points: Set(10),
        default_deadline_secs: Set(60),
        default_min_interval_secs: Set(5),
        default_interval_increment_secs: Set(5),
        default_max_interval_secs: Set(60),
        default_session_duration_secs: Set(3600),
        idle_timeout_secs: Set(300),
        memory_schema: Set(None),
        show_tasks: Set(true),
        parent_project_id_fk: Set(None),
        part_ordinal: Set(None),
    }
    .insert(db)
    .await
    .expect("insert project")
    .id;

    let session_id = sessions::ActiveModel {
        id: Set(Uuid::new_v4()),
        name: Set("s".to_string()),
        created_at: Set(Utc::now()),
        owner_id_fk: Set(None),
        status: Set(SessionStatus::Lobby),
        join_code: Set(join_code.to_string()),
        started_at: Set(None),
        finished_at: Set(None),
        paused_at: Set(None),
        paused_duration_secs: Set(None),
        project_id_fk: Set(project_id),
        game_server_id: Set(Some(state.server_id)),
        cancel_reason: Set(None),
        cancelled_by: Set(None),
    }
    .insert(db)
    .await
    .expect("insert session")
    .id;

    (session_id, owner)
}

fn ws_request(uri: &str, api_key: Option<&str>) -> Request<Body> {
    let mut builder = Request::builder()
        .method(Method::GET)
        .uri(uri)
        .header("Host", "gs.test")
        .header("Connection", "Upgrade")
        .header("Upgrade", "websocket")
        .header("Sec-WebSocket-Version", "13")
        .header("Sec-WebSocket-Key", "dGhlIHNhbXBsZSBub25jZQ==");
    if let Some(key) = api_key {
        builder = builder.header("X-API-Key", key);
    }
    builder.body(Body::empty()).unwrap()
}

#[tokio::test]
async fn upgrade_without_api_key_is_unauthorized() {
    let state = test_state(setup_db().await);
    let (_session, _owner) = seed_session(&state, "WSAUTA").await;
    let app = game_server::build_router(state);

    let resp = app
        .oneshot(ws_request("/ws/player/agent/WSAUTA", None))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn upgrade_with_unknown_pat_is_unauthorized() {
    let state = test_state(setup_db().await);
    let (_session, _owner) = seed_session(&state, "WSAUTB").await;
    let app = game_server::build_router(state);

    let resp = app
        .oneshot(ws_request("/ws/player/agent/WSAUTB", Some("ololo_bogus")))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

/// Auth must run before the WebSocket-handshake validation: an
/// unauthenticated caller learns nothing about the endpoint, not even that
/// it speaks WebSocket.
#[tokio::test]
async fn missing_api_key_beats_missing_upgrade_headers() {
    let state = test_state(setup_db().await);
    let (_session, _owner) = seed_session(&state, "WSAUTC").await;
    let app = game_server::build_router(state);

    let req = Request::builder()
        .method(Method::GET)
        .uri("/ws/player/agent/WSAUTC")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn pat_user_without_player_row_is_forbidden() {
    let state = test_state(setup_db().await);
    let (_session, _owner) = seed_session(&state, "WSAUTD").await;
    let stranger = insert_user(&state.db).await;
    insert_pat(&state.db, stranger, "ololo_stranger").await;
    let app = game_server::build_router(state);

    let resp = app
        .oneshot(ws_request(
            "/ws/player/agent/WSAUTD",
            Some("ololo_stranger"),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

/// The SEC-H1 exploit shape: a valid PAT for user B naming user A's
/// `player_id` in the query string must be refused, not honored.
#[tokio::test]
async fn spoofed_player_id_is_forbidden() {
    let state = test_state(setup_db().await);
    let (session_id, _owner) = seed_session(&state, "WSAUTE").await;
    let victim = insert_user(&state.db).await;
    let victim_player = insert_player(&state.db, session_id, victim).await;
    let attacker = insert_user(&state.db).await;
    insert_pat(&state.db, attacker, "ololo_attacker").await;
    let app = game_server::build_router(state);

    let resp = app
        .oneshot(ws_request(
            &format!("/ws/player/agent/WSAUTE?player_id={victim_player}"),
            Some("ololo_attacker"),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

/// Serve the router on an ephemeral port. `WebSocketUpgrade` extraction
/// needs a real hyper connection (`OnUpgrade` extension), so the happy path
/// cannot be driven through `oneshot`.
async fn serve(app: axum::Router) -> std::net::SocketAddr {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("local addr");
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });
    addr
}

async fn ws_connect(
    addr: std::net::SocketAddr,
    path_and_query: &str,
    api_key: &str,
) -> Result<
    tokio_tungstenite::tungstenite::http::Response<Option<Vec<u8>>>,
    tokio_tungstenite::tungstenite::Error,
> {
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;
    let mut req = format!("ws://{addr}{path_and_query}")
        .into_client_request()
        .expect("client request");
    req.headers_mut()
        .insert("X-API-Key", api_key.parse().expect("header value"));
    tokio_tungstenite::connect_async(req)
        .await
        .map(|(_ws, resp)| resp.map(|_| None))
}

#[tokio::test]
async fn member_with_valid_pat_upgrades() {
    let state = test_state(setup_db().await);
    let (session_id, _owner) = seed_session(&state, "WSAUTF").await;
    let member = insert_user(&state.db).await;
    let member_player = insert_player(&state.db, session_id, member).await;
    insert_pat(&state.db, member, "ololo_member").await;
    let addr = serve(game_server::build_router(state)).await;

    let resp = ws_connect(
        addr,
        &format!("/ws/player/agent/WSAUTF?player_id={member_player}"),
        "ololo_member",
    )
    .await
    .expect("member handshake accepted");
    assert_eq!(resp.status(), StatusCode::SWITCHING_PROTOCOLS);

    // …and the id-less legacy client still upgrades when the user has
    // exactly one row in the session.
    let resp = ws_connect(addr, "/ws/player/agent/WSAUTF", "ololo_member")
        .await
        .expect("legacy id-less handshake accepted");
    assert_eq!(resp.status(), StatusCode::SWITCHING_PROTOCOLS);
}
