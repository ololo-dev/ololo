//! Linked sessions: `POST/DELETE /api/sessions/:id/links` and the public
//! `GET /api/sessions/by-code/:code/links` — who may link what, how groups
//! merge and dissolve, and the combined standings across the group.

use arena_core::session_status::SessionStatus;
use axum::http::{Method, StatusCode};
use chrono::Utc;
use sea_orm::{ActiveModelTrait, Set};
use server::entities::{players, task_results, tasks, users};
use server::{AppState, build_router};
use tower::ServiceExt;
use uuid::Uuid;

use crate::common::*;

async fn create_project(app: &axum::Router, cookie: &str) -> String {
    create_project_named(app, cookie, "Linked rounds").await
}

async fn create_project_named(app: &axum::Router, cookie: &str, name: &str) -> String {
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
    let (status, _, body) = read_body(resp).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    body["id"].as_str().expect("project id").to_string()
}

/// Returns `(id, join_code)`.
async fn create_session(
    app: &axum::Router,
    cookie: &str,
    project_id: &str,
    name: &str,
) -> (Uuid, String) {
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
    let (status, _, body) = read_body(resp).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    (
        Uuid::parse_str(body["id"].as_str().expect("id")).expect("uuid"),
        body["join_code"].as_str().expect("join code").to_string(),
    )
}

async fn link(
    app: &axum::Router,
    cookie: &str,
    id: Uuid,
    code: &str,
) -> (StatusCode, serde_json::Value) {
    let resp = app
        .clone()
        .oneshot(req(
            Method::POST,
            &format!("/api/sessions/{id}/links"),
            Some(cookie),
            Some(serde_json::json!({ "join_code": code })),
        ))
        .await
        .expect("link");
    let (status, _, body) = read_body(resp).await;
    (status, body)
}

async fn links_of(app: &axum::Router, code: &str) -> (StatusCode, serde_json::Value) {
    let resp = app
        .clone()
        .oneshot(req(
            Method::GET,
            &format!("/api/sessions/by-code/{code}/links"),
            None,
            None,
        ))
        .await
        .expect("links by code");
    let (status, _, body) = read_body(resp).await;
    (status, body)
}

fn codes(body: &serde_json::Value) -> Vec<String> {
    body["sessions"]
        .as_array()
        .expect("sessions array")
        .iter()
        .map(|s| s["join_code"].as_str().expect("code").to_string())
        .collect()
}

#[tokio::test]
async fn host_links_two_sessions_and_both_pages_see_the_group() {
    let state = test_state().await;
    let app = build_router(state.clone());
    let (_, cookie) = register_and_login_default(app.clone(), "host@example.com").await;
    let project = create_project(&app, &cookie).await;
    let (a, code_a) = create_session(&app, &cookie, &project, "Round 1").await;
    let (_b, code_b) = create_session(&app, &cookie, &project, "Round 2").await;

    // Linked to nothing: 204 on either.
    let (status, _) = links_of(&app, &code_a).await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (status, body) = link(&app, &cookie, a, &code_b).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(codes(&body), vec![code_a.clone(), code_b.clone()]);
    assert_eq!(body["sessions"][0]["current"], true);
    assert_eq!(body["sessions"][1]["current"], false);
    assert_eq!(body["sessions"][1]["name"], "Round 2");
    assert_eq!(body["sessions"][1]["project_name"], "Linked rounds");
    assert_eq!(body["all_finished"], false);

    // The other session's page reads the same group, with itself current.
    let (status, from_b) = links_of(&app, &code_b.to_lowercase()).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(from_b["group_id"], body["group_id"]);
    assert_eq!(codes(&from_b), vec![code_a.clone(), code_b.clone()]);
    assert_eq!(from_b["sessions"][1]["current"], true);

    // Linking the same pair again changes nothing.
    let (status, again) = link(&app, &cookie, a, &code_b).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(again["group_id"], body["group_id"]);
    assert_eq!(codes(&again).len(), 2);
}

