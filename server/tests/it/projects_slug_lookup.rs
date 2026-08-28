//! Projects CRUD integration tests.
//!
//! Spec: contract.md FR-001 (project ownership), FR-002 (project list),
//! FR-005 (project delete RESTRICT), FR-006 (freeze-while-running),
//! FR-007 (archive via soft-retire), FR-008 (public flag).

use axum::http::{Method, StatusCode};
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
async fn get_by_slug_public_200() {
    let state = test_state().await;
    let app = build_router(state);
    let (_, cookie) = register_and_login(app.clone(), "alice@x.test", "password-12345").await;

    // Alice is first registered user → admin; create project, set slug + make public in one PATCH
    let (cs, cb) = create_project(&app, &cookie, "pub-project").await;
    assert_eq!(cs, StatusCode::CREATED, "create: {cb}");
    let pid = cb["id"].as_str().expect("pid").to_string();
    let resp = app
        .clone()
        .oneshot(req(
            Method::PATCH,
            &format!("/api/projects/{pid}"),
            Some(&cookie),
            Some(serde_json::json!({ "slug": "pub-slug", "public": true })),
        ))
        .await
        .expect("patch resp");
    let (ps, _, pb) = read_body(resp).await;
    assert_eq!(ps, StatusCode::OK, "patch slug+public: {pb}");

    // Fetch by slug without auth
    let resp = app
        .oneshot(req(
            Method::GET,
            "/api/projects/by-slug/pub-slug",
            None,
            None,
        ))
        .await
        .expect("get resp");
    let (status, _, body) = read_body(resp).await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    assert_eq!(body["slug"], serde_json::json!("pub-slug"));
}

