//! End-to-end integration tests for register / login / refresh / origin guard.
//!
//! Uses `sqlite::memory:` + `migration::Migrator::up` and `tower::ServiceExt::oneshot`.

use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use http_body_util::BodyExt;
use migration::{Migrator, MigratorTrait};
use sea_orm::Database;
use server::{AppState, AuthConfig, build_router};
use std::time::Duration;
use tower::ServiceExt;

async fn test_state() -> AppState {
    let db = Database::connect("sqlite::memory:").await.unwrap();
    Migrator::up(&db, None).await.unwrap();
    let cfg = AuthConfig {
        jwt_signing_key: b"integration-test-secret-32-bytes-or-more-xxxxxxx".to_vec(),
        frontend_origins: vec!["http://localhost:5173".into()],
        access_ttl: Duration::from_secs(900),
        refresh_ttl: Duration::from_secs(30 * 86_400),
        max_agents_per_session: 16,
    };
    AppState::new(db, cfg)
}

fn json_body(v: serde_json::Value) -> Body {
    Body::from(serde_json::to_vec(&v).unwrap())
}

async fn read_body(resp: axum::response::Response) -> (StatusCode, Vec<String>, serde_json::Value) {
    let status = resp.status();
    let cookies: Vec<String> = resp
        .headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .map(|hv| hv.to_str().unwrap().to_string())
        .collect();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let value: serde_json::Value = if bytes.is_empty() {
        serde_json::Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null)
    };
    (status, cookies, value)
}

fn cookie_pair(set_cookies: &[String], name: &str) -> Option<String> {
    for sc in set_cookies {
        if let Some(rest) = sc.strip_prefix(&format!("{}=", name)) {
            // Take up to the first `;`.
            let value = rest.split(';').next().unwrap_or("");
            if value.is_empty() {
                continue;
            }
            return Some(format!("{}={}", name, value));
        }
    }
    None
}

#[tokio::test]
async fn register_then_login_returns_cookies() {
    let state = test_state().await;
    let app = build_router(state);

    // Register.
    let req = Request::builder()
        .method(Method::POST)
        .uri("/auth/register")
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::ORIGIN, "http://localhost:5173")
        .body(json_body(serde_json::json!({
            "email": "alice@example.test",
            "password": "secret-password-1",
            "display_name": "Alice"
        })))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    let (status, _cookies, body) = read_body(resp).await;
    assert_eq!(status, StatusCode::CREATED, "register status; body={body}");

    // Login.
    let req = Request::builder()
        .method(Method::POST)
        .uri("/auth/login")
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::ORIGIN, "http://localhost:5173")
        .body(json_body(serde_json::json!({
            "email": "alice@example.test",
            "password": "secret-password-1"
        })))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    let (status, cookies, body) = read_body(resp).await;
    assert_eq!(status, StatusCode::OK, "login body={body}");

    let access = cookies.iter().find(|c| c.starts_with("arena_access="));
    let refresh = cookies.iter().find(|c| c.starts_with("arena_refresh="));
    assert!(access.is_some(), "missing access cookie");
    assert!(refresh.is_some(), "missing refresh cookie");
    let access = access.unwrap();
    assert!(access.contains("HttpOnly"));
    assert!(access.contains("Secure"));
    assert!(access.contains("SameSite=Lax"));
    let refresh = refresh.unwrap();
    // Path=/ (widened from /auth/refresh) so the SvelteKit hooks can silent-refresh
    // the access token on ordinary page navigations.
    assert!(refresh.contains("Path=/"));
    assert!(refresh.contains("HttpOnly"));
    assert!(refresh.contains("Secure"));
    assert!(refresh.contains("SameSite=Lax"));
}

#[tokio::test]
async fn login_with_wrong_password_returns_401() {
    let state = test_state().await;
    let app = build_router(state);

    let req = Request::builder()
        .method(Method::POST)
        .uri("/auth/register")
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::ORIGIN, "http://localhost:5173")
        .body(json_body(serde_json::json!({
            "email": "bob@example.test",
            "password": "rightpw-12345"
        })))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);

    let req = Request::builder()
        .method(Method::POST)
        .uri("/auth/login")
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::ORIGIN, "http://localhost:5173")
        .body(json_body(serde_json::json!({
            "email": "bob@example.test",
            "password": "wrongpw"
        })))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn refresh_rotates_token() {
    let state = test_state().await;
    let app = build_router(state);

    // Register + login to obtain refresh cookie.
    let _ = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/auth/register")
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::ORIGIN, "http://localhost:5173")
                .body(json_body(serde_json::json!({
                    "email": "carol@example.test",
                    "password": "rightpw-12345"
                })))
                .unwrap(),
        )
        .await
        .unwrap();
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/auth/login")
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::ORIGIN, "http://localhost:5173")
                .body(json_body(serde_json::json!({
                    "email": "carol@example.test",
                    "password": "rightpw-12345"
                })))
                .unwrap(),
        )
        .await
        .unwrap();
    let (_, login_cookies, _) = read_body(resp).await;
    let refresh_pair = cookie_pair(&login_cookies, "arena_refresh").expect("refresh present");

    // First refresh: should succeed and return a NEW refresh cookie.
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/auth/refresh")
                .header(header::ORIGIN, "http://localhost:5173")
                .header(header::COOKIE, &refresh_pair)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let (status, cookies, _) = read_body(resp).await;
    assert_eq!(status, StatusCode::OK);
    let new_refresh_pair = cookie_pair(&cookies, "arena_refresh").expect("rotated refresh present");
    assert_ne!(
        refresh_pair, new_refresh_pair,
        "refresh cookie should rotate"
    );

    // Replaying the rotated cookie is covered by the two grace-window tests
    // below: tolerated while fresh, treated as reuse once stale.
}

