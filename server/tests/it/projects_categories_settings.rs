#![allow(dead_code)]

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
async fn create_project_invalid_category_422() {
    let state = test_state().await;
    let app = build_router(state);
    let (_, cookie) = register_and_login(app.clone(), "alice@x.test", "password-12345").await;

    let resp = app
        .oneshot(req(
            Method::POST,
            "/api/projects",
            Some(&cookie),
            Some(serde_json::json!({
                "name": "Bad Cat",
                "category": "NotInList"
            })),
        ))
        .await
        .expect("resp");
    let (status, _, body) = read_body(resp).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "body={body}");
    assert_eq!(body["error"], serde_json::json!("invalid_category"));
}

/// POST with 11 tags → 422 invalid_tags.
#[tokio::test]
async fn create_project_invalid_tags_too_many_422() {
    let state = test_state().await;
    let app = build_router(state);
    let (_, cookie) = register_and_login(app.clone(), "alice@x.test", "password-12345").await;

    let tags: Vec<String> = (0..21).map(|i| format!("tag{i}")).collect();
    let resp = app
        .oneshot(req(
            Method::POST,
            "/api/projects",
            Some(&cookie),
            Some(serde_json::json!({ "name": "Too Many Tags", "tags": tags })),
        ))
        .await
        .expect("resp");
    let (status, _, body) = read_body(resp).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "body={body}");
    assert_eq!(body["error"], serde_json::json!("invalid_tags"));
}

/// POST with a tag >128 chars → 422 invalid_tags.
#[tokio::test]
async fn create_project_invalid_tags_too_long_422() {
    let state = test_state().await;
    let app = build_router(state);
    let (_, cookie) = register_and_login(app.clone(), "alice@x.test", "password-12345").await;

    let long_tag = "x".repeat(129);
    let resp = app
        .oneshot(req(
            Method::POST,
            "/api/projects",
            Some(&cookie),
            Some(serde_json::json!({ "name": "Long Tag", "tags": [long_tag] })),
        ))
        .await
        .expect("resp");
    let (status, _, body) = read_body(resp).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "body={body}");
    assert_eq!(body["error"], serde_json::json!("invalid_tags"));
}

/// POST with http:// cover URL → 422 invalid_cover_image_url.
#[tokio::test]
async fn create_project_invalid_cover_url_422() {
    let state = test_state().await;
    let app = build_router(state);
    let (_, cookie) = register_and_login(app.clone(), "alice@x.test", "password-12345").await;

    let resp = app
        .oneshot(req(
            Method::POST,
            "/api/projects",
            Some(&cookie),
            Some(serde_json::json!({
                "name": "Bad Cover",
                "cover_image_url": "http://insecure.example.com/img.png"
            })),
        ))
        .await
        .expect("resp");
    let (status, _, body) = read_body(resp).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "body={body}");
    assert_eq!(body["error"], serde_json::json!("invalid_cover_image_url"));
}

/// PATCH with clear_cover_image: true sets cover_image_url to null.
#[tokio::test]
async fn patch_project_clear_cover_image_true_nulls_url() {
    let state = test_state().await;
    let app = build_router(state);
    let (_, cookie) = register_and_login(app.clone(), "alice@x.test", "password-12345").await;

    // Create with a cover URL.
    let resp = app
        .clone()
        .oneshot(req(
            Method::POST,
            "/api/projects",
            Some(&cookie),
            Some(serde_json::json!({
                "name": "Has Cover",
                "cover_image_url": "https://ik.imagekit.io/demo/cover.png"
            })),
        ))
        .await
        .expect("create resp");
    let (status, _, body) = read_body(resp).await;
    assert_eq!(status, StatusCode::CREATED, "body={body}");
    let pid = body["id"].as_str().expect("pid").to_string();
    assert!(
        !body["cover_image_url"].is_null(),
        "cover_image_url should be set after create"
    );

    // PATCH with clear_cover_image: true.
    let resp = app
        .oneshot(req(
            Method::PATCH,
            &format!("/api/projects/{pid}"),
            Some(&cookie),
            Some(serde_json::json!({ "clear_cover_image": true })),
        ))
        .await
        .expect("patch resp");
    let (status, _, body) = read_body(resp).await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    assert!(
        body["cover_image_url"].is_null(),
        "cover_image_url should be null after clear, got {body}"
    );
}

/// PATCH with clear_cover_image: false does NOT clear cover_image_url.
#[tokio::test]
async fn patch_project_clear_cover_image_false_preserves_url() {
    let state = test_state().await;
    let app = build_router(state);
    let (_, cookie) = register_and_login(app.clone(), "alice@x.test", "password-12345").await;

    let resp = app
        .clone()
        .oneshot(req(
            Method::POST,
            "/api/projects",
            Some(&cookie),
            Some(serde_json::json!({
                "name": "Has Cover2",
                "cover_image_url": "https://ik.imagekit.io/demo/cover.png"
            })),
        ))
        .await
        .expect("create resp");
    let (status, _, body) = read_body(resp).await;
    assert_eq!(status, StatusCode::CREATED, "body={body}");
    let pid = body["id"].as_str().expect("pid").to_string();

    // PATCH with clear_cover_image: false — must preserve URL.
    let resp = app
        .oneshot(req(
            Method::PATCH,
            &format!("/api/projects/{pid}"),
            Some(&cookie),
            Some(serde_json::json!({ "clear_cover_image": false })),
        ))
        .await
        .expect("patch resp");
    let (status, _, body) = read_body(resp).await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    assert_eq!(
        body["cover_image_url"],
        serde_json::json!("https://ik.imagekit.io/demo/cover.png"),
        "cover_image_url should be preserved when clear_cover_image: false, got {body}"
    );
}