#[tokio::test]
async fn get_by_slug_private_404() {
    let state = test_state().await;
    let app = build_router(state);
    let (_, cookie) = register_and_login(app.clone(), "alice@x.test", "password-12345").await;

    // Alice is admin; create a private project and set slug via PATCH
    let (cs, cb) = create_project_private(&app, &cookie, "priv-project").await;
    assert_eq!(cs, StatusCode::CREATED, "create: {cb}");
    let pid = cb["id"].as_str().expect("pid").to_string();
    let resp = app
        .clone()
        .oneshot(req(
            Method::PATCH,
            &format!("/api/projects/{pid}"),
            Some(&cookie),
            Some(serde_json::json!({ "slug": "priv-slug" })),
        ))
        .await
        .expect("patch resp");
    let (ps, _, pb) = read_body(resp).await;
    assert_eq!(ps, StatusCode::OK, "patch slug: {pb}");

    // Fetch by slug without auth — must return 404 (no 403 — enumeration protection)
    let resp = app
        .oneshot(req(
            Method::GET,
            "/api/projects/by-slug/priv-slug",
            None,
            None,
        ))
        .await
        .expect("get resp");
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn get_by_user_slug_owner_200() {
    let state = test_state().await;
    let app = build_router(state);
    let (uid, cookie) = register_and_login(app.clone(), "alice@x.test", "password-12345").await;

    // Alice is admin; create project and set slug via PATCH
    let (cs, cb) = create_project(&app, &cookie, "owner-proj").await;
    assert_eq!(cs, StatusCode::CREATED, "create: {cb}");
    let pid = cb["id"].as_str().expect("pid").to_string();
    let resp = app
        .clone()
        .oneshot(req(
            Method::PATCH,
            &format!("/api/projects/{pid}"),
            Some(&cookie),
            Some(serde_json::json!({ "slug": "owner-slug" })),
        ))
        .await
        .expect("patch resp");
    let (ps, _, pb) = read_body(resp).await;
    assert_eq!(ps, StatusCode::OK, "patch slug: {pb}");

    let resp = app
        .oneshot(req(
            Method::GET,
            &format!("/api/projects/u/{uid}/by-slug/owner-slug"),
            Some(&cookie),
            None,
        ))
        .await
        .expect("get resp");
    let (status, _, body) = read_body(resp).await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    assert_eq!(body["slug"], serde_json::json!("owner-slug"));
}

#[tokio::test]
async fn get_by_user_slug_wrong_owner_404() {
    let state = test_state().await;
    let app = build_router(state);
    let (uid_a, cookie_a) = register_and_login(app.clone(), "alice@x.test", "password-12345").await;
    let (_, cookie_b) = register_and_login(app.clone(), "bob@x.test", "password-12345").await;

    // Alice is admin; create project and set slug via PATCH
    let (cs, cb) = create_project(&app, &cookie_a, "alice-proj").await;
    assert_eq!(cs, StatusCode::CREATED, "create: {cb}");
    let pid = cb["id"].as_str().expect("pid").to_string();
    let resp = app
        .clone()
        .oneshot(req(
            Method::PATCH,
            &format!("/api/projects/{pid}"),
            Some(&cookie_a),
            Some(serde_json::json!({ "slug": "alice-slug" })),
        ))
        .await
        .expect("patch resp");
    let (ps, _, pb) = read_body(resp).await;
    assert_eq!(ps, StatusCode::OK, "patch slug: {pb}");

    // Bob tries to access Alice's project by user+slug — must 404
    let resp = app
        .oneshot(req(
            Method::GET,
            &format!("/api/projects/u/{uid_a}/by-slug/alice-slug"),
            Some(&cookie_b),
            None,
        ))
        .await
        .expect("get resp");
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

// ─────────────────────────── WP-004: has_active_sessions ─────────────────────

#[tokio::test]
async fn get_one_has_active_sessions_false() {
    let state = test_state().await;
    let app = build_router(state);
    let (_, cookie) = register_and_login(app.clone(), "alice@x.test", "password-12345").await;
    let (_, body) = create_project(&app, &cookie, "proj").await;
    let pid = body["id"].as_str().expect("pid").to_string();

    let resp = app
        .oneshot(req(
            Method::GET,
            &format!("/api/projects/{pid}"),
            Some(&cookie),
            None,
        ))
        .await
        .expect("get resp");
    let (status, _, body) = read_body(resp).await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    assert_eq!(
        body["has_active_sessions"],
        serde_json::json!(false),
        "no sessions → has_active_sessions should be false, got {body}"
    );
}

#[tokio::test]
async fn get_one_has_active_sessions_true() {
    let state = test_state().await;
    let app = build_router(state);
    let (_, cookie) = register_and_login(app.clone(), "alice@x.test", "password-12345").await;
    let (_, body) = create_project(&app, &cookie, "proj").await;
    let pid = body["id"].as_str().expect("pid").to_string();

    // Create a session in lobby status (default)
    let (ss, _) = create_session(&app, &cookie, "arena-session", &pid).await;
    assert_eq!(ss, StatusCode::CREATED);

    let resp = app
        .oneshot(req(
            Method::GET,
            &format!("/api/projects/{pid}"),
            Some(&cookie),
            None,
        ))
        .await
        .expect("get resp");
    let (status, _, body) = read_body(resp).await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    assert_eq!(
        body["has_active_sessions"],
        serde_json::json!(true),
        "lobby session → has_active_sessions should be true, got {body}"
    );
}

// ─────────────────────────── Slug / Description tests ────────────────────────

#[tokio::test]
async fn create_project_has_description_default_empty() {
    let state = test_state().await;
    let app = build_router(state);
    let (_, cookie) = register_and_login(app.clone(), "alice@x.test", "password-12345").await;

    let (status, body) = create_project(&app, &cookie, "my-project").await;
    assert_eq!(status, StatusCode::CREATED, "body={body}");
    // description field must be present and default to empty string
    assert_eq!(
        body["description"],
        serde_json::json!(""),
        "description should default to empty string, got {body}"
    );
}

#[tokio::test]
async fn create_project_slug_persists() {
    let state = test_state().await;
    let app = build_router(state);
    let (_, cookie) = register_and_login(app.clone(), "alice@x.test", "password-12345").await;

    let (status, body) =
        create_project_with_slug(&app, &cookie, "My Project", Some("my-project")).await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    assert_eq!(
        body["slug"],
        serde_json::json!("my-project"),
        "slug should persist as provided, got {body}"
    );
}

#[tokio::test]
async fn create_project_auto_slug_from_name() {
    let state = test_state().await;
    let app = build_router(state);
    let (_, cookie) = register_and_login(app.clone(), "alice@x.test", "password-12345").await;

    // No explicit slug — server should leave slug as null
    let (status, body) = create_project(&app, &cookie, "Hello World").await;
    assert_eq!(status, StatusCode::CREATED, "body={body}");
    assert!(
        body["slug"].is_null(),
        "slug should be null when not explicitly set, got {body}"
    );
}

#[tokio::test]
async fn duplicate_slug_same_owner_409() {
    let state = test_state().await;
    let app = build_router(state);
    let (_, cookie) = register_and_login(app.clone(), "alice@x.test", "password-12345").await;

    let (s1, _) = create_project_with_slug(&app, &cookie, "Project A", Some("my-slug")).await;
    assert_eq!(s1, StatusCode::OK);

    // Second project with same slug for same owner — should conflict
    let (s2, body2) = create_project_with_slug(&app, &cookie, "Project B", Some("my-slug")).await;
    assert_eq!(
        s2,
        StatusCode::CONFLICT,
        "duplicate slug same owner should be 409, got body={body2}"
    );
}

#[tokio::test]
async fn non_admin_patch_slug_forbidden_403() {
    let state = test_state().await;
    let app = build_router(state);
    // alice is first registered → admin; bob is second → not admin
    let (_, _cookie_a) = register_and_login(app.clone(), "alice@x.test", "password-12345").await;
    let (_, cookie_b) = register_and_login(app.clone(), "bob@x.test", "password-12345").await;

    // Bob creates his own project
    let (cs, cb) = create_project(&app, &cookie_b, "bob-proj").await;
    assert_eq!(cs, StatusCode::CREATED, "create: {cb}");
    let pid = cb["id"].as_str().expect("pid").to_string();

    // Bob (non-admin) tries to set a slug — should be forbidden
    let resp = app
        .oneshot(req(
            Method::PATCH,
            &format!("/api/projects/{pid}"),
            Some(&cookie_b),
            Some(serde_json::json!({ "slug": "bobs-slug" })),
        ))
        .await
        .expect("patch resp");
    let (status, _, body) = read_body(resp).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "body={body}");
}

// ─────────────────────────── WP-005/006: metadata fields ─────────────────────

/// POST with valid category, tags, cover_image_url → 201, all fields persisted in response.
#[tokio::test]
async fn create_project_with_metadata_201() {
    let state = test_state().await;
    let app = build_router(state);
    let (_, cookie) = register_and_login(app.clone(), "alice@x.test", "password-12345").await;

    let resp = app
        .oneshot(req(
            Method::POST,
            "/api/projects",
            Some(&cookie),
            Some(serde_json::json!({
                "name": "Meta Project",
                "category": "Algorithms",
                "tags": ["rust", "async"],
                "cover_image_url": "https://ik.imagekit.io/demo/cover.png"
            })),
        ))
        .await
        .expect("create resp");
    let (status, _, body) = read_body(resp).await;
    assert_eq!(status, StatusCode::CREATED, "body={body}");
    assert_eq!(body["category"], serde_json::json!("Algorithms"));
    assert_eq!(body["tags"], serde_json::json!(["rust", "async"]));
    assert_eq!(
        body["cover_image_url"],
        serde_json::json!("https://ik.imagekit.io/demo/cover.png")
    );
}

// POST with category not in seeded allowlist → 422 invalid_category.
