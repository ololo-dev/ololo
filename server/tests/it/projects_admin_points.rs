#![allow(dead_code)]

//! Projects CRUD integration tests.
//!
//! Spec: contract.md FR-001 (project ownership), FR-002 (project list),
//! FR-005 (project delete RESTRICT), FR-006 (freeze-while-running),
//! FR-007 (archive via soft-retire), FR-008 (public flag).

use axum::http::{Method, StatusCode};
use sea_orm::ConnectionTrait;
use server::build_router;
use tower::ServiceExt;

async fn create_project(
    app: &axum::Router,
    cookie: &str,
    name: &str,
) -> (StatusCode, serde_json::Value) {
    let resp = app
        .clone()
        .oneshot(req(
            Method::POST,
            "/api/projects",
            Some(cookie),
            Some(serde_json::json!({ "name": name })),
        ))
        .await
        .expect("create project resp");
    let (s, _, b) = read_body(resp).await;
    (s, b)
}

async fn create_project_private(
    app: &axum::Router,
    cookie: &str,
    name: &str,
) -> (StatusCode, serde_json::Value) {
    let resp = app
        .clone()
        .oneshot(req(
            Method::POST,
            "/api/projects",
            Some(cookie),
            Some(serde_json::json!({ "name": name, "public": false })),
        ))
        .await
        .expect("create project resp");
    let (s, _, b) = read_body(resp).await;
    (s, b)
}

async fn create_session(
    app: &axum::Router,
    cookie: &str,
    name: &str,
    project_id: &str,
) -> (StatusCode, serde_json::Value) {
    let resp = app
        .clone()
        .oneshot(req(
            Method::POST,
            "/api/sessions",
            Some(cookie),
            Some(serde_json::json!({ "name": name, "project_id": project_id })),
        ))
        .await
        .expect("create session resp");
    let (s, _, b) = read_body(resp).await;
    (s, b)
}

async fn create_project_with_slug(
    app: &axum::Router,
    cookie: &str,
    name: &str,
    slug: Option<&str>,
) -> (StatusCode, serde_json::Value) {
    let resp = app
        .clone()
        .oneshot(req(
            Method::POST,
            "/api/projects",
            Some(cookie),
            Some(serde_json::json!({ "name": name })),
        ))
        .await
        .expect("create project resp");
    let (post_status, _, post_body) = read_body(resp).await;
    let slug = match slug {
        None => return (post_status, post_body),
        Some(_) if post_status != StatusCode::CREATED => return (post_status, post_body),
        Some(s) => s,
    };
    let pid = post_body["id"]
        .as_str()
        .expect("project id for slug patch")
        .to_string();
    let resp = app
        .clone()
        .oneshot(req(
            Method::PATCH,
            &format!("/api/projects/{pid}"),
            Some(cookie),
            Some(serde_json::json!({ "slug": slug })),
        ))
        .await
        .expect("patch slug resp");
    let (status, _, body) = read_body(resp).await;
    (status, body)
}

// ─────────────────────────── Tests ───────────────────────────

use crate::common;
use crate::common::*;

#[tokio::test]
async fn create_project_allowed_for_non_admin_when_setting_true() {
    let state = test_state().await;
    let app = build_router(state);
    // Alice is admin; Bob is non-admin.
    let (_, alice_cookie) =
        register_and_login(app.clone(), "alice@gate3.test", "password-12345").await;
    let (_, bob_cookie) = register_and_login(app.clone(), "bob@gate3.test", "password-12345").await;

    // Ensure setting is "true" (it is after migration, but be explicit).
    set_project_creation_setting(&app, &alice_cookie, "true").await;

    // Bob (non-admin) creates a project — must succeed.
    let (status, body) = create_project(&app, &bob_cookie, "Bob's Project").await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "non-admin should be allowed when setting=true; body={body}"
    );
}

// ──────────── Admin cross-ownership tests (Contract: FR-001, FR-002, FR-003, FR-004) ────────────

#[tokio::test]
async fn admin_can_get_one_not_owned() {
    let state = test_state().await;
    let app = build_router(state);
    // admin registers first → is_admin = true
    let (_, admin_cookie) = register_and_login(app.clone(), "admin@x.test", "password-12345").await;
    // user_a registers second → not admin
    let (_, cookie_a) = register_and_login(app.clone(), "usera@x.test", "password-12345").await;

    // user_a creates a private project
    let (s, body) = create_project_private(&app, &cookie_a, "user-a-private").await;
    assert_eq!(s, StatusCode::CREATED, "create: {body}");
    let pid = body["id"].as_str().expect("pid").to_string();

    // admin can GET it
    let resp = app
        .oneshot(req(
            Method::GET,
            &format!("/api/projects/{pid}"),
            Some(&admin_cookie),
            None,
        ))
        .await
        .expect("get resp");
    let (status, _, body) = read_body(resp).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "admin should see private project: {body}"
    );
    assert_eq!(body["id"], serde_json::json!(pid));
}

