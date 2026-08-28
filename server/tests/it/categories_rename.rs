//! `PATCH /api/admin/categories/:id` — renaming a category.
//!
//! The cascade is the load-bearing behaviour: `projects.category` stores the
//! category *name*, not a foreign key, so a rename that doesn't rewrite the
//! projects would leave them pointing at a category that no longer exists
//! (they'd vanish from the category sidebar and fail category validation).

use axum::http::{Method, StatusCode};
use server::build_router;
use tower::ServiceExt;

use crate::common;
use crate::common::{
    make_user_admin, read_body_json, register_and_login_default, req_with_cookie, test_state,
};

/// Resolve a seeded category's id by name.
async fn category_id(app: &axum::Router, cookie: &str, name: &str) -> i64 {
    let resp = app
        .clone()
        .oneshot(req_with_cookie(
            Method::GET,
            "/api/categories",
            cookie,
            None,
        ))
        .await
        .expect("list categories");
    let (_, body) = read_body_json(resp).await;
    body["categories"]
        .as_array()
        .expect("categories array")
        .iter()
        .find(|c| c["name"] == name)
        .unwrap_or_else(|| panic!("category {name} not seeded: {body}"))["id"]
        .as_i64()
        .expect("id")
}

async fn rename(
    app: &axum::Router,
    cookie: &str,
    id: i64,
    new_name: &str,
) -> (StatusCode, serde_json::Value) {
    let resp = app
        .clone()
        .oneshot(req_with_cookie(
            Method::PATCH,
            &format!("/api/admin/categories/{id}"),
            cookie,
            Some(serde_json::json!({ "name": new_name })),
        ))
        .await
        .expect("rename resp");
    read_body_json(resp).await
}

#[tokio::test]
async fn rename_cascades_to_projects_using_the_old_name() {
    let state = test_state().await;
    let app = build_router(state.clone());
    let (admin_id, cookie) = register_and_login_default(app.clone(), "admin-cat1@x.test").await;
    make_user_admin(&state, admin_id).await;

    // A project filed under the category we're about to rename.
    let resp = app
        .clone()
        .oneshot(req_with_cookie(
            Method::POST,
            "/api/projects",
            &cookie,
            Some(serde_json::json!({ "name": "Sorting Kata", "category": "Algorithms" })),
        ))
        .await
        .expect("create project");
    let (sc, project) = read_body_json(resp).await;
    assert_eq!(sc, StatusCode::CREATED, "{project}");
    let project_id = project["id"].as_str().expect("project id").to_string();

    let id = category_id(&app, &cookie, "Algorithms").await;
    let (sc, body) = rename(&app, &cookie, id, "Algos").await;
    assert_eq!(sc, StatusCode::OK, "{body}");
    assert_eq!(body["category"]["name"], "Algos");
    assert_eq!(body["projects_updated"], 1, "the one project was rewritten");

    // The category itself is renamed …
    let resp = app
        .clone()
        .oneshot(req_with_cookie(
            Method::GET,
            "/api/categories",
            &cookie,
            None,
        ))
        .await
        .expect("list");
    let (_, list) = read_body_json(resp).await;
    let names: Vec<&str> = list["categories"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|c| c["name"].as_str())
        .collect();
    assert!(names.contains(&"Algos"), "renamed: {names:?}");
    assert!(!names.contains(&"Algorithms"), "old name gone: {names:?}");

    // … and the project moved with it, rather than being orphaned.
    let resp = app
        .clone()
        .oneshot(req_with_cookie(
            Method::GET,
            &format!("/api/projects/{project_id}"),
            &cookie,
            None,
        ))
        .await
        .expect("get project");
    let (_, proj) = read_body_json(resp).await;
    assert_eq!(
        proj["category"], "Algos",
        "project follows the rename: {proj}"
    );
}

#[tokio::test]
async fn rename_reports_usage_count_in_the_list() {
    let state = test_state().await;
    let app = build_router(state.clone());
    let (admin_id, cookie) = register_and_login_default(app.clone(), "admin-cat2@x.test").await;
    make_user_admin(&state, admin_id).await;

    for name in ["P1", "P2"] {
        let resp = app
            .clone()
            .oneshot(req_with_cookie(
                Method::POST,
                "/api/projects",
                &cookie,
                Some(serde_json::json!({ "name": name, "category": "Math" })),
            ))
            .await
            .expect("create project");
        let (sc, b) = read_body_json(resp).await;
        assert_eq!(sc, StatusCode::CREATED, "{b}");
    }

    let resp = app
        .clone()
        .oneshot(req_with_cookie(
            Method::GET,
            "/api/categories",
            &cookie,
            None,
        ))
        .await
        .expect("list");
    let (_, list) = read_body_json(resp).await;
    let math = list["categories"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "Math")
        .expect("Math category");
    assert_eq!(math["project_count"], 2, "usage count surfaced: {math}");
}

#[tokio::test]
async fn rename_to_an_existing_name_conflicts() {
    let state = test_state().await;
    let app = build_router(state.clone());
    let (admin_id, cookie) = register_and_login_default(app.clone(), "admin-cat3@x.test").await;
    make_user_admin(&state, admin_id).await;

    let id = category_id(&app, &cookie, "Algorithms").await;
    let (sc, body) = rename(&app, &cookie, id, "Math").await;
    assert_eq!(sc, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["error"], "duplicate");
}

#[tokio::test]
async fn rename_to_the_same_name_is_a_no_op() {
    let state = test_state().await;
    let app = build_router(state.clone());
    let (admin_id, cookie) = register_and_login_default(app.clone(), "admin-cat4@x.test").await;
    make_user_admin(&state, admin_id).await;

    let id = category_id(&app, &cookie, "Algorithms").await;
    let (sc, body) = rename(&app, &cookie, id, "Algorithms").await;
    assert_eq!(sc, StatusCode::OK, "{body}");
    assert_eq!(body["projects_updated"], 0);
}

#[tokio::test]
async fn rename_validates_name_and_id() {
    let state = test_state().await;
    let app = build_router(state.clone());
    let (admin_id, cookie) = register_and_login_default(app.clone(), "admin-cat5@x.test").await;
    make_user_admin(&state, admin_id).await;

    let id = category_id(&app, &cookie, "Algorithms").await;

    let (sc, body) = rename(&app, &cookie, id, "   ").await;
    assert_eq!(
        sc,
        StatusCode::UNPROCESSABLE_ENTITY,
        "blank rejected: {body}"
    );
    assert_eq!(body["error"], "invalid_name");

    let (sc, body) = rename(&app, &cookie, id, &"x".repeat(101)).await;
    assert_eq!(
        sc,
        StatusCode::UNPROCESSABLE_ENTITY,
        "too long rejected: {body}"
    );

    let (sc, _) = rename(&app, &cookie, 999_999, "Ghost").await;
    assert_eq!(sc, StatusCode::NOT_FOUND, "unknown id");
}

#[tokio::test]
async fn rename_requires_admin() {
    let state = test_state().await;
    let app = build_router(state.clone());
    // First registered user is auto-admin — burn that slot, then register the
    // non-admin actually under test.
    let (first_id, first_cookie) =
        register_and_login_default(app.clone(), "first-cat6@x.test").await;
    make_user_admin(&state, first_id).await;
    let id = category_id(&app, &first_cookie, "Algorithms").await;

    let (_uid, cookie) = register_and_login_default(app.clone(), "user-cat6@x.test").await;
    let (sc, _) = rename(&app, &cookie, id, "Nope").await;
    assert_eq!(sc, StatusCode::FORBIDDEN);
}