/// Backdate every revoked refresh row so the next replay falls outside the
/// rotation-race grace window, without sleeping through it in the test.
async fn age_out_revoked_tokens(db: &sea_orm::DatabaseConnection) {
    use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
    let stale = chrono::Utc::now() - chrono::Duration::seconds(120);
    let rows = arena_core::entities::refresh_tokens::Entity::find()
        .filter(arena_core::entities::refresh_tokens::Column::RevokedAt.is_not_null())
        .all(db)
        .await
        .expect("load revoked rows");
    for row in rows {
        let mut am: arena_core::entities::refresh_tokens::ActiveModel = row.into();
        am.revoked_at = sea_orm::Set(Some(stale));
        sea_orm::ActiveModelTrait::update(am, db)
            .await
            .expect("backdate revoked_at");
    }
}

#[tokio::test]
async fn refresh_race_inside_grace_window_keeps_the_session() {
    // The SSR hook, the browser's expiry timer and parallel 401 retries all
    // refresh independently, so the losers of a rotation race present a token
    // that was revoked moments ago. That must hand out a working token rather
    // than log the user out.
    let state = test_state().await;
    let app = build_router(state);

    app.clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/auth/register")
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::ORIGIN, "http://localhost:5173")
                .body(json_body(serde_json::json!({
                    "email": "race@example.test",
                    "password": "rightpw-12345"
                })))
                .unwrap(),
        )
        .await
        .unwrap();
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/auth/login")
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::ORIGIN, "http://localhost:5173")
                .body(json_body(serde_json::json!({
                    "email": "race@example.test",
                    "password": "rightpw-12345"
                })))
                .unwrap(),
        )
        .await
        .unwrap();
    let (_, login_cookies, _) = read_body(resp).await;
    let r0 = cookie_pair(&login_cookies, "arena_refresh").expect("refresh present");

    // Winner rotates R0 -> R1.
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/auth/refresh")
                .header(header::ORIGIN, "http://localhost:5173")
                .header(header::COOKIE, &r0)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let (status, cookies, _) = read_body(resp).await;
    assert_eq!(status, StatusCode::OK);
    let r1 = cookie_pair(&cookies, "arena_refresh").expect("rotated refresh present");

    // Loser presents R0 immediately after — served, not punished.
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/auth/refresh")
                .header(header::ORIGIN, "http://localhost:5173")
                .header(header::COOKIE, &r0)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "a rotation race inside the grace window must not end the session"
    );

    // ...and the winner's token still works: no family revocation happened.
    let resp = app
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/auth/refresh")
                .header(header::ORIGIN, "http://localhost:5173")
                .header(header::COOKIE, &r1)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK, "family must stay alive");
}

#[tokio::test]
async fn refresh_prefers_a_live_cookie_over_a_stale_duplicate() {
    // Widening the refresh cookie from Path=/auth/refresh to Path=/ left older
    // browsers holding two cookies of the same name; both are sent, narrower
    // path first. Reading only the first would replay a frozen, long-revoked
    // value on every refresh and kill the session.
    let state = test_state().await;
    let db = state.db.clone();
    let app = build_router(state);

    app.clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/auth/register")
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::ORIGIN, "http://localhost:5173")
                .body(json_body(serde_json::json!({
                    "email": "dup@example.test",
                    "password": "rightpw-12345"
                })))
                .unwrap(),
        )
        .await
        .unwrap();
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/auth/login")
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::ORIGIN, "http://localhost:5173")
                .body(json_body(serde_json::json!({
                    "email": "dup@example.test",
                    "password": "rightpw-12345"
                })))
                .unwrap(),
        )
        .await
        .unwrap();
    let (_, login_cookies, _) = read_body(resp).await;
    let stale = cookie_pair(&login_cookies, "arena_refresh").expect("refresh present");

    // Rotate once so `stale` becomes the frozen legacy value, then age it past
    // the grace window — exactly the state a long-lived browser is in.
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/auth/refresh")
                .header(header::ORIGIN, "http://localhost:5173")
                .header(header::COOKIE, &stale)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let (status, cookies, _) = read_body(resp).await;
    assert_eq!(status, StatusCode::OK);
    let live = cookie_pair(&cookies, "arena_refresh").expect("rotated refresh present");
    age_out_revoked_tokens(&db).await;

    // Browser sends both, stale (narrower path) first.
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/auth/refresh")
                .header(header::ORIGIN, "http://localhost:5173")
                .header(header::COOKIE, format!("{stale}; {live}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let (status, cookies, _) = read_body(resp).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "the live duplicate must be found and used"
    );

    // The response also expires the legacy copy so the duplicate goes away.
    assert!(
        cookies
            .iter()
            .any(|c| c.starts_with("arena_refresh=;") && c.contains("Path=/auth/refresh")),
        "legacy-path refresh cookie should be expired: {cookies:?}"
    );
}

