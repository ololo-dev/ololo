//! Integration tests for the admin settings API — `allow_user_project_creation` key.
//!
//! Spec: Contract AC-001–AC-005.
//!
//! Routes exercised:
//!   GET  /api/admin/settings          — AC-001
//!   PUT  /api/admin/settings          — AC-002, AC-003, AC-004, AC-005

use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use http_body_util::BodyExt;
use migration::{Migrator, MigratorTrait};
use server::{AppState, AuthConfig, build_router};
use std::time::Duration;
use tower::ServiceExt;
use uuid::Uuid;

const ORIGIN: &str = "http://localhost:5173";

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

fn json_body(v: serde_json::Value) -> Body {
    Body::from(serde_json::to_vec(&v).expect("serialize body"))
}

async fn read_body(resp: axum::response::Response) -> (StatusCode, serde_json::Value) {
    let status = resp.status();
    let bytes = resp
        .into_body()
        .collect()
        .await
        .expect("collect body")
        .to_bytes();
    let value = if bytes.is_empty() {
        serde_json::Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null)
    };
    (status, value)
}

fn cookie_pair(set_cookies: &[String], name: &str) -> Option<String> {
    for sc in set_cookies {
        if let Some(rest) = sc.strip_prefix(&format!("{name}=")) {
            let value = rest.split(';').next().unwrap_or("");
            if !value.is_empty() {
                return Some(format!("{name}={value}"));
            }
        }
    }
    None
}

/// Register + login, return (user_id, access_cookie_header).
/// The first caller per in-memory DB becomes the admin user.
async fn register_and_login(app: axum::Router, email: &str, password: &str) -> (Uuid, String) {
    let register_resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/auth/register")
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::ORIGIN, ORIGIN)
                .body(json_body(
                    serde_json::json!({ "email": email, "password": password }),
                ))
                .expect("build register req"),
        )
        .await
        .expect("register resp");
    let status = register_resp.status();
    let bytes = register_resp
        .into_body()
        .collect()
        .await
        .expect("collect register body")
        .to_bytes();
    let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
    assert_eq!(status, StatusCode::CREATED, "register: {body}");
    let user_id: Uuid = serde_json::from_value(body["id"].clone()).expect("user id");

    let login_resp = app
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/auth/login")
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::ORIGIN, ORIGIN)
                .body(json_body(
                    serde_json::json!({ "email": email, "password": password }),
                ))
                .expect("build login req"),
        )
        .await
        .expect("login resp");
    let login_status = login_resp.status();
    let cookies: Vec<String> = login_resp
        .headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .filter_map(|hv| hv.to_str().ok().map(String::from))
        .collect();
    let _ = login_resp.into_body().collect().await;
    assert_eq!(login_status, StatusCode::OK, "login");
    let access = cookie_pair(&cookies, "arena_access").expect("access cookie");
    (user_id, access)
}

fn put_settings_req(cookie: &str, key: &str, value: &str) -> Request<Body> {
    Request::builder()
        .method(Method::PUT)
        .uri("/api/admin/settings")
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::ORIGIN, ORIGIN)
        .header(header::COOKIE, cookie)
        .body(json_body(serde_json::json!({ "key": key, "value": value })))
        .expect("build put settings req")
}

fn get_settings_req(cookie: &str) -> Request<Body> {
    Request::builder()
        .method(Method::GET)
        .uri("/api/admin/settings")
        .header(header::ORIGIN, ORIGIN)
        .header(header::COOKIE, cookie)
        .body(Body::empty())
        .expect("build get settings req")
}

// ─────────────────────────────────── Tests ───────────────────────────────────

/// AC-001: GET /api/admin/settings response includes the `allow_user_project_creation` key.
#[tokio::test]
async fn settings_get_includes_allow_user_project_creation() {
    let app = build_router(test_state().await);
    let (_, cookie) =
        register_and_login(app.clone(), "admin@settings1.test", "password-12345").await;

    let (status, body) = read_body(
        app.oneshot(get_settings_req(&cookie))
            .await
            .expect("get resp"),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "GET settings: {body}");
    assert!(
        body.get("allow_user_project_creation").is_some(),
        "allow_user_project_creation key should be present in GET response; body={body}"
    );
}

