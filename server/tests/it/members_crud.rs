//! WP-012 — session members CRUD with owner-managed membership.
//!
//! Spec: contract.md FR-009 (members read sessions), FR-010 (owner manages
//! members), FR-038 (cascade). FR-040 (observer-token revocation on
//! removal) is deferred to WP-018; the DELETE handler carries a TODO
//! marker referencing WP-018.
//!
//! Deviation from the work-package brief (documented):
//! `session_members` has a composite primary key (session_id_fk,
//! user_id_fk) and a `joined_at` timestamp — there is no synthetic `id`
//! UUID column and no `created_at`. The signed contract + live migration
//! `m20260502_000004_alter_sessions_owner_state` are the source of
//! truth, so `MemberSummary` exposes `{session_id, user_id, role,
//! joined_at}` instead of the brief's `{id, user_id, role, created_at}`.
//!
//! Idempotency choice (test 13): removing a non-existent member returns
//! 404 (not silent 204). This mirrors the REST convention used by
//! `sessions` DELETE for missing rows and avoids leaking existence to
//! outsiders (outsiders still see 404 on the parent session check).

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
    let (status, _, body) = read_body(resp).await;
    (status, body)
}

async fn create_session(
    app: &axum::Router,
    cookie: &str,
    name: &str,
) -> (StatusCode, serde_json::Value) {
    let (ps, pb) = create_project(app, cookie, &format!("proj-for-{name}")).await;
    assert_eq!(ps, StatusCode::CREATED, "implicit project: {pb}");
    let project_id = pb["id"].as_str().expect("project id").to_string();

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
    let (status, _, body) = read_body(resp).await;
    (status, body)
}

async fn add_member(
    app: &axum::Router,
    cookie: &str,
    session_id: &str,
    user_id: Uuid,
    role: &str,
) -> (StatusCode, serde_json::Value) {
    let resp = app
        .clone()
        .oneshot(req(
            Method::POST,
            &format!("/api/sessions/{session_id}/members"),
            Some(cookie),
            Some(serde_json::json!({ "user_id": user_id, "role": role })),
        ))
        .await
        .expect("add member resp");
    let (status, _, body) = read_body(resp).await;
    (status, body)
}

use crate::common;
use crate::common::*;

#[tokio::test]
async fn add_member_owner_succeeds_201() {
    let state = test_state().await;
    let app = build_router(state);
    let (_, cookie_a) = register_and_login(app.clone(), "alice@x.test", "password-12345").await;
    let (uid_b, _) = register_and_login(app.clone(), "bob@x.test", "password-12345").await;
    let (_, s1) = create_session(&app, &cookie_a, "alice-1").await;
    let sid = s1["id"].as_str().expect("id");

    let (status, body) = add_member(&app, &cookie_a, sid, uid_b, "member").await;
    assert_eq!(status, StatusCode::CREATED, "body={body}");
    assert_eq!(body["user_id"], serde_json::json!(uid_b.to_string()));
    // `member` is a DEPRECATED alias normalised to canonical `participant`.
    assert_eq!(body["role"], serde_json::json!("participant"));
    assert_eq!(body["session_id"], serde_json::json!(sid));
    assert!(body["joined_at"].is_string());
}

#[tokio::test]
async fn add_member_duplicate_returns_409() {
    let state = test_state().await;
    let app = build_router(state);
    let (_, cookie_a) = register_and_login(app.clone(), "alice@x.test", "password-12345").await;
    let (uid_b, _) = register_and_login(app.clone(), "bob@x.test", "password-12345").await;
    let (_, s1) = create_session(&app, &cookie_a, "alice-1").await;
    let sid = s1["id"].as_str().expect("id");

    let (status, _) = add_member(&app, &cookie_a, sid, uid_b, "member").await;
    assert_eq!(status, StatusCode::CREATED);
    let (status, body) = add_member(&app, &cookie_a, sid, uid_b, "viewer").await;
    assert_eq!(status, StatusCode::CONFLICT, "body={body}");
    assert_eq!(body, serde_json::json!({"error":"already_member"}));
}

