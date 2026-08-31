//! Authorization on the artifact read endpoints
//! (`…/artifacts/:probe_id` and `…/repo-file`): owner-only with an admin
//! override, strangers rejected, and the repo-file path filters that keep
//! the endpoint from becoming a generic source browser. These paths guard
//! player-private data and had zero coverage before this file.

use axum::http::{Method, StatusCode};
use server::build_router;
use tower::ServiceExt;
use uuid::Uuid;

use crate::common::{
    make_user_admin, read_body_json, register_and_login_default, req_with_cookie, test_state,
};

async fn create_session(app: &axum::Router, cookie: &str) -> String {
    let resp = app
        .clone()
        .oneshot(req_with_cookie(
            Method::POST,
            "/api/projects",
            cookie,
            Some(serde_json::json!({ "name": format!("proj-{}", Uuid::new_v4()) })),
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
            cookie,
            Some(serde_json::json!({
                "name": format!("sess-{}", Uuid::new_v4()),
                "project_id": project_id
            })),
        ))
        .await
        .expect("create session");
    let (sc, sb) = read_body_json(resp).await;
    assert_eq!(sc, StatusCode::CREATED, "session: {sb}");
    sb["join_code"].as_str().expect("join_code").to_string()
}

async fn join_session(app: &axum::Router, cookie: &str, code: &str) -> String {
    let resp = app
        .clone()
        .oneshot(req_with_cookie(
            Method::POST,
            "/api/sessions/join",
            cookie,
            Some(serde_json::json!({ "code": code })),
        ))
        .await
        .expect("join resp");
    let (sc, body) = read_body_json(resp).await;
    assert_eq!(sc, StatusCode::CREATED, "join failed: {body}");
    body["player_id"].as_str().expect("player_id").to_string()
}

/// One session with a joined player, plus a stranger and an admin.
async fn artifact_fixture() -> (axum::Router, String, String, Cookies) {
    let state = test_state().await;
    let app = build_router(state.clone());
    let (_owner_id, owner) = register_and_login_default(app.clone(), "owner-art@x.test").await;
    let (_joiner_id, joiner) = register_and_login_default(app.clone(), "joiner-art@x.test").await;
    let (_stranger_id, stranger) =
        register_and_login_default(app.clone(), "stranger-art@x.test").await;
    let (admin_id, admin) = register_and_login_default(app.clone(), "admin-art@x.test").await;
    make_user_admin(&state, admin_id).await;

    let code = create_session(&app, &owner).await;
    let player_id = join_session(&app, &joiner, &code).await;
    (
        app,
        code,
        player_id,
        Cookies {
            joiner,
            stranger,
            admin,
        },
    )
}

struct Cookies {
    joiner: String,
    stranger: String,
    admin: String,
}

async fn get_status(app: &axum::Router, uri: &str, cookie: &str) -> StatusCode {
    let resp = app
        .clone()
        .oneshot(req_with_cookie(Method::GET, uri, cookie, None))
        .await
        .expect("get");
    resp.status()
}

#[tokio::test]
async fn a_spectator_passes_authz_on_a_public_project() {
    // Fixture projects are public (non-admin creators cannot make private
    // ones), so a signed-in stranger clears authorization on both visual
    // endpoints — screenshots and screencasts are the public face of a run.
    // 404, not 403: the probe/file genuinely does not exist.
    let (app, code, player_id, cookies) = artifact_fixture().await;
    let probe_uri = format!(
        "/api/sessions/{code}/players/{player_id}/artifacts/{}",
        Uuid::new_v4()
    );
    assert_eq!(
        get_status(&app, &probe_uri, &cookies.stranger).await,
        StatusCode::NOT_FOUND,
        "a spectator is authorized on a public project"
    );
    let file_uri = format!("/api/sessions/{code}/players/{player_id}/repo-file?path=shot.png");
    assert_eq!(
        get_status(&app, &file_uri, &cookies.stranger).await,
        StatusCode::NOT_FOUND,
        "a spectator may fetch run images on a public project"
    );
}

#[tokio::test]
async fn owner_and_admin_pass_authz_and_get_not_found_for_unknown_probe() {
    let (app, code, player_id, cookies) = artifact_fixture().await;
    let uri = format!(
        "/api/sessions/{code}/players/{player_id}/artifacts/{}",
        Uuid::new_v4()
    );
    // 404 (probe does not exist), NOT 403 — authorization passed.
    assert_eq!(
        get_status(&app, &uri, &cookies.joiner).await,
        StatusCode::NOT_FOUND,
        "the player's own user is authorized"
    );
    assert_eq!(
        get_status(&app, &uri, &cookies.admin).await,
        StatusCode::NOT_FOUND,
        "an admin may view any player's artifacts"
    );
}

#[tokio::test]
async fn unknown_session_and_player_yield_not_found() {
    let (app, code, _player_id, cookies) = artifact_fixture().await;
    let bad_session = format!(
        "/api/sessions/ZZZZZZ/players/{}/artifacts/{}",
        Uuid::new_v4(),
        Uuid::new_v4()
    );
    assert_eq!(
        get_status(&app, &bad_session, &cookies.joiner).await,
        StatusCode::NOT_FOUND
    );
    let bad_player = format!(
        "/api/sessions/{code}/players/{}/artifacts/{}",
        Uuid::new_v4(),
        Uuid::new_v4()
    );
    assert_eq!(
        get_status(&app, &bad_player, &cookies.joiner).await,
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn repo_file_rejects_non_images_and_sneaky_paths() {
    let (app, code, player_id, cookies) = artifact_fixture().await;
    let base = format!("/api/sessions/{code}/players/{player_id}/repo-file");
    // Not an allowed image extension → never reaches git.
    for path in ["notes.txt", "src%2Fmain.rs", "index.html"] {
        assert_eq!(
            get_status(&app, &format!("{base}?path={path}"), &cookies.joiner).await,
            StatusCode::NOT_FOUND,
            "non-image path {path} must be rejected"
        );
    }
    // Option-looking, rev-syntax, and traversal-shaped input is refused
    // outright even with an image extension.
    for path in ["-evil.png", "..%2Fescape.png", "HEAD%3Ax.png"] {
        assert_eq!(
            get_status(&app, &format!("{base}?path={path}"), &cookies.joiner).await,
            StatusCode::NOT_FOUND,
            "sneaky path {path} must be rejected"
        );
    }
}
