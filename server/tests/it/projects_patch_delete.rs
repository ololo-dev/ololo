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
async fn list_projects_excludes_archived_by_default() {
    let state = test_state().await;
    let app = build_router(state);
    let (_, cookie) = register_and_login(app.clone(), "alice@x.test", "password-12345").await;

    let (_, b1) = create_project(&app, &cookie, "active").await;
    let (_, b2) = create_project(&app, &cookie, "archived").await;
    let pid_active = b1["id"].as_str().expect("id").to_string();
    let pid_archived = b2["id"].as_str().expect("id").to_string();

    // Archive the second project.
    app.clone()
        .oneshot(req(
            Method::PATCH,
            &format!("/api/projects/{pid_archived}"),
            Some(&cookie),
            Some(serde_json::json!({ "archived": true })),
        ))
        .await
        .expect("patch resp");

    // Default list should only return the active one.
    let resp = app
        .clone()
        .oneshot(req(Method::GET, "/api/projects", Some(&cookie), None))
        .await
        .expect("list resp");
    let (status, _, body) = read_body(resp).await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    let ids: Vec<&str> = body["projects"]
        .as_array()
        .expect("array")
        .iter()
        .filter_map(|p| p["id"].as_str())
        .collect();
    assert!(ids.contains(&pid_active.as_str()), "active missing");
    assert!(
        !ids.contains(&pid_archived.as_str()),
        "archived should be excluded"
    );
}

#[tokio::test]
async fn list_projects_include_archived_query_param() {
    let state = test_state().await;
    let app = build_router(state);
    let (_, cookie) = register_and_login(app.clone(), "alice@x.test", "password-12345").await;

    let (_, body) = create_project(&app, &cookie, "archived2").await;
    let pid = body["id"].as_str().expect("id").to_string();

    // Archive it.
    app.clone()
        .oneshot(req(
            Method::PATCH,
            &format!("/api/projects/{pid}"),
            Some(&cookie),
            Some(serde_json::json!({ "archived": true })),
        ))
        .await
        .expect("patch resp");

    // With include_archived=true the project appears.
    let resp = app
        .oneshot(req(
            Method::GET,
            "/api/projects?include_archived=true",
            Some(&cookie),
            None,
        ))
        .await
        .expect("list resp");
    let (status, _, body) = read_body(resp).await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    let ids: Vec<&str> = body["projects"]
        .as_array()
        .expect("array")
        .iter()
        .filter_map(|p| p["id"].as_str())
        .collect();
    assert!(
        ids.contains(&pid.as_str()),
        "archived project missing from include_archived=true list"
    );
}