#[tokio::test]
async fn add_member_invalid_role_422() {
    let state = test_state().await;
    let app = build_router(state);
    let (_, cookie_a) = register_and_login(app.clone(), "alice@x.test", "password-12345").await;
    let (uid_b, _) = register_and_login(app.clone(), "bob@x.test", "password-12345").await;
    let (_, s1) = create_session(&app, &cookie_a, "alice-1").await;
    let sid = s1["id"].as_str().expect("id");

    let (status, _) = add_member(&app, &cookie_a, sid, uid_b, "admin").await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn add_member_participant_role_201() {
    // FR-010: `participant` is the canonical role name.
    let state = test_state().await;
    let app = build_router(state);
    let (_, cookie_a) = register_and_login(app.clone(), "alice@x.test", "password-12345").await;
    let (uid_b, _) = register_and_login(app.clone(), "bob@x.test", "password-12345").await;
    let (_, s1) = create_session(&app, &cookie_a, "alice-1").await;
    let sid = s1["id"].as_str().expect("id");

    let (status, body) = add_member(&app, &cookie_a, sid, uid_b, "participant").await;
    assert_eq!(status, StatusCode::CREATED, "body={body}");
    assert_eq!(body["role"], serde_json::json!("participant"));
}

#[tokio::test]
async fn add_member_observer_role_201() {
    // FR-010: `observer` is the canonical role name.
    let state = test_state().await;
    let app = build_router(state);
    let (_, cookie_a) = register_and_login(app.clone(), "alice@x.test", "password-12345").await;
    let (uid_b, _) = register_and_login(app.clone(), "bob@x.test", "password-12345").await;
    let (_, s1) = create_session(&app, &cookie_a, "alice-1").await;
    let sid = s1["id"].as_str().expect("id");

    let (status, body) = add_member(&app, &cookie_a, sid, uid_b, "observer").await;
    assert_eq!(status, StatusCode::CREATED, "body={body}");
    assert_eq!(body["role"], serde_json::json!("observer"));
}

#[tokio::test]
async fn add_member_unknown_user_422() {
    let state = test_state().await;
    let app = build_router(state);
    let (_, cookie_a) = register_and_login(app.clone(), "alice@x.test", "password-12345").await;
    let (_, s1) = create_session(&app, &cookie_a, "alice-1").await;
    let sid = s1["id"].as_str().expect("id");

    let unknown = Uuid::new_v4();
    let (status, body) = add_member(&app, &cookie_a, sid, unknown, "member").await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "body={body}");
    assert_eq!(body, serde_json::json!({"error":"unknown_user"}));
}

#[tokio::test]
async fn add_member_non_owner_forbidden_403() {
    let state = test_state().await;
    let app = build_router(state);
    let (_, cookie_a) = register_and_login(app.clone(), "alice@x.test", "password-12345").await;
    let (uid_b, cookie_b) = register_and_login(app.clone(), "bob@x.test", "password-12345").await;
    let (uid_c, _) = register_and_login(app.clone(), "carol@x.test", "password-12345").await;
    let (_, s1) = create_session(&app, &cookie_a, "alice-1").await;
    let sid = s1["id"].as_str().expect("id");

    // Make bob a member first.
    let (status, _) = add_member(&app, &cookie_a, sid, uid_b, "member").await;
    assert_eq!(status, StatusCode::CREATED);

    // bob (member, not owner) cannot add carol.
    let (status, body) = add_member(&app, &cookie_b, sid, uid_c, "member").await;
    assert_eq!(status, StatusCode::FORBIDDEN, "body={body}");
    assert_eq!(body, serde_json::json!({"error":"forbidden"}));
}