/// AC-002: PUT allow_user_project_creation = "true" → 200; subsequent GET returns "true".
#[tokio::test]
async fn settings_put_allow_project_creation_true_persists() {
    let app = build_router(test_state().await);
    let (_, cookie) =
        register_and_login(app.clone(), "admin@settings2.test", "password-12345").await;

    let (status, body) = read_body(
        app.clone()
            .oneshot(put_settings_req(
                &cookie,
                "allow_user_project_creation",
                "true",
            ))
            .await
            .expect("put resp"),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "PUT true: {body}");

    let (get_status, get_body) = read_body(
        app.oneshot(get_settings_req(&cookie))
            .await
            .expect("get resp"),
    )
    .await;
    assert_eq!(get_status, StatusCode::OK, "GET after PUT: {get_body}");
    assert_eq!(
        get_body["allow_user_project_creation"],
        serde_json::json!("true"),
        "GET should reflect persisted value; body={get_body}"
    );
}

/// AC-003: PUT allow_user_project_creation = "false" → 200; subsequent GET returns "false".
#[tokio::test]
async fn settings_put_allow_project_creation_false_persists() {
    let app = build_router(test_state().await);
    let (_, cookie) =
        register_and_login(app.clone(), "admin@settings3.test", "password-12345").await;

    let (status, body) = read_body(
        app.clone()
            .oneshot(put_settings_req(
                &cookie,
                "allow_user_project_creation",
                "false",
            ))
            .await
            .expect("put resp"),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "PUT false: {body}");

    let (get_status, get_body) = read_body(
        app.oneshot(get_settings_req(&cookie))
            .await
            .expect("get resp"),
    )
    .await;
    assert_eq!(get_status, StatusCode::OK, "GET after PUT: {get_body}");
    assert_eq!(
        get_body["allow_user_project_creation"],
        serde_json::json!("false"),
        "GET should reflect persisted value; body={get_body}"
    );
}

/// AC-004: PUT allow_user_project_creation = "yes" → 422.
#[tokio::test]
async fn settings_put_allow_project_creation_yes_is_422() {
    let app = build_router(test_state().await);
    let (_, cookie) =
        register_and_login(app.clone(), "admin@settings4.test", "password-12345").await;

    let (status, body) = read_body(
        app.oneshot(put_settings_req(
            &cookie,
            "allow_user_project_creation",
            "yes",
        ))
        .await
        .expect("put resp"),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "\"yes\" should be invalid; body={body}"
    );
}

/// AC-005: PUT allow_user_project_creation = "1" → 422.
#[tokio::test]
async fn settings_put_allow_project_creation_one_is_422() {
    let app = build_router(test_state().await);
    let (_, cookie) =
        register_and_login(app.clone(), "admin@settings5.test", "password-12345").await;

    let (status, body) = read_body(
        app.oneshot(put_settings_req(
            &cookie,
            "allow_user_project_creation",
            "1",
        ))
        .await
        .expect("put resp"),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "\"1\" should be invalid; body={body}"
    );
}

fn get_me_req(cookie: &str) -> Request<Body> {
    Request::builder()
        .method(Method::GET)
        .uri("/api/users/me")
        .header(header::ORIGIN, ORIGIN)
        .header(header::COOKIE, cookie)
        .body(Body::empty())
        .expect("build get me req")
}

/// FR-001 / AC-1: GET /api/users/me returns `allow_project_creation: false` when
/// the setting is explicitly set to "false".
#[tokio::test]
async fn get_me_returns_allow_project_creation_false_when_disabled() {
    let app = build_router(test_state().await);
    let (_, cookie) = register_and_login(app.clone(), "admin@me1.test", "password-12345").await;

    // Explicitly disable.
    let (put_status, put_body) = read_body(
        app.clone()
            .oneshot(put_settings_req(
                &cookie,
                "allow_user_project_creation",
                "false",
            ))
            .await
            .expect("put resp"),
    )
    .await;
    assert_eq!(put_status, StatusCode::OK, "PUT false: {put_body}");

    let (status, body) =
        read_body(app.oneshot(get_me_req(&cookie)).await.expect("get me resp")).await;
    assert_eq!(status, StatusCode::OK, "GET /api/users/me: {body}");
    assert_eq!(
        body["allow_project_creation"],
        serde_json::json!(false),
        "allow_project_creation should be false when setting is 'false'; body={body}"
    );
}