#[tokio::test]
async fn unlinking_one_of_two_dissolves_the_group() {
    let state = test_state().await;
    let app = build_router(state.clone());
    let (_, cookie) = register_and_login_default(app.clone(), "host@example.com").await;
    let project = create_project(&app, &cookie).await;
    let (a, code_a) = create_session(&app, &cookie, &project, "Round 1").await;
    let (_b, code_b) = create_session(&app, &cookie, &project, "Round 2").await;
    let (c, code_c) = create_session(&app, &cookie, &project, "Round 3").await;
    link(&app, &cookie, a, &code_b).await;
    link(&app, &cookie, a, &code_c).await;
    assert_eq!(codes(&links_of(&app, &code_a).await.1).len(), 3);

    let resp = app
        .clone()
        .oneshot(req(
            Method::DELETE,
            &format!("/api/sessions/{c}/links"),
            Some(&cookie),
            None,
        ))
        .await
        .expect("unlink c");
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);
    assert_eq!(links_of(&app, &code_c).await.0, StatusCode::NO_CONTENT);
    assert_eq!(
        codes(&links_of(&app, &code_a).await.1),
        vec![code_a.clone(), code_b.clone()]
    );

    // Taking A out leaves B alone — and a session alone is in no group.
    let resp = app
        .clone()
        .oneshot(req(
            Method::DELETE,
            &format!("/api/sessions/{a}/links"),
            Some(&cookie),
            None,
        ))
        .await
        .expect("unlink a");
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);
    assert_eq!(links_of(&app, &code_a).await.0, StatusCode::NO_CONTENT);
    assert_eq!(links_of(&app, &code_b).await.0, StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn linking_two_groups_merges_them() {
    let state = test_state().await;
    let app = build_router(state.clone());
    let (_, cookie) = register_and_login_default(app.clone(), "host@example.com").await;
    let project = create_project(&app, &cookie).await;
    let (a, code_a) = create_session(&app, &cookie, &project, "A").await;
    let (b, code_b) = create_session(&app, &cookie, &project, "B").await;
    let (_c, code_c) = create_session(&app, &cookie, &project, "C").await;
    let (_d, code_d) = create_session(&app, &cookie, &project, "D").await;
    link(&app, &cookie, a, &code_b).await;
    let (_, cd) = link(&app, &cookie, _c, &code_d).await;
    let (status, merged) = link(&app, &cookie, b, &code_c).await;
    assert_eq!(status, StatusCode::OK, "{merged}");
    let mut all = codes(&merged);
    all.sort();
    let mut want = vec![code_a, code_b, code_c.clone(), code_d];
    want.sort();
    assert_eq!(all, want);
    // D now answers with the merged group, not its old one.
    let (_, from_d) = links_of(&app, &want[3]).await;
    assert_eq!(from_d["group_id"], merged["group_id"]);
    assert_ne!(from_d["group_id"], cd["group_id"]);
}

#[tokio::test]
async fn link_rejects_self_bad_codes_and_strangers() {
    let state = test_state().await;
    let app = build_router(state.clone());
    let (_, host) = register_and_login_default(app.clone(), "host@example.com").await;
    let (_, other) = register_and_login_default(app.clone(), "other@example.com").await;
    let project = create_project(&app, &host).await;
    let (a, code_a) = create_session(&app, &host, &project, "A").await;
    let (_b, code_b) = create_session(&app, &host, &project, "B").await;

    let (status, body) = link(&app, &host, a, &code_a).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["error"], "link_to_self");

    let (status, body) = link(&app, &host, a, "nope").await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");

    let (status, _) = link(&app, &host, a, "ZZZZZZ").await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // A stranger cannot link somebody else's session (404: not theirs to see).
    let (status, _) = link(&app, &other, a, &code_b).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // Nor unlink it.
    let resp = app
        .clone()
        .oneshot(req(
            Method::DELETE,
            &format!("/api/sessions/{a}/links"),
            Some(&other),
            None,
        ))
        .await
        .expect("unlink as stranger");
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);

    // Anonymous: 401.
    let resp = app
        .clone()
        .oneshot(req(
            Method::POST,
            &format!("/api/sessions/{a}/links"),
            None,
            Some(serde_json::json!({ "join_code": code_b })),
        ))
        .await
        .expect("link anonymously");
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(links_of(&app, &code_a).await.0, StatusCode::NO_CONTENT);
}