#[tokio::test]
async fn patch_project_nonowner_403() {
    let state = test_state().await;
    let app = build_router(state);
    let (_, cookie_a) = register_and_login(app.clone(), "alice@x.test", "password-12345").await;
    let (_, cookie_b) = register_and_login(app.clone(), "bob@x.test", "password-12345").await;
    let (_, body) = create_project(&app, &cookie_a, "proj").await;
    let pid = body["id"].as_str().expect("pid").to_string();

    let resp = app
        .oneshot(req(
            Method::PATCH,
            &format!("/api/projects/{pid}"),
            Some(&cookie_b),
            Some(serde_json::json!({ "name": "hijack" })),
        ))
        .await
        .expect("patch resp");
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn delete_project_no_sessions_204() {
    let state = test_state().await;
    let app = build_router(state);
    let (_, cookie) = register_and_login(app.clone(), "alice@x.test", "password-12345").await;
    let (_, body) = create_project(&app, &cookie, "proj").await;
    let pid = body["id"].as_str().expect("pid").to_string();

    let resp = app
        .clone()
        .oneshot(req(
            Method::DELETE,
            &format!("/api/projects/{pid}"),
            Some(&cookie),
            None,
        ))
        .await
        .expect("delete resp");
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);

    // Verify gone.
    let resp = app
        .oneshot(req(
            Method::GET,
            &format!("/api/projects/{pid}"),
            Some(&cookie),
            None,
        ))
        .await
        .expect("get resp");
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

/// DELETE is RESTRICT when any session references the project.
#[tokio::test]
async fn delete_project_has_sessions_409_project_in_use() {
    let state = test_state().await;
    let app = build_router(state);
    let (_, cookie) = register_and_login(app.clone(), "alice@x.test", "password-12345").await;
    let (_, body) = create_project(&app, &cookie, "proj").await;
    let pid = body["id"].as_str().expect("pid").to_string();

    let (ss, _) = create_session(&app, &cookie, "arena", &pid).await;
    assert_eq!(ss, StatusCode::CREATED);

    let resp = app
        .oneshot(req(
            Method::DELETE,
            &format!("/api/projects/{pid}"),
            Some(&cookie),
            None,
        ))
        .await
        .expect("delete resp");
    let (status, _, body) = read_body(resp).await;
    assert_eq!(status, StatusCode::CONFLICT, "body={body}");
    assert_eq!(body["error"], serde_json::json!("project_in_use"));
    assert!(
        body["session_count"].as_i64().unwrap_or(0) >= 1,
        "session_count should be ≥ 1"
    );
}

#[tokio::test]
async fn delete_project_nonowner_403() {
    let state = test_state().await;
    let app = build_router(state);
    let (_, cookie_a) = register_and_login(app.clone(), "alice@x.test", "password-12345").await;
    let (_, cookie_b) = register_and_login(app.clone(), "bob@x.test", "password-12345").await;
    let (_, body) = create_project(&app, &cookie_a, "proj").await;
    let pid = body["id"].as_str().expect("pid").to_string();

    let resp = app
        .oneshot(req(
            Method::DELETE,
            &format!("/api/projects/{pid}"),
            Some(&cookie_b),
            None,
        ))
        .await
        .expect("delete resp");
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn unauth_request_401() {
    let state = test_state().await;
    let app = build_router(state);
    let fake_id = Uuid::new_v4();

    let cases: &[(Method, String, Option<serde_json::Value>)] = &[
        // GET /api/projects and GET /api/projects/:id are intentionally public
        // (Optional<AccessClaims>) — unauthenticated callers see public projects.
        // POST requires auth → 401.
        (
            Method::POST,
            "/api/projects".into(),
            Some(serde_json::json!({ "name": "x" })),
        ),
        // GET /api/projects/:id is also intentionally public — returns 404 for
        // unknown ids (not 401), so excluded from this 401 test.
        (
            Method::PATCH,
            format!("/api/projects/{fake_id}"),
            Some(serde_json::json!({ "name": "x" })),
        ),
        (Method::DELETE, format!("/api/projects/{fake_id}"), None),
    ];
    for (m, uri, body) in cases {
        let resp = app
            .clone()
            .oneshot(req(m.clone(), uri, None, body.clone()))
            .await
            .expect("req");
        assert_eq!(
            resp.status(),
            StatusCode::UNAUTHORIZED,
            "method={m} uri={uri}"
        );
    }
}

// ─────────────────────────── WP-002: PATCH slug/description ─────────────────

#[tokio::test]
async fn patch_project_slug_valid_200() {
    let state = test_state().await;
    let app = build_router(state);
    let (_, cookie) = register_and_login(app.clone(), "alice@x.test", "password-12345").await;
    let (_, body) = create_project(&app, &cookie, "my-project").await;
    let pid = body["id"].as_str().expect("pid").to_string();

    let resp = app
        .oneshot(req(
            Method::PATCH,
            &format!("/api/projects/{pid}"),
            Some(&cookie),
            Some(serde_json::json!({ "slug": "new-slug" })),
        ))
        .await
        .expect("patch resp");
    let (status, _, body) = read_body(resp).await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    assert_eq!(body["slug"], serde_json::json!("new-slug"));
}

#[tokio::test]
async fn patch_project_slug_invalid_422() {
    let state = test_state().await;
    let app = build_router(state);
    let (_, cookie) = register_and_login(app.clone(), "alice@x.test", "password-12345").await;
    let (_, body) = create_project(&app, &cookie, "my-project").await;
    let pid = body["id"].as_str().expect("pid").to_string();

    let resp = app
        .oneshot(req(
            Method::PATCH,
            &format!("/api/projects/{pid}"),
            Some(&cookie),
            Some(serde_json::json!({ "slug": "Not A Slug!" })),
        ))
        .await
        .expect("patch resp");
    let (status, _, body) = read_body(resp).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "body={body}");
    assert_eq!(body["error"], serde_json::json!("invalid_slug"));
}

#[tokio::test]
async fn patch_project_slug_duplicate_same_owner_409() {
    let state = test_state().await;
    let app = build_router(state);
    let (_, cookie) = register_and_login(app.clone(), "alice@x.test", "password-12345").await;
    // Create two projects
    let (_, b1) = create_project_with_slug(&app, &cookie, "Project A", Some("taken-slug")).await;
    assert_eq!(b1["slug"], serde_json::json!("taken-slug"));
    let (_, b2) = create_project(&app, &cookie, "Project B").await;
    let pid2 = b2["id"].as_str().expect("pid").to_string();

    // Try to patch project B's slug to the already-taken slug
    let resp = app
        .oneshot(req(
            Method::PATCH,
            &format!("/api/projects/{pid2}"),
            Some(&cookie),
            Some(serde_json::json!({ "slug": "taken-slug" })),
        ))
        .await
        .expect("patch resp");
    let (status, _, body) = read_body(resp).await;
    assert_eq!(status, StatusCode::CONFLICT, "body={body}");
    assert_eq!(body["error"], serde_json::json!("slug_conflict"));
}

#[tokio::test]
async fn patch_project_description_200() {
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
            Some(serde_json::json!({ "description": "A great project" })),
        ))
        .await
        .expect("patch resp");
    let (status, _, body) = read_body(resp).await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    assert_eq!(body["description"], serde_json::json!("A great project"));
}

