//! `GET /api/admin/sessions` — the instance-wide session registry.
//!
//! The point of this endpoint is reach: `GET /api/sessions` answers with the
//! caller's own sessions, so an admin hunting a stuck session started by
//! somebody else has nothing to look at. These tests pin that reach, and the
//! filters that make a full-instance list usable.

use axum::http::{Method, StatusCode};
use server::build_router;
use tower::ServiceExt;

use crate::common;
use crate::common::*;

async fn create_project(app: &axum::Router, cookie: &str, name: &str) -> String {
    let resp = app
        .clone()
        .oneshot(req(
            Method::POST,
            "/api/projects",
            Some(cookie),
            Some(serde_json::json!({ "name": name })),
        ))
        .await
        .expect("create project");
    let (status, body) = read_body_json(resp).await;
    assert_eq!(status, StatusCode::CREATED, "body={body}");
    body["id"].as_str().expect("project id").to_string()
}

async fn create_session(app: &axum::Router, cookie: &str, name: &str, project_id: &str) -> String {
    let resp = app
        .clone()
        .oneshot(req(
            Method::POST,
            "/api/sessions",
            Some(cookie),
            Some(serde_json::json!({ "name": name, "project_id": project_id })),
        ))
        .await
        .expect("create session");
    let (status, body) = read_body_json(resp).await;
    assert_eq!(status, StatusCode::CREATED, "body={body}");
    body["id"].as_str().expect("session id").to_string()
}

async fn admin_list(app: &axum::Router, cookie: &str, query: &str) -> serde_json::Value {
    let path = if query.is_empty() {
        "/api/admin/sessions".to_string()
    } else {
        format!("/api/admin/sessions?{query}")
    };
    let resp = app
        .clone()
        .oneshot(req(Method::GET, &path, Some(cookie), None))
        .await
        .expect("admin list");
    let (status, body) = read_body_json(resp).await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    body
}

#[tokio::test]
async fn admin_sees_sessions_they_neither_own_nor_joined() {
    let state = test_state().await;
    let app = build_router(state.clone());

    let (owner_id, owner_cookie) =
        register_and_login(app.clone(), "owner@as.test", "password-12345").await;
    let project = create_project(&app, &owner_cookie, "as-reach").await;
    create_session(&app, &owner_cookie, "someone elses run", &project).await;

    let (admin_id, admin_cookie) =
        register_and_login(app.clone(), "admin@as.test", "password-12345").await;
    make_user_admin(&state, admin_id).await;

    // The admin's own listing is empty — they own nothing and joined nothing.
    let resp = app
        .clone()
        .oneshot(req(Method::GET, "/api/sessions", Some(&admin_cookie), None))
        .await
        .expect("own list");
    let (status, own) = read_body_json(resp).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(own["sessions"].as_array().expect("array").len(), 0);

    // The registry still reaches it, with the project and owner resolved.
    let body = admin_list(&app, &admin_cookie, "").await;
    assert_eq!(body["total"], serde_json::json!(1));
    let row = &body["sessions"][0];
    assert_eq!(row["name"], serde_json::json!("someone elses run"));
    assert_eq!(row["project_name"], serde_json::json!("as-reach"));
    assert_eq!(row["owner_id"], serde_json::json!(owner_id));
    assert!(row["owner_display_name"].as_str().is_some());
    assert!(row["join_code"].as_str().is_some_and(|c| c.len() == 6));
    // Nobody joined, so the count is a real zero rather than a missing field.
    assert_eq!(row["player_count"], serde_json::json!(0));
}

#[tokio::test]
async fn non_admin_is_refused_the_registry() {
    let state = test_state().await;
    let app = build_router(state);
    // The very first account on a fresh instance is made admin automatically,
    // so the plain user under test has to be the second one.
    register_and_login(app.clone(), "first@as.test", "password-12345").await;
    let (_, cookie) = register_and_login(app.clone(), "plain@as.test", "password-12345").await;

    let resp = app
        .clone()
        .oneshot(req(Method::GET, "/api/admin/sessions", Some(&cookie), None))
        .await
        .expect("resp");
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);

    // And anonymously it is a 401, not a 404 that would hint the route exists
    // only for some callers.
    let resp = app
        .clone()
        .oneshot(req(Method::GET, "/api/admin/sessions", None, None))
        .await
        .expect("resp");
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn filters_narrow_the_instance_wide_list() {
    let state = test_state().await;
    let app = build_router(state.clone());

    let (_, owner) = register_and_login(app.clone(), "owner@as2.test", "password-12345").await;
    let alpha = create_project(&app, &owner, "alpha").await;
    let beta = create_project(&app, &owner, "beta").await;
    create_session(&app, &owner, "alpha morning", &alpha).await;
    create_session(&app, &owner, "alpha evening", &alpha).await;
    let beta_id = create_session(&app, &owner, "beta run", &beta).await;

    let (admin_id, admin) =
        register_and_login(app.clone(), "admin@as2.test", "password-12345").await;
    make_user_admin(&state, admin_id).await;

    assert_eq!(
        admin_list(&app, &admin, "").await["total"],
        serde_json::json!(3)
    );

    // By project.
    let body = admin_list(&app, &admin, &format!("project_id={alpha}")).await;
    assert_eq!(body["total"], serde_json::json!(2));

    // By name fragment.
    let body = admin_list(&app, &admin, "q=evening").await;
    assert_eq!(body["total"], serde_json::json!(1));
    assert_eq!(
        body["sessions"][0]["name"],
        serde_json::json!("alpha evening")
    );

    // By join code — the identifier an admin actually has in hand, because it
    // is what a player reports when something goes wrong.
    let resp = app
        .clone()
        .oneshot(req(
            Method::GET,
            &format!("/api/sessions/{beta_id}"),
            Some(&owner),
            None,
        ))
        .await
        .expect("session");
    let (_, session) = read_body_json(resp).await;
    let code = session["join_code"].as_str().expect("code").to_string();
    let body = admin_list(&app, &admin, &format!("q={code}")).await;
    assert_eq!(body["total"], serde_json::json!(1));
    assert_eq!(body["sessions"][0]["id"], serde_json::json!(beta_id));

    // Every session here is in lobby, so the status filter is only honest if
    // a different status comes back empty.
    assert_eq!(
        admin_list(&app, &admin, "status=lobby").await["total"],
        serde_json::json!(3)
    );
    assert_eq!(
        admin_list(&app, &admin, "status=finished").await["total"],
        serde_json::json!(0)
    );
}