#[tokio::test]
async fn admin_can_patch_one_not_owned() {
    let state = test_state().await;
    let app = build_router(state);
    let (_, admin_cookie) = register_and_login(app.clone(), "admin@x.test", "password-12345").await;
    let (_, cookie_a) = register_and_login(app.clone(), "usera@x.test", "password-12345").await;

    let (s, body) = create_project(&app, &cookie_a, "original-name").await;
    assert_eq!(s, StatusCode::CREATED, "create: {body}");
    let pid = body["id"].as_str().expect("pid").to_string();

    let resp = app
        .oneshot(req(
            Method::PATCH,
            &format!("/api/projects/{pid}"),
            Some(&admin_cookie),
            Some(serde_json::json!({ "name": "admin-renamed" })),
        ))
        .await
        .expect("patch resp");
    let (status, _, body) = read_body(resp).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "admin should patch not-owned: {body}"
    );
    assert_eq!(body["name"], serde_json::json!("admin-renamed"));
}

#[tokio::test]
async fn admin_patch_slug_collides_in_owner_namespace() {
    let state = test_state().await;
    let app = build_router(state);
    let (_, admin_cookie) = register_and_login(app.clone(), "admin@x.test", "password-12345").await;
    let (_, cookie_a) = register_and_login(app.clone(), "usera@x.test", "password-12345").await;

    // user_a has two projects; admin sets slug "taken-slug" on project-1
    let (s1, b1) = create_project(&app, &cookie_a, "Project A").await;
    assert_eq!(s1, StatusCode::CREATED, "create A: {b1}");
    let pid1 = b1["id"].as_str().expect("pid1").to_string();
    let resp = app
        .clone()
        .oneshot(req(
            Method::PATCH,
            &format!("/api/projects/{pid1}"),
            Some(&admin_cookie),
            Some(serde_json::json!({ "slug": "taken-slug" })),
        ))
        .await
        .expect("set slug resp");
    let (ss, _, sb) = read_body(resp).await;
    assert_eq!(ss, StatusCode::OK, "set slug on project A: {sb}");

    let (s2, b2) = create_project(&app, &cookie_a, "Project B").await;
    assert_eq!(s2, StatusCode::CREATED, "create B: {b2}");
    let pid2 = b2["id"].as_str().expect("pid2").to_string();

    // admin tries to set project B's slug to "taken-slug" (same owner namespace) → 409
    let resp = app
        .oneshot(req(
            Method::PATCH,
            &format!("/api/projects/{pid2}"),
            Some(&admin_cookie),
            Some(serde_json::json!({ "slug": "taken-slug" })),
        ))
        .await
        .expect("patch resp");
    let (status, _, body) = read_body(resp).await;
    assert_eq!(
        status,
        StatusCode::CONFLICT,
        "should conflict in owner namespace: {body}"
    );
    assert_eq!(body["error"], serde_json::json!("slug_conflict"));
}

#[tokio::test]
async fn admin_patch_slug_no_collision_across_owner_namespace() {
    let state = test_state().await;
    let app = build_router(state);
    let (_, admin_cookie) = register_and_login(app.clone(), "admin@x.test", "password-12345").await;
    let (_, cookie_a) = register_and_login(app.clone(), "usera@x.test", "password-12345").await;
    let (_, cookie_b) = register_and_login(app.clone(), "userb@x.test", "password-12345").await;

    // user_a has slug "shared-slug" — set via admin on a private project
    let (sa, ba) = create_project(&app, &cookie_a, "A Project").await;
    assert_eq!(sa, StatusCode::CREATED, "create A: {ba}");
    let pid_a = ba["id"].as_str().expect("pid_a").to_string();
    // make private first (avoids the public-slug unique constraint), then set slug
    let resp = app
        .clone()
        .oneshot(req(
            Method::PATCH,
            &format!("/api/projects/{pid_a}"),
            Some(&admin_cookie),
            Some(serde_json::json!({ "public": false, "slug": "shared-slug" })),
        ))
        .await
        .expect("set slug resp");
    let (ss, _, sb) = read_body(resp).await;
    assert_eq!(ss, StatusCode::OK, "set slug for user_a: {sb}");

    // user_b creates a project; admin makes it private and sets its slug to "shared-slug" (different owner namespace) → 200
    let (sc, bc) = create_project(&app, &cookie_b, "B Project").await;
    assert_eq!(sc, StatusCode::CREATED, "create B: {bc}");
    let pid_b = bc["id"].as_str().expect("pid_b").to_string();

    let resp = app
        .oneshot(req(
            Method::PATCH,
            &format!("/api/projects/{pid_b}"),
            Some(&admin_cookie),
            Some(serde_json::json!({ "public": false, "slug": "shared-slug" })),
        ))
        .await
        .expect("patch resp");
    let (status, _, body) = read_body(resp).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "slug should not collide across owners: {body}"
    );
    assert_eq!(body["slug"], serde_json::json!("shared-slug"));
}