#[tokio::test]
async fn add_member_to_nonexistent_session_404() {
    let state = test_state().await;
    let app = build_router(state);
    let (_, cookie_a) = register_and_login(app.clone(), "alice@x.test", "password-12345").await;
    let (uid_b, _) = register_and_login(app.clone(), "bob@x.test", "password-12345").await;

    let missing_session_id = Uuid::new_v4().to_string();
    let (status, _) = add_member(&app, &cookie_a, &missing_session_id, uid_b, "member").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn list_members_owner_sees_all() {
    let state = test_state().await;
    let app = build_router(state);
    let (_, cookie_a) = register_and_login(app.clone(), "alice@x.test", "password-12345").await;
    let (uid_b, _) = register_and_login(app.clone(), "bob@x.test", "password-12345").await;
    let (_, s1) = create_session(&app, &cookie_a, "alice-1").await;
    let sid = s1["id"].as_str().expect("id");

    let (status, _) = add_member(&app, &cookie_a, sid, uid_b, "member").await;
    assert_eq!(status, StatusCode::CREATED);

    let resp = app
        .oneshot(req(
            Method::GET,
            &format!("/api/sessions/{sid}/members"),
            Some(&cookie_a),
            None,
        ))
        .await
        .expect("list resp");
    let (status, _, body) = read_body(resp).await;
    assert_eq!(status, StatusCode::OK);
    let members = body["members"].as_array().expect("members array");
    assert_eq!(members.len(), 1);
    assert_eq!(members[0]["user_id"], serde_json::json!(uid_b.to_string()));
    assert_eq!(members[0]["role"], serde_json::json!("participant"));
}

#[tokio::test]
async fn list_members_member_sees_all() {
    let state = test_state().await;
    let app = build_router(state);
    let (_, cookie_a) = register_and_login(app.clone(), "alice@x.test", "password-12345").await;
    let (uid_b, cookie_b) = register_and_login(app.clone(), "bob@x.test", "password-12345").await;
    let (_, s1) = create_session(&app, &cookie_a, "alice-1").await;
    let sid = s1["id"].as_str().expect("id");
    let (status, _) = add_member(&app, &cookie_a, sid, uid_b, "member").await;
    assert_eq!(status, StatusCode::CREATED);

    let resp = app
        .oneshot(req(
            Method::GET,
            &format!("/api/sessions/{sid}/members"),
            Some(&cookie_b),
            None,
        ))
        .await
        .expect("list resp");
    let (status, _, body) = read_body(resp).await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    let members = body["members"].as_array().expect("members array");
    assert_eq!(members.len(), 1);
}

#[tokio::test]
async fn list_members_outsider_404() {
    let state = test_state().await;
    let app = build_router(state);
    let (_, cookie_a) = register_and_login(app.clone(), "alice@x.test", "password-12345").await;
    let (_, cookie_c) = register_and_login(app.clone(), "carol@x.test", "password-12345").await;
    let (_, s1) = create_session(&app, &cookie_a, "alice-1").await;
    let sid = s1["id"].as_str().expect("id");

    let resp = app
        .oneshot(req(
            Method::GET,
            &format!("/api/sessions/{sid}/members"),
            Some(&cookie_c),
            None,
        ))
        .await
        .expect("list resp");
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn delete_member_owner_succeeds_204() {
    let state = test_state().await;
    let app = build_router(state);
    let (_, cookie_a) = register_and_login(app.clone(), "alice@x.test", "password-12345").await;
    let (uid_b, _) = register_and_login(app.clone(), "bob@x.test", "password-12345").await;
    let (_, s1) = create_session(&app, &cookie_a, "alice-1").await;
    let sid = s1["id"].as_str().expect("id");
    let (status, _) = add_member(&app, &cookie_a, sid, uid_b, "member").await;
    assert_eq!(status, StatusCode::CREATED);

    let resp = app
        .clone()
        .oneshot(req(
            Method::DELETE,
            &format!("/api/sessions/{sid}/members/{uid_b}"),
            Some(&cookie_a),
            None,
        ))
        .await
        .expect("delete resp");
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);

    // Verify removal via GET.
    let resp = app
        .oneshot(req(
            Method::GET,
            &format!("/api/sessions/{sid}/members"),
            Some(&cookie_a),
            None,
        ))
        .await
        .expect("list resp");
    let (_, _, body) = read_body(resp).await;
    let members = body["members"].as_array().expect("members array");
    assert_eq!(members.len(), 0);
}

#[tokio::test]
async fn delete_member_owner_self_returns_409_cannot_remove_owner() {
    let state = test_state().await;
    let app = build_router(state);
    let (uid_a, cookie_a) = register_and_login(app.clone(), "alice@x.test", "password-12345").await;
    let (_, s1) = create_session(&app, &cookie_a, "alice-1").await;
    let sid = s1["id"].as_str().expect("id");

    let resp = app
        .oneshot(req(
            Method::DELETE,
            &format!("/api/sessions/{sid}/members/{uid_a}"),
            Some(&cookie_a),
            None,
        ))
        .await
        .expect("delete resp");
    let (status, _, body) = read_body(resp).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body, serde_json::json!({"error":"cannot_remove_owner"}));
}