/// A group is one brief in several rooms: sessions of different projects
/// never link, neither by the link call nor at creation.
#[tokio::test]
async fn sessions_of_different_projects_do_not_link() {
    let state = test_state().await;
    let app = build_router(state.clone());
    let (_, host) = register_and_login_default(app.clone(), "host@example.com").await;
    let fizz = create_project_named(&app, &host, "FizzBuzz").await;
    let weather = create_project_named(&app, &host, "Weather Widget").await;
    let (a, code_a) = create_session(&app, &host, &fizz, "Fizz room").await;
    let (_w, code_w) = create_session(&app, &host, &weather, "Weather room").await;

    let (status, body) = link(&app, &host, a, &code_w).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["error"], "link_across_projects");
    assert_eq!(links_of(&app, &code_a).await.0, StatusCode::NO_CONTENT);
    assert_eq!(links_of(&app, &code_w).await.0, StatusCode::NO_CONTENT);

    let resp = app
        .clone()
        .oneshot(req(
            Method::POST,
            "/api/sessions",
            Some(&host),
            Some(serde_json::json!({
                "name": "Weather room 2", "project_id": weather, "host": true, "link_with": code_a
            })),
        ))
        .await
        .expect("create linked across projects");
    let (status, _, body) = read_body(resp).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["error"], "link_across_projects");
    assert_eq!(links_of(&app, &code_a).await.0, StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn host_cannot_link_a_session_they_do_not_host_but_an_admin_can() {
    let state = test_state().await;
    let app = build_router(state.clone());
    // First registrant is the admin; the second a plain host.
    let (_, admin) = register_and_login_default(app.clone(), "admin@example.com").await;
    let (host_id, host) = register_and_login_default(app.clone(), "host@example.com").await;
    let project = create_project(&app, &admin).await;
    give_project(&state, &project, host_id).await;
    let (mine, _code_mine) = create_session(&app, &host, &project, "Mine").await;
    let (_theirs, code_theirs) = create_session(&app, &admin, &project, "Theirs").await;

    let (status, body) = link(&app, &host, mine, &code_theirs).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");

    let (status, body) = link(&app, &admin, mine, &code_theirs).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(codes(&body).len(), 2);
}

async fn insert_user(state: &AppState, name: &str) -> Uuid {
    let id = Uuid::new_v4();
    users::ActiveModel {
        id: Set(id),
        email: Set(format!("{name}-{id}@example.com")),
        password_hash: Set(None),
        display_name: Set(name.to_string()),
        created_at: Set(Utc::now()),
        updated_at: Set(Utc::now()),
        is_admin: Set(false),
        avatar_url: Set(Some(format!("https://avatars.example/{name}.png"))),
        email_verified: Set(false),
        username: Set(Some(name.to_lowercase())),
        plan: Set(arena_core::quota::PLAN_PREMIUM.to_string()),
        judge_run_limit: Set(None),
        judge_run_credits: Set(0),
    }
    .insert(&state.db)
    .await
    .expect("insert user");
    id
}

async fn insert_player(state: &AppState, session_id: Uuid, user: Option<Uuid>, name: &str) -> Uuid {
    let id = Uuid::new_v4();
    players::ActiveModel {
        id: Set(id),
        session_id_fk: Set(session_id),
        user_id_fk: Set(user),
        display_name: Set(name.to_string()),
        fingerprint: Set(None),
        metadata_json: Set(None),
        joined_at: Set(Utc::now()),
        reconnected_at: Set(None),
        revoked_at: Set(None),
        agent_connected: Set(false),
        agent_last_seen_at: Set(None),
    }
    .insert(&state.db)
    .await
    .expect("insert player");
    id
}

async fn insert_task(state: &AppState, project_id: Uuid) -> Uuid {
    let id = Uuid::new_v4();
    tasks::ActiveModel {
        id: Set(id),
        project_id_fk: Set(project_id),
        ordinal: Set(1),
        title: Set("Task".to_string()),
        content: Set(String::new()),
        test_template: Set(serde_json::json!({ "kind": "shell", "command_template": "echo ok" })),
        created_at: Set(Utc::now()),
        tags: Set(String::new()),
        point_value: Set(10),
        deadline_secs: Set(Some(300)),
        min_interval_secs: Set(Some(5)),
        interval_increment_secs: Set(Some(0)),
        max_interval_secs: Set(Some(300)),
        fail_points: Set(0),
        no_response_points: Set(0),
        ..Default::default()
    }
    .insert(&state.db)
    .await
    .expect("insert task");
    id
}

async fn score(state: &AppState, session_id: Uuid, player_id: Uuid, task_id: Uuid, points: i32) {
    task_results::ActiveModel {
        id: Set(Uuid::new_v4()),
        session_id_fk: Set(session_id),
        player_id_fk: Set(player_id),
        task_id: Set(Some(task_id)),
        answer: Set("ok".to_string()),
        created_at: Set(Utc::now()),
        point_delta: Set(points),
        kind: Set(task_results::KIND_PROBE.to_string()),
        is_bonus: Set(false),
    }
    .insert(&state.db)
    .await
    .expect("insert result");
}

async fn finish(state: &AppState, session_id: Uuid) {
    use sea_orm::EntityTrait;
    let row = server::entities::sessions::Entity::find_by_id(session_id)
        .one(&state.db)
        .await
        .expect("find")
        .expect("session");
    let mut am: server::entities::sessions::ActiveModel = row.into();
    am.status = Set(SessionStatus::Finished);
    am.finished_at = Set(Some(Utc::now()));
    am.update(&state.db).await.expect("finish session");
}

#[tokio::test]
async fn combined_standings_add_up_by_account_across_the_group() {
    let state = test_state().await;
    let app = build_router(state.clone());
    let (_, cookie) = register_and_login_default(app.clone(), "host@example.com").await;
    let project = create_project(&app, &cookie).await;
    let project_id = Uuid::parse_str(&project).expect("project uuid");
    let task = insert_task(&state, project_id).await;
    let (a, code_a) = create_session(&app, &cookie, &project, "Round 1").await;
    let (b, code_b) = create_session(&app, &cookie, &project, "Round 2").await;
    link(&app, &cookie, a, &code_b).await;

    let ada = insert_user(&state, "Ada").await;
    let bob = insert_user(&state, "Bob").await;
    // Ada plays both rounds, Bob the first only, a guest the second only.
    let ada_a = insert_player(&state, a, Some(ada), "Ada").await;
    let bob_a = insert_player(&state, a, Some(bob), "Bob").await;
    let ada_b = insert_player(&state, b, Some(ada), "Ada (round 2)").await;
    let guest_b = insert_player(&state, b, None, "Guest").await;
    score(&state, a, ada_a, task, 30).await;
    score(&state, a, bob_a, task, 50).await;
    score(&state, b, ada_b, task, 40).await;
    score(&state, b, guest_b, task, 60).await;

    // Still running: the standings are there, but not final.
    let (status, body) = links_of(&app, &code_a).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["all_finished"], false);
    let lb = body["leaderboard"].as_array().expect("leaderboard").clone();
    assert_eq!(lb.len(), 3, "{body}");
    // Ada: 30 + 40 = 70 over two rounds, on top.
    assert_eq!(lb[0]["display_name"], "Ada");
    assert_eq!(lb[0]["total_points"], 70);
    assert_eq!(lb[0]["sessions_played"], 2);
    assert_eq!(lb[0]["user_id"], ada.to_string());
    assert_eq!(lb[0]["username"], "ada");
    assert_eq!(lb[0]["per_session"][0]["session_id"], a.to_string());
    assert_eq!(lb[0]["per_session"][0]["points"], 30);
    assert_eq!(lb[0]["per_session"][0]["rank"], 2);
    assert_eq!(lb[0]["per_session"][1]["session_id"], b.to_string());
    assert_eq!(lb[0]["per_session"][1]["points"], 40);
    assert_eq!(lb[0]["per_session"][1]["rank"], 2);
    // The guest (60, one round) above Bob (50, one round); the guest has no account.
    assert_eq!(lb[1]["display_name"], "Guest");
    assert_eq!(lb[1]["total_points"], 60);
    assert!(lb[1]["user_id"].is_null());
    assert_eq!(lb[2]["display_name"], "Bob");
    assert_eq!(lb[2]["total_points"], 50);
    assert_eq!(lb[2]["sessions_played"], 1);
    assert_eq!(body["sessions"][0]["player_count"], 2);

    finish(&state, a).await;
    assert_eq!(links_of(&app, &code_a).await.1["all_finished"], false);
    finish(&state, b).await;
    let (_, done) = links_of(&app, &code_a).await;
    assert_eq!(done["all_finished"], true);
    assert_eq!(done["sessions"][0]["status"], "finished");
}