#[tokio::test]
async fn admin_can_delete_one_not_owned() {
    let state = test_state().await;
    let app = build_router(state);
    let (_, admin_cookie) = register_and_login(app.clone(), "admin@x.test", "password-12345").await;
    let (_, cookie_a) = register_and_login(app.clone(), "usera@x.test", "password-12345").await;

    let (s, body) = create_project(&app, &cookie_a, "to-delete").await;
    assert_eq!(s, StatusCode::CREATED, "create: {body}");
    let pid = body["id"].as_str().expect("pid").to_string();

    let resp = app
        .clone()
        .oneshot(req(
            Method::DELETE,
            &format!("/api/projects/{pid}"),
            Some(&admin_cookie),
            None,
        ))
        .await
        .expect("delete resp");
    assert_eq!(
        resp.status(),
        StatusCode::NO_CONTENT,
        "admin should delete not-owned"
    );

    // Verify gone
    let resp = app
        .oneshot(req(
            Method::GET,
            &format!("/api/projects/{pid}"),
            Some(&admin_cookie),
            None,
        ))
        .await
        .expect("get resp");
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn non_admin_cannot_get_private_project_of_other_user() {
    // Regression guard: verifies non-admin path is unchanged after admin bypass introduced.
    let state = test_state().await;
    let app = build_router(state);
    // alice registers first → admin (can create private projects)
    let (_, cookie_alice) = register_and_login(app.clone(), "alice@x.test", "password-12345").await;
    // bob registers second → not admin
    let (_, cookie_bob) = register_and_login(app.clone(), "bob@x.test", "password-12345").await;

    // alice (admin) creates a private project
    let (s, body) = create_project_private(&app, &cookie_alice, "alice-private").await;
    assert_eq!(s, StatusCode::CREATED, "create: {body}");
    let pid = body["id"].as_str().expect("pid").to_string();

    // bob (non-admin) tries to GET alice's private project → 403
    let resp = app
        .oneshot(req(
            Method::GET,
            &format!("/api/projects/{pid}"),
            Some(&cookie_bob),
            None,
        ))
        .await
        .expect("get resp");
    assert_eq!(
        resp.status(),
        StatusCode::FORBIDDEN,
        "non-admin must not see private project of other user"
    );
}

/// AC-009: Fail-closed — POST /api/projects by non-admin returns 403 when row is absent.
#[tokio::test]
async fn create_project_blocked_when_setting_row_absent_fail_closed() {
    let state = test_state().await;
    // Remove the seeded row to simulate an upgrade where the migration ran
    // but the row wasn't present before (ON CONFLICT DO NOTHING did nothing).
    state
        .db
        .execute_unprepared("DELETE FROM app_settings WHERE key = 'allow_user_project_creation'")
        .await
        .expect("delete setting row");

    let app = build_router(state);
    // Alice registers first → admin; Bob registers second → non-admin.
    // We register Alice just to ensure Bob is not the first (admin) user.
    let (_, _alice_cookie) =
        register_and_login(app.clone(), "alice@gate4.test", "password-12345").await;
    let (_, bob_cookie) = register_and_login(app.clone(), "bob@gate4.test", "password-12345").await;

    // Bob (non-admin) tries to create — absent row is treated as "false".
    let (status, body) = create_project(&app, &bob_cookie, "Bob's Project").await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "absent setting row should be fail-closed (403); body={body}"
    );
}

// ── points + points_range on ProjectSummary ──────────────────────────────

#[tokio::test]
async fn create_project_returns_points_defaults_and_null_range() {
    let state = test_state().await;
    let app = build_router(state);
    let (_, cookie) = register_and_login(app.clone(), "alice@pts.test", "password-12345").await;

    let (status, body) = create_project(&app, &cookie, "pts-proj").await;
    assert_eq!(status, StatusCode::CREATED, "body={body}");
    // Hard baseline defaults: 10 / -5 / -10 / 10.
    assert_eq!(body["points"]["value"], serde_json::json!(10));
    assert_eq!(body["points"]["fail"], serde_json::json!(-5));
    assert_eq!(body["points"]["no_response"], serde_json::json!(-10));
    assert_eq!(body["points"]["completion_bonus"], serde_json::json!(10));
    // No tasks yet → points_range is null.
    assert!(body["points_range"].is_null());
}

#[tokio::test]
async fn create_project_honors_explicit_points_overrides() {
    let state = test_state().await;
    let app = build_router(state);
    let (_, cookie) = register_and_login(app.clone(), "alice@pts2.test", "password-12345").await;

    let resp = app
        .clone()
        .oneshot(req(
            Method::POST,
            "/api/projects",
            Some(&cookie),
            Some(serde_json::json!({
                "name": "pts-override",
                "points": { "value": 42, "fail": -1, "no_response": -2, "completion_bonus": 7 }
            })),
        ))
        .await
        .expect("resp");
    let (status, _, body) = read_body(resp).await;
    assert_eq!(status, StatusCode::CREATED, "body={body}");
    assert_eq!(body["points"]["value"], serde_json::json!(42));
    assert_eq!(body["points"]["fail"], serde_json::json!(-1));
    assert_eq!(body["points"]["no_response"], serde_json::json!(-2));
    assert_eq!(body["points"]["completion_bonus"], serde_json::json!(7));
}