/// FR-001 / AC-1: GET /api/users/me returns `allow_project_creation: true` when the
/// setting is explicitly set to "true".
#[tokio::test]
async fn get_me_returns_allow_project_creation_true_when_enabled() {
    let app = build_router(test_state().await);
    let (_, cookie) = register_and_login(app.clone(), "admin@me2.test", "password-12345").await;

    // Set the setting to true via the admin endpoint.
    let (put_status, put_body) = read_body(
        app.clone()
            .oneshot(put_settings_req(
                &cookie,
                "allow_user_project_creation",
                "true",
            ))
            .await
            .expect("put resp"),
    )
    .await;
    assert_eq!(put_status, StatusCode::OK, "PUT true: {put_body}");

    let (status, body) =
        read_body(app.oneshot(get_me_req(&cookie)).await.expect("get me resp")).await;
    assert_eq!(status, StatusCode::OK, "GET /api/users/me: {body}");
    assert_eq!(
        body["allow_project_creation"],
        serde_json::json!(true),
        "allow_project_creation should be true after setting to 'true'; body={body}"
    );
}

// ---------------------------------------------------------------------------
// Account plans: tier judge-run limit settings + quota surface on /me
// ---------------------------------------------------------------------------

/// PUT plan_free_judge_run_limit with a non-negative integer persists and is
/// echoed by GET; whitespace is normalised away.
#[tokio::test]
async fn settings_put_plan_limits_persist() {
    let app = build_router(test_state().await);
    let (_, cookie) = register_and_login(app.clone(), "admin@plans1.test", "password-12345").await;

    for (key, value) in [
        ("plan_free_judge_run_limit", " 42 "),
        ("plan_premium_judge_run_limit", "5000"),
    ] {
        let (status, body) = read_body(
            app.clone()
                .oneshot(put_settings_req(&cookie, key, value))
                .await
                .expect("put resp"),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "PUT {key}: {body}");
    }

    let (get_status, get_body) = read_body(
        app.oneshot(get_settings_req(&cookie))
            .await
            .expect("get resp"),
    )
    .await;
    assert_eq!(get_status, StatusCode::OK, "GET after PUT: {get_body}");
    assert_eq!(
        get_body["plan_free_judge_run_limit"],
        serde_json::json!("42")
    );
    assert_eq!(
        get_body["plan_premium_judge_run_limit"],
        serde_json::json!("5000")
    );
}

/// Non-integer and negative plan limits are rejected with 422.
#[tokio::test]
async fn settings_put_plan_limit_invalid_is_422() {
    let app = build_router(test_state().await);
    let (_, cookie) = register_and_login(app.clone(), "admin@plans2.test", "password-12345").await;

    for value in ["lots", "-5", "1.5", ""] {
        let (status, body) = read_body(
            app.clone()
                .oneshot(put_settings_req(
                    &cookie,
                    "plan_free_judge_run_limit",
                    value,
                ))
                .await
                .expect("put resp"),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::UNPROCESSABLE_ENTITY,
            "{value:?} should be invalid; body={body}"
        );
    }
}