/// GET /api/projects/:id response includes category, tags (array), cover_image_url.
#[tokio::test]
async fn get_project_response_includes_new_fields() {
    let state = test_state().await;
    let app = build_router(state);
    let (_, cookie) = register_and_login(app.clone(), "alice@x.test", "password-12345").await;

    let resp = app
        .clone()
        .oneshot(req(
            Method::POST,
            "/api/projects",
            Some(&cookie),
            Some(serde_json::json!({
                "name": "Full Meta",
                "category": "Web",
                "tags": ["go", "distributed"],
                "cover_image_url": "https://ik.imagekit.io/demo/hackathon.png"
            })),
        ))
        .await
        .expect("create resp");
    let (status, _, body) = read_body(resp).await;
    assert_eq!(status, StatusCode::CREATED, "body={body}");
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
    assert_eq!(body["category"], serde_json::json!("Web"));
    assert_eq!(body["tags"], serde_json::json!(["go", "distributed"]));
    assert_eq!(
        body["cover_image_url"],
        serde_json::json!("https://ik.imagekit.io/demo/hackathon.png")
    );
}

/// GET /api/projects/categories returns the seeded category list.
#[tokio::test]
async fn get_categories_returns_seeded_list() {
    let state = test_state().await;
    let app = build_router(state);
    let (_, cookie) = register_and_login(app.clone(), "alice@x.test", "password-12345").await;

    let resp = app
        .oneshot(req(
            Method::GET,
            "/api/projects/categories",
            Some(&cookie),
            None,
        ))
        .await
        .expect("categories resp");
    let (status, _, body) = read_body(resp).await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    let cats = body["categories"].as_array().expect("categories array");
    assert!(!cats.is_empty(), "seeded categories should not be empty");
    // Migration seeds: Algorithms, Data Structures, Math, Web, API, CLI, Game, Data Science, Other
    let names: Vec<&str> = cats.iter().filter_map(|v| v.as_str()).collect();
    assert!(
        names.contains(&"Algorithms"),
        "Algorithms should be in seeded list, got {names:?}"
    );
}

/// GET /api/projects/categories honors the admin-set order: after a reorder
/// (Settings → Categories drag), the landing/catalog list reflects it. The
/// first admin registered owns the reorder endpoint.
#[tokio::test]
async fn get_categories_reflects_admin_reorder() {
    let state = test_state().await;
    let app = build_router(state);
    let (_, cookie) = register_and_login(app.clone(), "admin@cat.test", "password-12345").await;

    // Read the admin list (ordered by ordinal) to get category ids.
    let resp = app
        .clone()
        .oneshot(req(Method::GET, "/api/categories", Some(&cookie), None))
        .await
        .expect("list");
    let (_, _, body) = read_body(resp).await;
    let cats = body["categories"].as_array().expect("categories");
    let ids: Vec<i64> = cats.iter().map(|c| c["id"].as_i64().unwrap()).collect();
    let names: Vec<String> = cats
        .iter()
        .map(|c| c["name"].as_str().unwrap().to_string())
        .collect();
    assert!(ids.len() >= 3, "need a few seeded categories to reorder");

    // Reverse the order and persist it.
    let reversed: Vec<i64> = ids.iter().rev().copied().collect();
    let resp = app
        .clone()
        .oneshot(req(
            Method::PATCH,
            "/api/admin/categories/reorder",
            Some(&cookie),
            Some(serde_json::json!({ "ids": reversed })),
        ))
        .await
        .expect("reorder");
    let (status, _, body) = read_body(resp).await;
    assert_eq!(status, StatusCode::OK, "reorder: {body}");

    // The public list the landing reads now matches the reversed order.
    let resp = app
        .oneshot(req(
            Method::GET,
            "/api/projects/categories",
            Some(&cookie),
            None,
        ))
        .await
        .expect("public list");
    let (_, _, body) = read_body(resp).await;
    let got: Vec<String> = body["categories"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    let mut want = names.clone();
    want.reverse();
    assert_eq!(got, want, "public categories must follow the admin reorder");
}

// ─────────────────────── allow_user_project_creation gate tests ───────────────

/// AC-006: Non-admin POST /api/projects returns 403 when setting is "false".
#[tokio::test]
async fn create_project_blocked_for_non_admin_when_setting_false() {
    let state = test_state().await;
    let app = build_router(state);
    // Alice registers first → becomes admin.
    let (_, alice_cookie) =
        register_and_login(app.clone(), "alice@gate1.test", "password-12345").await;
    // Bob registers second → non-admin.
    let (_, bob_cookie) = register_and_login(app.clone(), "bob@gate1.test", "password-12345").await;

    // Admin disables project creation.
    set_project_creation_setting(&app, &alice_cookie, "false").await;

    // Bob (non-admin) tries to create a project.
    let (status, body) = create_project(&app, &bob_cookie, "Bob's Project").await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "non-admin should be blocked when setting=false; body={body}"
    );
}

/// AC-007: Admin POST /api/projects returns 201 even when setting is "false".
#[tokio::test]
async fn create_project_allowed_for_admin_when_setting_false() {
    let state = test_state().await;
    let app = build_router(state);
    // Alice is admin (first registered).
    let (_, alice_cookie) =
        register_and_login(app.clone(), "alice@gate2.test", "password-12345").await;

    // Admin disables project creation.
    set_project_creation_setting(&app, &alice_cookie, "false").await;

    // Alice (admin) creates a project — must succeed.
    let (status, body) = create_project(&app, &alice_cookie, "Alice's Project").await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "admin should always be allowed to create; body={body}"
    );
}

// AC-008: Non-admin POST /api/projects returns 201 when setting is "true".