/// The project owner (not just an admin) can declare, replace and clear the
/// session-memory schema — this is what the edit form PATCHes.
#[tokio::test]
async fn patch_project_memory_schema_roundtrip() {
    let state = test_state().await;
    let app = build_router(state);
    let (_, cookie) = register_and_login(app.clone(), "alice@x.test", "password-12345").await;
    let (_, body) = create_project(&app, &cookie, "proj").await;
    let pid = body["id"].as_str().expect("pid").to_string();

    let patch = |schema: serde_json::Value| {
        let app = app.clone();
        let cookie = cookie.clone();
        let pid = pid.clone();
        async move {
            let resp = app
                .oneshot(req(
                    Method::PATCH,
                    &format!("/api/projects/{pid}"),
                    Some(&cookie),
                    Some(serde_json::json!({ "memory_schema": schema })),
                ))
                .await
                .expect("patch resp");
            read_body(resp).await
        }
    };

    // Declare a schema; scalars of every allowed kind survive.
    let (status, _, body) = patch(serde_json::json!({"dev": "npm run dev", "port": 1234})).await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    assert_eq!(
        body["memory_schema"],
        serde_json::json!({"dev": "npm run dev", "port": 1234})
    );

    // It is persisted, not just echoed.
    let resp = app
        .clone()
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
        body["memory_schema"],
        serde_json::json!({"dev": "npm run dev", "port": 1234})
    );

    // A non-scalar default is rejected rather than stored.
    let (status, _, body) = patch(serde_json::json!({"dev": ["npm", "run", "dev"]})).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "body={body}");

    // Explicit null clears it — the player's Memory tab goes away again.
    let (status, _, body) = patch(serde_json::Value::Null).await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    assert_eq!(body["memory_schema"], serde_json::Value::Null);
}

// ─────────────────────────── WP-003: by-slug endpoints ───────────────────────
