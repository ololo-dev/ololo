//! Coverage for src/lib.rs: `build_router`, the `/health` route, the
//! fallback handler, and the `log_request` middleware (exercised on every
//! routed request).

use std::sync::Arc;

use axum::body::Body;
use axum::http::Uri;
use axum::http::{Method, Request, StatusCode};
use dashmap::DashMap;
use game_server::state::GameServerState;
use game_server::zmq_pub::NoopEventPublisher;
use http_body_util::BodyExt;
use jsonwebtoken::{DecodingKey, EncodingKey};
use tokio::sync::Semaphore;
use tower::ServiceExt;
use uuid::Uuid;

async fn test_state() -> GameServerState {
    let secret = b"test-secret-32-bytes-or-more-xxxxxxx".to_vec();
    GameServerState {
        db: sea_orm::Database::connect("sqlite::memory:")
            .await
            .expect("connected"),
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

#[tokio::test]
async fn health_returns_ok() {
    let app = game_server::build_router(test_state().await);
    let req = Request::builder()
        .method(Method::GET)
        .uri("/health")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(&body[..], b"ok");
}

#[tokio::test]
async fn fallback_returns_404_with_route() {
    let app = game_server::build_router(test_state().await);
    let req = Request::builder()
        .method(Method::GET)
        .uri("/does/not/exist")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let text = String::from_utf8_lossy(&body);
    assert!(text.contains("no route"), "body was: {text}");
    // URI is echoed back.
    assert!(text.contains("/does/not/exist"));
}

#[tokio::test]
async fn log_request_middleware_runs_on_routed_request() {
    // Any request exercises log_request (the from_fn middleware on the router).
    let app = game_server::build_router(test_state().await);
    let uri: Uri = "/health".parse().unwrap();
    let req = Request::builder()
        .method(Method::GET)
        .uri(uri.clone())
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}
