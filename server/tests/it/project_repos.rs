//! A project's repository — the code its sessions start from: named on any
//! project, validated like something every player's machine will clone,
//! carried by exports and seeds, and handed to whoever joins a session.

use axum::http::{Method, StatusCode};
use server::build_router;
use tower::ServiceExt;

use crate::common::*;

struct World {
    app: axum::Router,
    admin: String,
    bob: String,
    carol: String,
}

async fn world() -> World {
    let state = test_state().await;
    let judges = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root")
        .join("judges");
    server::seed::judges::seed_judges_with_dir(&state.db, &judges)
        .await
        .expect("seed judges");
    let app = build_router(state);
    let (_, admin) = register_and_login_default(app.clone(), "admin@x.test").await;
    let (_, bob) = register_and_login_default(app.clone(), "bob@x.test").await;
    let (_, carol) = register_and_login_default(app.clone(), "carol@x.test").await;
    set_project_creation_setting(&app, &admin, "true").await;
    World {
        app,
        admin,
        bob,
        carol,
    }
}

async fn call(
    app: &axum::Router,
    method: Method,
    uri: &str,
    cookie: &str,
    body: Option<serde_json::Value>,
) -> (StatusCode, serde_json::Value) {
    let resp = app
        .clone()
        .oneshot(req(method, uri, Some(cookie), body))
        .await
        .expect("response");
    read_body_json(resp).await
}

const STARTER: &str = "https://github.com/ololo-dev/starter.git";