#[tokio::test]
async fn an_unknown_status_is_rejected_rather_than_ignored() {
    let state = test_state().await;
    let app = build_router(state.clone());
    let (admin_id, admin) =
        register_and_login(app.clone(), "admin@as3.test", "password-12345").await;
    make_user_admin(&state, admin_id).await;

    // Silently dropping the filter would answer with every session on the
    // instance, which reads as "there are no running ones" to the caller.
    let resp = app
        .clone()
        .oneshot(req(
            Method::GET,
            "/api/admin/sessions?status=runnning",
            Some(&admin),
            None,
        ))
        .await
        .expect("resp");
    let (status, body) = read_body_json(resp).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "body={body}");
    assert_eq!(body["error"], serde_json::json!("invalid_status"));
}

#[tokio::test]
async fn pages_report_the_full_total_not_the_page_size() {
    let state = test_state().await;
    let app = build_router(state.clone());

    let (_, owner) = register_and_login(app.clone(), "owner@as4.test", "password-12345").await;
    let project = create_project(&app, &owner, "paged").await;
    for i in 0..5 {
        create_session(&app, &owner, &format!("run {i}"), &project).await;
    }

    let (admin_id, admin) =
        register_and_login(app.clone(), "admin@as4.test", "password-12345").await;
    make_user_admin(&state, admin_id).await;

    let body = admin_list(&app, &admin, "per_page=2&page=1").await;
    assert_eq!(body["total"], serde_json::json!(5));
    assert_eq!(body["sessions"].as_array().expect("array").len(), 2);

    let last = admin_list(&app, &admin, "per_page=2&page=3").await;
    assert_eq!(last["total"], serde_json::json!(5));
    assert_eq!(last["sessions"].as_array().expect("array").len(), 1);

    // Newest first, so page 1 and page 3 cannot share a row.
    assert_ne!(body["sessions"][0]["id"], last["sessions"][0]["id"]);
}

#[tokio::test]
async fn admin_can_delete_a_session_they_do_not_own() {
    let state = test_state().await;
    let app = build_router(state.clone());

    let (_, owner) = register_and_login(app.clone(), "owner@as5.test", "password-12345").await;
    let project = create_project(&app, &owner, "removable").await;
    let session = create_session(&app, &owner, "doomed", &project).await;

    let (admin_id, admin) =
        register_and_login(app.clone(), "admin@as5.test", "password-12345").await;
    make_user_admin(&state, admin_id).await;

    // Cancel already worked for admins; delete did not, which read as a
    // broken button next to a working one in the registry.
    let resp = app
        .clone()
        .oneshot(req(
            Method::DELETE,
            &format!("/api/sessions/{session}"),
            Some(&admin),
            None,
        ))
        .await
        .expect("delete");
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);

    assert_eq!(
        admin_list(&app, &admin, "").await["total"],
        serde_json::json!(0)
    );
}

#[tokio::test]
async fn a_plain_user_still_cannot_delete_someone_elses_session() {
    let state = test_state().await;
    let app = build_router(state);

    let (_, owner) = register_and_login(app.clone(), "owner@as6.test", "password-12345").await;
    let project = create_project(&app, &owner, "guarded").await;
    let session = create_session(&app, &owner, "not yours", &project).await;

    let (_, other) = register_and_login(app.clone(), "other@as6.test", "password-12345").await;
    let resp = app
        .clone()
        .oneshot(req(
            Method::DELETE,
            &format!("/api/sessions/{session}"),
            Some(&other),
            None,
        ))
        .await
        .expect("delete");
    assert!(
        resp.status() == StatusCode::FORBIDDEN || resp.status() == StatusCode::NOT_FOUND,
        "got {}",
        resp.status()
    );
}
