//! `/api/sessions/resolve` — join-code lookup.
//!
//! This endpoint is the head of a chain that silently merged two players into
//! one. Join codes are stored uppercase and thirteen other lookups normalise
//! before comparing; this one did not, so a lowercase code returned 404
//! `session_not_found`. The CLI treats that 404 as "no resolve payload" and
//! connects to the agent socket WITHOUT a `player_id`, which the game server
//! then had to guess at — and it guessed the wrong player (session VKIBCB).
//!
//! The chain had no integration coverage at all because reaching this handler
//! needs a valid PAT and no test helper minted one. It does now.

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use chrono::Utc;
use http_body_util::BodyExt;
use migration::{Migrator, MigratorTrait};
use sea_orm::{ActiveModelTrait, DatabaseConnection, EntityTrait, Set};
use server::entities::{cli_tokens, game_servers, players, projects, sessions, users};
use server::{AppState, AuthConfig, build_router};
use std::time::Duration;
use tower::ServiceExt;
use uuid::Uuid;

const ORIGIN: &str = "http://localhost:5173";
const JOIN_CODE: &str = "ABCDEF";

async fn test_state() -> AppState {
    let db = sea_orm::Database::connect("sqlite::memory:")
        .await
        .expect("sqlite connect");
    Migrator::up(&db, None).await.expect("migrate up");
    let cfg = AuthConfig {
        jwt_signing_key: b"integration-test-secret-32-bytes-or-more-xxxxxxx".to_vec(),
        frontend_origins: vec![ORIGIN.to_string()],
        access_ttl: Duration::from_secs(900),
        refresh_ttl: Duration::from_secs(30 * 86_400),
        max_agents_per_session: 16,
    };
    AppState::new(db, cfg)
}

/// Mint a PAT for `user_id` by storing its hash, the way the real issuer does.
async fn mint_pat(db: &DatabaseConnection, user_id: Uuid) -> String {
    let token = format!("ololo_{}", "a".repeat(64));
    cli_tokens::ActiveModel {
        id: Set(Uuid::new_v4()),
        token_hash: Set(server::auth::pat::hash_pat(&token)),
        user_id: Set(user_id),
        created_at: Set(Utc::now().into()),
        expires_at: Set((Utc::now() + chrono::Duration::days(30)).into()),
    }
    .insert(db)
    .await
    .expect("insert cli token");
    token
}

/// A running session this user is a member of, on a live game server.
async fn seed_resolvable_session(db: &DatabaseConnection) -> (Uuid, Uuid, String) {
    let user_id = Uuid::new_v4();
    users::ActiveModel {
        id: Set(user_id),
        email: Set(format!("u{user_id}@example.com")),
        password_hash: Set(None),
        display_name: Set("tester".to_string()),
        created_at: Set(Utc::now()),
        updated_at: Set(Utc::now()),
        is_admin: Set(false),
        avatar_url: Set(None),
        email_verified: Set(true),
        username: Set(None),
        plan: Set(arena_core::quota::PLAN_PREMIUM.to_string()),
        judge_run_limit: Set(None),
        judge_run_credits: Set(0),
    }
    .insert(db)
    .await
    .expect("insert user");

    let server_id = Uuid::new_v4();
    game_servers::ActiveModel {
        id: Set(server_id),
        url: Set("ws://localhost:8081".to_string()),
        zmq_url: Set(None),
        display_name: Set(None),
        capacity: Set(64),
        active_sessions: Set(0),
        status: Set("active".to_string()),
        last_heartbeat: Set(Utc::now()),
        created_at: Set(Utc::now()),
        updated_at: Set(Utc::now()),
    }
    .insert(db)
    .await
    .expect("insert game server");

    let project_id = Uuid::new_v4();
    projects::ActiveModel {
        id: Set(project_id),
        name: Set("proj".to_string()),
        slug: Set(None),
        description: Set(String::new()),
        category: Set(None),
        tags: Set(String::new()),
        cover_image_url: Set(None),
        owner_user_id_fk: Set(user_id),
        public: Set(true),
        archived_at: Set(None),
        created_at: Set(Utc::now()),
        updated_at: Set(Utc::now()),
        default_value_points: Set(10),
        default_fail_points: Set(-5),
        default_no_response_points: Set(-10),
        default_completion_bonus_points: Set(10),
        default_deadline_secs: Set(60),
        default_session_duration_secs: Set(3600),
        idle_timeout_secs: Set(300),
        default_min_interval_secs: Set(5),
        default_interval_increment_secs: Set(5),
        default_max_interval_secs: Set(60),
        memory_schema: Set(None),
        show_tasks: Set(true),
        parent_project_id_fk: Set(None),
        part_ordinal: Set(None),
    }
    .insert(db)
    .await
    .expect("insert project");

    let session_id = Uuid::new_v4();
    sessions::ActiveModel {
        id: Set(session_id),
        name: Set("s".to_string()),
        created_at: Set(Utc::now()),
        owner_id_fk: Set(Some(user_id)),
        status: Set(arena_core::session_status::SessionStatus::Running),
        join_code: Set(JOIN_CODE.to_string()),
        started_at: Set(Some(Utc::now())),
        finished_at: Set(None),
        paused_at: Set(None),
        paused_duration_secs: Set(None),
        project_id_fk: Set(project_id),
        game_server_id: Set(Some(server_id)),
        cancel_reason: Set(None),
        cancelled_by: Set(None),
    }
    .insert(db)
    .await
    .expect("insert session");

    let player_id = Uuid::new_v4();
    players::ActiveModel {
        id: Set(player_id),
        session_id_fk: Set(session_id),
        user_id_fk: Set(Some(user_id)),
        display_name: Set("p".to_string()),
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
    .expect("insert player");

    let pat = mint_pat(db, user_id).await;
    (session_id, player_id, pat)
}

async fn resolve(state: &AppState, code: &str, pat: &str) -> (StatusCode, String) {
    let resp = build_router(state.clone())
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri(format!("/api/sessions/resolve?join_code={code}"))
                .header("X-API-Key", pat)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("request");
    let status = resp.status();
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    (status, String::from_utf8_lossy(&body).to_string())
}

#[tokio::test]
async fn resolve_finds_the_session_whatever_the_join_code_case() {
    let state = test_state().await;
    let (session_id, player_id, pat) = seed_resolvable_session(&state.db).await;

    for code in [JOIN_CODE, "abcdef", "AbCdEf", "  abcdef  "] {
        let (status, body) = resolve(&state, code.trim(), &pat).await;
        assert_eq!(
            status,
            StatusCode::OK,
            "join code {code:?} must resolve — a 404 here makes the CLI connect \
             without a player_id, which the game server used to guess at. Body: {body}"
        );
        assert!(
            body.contains(&player_id.to_string()),
            "the caller's own player_id must come back for {code:?}: {body}"
        );
        assert!(body.contains(&session_id.to_string()));
    }
}

#[tokio::test]
async fn resolve_still_404s_for_a_code_that_does_not_exist() {
    let state = test_state().await;
    let (_session_id, _player_id, pat) = seed_resolvable_session(&state.db).await;

    let (status, body) = resolve(&state, "ZZZZZZ", &pat).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "body: {body}");
    assert!(body.contains("session_not_found"), "body: {body}");
}

