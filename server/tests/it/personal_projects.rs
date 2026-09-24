//! Personal projects: a user's own work turned into a playable project —
//! public unless its owner keeps it private — through
//! `/api/personal-projects`, and the rules every other surface keeps for
//! them.

use arena_core::entities::{task_judges, tasks};
use axum::http::{Method, StatusCode};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder};
use server::build_router;
use tower::ServiceExt;

use crate::common::*;

fn judges_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root")
        .join("judges")
}

struct World {
    state: server::AppState,
    app: axum::Router,
    admin: String,
    bob: String,
    bob_id: uuid::Uuid,
    carol: String,
    carol_id: uuid::Uuid,
}

/// An instance with the shipped judges, an admin (the first account),
/// and two ordinary users.
async fn world() -> World {
    let state = test_state().await;
    server::seed::judges::seed_judges_with_dir(&state.db, &judges_dir())
        .await
        .expect("seed judges");
    let app = build_router(state.clone());
    let (_, admin) = register_and_login_default(app.clone(), "admin@x.test").await;
    let (bob_id, bob) = register_and_login_default(app.clone(), "bob@x.test").await;
    let (carol_id, carol) = register_and_login_default(app.clone(), "carol@x.test").await;
    set_project_creation_setting(&app, &admin, "true").await;
    World {
        state,
        app,
        admin,
        bob,
        bob_id,
        carol,
        carol_id,
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

/// Judges both editions ship (the open platform has no `correctness`), so
/// the suite reads the same in app and platform.
const PANEL: [&str; 2] = ["code-quality", "test-quality"];

fn csv_request() -> serde_json::Value {
    serde_json::json!({
        "description": "Add a CSV export to the reports page, with the same filters as the table.",
        "tasks": [
            { "title": "Serve the report as CSV", "description": "GET /reports.csv honours every table filter." },
            { "title": "Add the export button" }
        ],
        "judges": PANEL
    })
}

/// [`csv_request`], kept private.
fn private_csv_request() -> serde_json::Value {
    let mut body = csv_request();
    body["public"] = serde_json::json!(false);
    body
}

async fn anonymous(app: &axum::Router, uri: &str) -> (StatusCode, serde_json::Value) {
    let resp = app
        .clone()
        .oneshot(req(Method::GET, uri, None, None))
        .await
        .expect("response");
    read_body_json(resp).await
}

async fn username(app: &axum::Router, cookie: &str) -> String {
    let (_, me) = call(app, Method::GET, "/api/users/me", cookie, None).await;
    me["username"].as_str().expect("a username").to_string()
}

async fn set_setting(w: &World, key: &str, value: &str) {
    let (status, body) = call(
        &w.app,
        Method::PUT,
        "/api/admin/settings",
        &w.admin,
        Some(serde_json::json!({ "key": key, "value": value })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "set {key}={value}: {body}");
}

async fn set_plan(w: &World, user_id: uuid::Uuid, plan: &str) {
    use sea_orm::{ActiveModelTrait, Set};
    let user = arena_core::entities::users::Entity::find_by_id(user_id)
        .one(&w.state.db)
        .await
        .unwrap()
        .expect("user");
    let mut am: arena_core::entities::users::ActiveModel = user.into();
    am.plan = Set(plan.to_string());
    am.update(&w.state.db).await.expect("set plan");
}

/// Whether `id` is among the projects of a `{ projects: [...] }` body.
fn lists(body: &serde_json::Value, id: &serde_json::Value) -> bool {
    body["projects"]
        .as_array()
        .is_some_and(|all| all.iter().any(|p| &p["id"] == id))
}

/// A session of `project_id`, started and joined by its owner.
async fn play(w: &World, cookie: &str, project_id: &str) -> String {
    let (status, session) = call(
        &w.app,
        Method::POST,
        "/api/sessions",
        cookie,
        Some(serde_json::json!({ "name": "CSV export", "project_id": project_id })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{session}");
    let code = session["join_code"].as_str().unwrap().to_string();
    let (status, joined) = call(
        &w.app,
        Method::POST,
        "/api/sessions/join",
        cookie,
        Some(serde_json::json!({ "code": code })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{joined}");
    code
}

#[tokio::test]
async fn a_description_and_a_map_become_a_playable_project() {
    let w = world().await;
    let (status, project) = call(
        &w.app,
        Method::POST,
        "/api/personal-projects",
        &w.bob,
        Some(csv_request()),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{project}");
    assert_eq!(project["kind"], "personal");
    assert_eq!(project["public"], true, "public unless kept private");
    assert_eq!(project["owner_user_id"], w.bob_id.to_string());
    assert_eq!(project["task_count"], 2);
    assert_eq!(project["idle_timeout_secs"], 1800);
    assert_eq!(project["session_duration_secs"], 7200);
    let slug = project["slug"].as_str().expect("a slug to start it by");
    assert!(slug.starts_with("add-a-csv-export"), "{slug}");
    // Two judges on each of two tasks, plus the session report.
    assert_eq!(project["judge_review_count"], 5);

    let project_id: uuid::Uuid = project["id"].as_str().unwrap().parse().unwrap();
    let rows = tasks::Entity::find()
        .filter(tasks::Column::ProjectIdFk.eq(project_id))
        .order_by_asc(tasks::Column::Ordinal)
        .all(&w.state.db)
        .await
        .unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].title, "Serve the report as CSV");
    assert!(
        rows[0]
            .content
            .starts_with("GET /reports.csv honours every table filter.")
    );
    assert!(rows[1].content.contains("task 2 of 2"));
    assert!(rows[1].content.contains(".ololo/task-2-done.md"));
    let contract = rows[0].evaluation.as_ref().expect("open-ended");
    assert_eq!(contract["kind"], "open_ended");
    assert_eq!(contract["completion"]["probe"], "Definition of done");
    assert_eq!(rows[0].point_value, 100);
    assert_eq!(rows[0].health_points, 20);

    let panel = task_judges::Entity::find()
        .filter(task_judges::Column::TaskId.eq(rows[1].id))
        .order_by_asc(task_judges::Column::Ordinal)
        .all(&w.state.db)
        .await
        .unwrap();
    assert_eq!(panel.len(), 2, "the second task carries only its panel");
    assert!(panel.iter().all(|tj| tj.weight == Some(1.0)), "{panel:?}");

    // The owner finds it by slug — what `ololo start <slug>` does.
    let (status, by_slug) = call(
        &w.app,
        Method::GET,
        &format!("/api/projects/by-slug/{slug}"),
        &w.bob,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{by_slug}");
    assert_eq!(by_slug["id"], project["id"]);
    assert_eq!(by_slug["kind"], "personal");

    // What they asked for stays theirs.
    let (status, _) = call(
        &w.app,
        Method::GET,
        &format!("/api/personal-projects/{project_id}"),
        &w.carol,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // The owner reads back what they asked for.
    let (status, full) = call(
        &w.app,
        Method::GET,
        &format!("/api/personal-projects/{project_id}"),
        &w.bob,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{full}");
    assert_eq!(full["editable"], true);
    assert_eq!(full["spec"]["tasks"][1]["title"], "Add the export button");
    assert_eq!(full["spec"]["judges"], serde_json::json!(PANEL));

    // The catalog list marks it.
    let (status, list) = call(&w.app, Method::GET, "/api/projects", &w.bob, None).await;
    assert_eq!(status, StatusCode::OK);
    let mine = list["projects"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["id"] == project["id"])
        .expect("listed for its owner");
    assert_eq!(mine["kind"], "personal");
}

#[tokio::test]
async fn an_empty_map_makes_one_task_for_the_whole_project() {
    let w = world().await;
    let (status, project) = call(
        &w.app,
        Method::POST,
        "/api/personal-projects",
        &w.bob,
        Some(serde_json::json!({
            "description": "# Fix the flaky login test\nIt fails one run in five on CI.",
            "judges": ["code-quality"],
            "session_duration_secs": 3600
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{project}");
    assert_eq!(project["name"], "Fix the flaky login test");
    assert_eq!(project["task_count"], 1);
    // One judge plus the report.
    assert_eq!(project["judge_review_count"], 2);
    let project_id: uuid::Uuid = project["id"].as_str().unwrap().parse().unwrap();
    let task = tasks::Entity::find()
        .filter(tasks::Column::ProjectIdFk.eq(project_id))
        .one(&w.state.db)
        .await
        .unwrap()
        .expect("one task");
    assert_eq!(task.title, "Fix the flaky login test");
    assert!(task.content.contains(".ololo/task-done.md"));
    let contract = task.evaluation.expect("open-ended");
    assert_eq!(contract["completion"]["deadline_secs"], 3600);
}

#[tokio::test]
async fn a_bad_request_names_the_field() {
    let w = world().await;
    let too_many: Vec<serde_json::Value> = (0..11)
        .map(|i| serde_json::json!({ "title": format!("Step {i}") }))
        .collect();
    for (body, field) in [
        (serde_json::json!({ "description": "   " }), "description"),
        (
            serde_json::json!({ "description": "d", "tasks": too_many }),
            "tasks",
        ),
        (
            serde_json::json!({ "description": "d", "tasks": [{ "title": " " }] }),
            "tasks",
        ),
        (
            serde_json::json!({ "description": "d", "judges": ["task-anti-cheat"] }),
            "judges",
        ),
        (
            serde_json::json!({ "description": "d", "judges": [] }),
            "judges",
        ),
        (
            serde_json::json!({ "description": "d", "session_duration_secs": 600 }),
            "session_duration_secs",
        ),
    ] {
        let (status, err) = call(
            &w.app,
            Method::POST,
            "/api/personal-projects",
            &w.bob,
            Some(body.clone()),
        )
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body} → {err}");
        assert_eq!(err["error"], "invalid_personal_project", "{err}");
        assert_eq!(err["field"], field, "{body} → {err}");
    }
    let (status, _) = call(
        &w.app,
        Method::POST,
        "/api/personal-projects",
        &w.bob,
        Some(serde_json::json!({ "description": "d", "surprise": 1 })),
    )
    .await;
    assert!(status.is_client_error(), "unknown fields are refused");
}

#[tokio::test]
async fn the_creation_switch_gates_everyone_but_admins() {
    let w = world().await;
    set_project_creation_setting(&w.app, &w.admin, "false").await;
    let (status, err) = call(
        &w.app,
        Method::POST,
        "/api/personal-projects",
        &w.bob,
        Some(csv_request()),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{err}");
    let (status, options) = call(
        &w.app,
        Method::GET,
        "/api/personal-projects/options",
        &w.bob,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(options["creation_allowed"], false);

    let (status, _) = call(
        &w.app,
        Method::POST,
        "/api/personal-projects",
        &w.admin,
        Some(csv_request()),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
}

#[tokio::test]
async fn options_offer_the_change_reviewers_only() {
    let w = world().await;
    let (status, options) = call(
        &w.app,
        Method::GET,
        "/api/personal-projects/options",
        &w.bob,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{options}");
    assert_eq!(options["creation_allowed"], true);
    let slugs: Vec<&str> = options["judges"]
        .as_array()
        .unwrap()
        .iter()
        .map(|j| j["slug"].as_str().unwrap())
        .collect();
    // Whatever this edition ships of the change reviewers, in the form's
    // order — never a report, execution, penalty or session-wide judge.
    let shipped: Vec<String> = std::fs::read_dir(judges_dir())
        .unwrap()
        .filter_map(|e| {
            let name = e.ok()?.file_name().into_string().ok()?;
            name.strip_suffix(".md").map(str::to_string)
        })
        .collect();
    let offered = [
        "correctness",
        "code-quality",
        "test-quality",
        "architecture",
        "data",
        "performance",
        "ux-review",
        "build-review",
    ];
    let expected: Vec<&str> = offered
        .iter()
        .copied()
        .filter(|slug| shipped.iter().any(|s| s == slug))
        .collect();
    assert_eq!(slugs, expected);
    for never in [
        "general",
        "golf-verify",
        "task-anti-cheat",
        "from-scratch",
        "agentic",
    ] {
        assert!(!slugs.contains(&never), "{never} is not a personal judge");
    }
    let defaults: Vec<&str> = options["judges"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|j| j["default"] == true)
        .map(|j| j["slug"].as_str().unwrap())
        .collect();
    assert_eq!(defaults.len(), 3, "{defaults:?}");
    // The list is in the form's order; what matters is that the default
    // panel has a judge asking whether the change works at all.
    assert!(
        defaults.contains(&"correctness") || defaults.contains(&"build-review"),
        "a panel answers whether it works: {defaults:?}"
    );
    assert_eq!(options["limits"]["max_tasks"], 10);
    assert_eq!(options["session"]["default_secs"], 7200);
}

#[tokio::test]
async fn a_project_is_rebuilt_until_its_first_session_then_frozen() {
    let w = world().await;
    let (_, project) = call(
        &w.app,
        Method::POST,
        "/api/personal-projects",
        &w.bob,
        Some(csv_request()),
    )
    .await;
    let id = project["id"].as_str().unwrap().to_string();
    let slug = project["slug"].as_str().unwrap().to_string();

    let (status, rebuilt) = call(
        &w.app,
        Method::PUT,
        &format!("/api/personal-projects/{id}"),
        &w.bob,
        Some(serde_json::json!({
            "name": "CSV export",
            "description": "Add a CSV export to the reports page.",
            "tasks": [{ "title": "One" }, { "title": "Two" }, { "title": "Three" }]
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{rebuilt}");
    assert_eq!(rebuilt["task_count"], 3);
    assert_eq!(rebuilt["name"], "CSV export");
    assert_eq!(rebuilt["slug"], slug, "the start command keeps working");
    assert_eq!(rebuilt["judge_review_count"], 10);

    let (status, _) = call(
        &w.app,
        Method::PUT,
        &format!("/api/personal-projects/{id}"),
        &w.carol,
        Some(csv_request()),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "only the owner edits");

    let (status, session) = call(
        &w.app,
        Method::POST,
        "/api/sessions",
        &w.bob,
        Some(serde_json::json!({ "name": "CSV export", "project_id": id })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{session}");

    let (status, err) = call(
        &w.app,
        Method::PUT,
        &format!("/api/personal-projects/{id}"),
        &w.bob,
        Some(csv_request()),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{err}");
    assert_eq!(err["error"], "project_frozen");
    let (_, full) = call(
        &w.app,
        Method::GET,
        &format!("/api/personal-projects/{id}"),
        &w.bob,
        None,
    )
    .await;
    assert_eq!(full["editable"], false);
    assert_eq!(full["session_count"], 1);
}

#[tokio::test]
async fn the_generic_editors_leave_personal_projects_alone() {
    let w = world().await;
    let (_, project) = call(
        &w.app,
        Method::POST,
        "/api/personal-projects",
        &w.bob,
        Some(csv_request()),
    )
    .await;
    let id = project["id"].as_str().unwrap();

    let (status, err) = call(
        &w.app,
        Method::PATCH,
        &format!("/api/projects/{id}"),
        &w.bob,
        Some(serde_json::json!({ "name": "Renamed", "public": false })),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{err}");
    assert_eq!(err["error"], "personal_project");

    // Who sees it may move — tasks or not, played or not.
    for public in [false, true] {
        let (status, patched) = call(
            &w.app,
            Method::PATCH,
            &format!("/api/projects/{id}"),
            &w.bob,
            Some(serde_json::json!({ "public": public })),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{patched}");
        assert_eq!(patched["public"], public);
    }

    let (status, err) = call(
        &w.app,
        Method::POST,
        &format!("/api/projects/{id}/tasks"),
        &w.bob,
        Some(serde_json::json!({
            "title": "Sneak in",
            "test_template": { "kind": "shell", "command_template": "true" }
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{err}");
    assert_eq!(err["error"], "personal_project");

    let (status, archived) = call(
        &w.app,
        Method::PATCH,
        &format!("/api/projects/{id}"),
        &w.bob,
        Some(serde_json::json!({ "archived": true })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "archiving is fine: {archived}");
    assert_eq!(archived["kind"], "personal");
}

#[tokio::test]
async fn the_join_code_lets_players_in_but_only_the_owner_starts_a_session() {
    let w = world().await;
    let (_, private) = call(
        &w.app,
        Method::POST,
        "/api/personal-projects",
        &w.bob,
        Some(private_csv_request()),
    )
    .await;
    let id = private["id"].as_str().unwrap();
    let (status, session) = call(
        &w.app,
        Method::POST,
        "/api/sessions",
        &w.bob,
        Some(serde_json::json!({ "name": "CSV export", "project_id": id })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{session}");
    let code = session["join_code"]
        .as_str()
        .expect("join code")
        .to_string();

    // The code is the invitation, private project or not: whoever has it
    // plays, in their own copy of the repository.
    let (status, joined) = call(
        &w.app,
        Method::POST,
        "/api/sessions/join",
        &w.carol,
        Some(serde_json::json!({ "code": code })),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "a player with the code joins: {joined}"
    );
    let (status, joined) = call(
        &w.app,
        Method::POST,
        "/api/sessions/join",
        &w.bob,
        Some(serde_json::json!({ "code": code })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "the owner joins: {joined}");

    // Its owner can also add a player by hand.
    let session_id = session["id"].as_str().unwrap();
    let (status, added) = call(
        &w.app,
        Method::POST,
        &format!("/api/sessions/{session_id}/members"),
        &w.bob,
        Some(serde_json::json!({ "user_id": w.bob_id, "role": "participant" })),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "already in: {added}");
    let (status, members) = call(
        &w.app,
        Method::GET,
        &format!("/api/sessions/{session_id}/members"),
        &w.bob,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{members}");
    let players: Vec<String> = members["members"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["user_id"].as_str().unwrap().to_string())
        .collect();
    assert!(players.contains(&w.carol_id.to_string()), "{members}");

    // But a session of it is its owner's to start — private, or public.
    let (status, _) = call(
        &w.app,
        Method::POST,
        "/api/sessions",
        &w.carol,
        Some(serde_json::json!({ "name": "x", "project_id": id })),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (_, public) = call(
        &w.app,
        Method::POST,
        "/api/personal-projects",
        &w.bob,
        Some(csv_request()),
    )
    .await;
    let (status, err) = call(
        &w.app,
        Method::POST,
        "/api/sessions",
        &w.carol,
        Some(serde_json::json!({ "name": "x", "project_id": public["id"] })),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{err}");
    assert_eq!(err["error"], "personal_project");
}

#[tokio::test]
async fn a_personal_slug_never_shadows_a_catalog_one() {
    let w = world().await;
    // The admin publishes a catalog project under the slug a personal
    // project would take.
    let (status, catalog) = call(
        &w.app,
        Method::POST,
        "/api/projects",
        &w.admin,
        Some(serde_json::json!({ "name": "CSV export" })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{catalog}");
    let catalog_id = catalog["id"].as_str().unwrap();
    let (status, patched) = call(
        &w.app,
        Method::PATCH,
        &format!("/api/projects/{catalog_id}"),
        &w.admin,
        Some(serde_json::json!({ "slug": "csv-export", "public": true })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{patched}");

    let (status, personal) = call(
        &w.app,
        Method::POST,
        "/api/personal-projects",
        &w.bob,
        Some(serde_json::json!({ "name": "CSV export", "description": "My own take." })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{personal}");
    assert_eq!(personal["slug"], "csv-export-2");

    let (_, found) = call(
        &w.app,
        Method::GET,
        "/api/projects/by-slug/csv-export",
        &w.bob,
        None,
    )
    .await;
    assert_eq!(
        found["id"], catalog["id"],
        "the catalog project stays reachable"
    );
}

#[tokio::test]
async fn a_private_personal_session_stays_off_the_public_profile() {
    let w = world().await;
    let (_, project) = call(
        &w.app,
        Method::POST,
        "/api/personal-projects",
        &w.bob,
        Some(private_csv_request()),
    )
    .await;
    let id = project["id"].as_str().unwrap();
    let (_, session) = call(
        &w.app,
        Method::POST,
        "/api/sessions",
        &w.bob,
        Some(serde_json::json!({ "name": "CSV export", "project_id": id })),
    )
    .await;
    let code = session["join_code"].as_str().unwrap();
    let (status, _) = call(
        &w.app,
        Method::POST,
        "/api/sessions/join",
        &w.bob,
        Some(serde_json::json!({ "code": code })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let (_, me) = call(&w.app, Method::GET, "/api/users/me", &w.bob, None).await;
    let username = me["username"].as_str().expect("a username").to_string();
    let uri = format!("/api/users/by-username/{username}/sessions");

    let (status, theirs) = call(&w.app, Method::GET, &uri, &w.carol, None).await;
    assert_eq!(status, StatusCode::OK, "{theirs}");
    assert_eq!(theirs["total"], 0, "{theirs}");
    let (_, theirs) = anonymous(&w.app, &uri).await;
    assert_eq!(theirs["total"], 0, "anonymous: {theirs}");

    let (_, own) = call(&w.app, Method::GET, &uri, &w.bob, None).await;
    assert_eq!(own["total"], 1, "{own}");
    assert_eq!(own["sessions"][0]["personal"], true);
    assert_eq!(own["sessions"][0]["private"], true);

    // Nor does the landing show it while it is live.
    let (_, live) = anonymous(&w.app, "/api/public/active-sessions").await;
    assert_eq!(live["sessions"], serde_json::json!([]), "{live}");
}

#[tokio::test]
async fn a_public_personal_session_is_public_history() {
    let w = world().await;
    let (_, project) = call(
        &w.app,
        Method::POST,
        "/api/personal-projects",
        &w.bob,
        Some(csv_request()),
    )
    .await;
    let code = play(&w, &w.bob, project["id"].as_str().unwrap()).await;

    // The landing lists it like any live session...
    let (status, live) = anonymous(&w.app, "/api/public/active-sessions").await;
    assert_eq!(status, StatusCode::OK, "{live}");
    let codes: Vec<&str> = live["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|s| s["join_code"].as_str())
        .collect();
    assert_eq!(codes, [code.as_str()], "{live}");

    // ...and so does its owner's profile, for anyone.
    let uri = format!(
        "/api/users/by-username/{}/sessions",
        username(&w.app, &w.bob).await
    );
    for (who, body) in [
        (
            "carol",
            call(&w.app, Method::GET, &uri, &w.carol, None).await.1,
        ),
        ("anonymous", anonymous(&w.app, &uri).await.1),
    ] {
        assert_eq!(body["total"], 1, "{who}: {body}");
        assert_eq!(body["sessions"][0]["personal"], true, "{who}: {body}");
        assert_eq!(body["sessions"][0]["private"], false, "{who}: {body}");
    }
}

#[tokio::test]
async fn a_personal_project_is_public_unless_kept_private() {
    let w = world().await;
    let (_, public) = call(
        &w.app,
        Method::POST,
        "/api/personal-projects",
        &w.bob,
        Some(csv_request()),
    )
    .await;
    let (status, private) = call(
        &w.app,
        Method::POST,
        "/api/personal-projects",
        &w.bob,
        Some(private_csv_request()),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "no plans here: {private}");
    assert_eq!(private["public"], false);
    let profile = format!(
        "/api/users/by-username/{}/projects",
        username(&w.app, &w.bob).await
    );

    // The public one opens for anyone, and its owner's profile lists it...
    for id in [&public["id"], &private["id"]] {
        let id = id.as_str().unwrap();
        let (status, _) = anonymous(&w.app, &format!("/api/projects/{id}")).await;
        let expected = if id == public["id"] {
            StatusCode::OK
        } else {
            StatusCode::FORBIDDEN
        };
        assert_eq!(status, expected, "{id}");
    }
    let slug = private["slug"].as_str().unwrap();
    let (status, _) = call(
        &w.app,
        Method::GET,
        &format!("/api/projects/by-slug/{slug}"),
        &w.carol,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "a private one stays hidden");
    for (who, body) in [
        (
            "carol",
            call(&w.app, Method::GET, &profile, &w.carol, None).await.1,
        ),
        ("anonymous", anonymous(&w.app, &profile).await.1),
    ] {
        assert!(lists(&body, &public["id"]), "{who}: {body}");
        assert!(!lists(&body, &private["id"]), "{who}: {body}");
        assert_eq!(body["projects"][0]["kind"], "personal", "{who}: {body}");
    }
    let (_, own) = call(&w.app, Method::GET, &profile, &w.bob, None).await;
    assert!(
        lists(&own, &public["id"]) && lists(&own, &private["id"]),
        "{own}"
    );

    // ...but the catalog lists neither, except to their owner.
    for (who, body) in [
        (
            "carol",
            call(&w.app, Method::GET, "/api/projects", &w.carol, None)
                .await
                .1,
        ),
        ("anonymous", anonymous(&w.app, "/api/projects").await.1),
    ] {
        assert!(!lists(&body, &public["id"]), "{who}: {body}");
        assert!(!lists(&body, &private["id"]), "{who}: {body}");
    }
    let (_, mine) = call(&w.app, Method::GET, "/api/projects", &w.bob, None).await;
    assert!(
        lists(&mine, &public["id"]) && lists(&mine, &private["id"]),
        "{mine}"
    );
}

#[tokio::test]
async fn keeping_a_project_private_takes_premium_where_plans_are_on() {
    let w = world().await;
    set_setting(&w, "plans_enabled", "true").await;
    let options = |cookie: &str| {
        let app = w.app.clone();
        let cookie = cookie.to_string();
        async move {
            call(
                &app,
                Method::GET,
                "/api/personal-projects/options",
                &cookie,
                None,
            )
            .await
            .1
        }
    };
    assert_eq!(options(&w.bob).await["private_allowed"], false);
    assert_eq!(options(&w.admin).await["private_allowed"], true);

    let (status, err) = call(
        &w.app,
        Method::POST,
        "/api/personal-projects",
        &w.bob,
        Some(private_csv_request()),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{err}");
    assert_eq!(err["error"], "premium_required");

    // A free account keeps it public: the default, and no switching.
    let (status, project) = call(
        &w.app,
        Method::POST,
        "/api/personal-projects",
        &w.bob,
        Some(csv_request()),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{project}");
    let id = project["id"].as_str().unwrap().to_string();
    let (status, err) = call(
        &w.app,
        Method::PATCH,
        &format!("/api/projects/{id}"),
        &w.bob,
        Some(serde_json::json!({ "public": false })),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{err}");
    assert_eq!(err["error"], "premium_required");
    let (status, err) = call(
        &w.app,
        Method::PUT,
        &format!("/api/personal-projects/{id}"),
        &w.bob,
        Some(private_csv_request()),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{err}");
    assert_eq!(err["error"], "premium_required");

    // Premium may.
    set_plan(&w, w.bob_id, "premium").await;
    assert_eq!(options(&w.bob).await["private_allowed"], true);
    let (status, patched) = call(
        &w.app,
        Method::PATCH,
        &format!("/api/projects/{id}"),
        &w.bob,
        Some(serde_json::json!({ "public": false })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{patched}");
    assert_eq!(patched["public"], false);

    // When Premium lapses, a private project stays private: publishing it
    // is its owner's call, and keeping it is not making it.
    set_plan(&w, w.bob_id, "free").await;
    let (status, rebuilt) = call(
        &w.app,
        Method::PUT,
        &format!("/api/personal-projects/{id}"),
        &w.bob,
        Some(private_csv_request()),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{rebuilt}");
    assert_eq!(rebuilt["public"], false);
    let (status, rebuilt) = call(
        &w.app,
        Method::PUT,
        &format!("/api/personal-projects/{id}"),
        &w.bob,
        Some(csv_request()),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{rebuilt}");
    assert_eq!(
        rebuilt["public"], false,
        "an absent `public` leaves it as it is"
    );
}

#[tokio::test]
async fn a_task_can_bring_its_own_judges() {
    let w = world().await;
    let (status, project) = call(
        &w.app,
        Method::POST,
        "/api/personal-projects",
        &w.bob,
        Some(serde_json::json!({
            "description": "Add a CSV export to the reports page.",
            "tasks": [
                { "title": "Serve the report as CSV", "judges": ["test-quality"] },
                { "title": "Add the export button" }
            ],
            "judges": PANEL
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{project}");
    // Task 1: its own judge + the session report; task 2: the default two.
    assert_eq!(project["judge_review_count"], 4);

    let project_id: uuid::Uuid = project["id"].as_str().unwrap().parse().unwrap();
    let rows = tasks::Entity::find()
        .filter(tasks::Column::ProjectIdFk.eq(project_id))
        .order_by_asc(tasks::Column::Ordinal)
        .all(&w.state.db)
        .await
        .unwrap();
    let judges_of = |task_id: uuid::Uuid| {
        let db = w.state.db.clone();
        async move {
            let tj = task_judges::Entity::find()
                .filter(task_judges::Column::TaskId.eq(task_id))
                .order_by_asc(task_judges::Column::Ordinal)
                .all(&db)
                .await
                .unwrap();
            let mut slugs = Vec::new();
            for row in tj {
                let judge = arena_core::entities::judges::Entity::find_by_id(row.judge_id)
                    .one(&db)
                    .await
                    .unwrap()
                    .unwrap();
                slugs.push(judge.slug);
            }
            slugs
        }
    };
    let first = judges_of(rows[0].id).await;
    assert_eq!(first[0], "test-quality", "{first:?}");
    assert_eq!(first.len(), 2, "its judge and the report: {first:?}");
    assert_eq!(judges_of(rows[1].id).await, PANEL);
    let sheet = rows[0].evaluation.as_ref().unwrap();
    let keys: Vec<&str> = sheet["criteria"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["key"].as_str().unwrap())
        .collect();
    assert_eq!(keys, ["tests"], "its own judge's criteria only");

    // The owner reads back which task chose its own panel.
    let (_, full) = call(
        &w.app,
        Method::GET,
        &format!("/api/personal-projects/{project_id}"),
        &w.bob,
        None,
    )
    .await;
    assert_eq!(
        full["spec"]["tasks"][0]["judges"],
        serde_json::json!(["test-quality"])
    );
    assert!(full["spec"]["tasks"][1].get("judges").is_none(), "{full}");

    // A task's panel is held to the same rules as the default one.
    let (status, err) = call(
        &w.app,
        Method::POST,
        "/api/personal-projects",
        &w.bob,
        Some(serde_json::json!({
            "description": "d",
            "tasks": [{ "title": "One" }, { "title": "Two", "judges": ["task-anti-cheat"] }]
        })),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{err}");
    assert_eq!(err["field"], "tasks");
    assert!(
        err["detail"].as_str().unwrap().starts_with("task 2:"),
        "{err}"
    );
    let (status, err) = call(
        &w.app,
        Method::POST,
        "/api/personal-projects",
        &w.bob,
        Some(serde_json::json!({
            "description": "d",
            "tasks": [{ "title": "One", "judges": [] }]
        })),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{err}");
    assert_eq!(err["field"], "tasks");
}
