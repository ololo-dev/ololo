//! Who may open a player's run page (`GET /api/sessions/:code/players/:id`).
//!
//! The rule the docs promise: on a public project any signed-in visitor can
//! read the run — the report and the verdicts are the shareable half of a
//! session. What stays with the owner and admins is the inspection surface:
//! the git history endpoint keeps its own guard, and a non-public project
//! keeps the whole page to its own players.

use axum::http::{Method, StatusCode};
use server::build_router;
use tower::ServiceExt;
use uuid::Uuid;

use crate::common::{
    make_user_admin, read_body_json, register_and_login_default, req_with_cookie, test_state,
};

/// A project (public unless created by an admin asking otherwise), a session
/// on it, and one joined player.
async fn session_with_player(
    app: &axum::Router,
    creator: &str,
    joiner: &str,
    public: bool,
) -> (String, String) {
    let resp = app
        .clone()
        .oneshot(req_with_cookie(
            Method::POST,
            "/api/projects",
            creator,
            Some(serde_json::json!({
                "name": format!("proj-{}", Uuid::new_v4()),
                "public": public,
            })),
        ))
        .await
        .expect("create project");
    let (sc, pb) = read_body_json(resp).await;
    assert_eq!(sc, StatusCode::CREATED, "project: {pb}");
    let project_id = pb["id"].as_str().expect("project id").to_string();

    let resp = app
        .clone()
        .oneshot(req_with_cookie(
            Method::POST,
            "/api/sessions",
            creator,
            Some(serde_json::json!({
                "name": format!("sess-{}", Uuid::new_v4()),
                "project_id": project_id,
            })),
        ))
        .await
        .expect("create session");
    let (sc, sb) = read_body_json(resp).await;
    assert_eq!(sc, StatusCode::CREATED, "session: {sb}");
    let code = sb["join_code"].as_str().expect("join_code").to_string();

    let resp = app
        .clone()
        .oneshot(req_with_cookie(
            Method::POST,
            "/api/sessions/join",
            joiner,
            Some(serde_json::json!({ "code": code })),
        ))
        .await
        .expect("join");
    let (sc, jb) = read_body_json(resp).await;
    assert_eq!(sc, StatusCode::CREATED, "join: {jb}");
    let player_id = jb["player_id"].as_str().expect("player_id").to_string();
    (code, player_id)
}

#[tokio::test]
async fn a_signed_in_stranger_reads_a_public_run_but_not_its_history() {
    let state = test_state().await;
    let app = build_router(state.clone());
    let (_, owner) = register_and_login_default(app.clone(), "own-vis@x.test").await;
    let (_, joiner) = register_and_login_default(app.clone(), "join-vis@x.test").await;
    let (_, stranger) = register_and_login_default(app.clone(), "str-vis@x.test").await;

    // Non-admin creators always get a public project.
    let (code, player_id) = session_with_player(&app, &owner, &joiner, true).await;

    let snap_uri = format!("/api/sessions/{code}/players/{player_id}");
    let resp = app
        .clone()
        .oneshot(req_with_cookie(Method::GET, &snap_uri, &stranger, None))
        .await
        .expect("stranger snapshot");
    let (sc, body) = read_body_json(resp).await;
    assert_eq!(sc, StatusCode::OK, "stranger on a public run: {body}");
    assert_eq!(body["player_id"].as_str(), Some(player_id.as_str()));

    // The inspection surface is not part of the deal: diffs stay guarded.
    let hist_uri = format!("/api/sessions/{code}/players/{player_id}/history");
    let resp = app
        .clone()
        .oneshot(req_with_cookie(Method::GET, &hist_uri, &stranger, None))
        .await
        .expect("stranger history");
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn a_private_project_keeps_the_run_to_its_own_players() {
    let state = test_state().await;
    let app = build_router(state.clone());
    // Only an admin can create a non-public project.
    let (admin_id, admin) = register_and_login_default(app.clone(), "adm-vis@x.test").await;
    make_user_admin(&state, admin_id).await;
    let (_, joiner) = register_and_login_default(app.clone(), "join-priv@x.test").await;
    let (_, stranger) = register_and_login_default(app.clone(), "str-priv@x.test").await;

    let (code, player_id) = session_with_player(&app, &admin, &joiner, false).await;

    let snap_uri = format!("/api/sessions/{code}/players/{player_id}");
    let resp = app
        .clone()
        .oneshot(req_with_cookie(Method::GET, &snap_uri, &stranger, None))
        .await
        .expect("stranger snapshot");
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);

    // The visual endpoints follow the same rule: a private project keeps
    // its screenshots and screencasts to its own players.
    let art_uri = format!(
        "/api/sessions/{code}/players/{player_id}/artifacts/{}",
        Uuid::new_v4()
    );
    let resp = app
        .clone()
        .oneshot(req_with_cookie(Method::GET, &art_uri, &stranger, None))
        .await
        .expect("stranger artifact");
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);

    // The player themselves still reads it.
    let resp = app
        .clone()
        .oneshot(req_with_cookie(Method::GET, &snap_uri, &joiner, None))
        .await
        .expect("joiner snapshot");
    assert_eq!(resp.status(), StatusCode::OK);
}