#[tokio::test]
async fn resolve_rejects_a_caller_who_is_not_a_member() {
    let state = test_state().await;
    let (_session_id, _player_id, _pat) = seed_resolvable_session(&state.db).await;

    // A second user with a valid PAT but no player row in that session.
    let outsider = Uuid::new_v4();
    users::ActiveModel {
        id: Set(outsider),
        email: Set(format!("o{outsider}@example.com")),
        password_hash: Set(None),
        display_name: Set("outsider".to_string()),
        created_at: Set(Utc::now()),
        updated_at: Set(Utc::now()),
        is_admin: Set(false),
        avatar_url: Set(None),
        email_verified: Set(true),
        username: Set(None),
        plan: Set(arena_core::quota::PLAN_PREMIUM.to_string()),
        judge_run_limit: Set(None),
        judge_run_credits: Set(0),
    }
    .insert(&state.db)
    .await
    .expect("insert outsider");
    let token = format!("ololo_{}", "b".repeat(64));
    cli_tokens::ActiveModel {
        id: Set(Uuid::new_v4()),
        token_hash: Set(server::auth::pat::hash_pat(&token)),
        user_id: Set(outsider),
        created_at: Set(Utc::now().into()),
        expires_at: Set((Utc::now() + chrono::Duration::days(30)).into()),
    }
    .insert(&state.db)
    .await
    .expect("insert outsider token");

    // Normalising the code must not become a way in for a non-member.
    let (status, body) = resolve(&state, "abcdef", &token).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "body: {body}");
    assert!(body.contains("not_a_member"), "body: {body}");
}

#[tokio::test]
async fn resolve_503s_when_game_server_heartbeat_is_stale() {
    // ARCH-H1: a crashed game server keeps status="active" (its heartbeat just
    // stops), so resolve must exclude it on staleness or clients get a dead URL.
    let state = test_state().await;
    let (_session_id, _player_id, pat) = seed_resolvable_session(&state.db).await;

    // Age the (only) game server's heartbeat well past the stale threshold.
    let gs = game_servers::Entity::find()
        .one(&state.db)
        .await
        .unwrap()
        .expect("game server row");
    let mut am: game_servers::ActiveModel = gs.into();
    am.last_heartbeat = Set(Utc::now() - chrono::Duration::seconds(120));
    am.update(&state.db).await.unwrap();

    let (status, body) = resolve(&state, JOIN_CODE, &pat).await;
    assert_eq!(
        status,
        StatusCode::SERVICE_UNAVAILABLE,
        "stale server must not resolve: {body}"
    );
    assert!(body.contains("no_game_server"), "body: {body}");
}