/// GET /api/users/me carries the account plan and this month's judge-run
/// quota; a per-user override set through PATCH /api/admin/users/:id wins
/// over the tier limit, and an explicit null clears it back.
#[tokio::test]
async fn me_carries_plan_and_judge_run_quota() {
    let app = build_router(test_state().await);
    let (user_id, cookie) =
        register_and_login(app.clone(), "admin@plans3.test", "password-12345").await;

    let (status, body) =
        read_body(app.clone().oneshot(get_me_req(&cookie)).await.expect("me")).await;
    assert_eq!(status, StatusCode::OK, "GET me: {body}");
    // Registration lands on the free tier — premium is bought, not given
    // (billing exists now; the permissive default died with it).
    assert_eq!(body["plan"], serde_json::json!("free"));
    assert_eq!(body["judge_runs_used"], serde_json::json!(0));
    assert_eq!(body["judge_run_limit"], serde_json::json!(100));
    // Tiers are off until the switch is explicitly thrown.
    assert_eq!(body["plans_enabled"], serde_json::json!(false));

    let (status, put_body) = read_body(
        app.clone()
            .oneshot(put_settings_req(&cookie, "plans_enabled", "TRUE"))
            .await
            .expect("put resp"),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "PUT plans_enabled: {put_body}");
    let (status, body) =
        read_body(app.clone().oneshot(get_me_req(&cookie)).await.expect("me")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body["plans_enabled"],
        serde_json::json!(true),
        "case-insensitive true should enable tiers; body={body}"
    );

    let patch = |json: serde_json::Value| {
        Request::builder()
            .method(Method::PATCH)
            .uri(format!("/api/admin/users/{user_id}"))
            .header(header::ORIGIN, ORIGIN)
            .header(header::COOKIE, cookie.clone())
            .header(header::CONTENT_TYPE, "application/json")
            .body(json_body(json))
            .expect("request")
    };

    // Downgrade to free with a personal override.
    let (status, body) = read_body(
        app.clone()
            .oneshot(patch(
                serde_json::json!({ "plan": "free", "judge_run_limit": 7 }),
            ))
            .await
            .expect("patch resp"),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "PATCH plan: {body}");
    assert_eq!(body["user"]["plan"], serde_json::json!("free"));
    assert_eq!(body["user"]["judge_run_limit"], serde_json::json!(7));
    assert_eq!(
        body["user"]["judge_run_limit_effective"],
        serde_json::json!(7)
    );

    let (status, body) =
        read_body(app.clone().oneshot(get_me_req(&cookie)).await.expect("me")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["plan"], serde_json::json!("free"));
    assert_eq!(body["judge_run_limit"], serde_json::json!(7));

    // Clearing the override falls back to the free tier default.
    let (status, body) = read_body(
        app.clone()
            .oneshot(patch(serde_json::json!({ "judge_run_limit": null })))
            .await
            .expect("patch resp"),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "PATCH clear: {body}");
    assert_eq!(body["user"]["judge_run_limit"], serde_json::Value::Null);
    assert_eq!(
        body["user"]["judge_run_limit_effective"],
        serde_json::json!(100)
    );

    // An unknown plan value is rejected.
    let (status, body) = read_body(
        app.oneshot(patch(serde_json::json!({ "plan": "gold" })))
            .await
            .expect("patch resp"),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "unknown plan should 422; body={body}"
    );
}

/// `GET /api/public/plans` is anonymous: disabled by default with the tier
/// defaults; after the admin throws the switch and raises a limit the same
/// call reflects both.
#[tokio::test]
async fn public_plans_reflect_settings() {
    let app = build_router(test_state().await);

    let plans_req = || {
        Request::builder()
            .method(Method::GET)
            .uri("/api/public/plans")
            .header(header::ORIGIN, ORIGIN)
            .body(Body::empty())
            .expect("request")
    };

    let (status, body) = read_body(app.clone().oneshot(plans_req()).await.expect("resp")).await;
    assert_eq!(status, StatusCode::OK, "anonymous GET plans: {body}");
    assert_eq!(body["enabled"], serde_json::json!(false));
    assert_eq!(body["free"]["judge_run_limit"], serde_json::json!(100));
    assert_eq!(body["premium"]["judge_run_limit"], serde_json::json!(1000));

    let (_, cookie) = register_and_login(app.clone(), "admin@plans4.test", "password-12345").await;
    for (key, value) in [
        ("plans_enabled", "true"),
        ("plan_premium_judge_run_limit", "2500"),
    ] {
        let (status, body) = read_body(
            app.clone()
                .oneshot(put_settings_req(&cookie, key, value))
                .await
                .expect("put resp"),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "PUT {key}: {body}");
    }

    let (status, body) = read_body(app.clone().oneshot(plans_req()).await.expect("resp")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["enabled"], serde_json::json!(true));
    assert_eq!(body["premium"]["judge_run_limit"], serde_json::json!(2500));
}