#[tokio::test]
async fn a_project_names_the_repository_its_sessions_start_from() {
    let w = world().await;
    let (status, project) = call(
        &w.app,
        Method::POST,
        "/api/projects",
        &w.admin,
        Some(serde_json::json!({
            "name": "Fix the starter",
            "repo_url": STARTER,
            "repo_ref": "v1.0",
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{project}");
    assert_eq!(project["repo_url"], STARTER);
    assert_eq!(project["repo_ref"], "v1.0");
    let id = project["id"].as_str().unwrap().to_string();

    let (_, read) = call(
        &w.app,
        Method::GET,
        &format!("/api/projects/{id}"),
        &w.admin,
        None,
    )
    .await;
    assert_eq!(read["repo_url"], STARTER, "{read}");

    // A ref alone moves the checkout; the URL stays.
    let (status, moved) = call(
        &w.app,
        Method::PATCH,
        &format!("/api/projects/{id}"),
        &w.admin,
        Some(serde_json::json!({ "repo_ref": "main" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{moved}");
    assert_eq!(moved["repo_url"], STARTER);
    assert_eq!(moved["repo_ref"], "main");

    // A new URL starts over at its default branch.
    let (_, replaced) = call(
        &w.app,
        Method::PATCH,
        &format!("/api/projects/{id}"),
        &w.admin,
        Some(serde_json::json!({ "repo_url": "git@github.com:ololo-dev/other.git" })),
    )
    .await;
    assert_eq!(replaced["repo_url"], "git@github.com:ololo-dev/other.git");
    assert_eq!(replaced["repo_ref"], serde_json::Value::Null);

    // An empty URL drops it.
    let (_, dropped) = call(
        &w.app,
        Method::PATCH,
        &format!("/api/projects/{id}"),
        &w.admin,
        Some(serde_json::json!({ "repo_url": "" })),
    )
    .await;
    assert_eq!(dropped["repo_url"], serde_json::Value::Null, "{dropped}");

    // Only what a player's machine may safely clone.
    for body in [
        serde_json::json!({ "repo_url": "file:///etc" }),
        serde_json::json!({ "repo_url": "https://user:token@github.com/a/b" }),
        serde_json::json!({ "repo_url": "ext::sh -c id" }),
        serde_json::json!({ "repo_url": STARTER, "repo_ref": "--upload-pack=x" }),
        serde_json::json!({ "repo_ref": "main" }),
    ] {
        let (status, err) = call(
            &w.app,
            Method::PATCH,
            &format!("/api/projects/{id}"),
            &w.admin,
            Some(body.clone()),
        )
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body} → {err}");
        assert_eq!(err["error"], "invalid_repo", "{err}");
        assert!(
            err["detail"].as_str().is_some_and(|d| !d.is_empty()),
            "{err}"
        );
    }
}

#[tokio::test]
async fn a_personal_project_carries_its_owners_repository() {
    let w = world().await;
    let (status, project) = call(
        &w.app,
        Method::POST,
        "/api/personal-projects",
        &w.bob,
        Some(serde_json::json!({
            "description": "Add a CSV export to the reports page.",
            "judges": ["code-quality"],
            "repo_url": "git@github.com:bob/reports.git",
            "repo_ref": "develop",
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{project}");
    assert_eq!(project["repo_url"], "git@github.com:bob/reports.git");
    assert_eq!(project["repo_ref"], "develop");
    let id = project["id"].as_str().unwrap().to_string();
    let rebuild = |extra: serde_json::Value| {
        let mut body = serde_json::json!({
            "description": "Add a CSV export to the reports page, filters included.",
            "judges": ["code-quality"],
        });
        for (k, v) in extra.as_object().unwrap() {
            body[k] = v.clone();
        }
        body
    };

    // A rebuild that does not mention it leaves it; a blank one drops it.
    let (status, kept) = call(
        &w.app,
        Method::PUT,
        &format!("/api/personal-projects/{id}"),
        &w.bob,
        Some(rebuild(serde_json::json!({}))),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{kept}");
    assert_eq!(kept["repo_url"], "git@github.com:bob/reports.git");
    let (_, dropped) = call(
        &w.app,
        Method::PUT,
        &format!("/api/personal-projects/{id}"),
        &w.bob,
        Some(rebuild(
            serde_json::json!({ "repo_url": "", "repo_ref": "" }),
        )),
    )
    .await;
    assert_eq!(dropped["repo_url"], serde_json::Value::Null, "{dropped}");

    // The generic editor may move it on a personal project, any time.
    let (status, patched) = call(
        &w.app,
        Method::PATCH,
        &format!("/api/projects/{id}"),
        &w.bob,
        Some(serde_json::json!({ "repo_url": "https://github.com/bob/reports" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{patched}");
    assert_eq!(patched["repo_url"], "https://github.com/bob/reports");

    let (status, err) = call(
        &w.app,
        Method::PUT,
        &format!("/api/personal-projects/{id}"),
        &w.bob,
        Some(rebuild(
            serde_json::json!({ "repo_url": "http://github.com/bob/reports" }),
        )),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{err}");
    assert_eq!(err["error"], "invalid_personal_project");
    assert_eq!(err["field"], "repo_url");
}

#[tokio::test]
async fn joining_tells_a_player_what_the_project_is_even_a_private_one() {
    let w = world().await;
    let (_, project) = call(
        &w.app,
        Method::POST,
        "/api/personal-projects",
        &w.bob,
        Some(serde_json::json!({
            "description": "Add a CSV export to the reports page.",
            "judges": ["code-quality"],
            "public": false,
            "repo_url": "git@github.com:bob/reports.git",
        })),
    )
    .await;
    let id = project["id"].as_str().unwrap();
    let (status, session) = call(
        &w.app,
        Method::POST,
        "/api/sessions",
        &w.bob,
        Some(serde_json::json!({ "name": "CSV export", "project_id": id })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{session}");
    let code = session["join_code"].as_str().unwrap();

    // Carol cannot read the private project...
    let (status, _) = call(
        &w.app,
        Method::GET,
        &format!("/api/projects/{id}"),
        &w.carol,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    // ...but joining with the code tells her what she needs to set up:
    // the repository to clone, and that it is someone's own work to ask
    // about before uploading.
    for _ in 0..2 {
        let (status, joined) = call(
            &w.app,
            Method::POST,
            "/api/sessions/join",
            &w.carol,
            Some(serde_json::json!({ "code": code })),
        )
        .await;
        assert!(status.is_success(), "{joined}");
        assert_eq!(joined["project"]["id"], project["id"], "{joined}");
        assert_eq!(joined["project"]["kind"], "personal");
        assert_eq!(joined["project"]["public"], false);
        assert_eq!(
            joined["project"]["repo_url"],
            "git@github.com:bob/reports.git"
        );
    }
}

#[tokio::test]
async fn an_export_carries_the_repository_and_an_import_restores_it() {
    let w = world().await;
    let (_, project) = call(
        &w.app,
        Method::POST,
        "/api/projects",
        &w.admin,
        Some(serde_json::json!({ "name": "Fix the starter", "repo_url": STARTER, "repo_ref": "v1.0" })),
    )
    .await;
    let id = project["id"].as_str().unwrap();
    let (status, envelope) = call(
        &w.app,
        Method::GET,
        &format!("/api/admin/projects/{id}/export"),
        &w.admin,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{envelope}");
    assert_eq!(
        envelope["project"]["repo"],
        serde_json::json!({ "url": STARTER, "ref": "v1.0" })
    );

    let mut copy = envelope.clone();
    copy["project"]["name"] = serde_json::json!("Fix the starter again");
    copy["project"]["slug"] = serde_json::Value::Null;
    let (status, imported) = call(
        &w.app,
        Method::POST,
        "/api/admin/projects/import",
        &w.admin,
        Some(copy),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{imported}");
    let new_id = imported["project_id"]
        .as_str()
        .or_else(|| imported["id"].as_str())
        .expect("imported project id");
    let (_, read) = call(
        &w.app,
        Method::GET,
        &format!("/api/projects/{new_id}"),
        &w.admin,
        None,
    )
    .await;
    assert_eq!(read["repo_url"], STARTER, "{read}");
    assert_eq!(read["repo_ref"], "v1.0");

    // A repository-less export carries no key at all, so it still imports
    // into servers older than the field.
    let (_, plain) = call(
        &w.app,
        Method::POST,
        "/api/projects",
        &w.admin,
        Some(serde_json::json!({ "name": "Plain" })),
    )
    .await;
    let (_, envelope) = call(
        &w.app,
        Method::GET,
        &format!(
            "/api/admin/projects/{}/export",
            plain["id"].as_str().unwrap()
        ),
        &w.admin,
        None,
    )
    .await;
    assert!(envelope["project"].get("repo").is_none(), "{envelope}");

    // And an import naming a repository nobody should clone is refused.
    let mut bad = envelope.clone();
    bad["project"]["repo"] = serde_json::json!({ "url": "file:///etc" });
    let (status, err) = call(
        &w.app,
        Method::POST,
        "/api/admin/projects/import",
        &w.admin,
        Some(bad),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{err}");
}