#[tokio::test]
async fn a_session_created_with_link_with_joins_that_group() {
    let state = test_state().await;
    let app = build_router(state.clone());
    let (_, cookie) = register_and_login_default(app.clone(), "host@example.com").await;
    let project = create_project(&app, &cookie).await;
    let (_a, code_a) = create_session(&app, &cookie, &project, "Room 1").await;

    let resp = app
        .clone()
        .oneshot(req(
            Method::POST,
            "/api/sessions",
            Some(&cookie),
            Some(serde_json::json!({
                "name": "Room 2", "project_id": project, "host": true, "link_with": code_a.to_lowercase()
            })),
        ))
        .await
        .expect("create linked");
    let (status, _, body) = read_body(resp).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let code_b = body["join_code"].as_str().expect("code").to_string();
    let (status, group) = links_of(&app, &code_b).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(codes(&group), vec![code_a.clone(), code_b.clone()]);

    // A code that names nothing is refused, and no orphan lobby is left.
    let resp = app
        .clone()
        .oneshot(req(
            Method::POST,
            "/api/sessions",
            Some(&cookie),
            Some(serde_json::json!({
                "name": "Room 3", "project_id": project, "host": true, "link_with": "ZZZZZZ"
            })),
        ))
        .await
        .expect("create with a bad link");
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    let (_, group) = links_of(&app, &code_a).await;
    assert_eq!(codes(&group).len(), 2);
}