#[tokio::test]
async fn refresh_reuse_revokes_whole_family() {
    // SEC-M10: replaying a rotated (revoked) refresh token past the rotation-race
    // grace window is a reuse signal — the whole family is revoked, so even the
    // legitimate successor stops working.
    let state = test_state().await;
    let db = state.db.clone();
    let app = build_router(state);

    app.clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/auth/register")
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::ORIGIN, "http://localhost:5173")
                .body(json_body(serde_json::json!({
                    "email": "dave@example.test",
                    "password": "rightpw-12345"
                })))
                .unwrap(),
        )
        .await
        .unwrap();
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/auth/login")
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::ORIGIN, "http://localhost:5173")
                .body(json_body(serde_json::json!({
                    "email": "dave@example.test",
                    "password": "rightpw-12345"
                })))
                .unwrap(),
        )
        .await
        .unwrap();
    let (_, login_cookies, _) = read_body(resp).await;
    let r0 = cookie_pair(&login_cookies, "arena_refresh").expect("refresh present");

    // Rotate R0 -> R1.
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/auth/refresh")
                .header(header::ORIGIN, "http://localhost:5173")
                .header(header::COOKIE, &r0)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let (status, cookies, _) = read_body(resp).await;
    assert_eq!(status, StatusCode::OK);
    let r1 = cookie_pair(&cookies, "arena_refresh").expect("rotated refresh present");
    // Age the revocation out of the grace window: this is theft, not a race.
    age_out_revoked_tokens(&db).await;

    // Replay the revoked R0 → 401 and trip reuse detection.
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/auth/refresh")
                .header(header::ORIGIN, "http://localhost:5173")
                .header(header::COOKIE, &r0)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

    // The legitimate successor R1 must now be revoked too.
    let resp = app
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/auth/refresh")
                .header(header::ORIGIN, "http://localhost:5173")
                .header(header::COOKIE, &r1)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::UNAUTHORIZED,
        "reuse detection must revoke the whole family"
    );
}

#[tokio::test]
async fn origin_guard_blocks_disallowed_origin_on_post() {
    let state = test_state().await;
    let app = build_router(state);

    let req = Request::builder()
        .method(Method::POST)
        .uri("/auth/register")
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::ORIGIN, "http://evil.test")
        .body(json_body(serde_json::json!({
            "email": "x@x.test",
            "password": "secret-password-1"
        })))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    let (status, _cookies, body) = read_body(resp).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(body, serde_json::json!({"error": "origin_not_allowed"}));
}

#[tokio::test]
async fn origin_guard_allows_safe_method_get() {
    let state = test_state().await;
    let app = build_router(state);

    // GET /health from a disallowed origin should still pass (safe method).
    let req = Request::builder()
        .method(Method::GET)
        .uri("/health")
        .header(header::ORIGIN, "http://evil.test")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

/// The credential limiter gates /auth/login and /auth/register together:
/// with a cap of 2, register (1) + login (2) pass and the next login is 429.
#[tokio::test]
async fn auth_endpoints_rate_limited_after_cap() {
    let mut state = test_state().await;
    state.auth_rate_limiter = std::sync::Arc::new(server::rate_limiter::SlidingWindowLimiter::new(
        2,
        Duration::from_secs(60),
    ));
    let app = build_router(state);

    let register = Request::builder()
        .method(Method::POST)
        .uri("/auth/register")
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::ORIGIN, "http://localhost:5173")
        .body(json_body(serde_json::json!({
            "email": "bob@example.test",
            "password": "secret-password-1",
            "display_name": "Bob"
        })))
        .unwrap();
    let (status, _, body) = read_body(app.clone().oneshot(register).await.unwrap()).await;
    assert_eq!(status, StatusCode::CREATED, "register body={body}");

    let login = |app: axum::Router| async move {
        let req = Request::builder()
            .method(Method::POST)
            .uri("/auth/login")
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::ORIGIN, "http://localhost:5173")
            .body(json_body(serde_json::json!({
                "email": "bob@example.test",
                "password": "secret-password-1"
            })))
            .unwrap();
        read_body(app.oneshot(req).await.unwrap()).await
    };

    let (status, _, body) = login(app.clone()).await;
    assert_eq!(status, StatusCode::OK, "second request body={body}");

    let (status, _, body) = login(app).await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS, "body={body}");
    assert_eq!(body, serde_json::json!({"error": "rate_limited"}));
}