/// `GET /api/admin/users` — the account console: who is on which plan, how
/// much of the monthly judge-run allowance they have spent, and what limit
/// is actually in force. Admin-only; these numbers are what support and
/// billing decisions are made on.
#[tokio::test]
async fn admin_user_listing_reports_plan_usage_and_effective_limit() {
    use arena_core::entities::judge_run_ledger;
    use sea_orm::{ActiveModelTrait, ActiveValue::Set};

    let state = test_state().await;
    let db = state.db.clone();
    let app = build_router(state);

    // The first registrant is the admin; the second is an ordinary account.
    let (_admin_id, admin_cookie) =
        register_and_login(app.clone(), "console@users.test", "password-12345").await;
    let (member_id, member_cookie) =
        register_and_login(app.clone(), "counted@users.test", "password-12345").await;

    // Two metered runs this month, charged to the member's account.
    for _ in 0..2 {
        judge_run_ledger::ActiveModel {
            id: Set(Uuid::new_v4()),
            user_id_fk: Set(member_id),
            session_id: Set(Uuid::new_v4()),
            player_id: Set(Uuid::new_v4()),
            judge_id: Set(Uuid::new_v4()),
            created_at: Set(chrono::Utc::now()),
            source: Set("monthly".to_string()),
        }
        .insert(&db)
        .await
        .expect("ledger row");
    }

    let list_req = |cookie: &str| {
        Request::builder()
            .method(Method::GET)
            .uri("/api/admin/users")
            .header(header::ORIGIN, ORIGIN)
            .header(header::COOKIE, cookie.to_string())
            .body(Body::empty())
            .expect("build req")
    };

    // A non-admin never sees other people's accounts.
    let resp = app
        .clone()
        .oneshot(list_req(&member_cookie))
        .await
        .expect("member resp");
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);

    let (status, body) = read_body(
        app.clone()
            .oneshot(list_req(&admin_cookie))
            .await
            .expect("admin resp"),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "listing: {body}");

    let users = body["users"].as_array().expect("users array");
    let counted = users
        .iter()
        .find(|u| u["email"] == "counted@users.test")
        .expect("the metered account is listed");
    assert_eq!(counted["judge_runs_this_month"], 2, "this month's runs");
    assert_eq!(counted["plan"], arena_core::quota::PLAN_FREE);
    assert!(
        counted["judge_run_limit"].is_null(),
        "no per-user override by default"
    );
    assert_eq!(
        counted["judge_run_limit_effective"],
        arena_core::quota::DEFAULT_FREE_JUDGE_RUN_LIMIT,
        "with no override the tier limit is what applies"
    );

    let idle = users
        .iter()
        .find(|u| u["email"] == "console@users.test")
        .expect("every account is listed");
    assert_eq!(
        idle["judge_runs_this_month"], 0,
        "an idle account spent none"
    );
}

// ───────────────────── session replay switch (admin-only tool) ────────────────

fn me_req(cookie: &str) -> Request<Body> {
    Request::builder()
        .method(Method::GET)
        .uri("/api/users/me")
        .header(header::ORIGIN, ORIGIN)
        .header(header::COOKIE, cookie)
        .body(Body::empty())
        .expect("build me request")
}

/// The replay is offered unless an admin says otherwise: an instance that
/// never touched the switch keeps the bar it already had.
#[tokio::test]
async fn session_replay_defaults_to_on() {
    let app = build_router(test_state().await);
    let (_, cookie) = register_and_login(app.clone(), "admin@replay1.test", "password-12345").await;

    let (status, body) = read_body(app.oneshot(me_req(&cookie)).await.expect("me resp")).await;
    assert_eq!(status, StatusCode::OK, "GET me: {body}");
    assert_eq!(
        body["session_replay_enabled"],
        serde_json::json!(true),
        "an unset switch reads as on; body={body}"
    );
}

/// Turning it off reaches the pages through `/users/me`, which is where the
/// layout reads it — the bar is gated on this and on being an admin.
#[tokio::test]
async fn session_replay_switch_persists_and_reaches_me() {
    let app = build_router(test_state().await);
    let (_, cookie) = register_and_login(app.clone(), "admin@replay2.test", "password-12345").await;

    let (status, body) = read_body(
        app.clone()
            .oneshot(put_settings_req(&cookie, "session_replay_enabled", "FALSE"))
            .await
            .expect("put resp"),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "PUT off: {body}");

    let (_, settings) = read_body(
        app.clone()
            .oneshot(get_settings_req(&cookie))
            .await
            .expect("get resp"),
    )
    .await;
    assert_eq!(
        settings["session_replay_enabled"],
        serde_json::json!("false"),
        "stored lowercase so the gate's exact check matches; body={settings}"
    );

    let (_, me) = read_body(app.oneshot(me_req(&cookie)).await.expect("me resp")).await;
    assert_eq!(
        me["session_replay_enabled"],
        serde_json::json!(false),
        "the page reads the switch through me; body={me}"
    );
}

/// Anything that is not a boolean is refused, so the gate can never read a
/// value it does not understand.
#[tokio::test]
async fn session_replay_switch_refuses_non_boolean() {
    let app = build_router(test_state().await);
    let (_, cookie) = register_and_login(app.clone(), "admin@replay3.test", "password-12345").await;

    let (status, _) = read_body(
        app.oneshot(put_settings_req(
            &cookie,
            "session_replay_enabled",
            "sometimes",
        ))
        .await
        .expect("put resp"),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
}
