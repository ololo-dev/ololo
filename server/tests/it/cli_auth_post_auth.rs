//! SEC-H2 regression: `GET /auth/cli/post-auth` must never mint a PAT.
//!
//! Minting from a bare cookie-authenticated GET is an account-takeover CSRF
//! vector (an attacker lures a logged-in victim to the URL with the attacker's
//! `cli_token` and captures the victim's PAT). Minting lives only in
//! `POST /auth/cli/confirm` behind an explicit user action.

use crate::common;

use arena_core::entities::cli_tokens;
use axum::body::Body;
use axum::http::{Method, Request, header};
use sea_orm::EntityTrait;
use server::build_router;
use tower::ServiceExt;

use crate::common::{register_and_login_default, test_state};

#[tokio::test]
async fn get_post_auth_does_not_mint_pat_even_with_valid_cookie() {
    let state = test_state().await;
    let app = build_router(state.clone());

    // A logged-in victim.
    let (_uid, access) = register_and_login_default(app.clone(), "victim@example.test").await;

    // An attacker-chosen, well-formed cli_token not bound to this browser.
    let cli_token = "a".repeat(64);

    let resp = app
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri(format!("/auth/cli/post-auth?cli_token={cli_token}"))
                .header(header::COOKIE, format!("arena_access={access}"))
                .body(Body::empty())
                .expect("req"),
        )
        .await
        .expect("resp");

    // Redirects to the same-origin confirmation page instead of minting.
    assert!(resp.status().is_redirection(), "status {}", resp.status());
    let location = resp
        .headers()
        .get(header::LOCATION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert!(
        location.starts_with("/cli-auth?cli_token="),
        "should redirect to the confirmation page, got {location:?}"
    );

    // Crucially: no PAT was created.
    let tokens = cli_tokens::Entity::find()
        .all(&state.db)
        .await
        .expect("query cli_tokens");
    assert!(tokens.is_empty(), "GET post-auth must not mint a PAT");
}
