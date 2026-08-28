#![allow(dead_code)]

//! Projects CRUD integration tests.
//!
//! Spec: contract.md FR-001 (project ownership), FR-002 (project list),
//! FR-005 (project delete RESTRICT), FR-006 (freeze-while-running),
//! FR-007 (archive via soft-retire), FR-008 (public flag).

use axum::http::{Method, StatusCode};
use server::build_router;
use tower::ServiceExt;
use uuid::Uuid;

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
async fn create_project_201_owner_fields() {
    let state = test_state().await;
    let app = build_router(state);
    let (uid, cookie) = register_and_login(app.clone(), "alice@x.test", "password-12345").await;

    let (status, body) = create_project(&app, &cookie, "my-project").await;
    assert_eq!(status, StatusCode::CREATED, "body={body}");
    assert!(body["id"].is_string());
    assert_eq!(body["name"], serde_json::json!("my-project"));
    assert_eq!(body["owner_user_id"], serde_json::json!(uid.to_string()));
    assert_eq!(body["public"], serde_json::json!(true));
    assert!(body["archived_at"].is_null());
    assert!(body["created_at"].is_string());
}

#[tokio::test]
async fn create_project_missing_name_422() {
    let state = test_state().await;
    let app = build_router(state);
    let (_, cookie) = register_and_login(app.clone(), "alice@x.test", "password-12345").await;

    let resp = app
        .oneshot(req(
            Method::POST,
            "/api/projects",
            Some(&cookie),
            Some(serde_json::json!({})),
        ))
        .await
        .expect("resp");
    assert_eq!(resp.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn create_project_name_too_long_422() {
    let state = test_state().await;
    let app = build_router(state);
    let (_, cookie) = register_and_login(app.clone(), "alice@x.test", "password-12345").await;

    let long = "x".repeat(201);
    let (status, _) = create_project(&app, &cookie, &long).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn list_projects_owner_sees_own() {
    let state = test_state().await;
    let app = build_router(state);
    let (_, cookie) = register_and_login(app.clone(), "alice@x.test", "password-12345").await;

    let (s1, b1) = create_project(&app, &cookie, "p1").await;
    assert_eq!(s1, StatusCode::CREATED);
    let (s2, b2) = create_project(&app, &cookie, "p2").await;
    assert_eq!(s2, StatusCode::CREATED);
    let pid1 = b1["id"].as_str().expect("pid1");
    let pid2 = b2["id"].as_str().expect("pid2");

    let resp = app
        .oneshot(req(Method::GET, "/api/projects", Some(&cookie), None))
        .await
        .expect("list resp");
    let (status, _, body) = read_body(resp).await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    let ids: Vec<&str> = body["projects"]
        .as_array()
        .expect("projects array")
        .iter()
        .filter_map(|p| p["id"].as_str())
        .collect();
    assert!(ids.contains(&pid1), "p1 missing");
    assert!(ids.contains(&pid2), "p2 missing");
}

#[tokio::test]
async fn list_projects_excludes_other_users_private() {
    let state = test_state().await;
    let app = build_router(state);
    let (_, cookie_a) = register_and_login(app.clone(), "alice@x.test", "password-12345").await;
    let (_, cookie_b) = register_and_login(app.clone(), "bob@x.test", "password-12345").await;

    let (s, body) = create_project_private(&app, &cookie_a, "alice-private").await;
    assert_eq!(s, StatusCode::CREATED);
    let alice_pid = body["id"].as_str().expect("pid").to_string();

    let resp = app
        .oneshot(req(Method::GET, "/api/projects", Some(&cookie_b), None))
        .await
        .expect("list resp");
    let (status, _, body) = read_body(resp).await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    let ids: Vec<&str> = body["projects"]
        .as_array()
        .expect("projects array")
        .iter()
        .filter_map(|p| p["id"].as_str())
        .collect();
    assert!(
        !ids.contains(&alice_pid.as_str()),
        "bob should not see alice's private project"
    );
}

#[tokio::test]
async fn list_projects_includes_others_public() {
    let state = test_state().await;
    let app = build_router(state);
    let (_, cookie_a) = register_and_login(app.clone(), "alice@x.test", "password-12345").await;
    let (_, cookie_b) = register_and_login(app.clone(), "bob@x.test", "password-12345").await;

    // Alice creates a project, then makes it public.
    let (s, body) = create_project(&app, &cookie_a, "alice-public").await;
    assert_eq!(s, StatusCode::CREATED);
    let alice_pid = body["id"].as_str().expect("pid").to_string();
    let resp = app
        .clone()
        .oneshot(req(
            Method::PATCH,
            &format!("/api/projects/{alice_pid}"),
            Some(&cookie_a),
            Some(serde_json::json!({ "public": true })),
        ))
        .await
        .expect("patch resp");
    assert_eq!(resp.status(), StatusCode::OK);

    let resp = app
        .oneshot(req(Method::GET, "/api/projects", Some(&cookie_b), None))
        .await
        .expect("list resp");
    let (status, _, body) = read_body(resp).await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    let ids: Vec<&str> = body["projects"]
        .as_array()
        .expect("projects array")
        .iter()
        .filter_map(|p| p["id"].as_str())
        .collect();
    assert!(
        ids.contains(&alice_pid.as_str()),
        "bob should see alice's public project"
    );
}

#[tokio::test]
async fn get_project_owner_200() {
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
    assert_eq!(body["id"], serde_json::json!(pid));
    assert_eq!(body["name"], serde_json::json!("proj"));
}

#[tokio::test]
async fn get_project_nonowner_private_403() {
    let state = test_state().await;
    let app = build_router(state);
    let (_, cookie_a) = register_and_login(app.clone(), "alice@x.test", "password-12345").await;
    let (_, cookie_b) = register_and_login(app.clone(), "bob@x.test", "password-12345").await;
    let (_, body) = create_project_private(&app, &cookie_a, "private").await;
    let pid = body["id"].as_str().expect("pid").to_string();

    let resp = app
        .oneshot(req(
            Method::GET,
            &format!("/api/projects/{pid}"),
            Some(&cookie_b),
            None,
        ))
        .await
        .expect("get resp");
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn get_project_nonowner_public_200() {
    let state = test_state().await;
    let app = build_router(state);
    let (_, cookie_a) = register_and_login(app.clone(), "alice@x.test", "password-12345").await;
    let (_, cookie_b) = register_and_login(app.clone(), "bob@x.test", "password-12345").await;
    let (_, body) = create_project(&app, &cookie_a, "pub").await;
    let pid = body["id"].as_str().expect("pid").to_string();

    // Make it public.
    let resp = app
        .clone()
        .oneshot(req(
            Method::PATCH,
            &format!("/api/projects/{pid}"),
            Some(&cookie_a),
            Some(serde_json::json!({ "public": true })),
        ))
        .await
        .expect("patch resp");
    assert_eq!(resp.status(), StatusCode::OK);

    let resp = app
        .oneshot(req(
            Method::GET,
            &format!("/api/projects/{pid}"),
            Some(&cookie_b),
            None,
        ))
        .await
        .expect("get resp");
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn get_project_unknown_404() {
    let state = test_state().await;
    let app = build_router(state);
    let (_, cookie) = register_and_login(app.clone(), "alice@x.test", "password-12345").await;

    let resp = app
        .oneshot(req(
            Method::GET,
            &format!("/api/projects/{}", Uuid::new_v4()),
            Some(&cookie),
            None,
        ))
        .await
        .expect("get resp");
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn patch_project_rename_200() {
    let state = test_state().await;
    let app = build_router(state);
    let (_, cookie) = register_and_login(app.clone(), "alice@x.test", "password-12345").await;
    let (_, body) = create_project(&app, &cookie, "old-name").await;
    let pid = body["id"].as_str().expect("pid").to_string();

    let resp = app
        .oneshot(req(
            Method::PATCH,
            &format!("/api/projects/{pid}"),
            Some(&cookie),
            Some(serde_json::json!({ "name": "new-name" })),
        ))
        .await
        .expect("patch resp");
    let (status, _, body) = read_body(resp).await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    assert_eq!(body["name"], serde_json::json!("new-name"));
}

#[tokio::test]
async fn patch_project_toggle_public_200() {
    let state = test_state().await;
    let app = build_router(state);
    let (_, cookie) = register_and_login(app.clone(), "alice@x.test", "password-12345").await;
    let (_, body) = create_project(&app, &cookie, "proj").await;
    let pid = body["id"].as_str().expect("pid").to_string();

    let resp = app
        .oneshot(req(
            Method::PATCH,
            &format!("/api/projects/{pid}"),
            Some(&cookie),
            Some(serde_json::json!({ "public": true })),
        ))
        .await
        .expect("patch resp");
    let (status, _, body) = read_body(resp).await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    assert_eq!(body["public"], serde_json::json!(true));
}

#[tokio::test]
async fn patch_project_archive_sets_archived_at() {
    let state = test_state().await;
    let app = build_router(state);
    let (_, cookie) = register_and_login(app.clone(), "alice@x.test", "password-12345").await;
    let (_, body) = create_project(&app, &cookie, "proj").await;
    let pid = body["id"].as_str().expect("pid").to_string();

    // Archive via a sentinel value — implementation may accept `archived: true`
    // or an ISO timestamp string; we use the boolean variant.
    let resp = app
        .oneshot(req(
            Method::PATCH,
            &format!("/api/projects/{pid}"),
            Some(&cookie),
            Some(serde_json::json!({ "archived": true })),
        ))
        .await
        .expect("patch resp");
    let (status, _, body) = read_body(resp).await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    assert!(
        !body["archived_at"].is_null(),
        "archived_at should be set, got {body}"
    );
}