#[tokio::test]
async fn delete_member_non_owner_forbidden_403() {
    let state = test_state().await;
    let app = build_router(state);
    let (_, cookie_a) = register_and_login(app.clone(), "alice@x.test", "password-12345").await;
    let (uid_b, cookie_b) = register_and_login(app.clone(), "bob@x.test", "password-12345").await;
    let (uid_c, _) = register_and_login(app.clone(), "carol@x.test", "password-12345").await;
    let (_, s1) = create_session(&app, &cookie_a, "alice-1").await;
    let sid = s1["id"].as_str().expect("id");
    let (status, _) = add_member(&app, &cookie_a, sid, uid_b, "member").await;
    assert_eq!(status, StatusCode::CREATED);
    let (status, _) = add_member(&app, &cookie_a, sid, uid_c, "member").await;
    assert_eq!(status, StatusCode::CREATED);

    // bob (member) tries to remove carol.
    let resp = app
        .oneshot(req(
            Method::DELETE,
            &format!("/api/sessions/{sid}/members/{uid_c}"),
            Some(&cookie_b),
            None,
        ))
        .await
        .expect("delete resp");
    let (status, _, body) = read_body(resp).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(body, serde_json::json!({"error":"forbidden"}));
}

#[tokio::test]
async fn delete_nonexistent_member_returns_404() {
    let state = test_state().await;
    let app = build_router(state);
    let (_, cookie_a) = register_and_login(app.clone(), "alice@x.test", "password-12345").await;
    let (_, s1) = create_session(&app, &cookie_a, "alice-1").await;
    let sid = s1["id"].as_str().expect("id");

    let ghost = Uuid::new_v4();
    let resp = app
        .oneshot(req(
            Method::DELETE,
            &format!("/api/sessions/{sid}/members/{ghost}"),
            Some(&cookie_a),
            None,
        ))
        .await
        .expect("delete resp");
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn unauth_request_401() {
    let state = test_state().await;
    let app = build_router(state);
    let (_, cookie_a) = register_and_login(app.clone(), "alice@x.test", "password-12345").await;
    let (uid_b, _) = register_and_login(app.clone(), "bob@x.test", "password-12345").await;
    let (_, s1) = create_session(&app, &cookie_a, "alice-1").await;
    let sid = s1["id"].as_str().expect("id");

    // No cookie on POST members.
    let resp = app
        .clone()
        .oneshot(req(
            Method::POST,
            &format!("/api/sessions/{sid}/members"),
            None,
            Some(serde_json::json!({ "user_id": uid_b, "role": "member" })),
        ))
        .await
        .expect("post resp");
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

    // No cookie on GET members.
    let resp = app
        .clone()
        .oneshot(req(
            Method::GET,
            &format!("/api/sessions/{sid}/members"),
            None,
            None,
        ))
        .await
        .expect("get resp");
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

    // No cookie on DELETE.
    let resp = app
        .oneshot(req(
            Method::DELETE,
            &format!("/api/sessions/{sid}/members/{uid_b}"),
            None,
            None,
        ))
        .await
        .expect("delete resp");
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}
