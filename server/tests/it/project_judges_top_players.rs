//! Integration tests for the project-page sub-resources:
//! `GET /api/projects/:id/judges` and `GET /api/projects/:id/top-players`.

use arena_core::entities::{players, projects, sessions, task_results, tasks, users};
use arena_core::session_status::SessionStatus;
use axum::http::{Method, StatusCode};
use chrono::Utc;
use sea_orm::{ActiveModelTrait, Set};
use server::{AppState, build_router};
use tower::ServiceExt;
use uuid::Uuid;

use crate::common;
use crate::common::*;

fn judge_body(slug: &str) -> serde_json::Value {
    serde_json::json!({
        "slug": slug,
        "name": format!("Judge {slug}"),
        "description": format!("desc for {slug}"),
        "prompt": "prompt",
        "rating_scale": { "min": 0.0, "max": 10.0, "step": 1.0 },
    })
}

async fn create_judge(app: &axum::Router, cookie: &str, slug: &str) -> Uuid {
    let resp = app
        .clone()
        .oneshot(req(
            Method::POST,
            "/api/admin/judges",
            Some(cookie),
            Some(judge_body(slug)),
        ))
        .await
        .expect("create judge");
    let (s, _, b) = read_body(resp).await;
    assert_eq!(s, StatusCode::CREATED, "create_judge: {b}");
    b["id"].as_str().expect("id").parse().unwrap()
}

async fn insert_project(state: &AppState, owner: Uuid, public: bool) -> Uuid {
    let now = Utc::now();
    projects::ActiveModel {
        id: Set(Uuid::new_v4()),
        name: Set("proj".into()),
        slug: Set(None),
        description: Set(String::new()),
        category: Set(None),
        tags: Set("[]".into()),
        cover_image_url: Set(None),
        owner_user_id_fk: Set(owner),
        public: Set(public),
        archived_at: Set(None),
        created_at: Set(now),
        updated_at: Set(now),
        default_value_points: Set(10),
        default_fail_points: Set(-5),
        default_no_response_points: Set(-10),
        default_completion_bonus_points: Set(10),
        default_deadline_secs: Set(60),
        default_session_duration_secs: Set(3600),
        idle_timeout_secs: Set(300),
        default_min_interval_secs: Set(5),
        default_interval_increment_secs: Set(5),
        default_max_interval_secs: Set(60),
        memory_schema: Set(None),
        show_tasks: Set(true),
        parent_project_id_fk: Set(None),
        part_ordinal: Set(None),
    }
    .insert(&state.db)
    .await
    .expect("insert project")
    .id
}

async fn insert_task(state: &AppState, project_id: Uuid, ordinal: i32) -> Uuid {
    let now = Utc::now();
    let id = Uuid::new_v4();
    tasks::ActiveModel {
        id: Set(id),
        project_id_fk: Set(project_id),
        ordinal: Set(ordinal),
        title: Set(format!("task {ordinal}")),
        content: Set(String::new()),
        test_template: Set(serde_json::json!({"kind":"shell","command_template":"echo hi"})),
        created_at: Set(now),
        tags: Set("[]".into()),
        point_value: Set(10),
        deadline_secs: Set(None),
        min_interval_secs: Set(None),
        interval_increment_secs: Set(None),
        max_interval_secs: Set(None),
        fail_points: Set(-5),
        no_response_points: Set(-10),
        completion_bonus_points: Set(10),
        evaluation: Set(None),
    }
    .insert(&state.db)
    .await
    .expect("insert task");
    id
}

async fn attach_judge(
    app: &axum::Router,
    cookie: &str,
    project_id: Uuid,
    task_id: Uuid,
    judge_id: Uuid,
    ordinal: i32,
) {
    let resp = app
        .clone()
        .oneshot(req(
            Method::POST,
            &format!("/api/projects/{project_id}/tasks/{task_id}/judges"),
            Some(cookie),
            Some(serde_json::json!({ "judge_id": judge_id, "ordinal": ordinal })),
        ))
        .await
        .expect("attach judge");
    let (s, _, b) = read_body(resp).await;
    assert_eq!(s, StatusCode::CREATED, "attach_judge: {b}");
}

async fn insert_session(state: &AppState, project_id: Uuid, join_code: &str) -> Uuid {
    insert_session_finished_at(state, project_id, join_code, Utc::now()).await
}

/// The seasonal split reads the session's `finished_at`, so the season test
/// needs to place a whole session in the past — not just an award row.
async fn insert_session_finished_at(
    state: &AppState,
    project_id: Uuid,
    join_code: &str,
    finished_at: chrono::DateTime<Utc>,
) -> Uuid {
    let now = finished_at;
    let id = Uuid::new_v4();
    sessions::ActiveModel {
        id: Set(id),
        name: Set("s".into()),
        created_at: Set(now),
        owner_id_fk: Set(None),
        status: Set(SessionStatus::Finished),
        join_code: Set(join_code.into()),
        started_at: Set(Some(now)),
        finished_at: Set(Some(now)),
        paused_at: Set(None),
        paused_duration_secs: Set(None),
        project_id_fk: Set(project_id),
        game_server_id: Set(None),
        cancel_reason: Set(None),
        cancelled_by: Set(None),
    }
    .insert(&state.db)
    .await
    .expect("insert session");
    id
}

/// One account's run in a session: a player row plus a single task result
/// carrying the whole score. The board re-derives standings from the scoring
/// tables — there is no award snapshot to seed.
async fn insert_scored_run(
    state: &AppState,
    session_id: Uuid,
    user_id: Uuid,
    name: &str,
    points: i32,
) {
    let player_id = Uuid::new_v4();
    players::ActiveModel {
        id: Set(player_id),
        session_id_fk: Set(session_id),
        user_id_fk: Set(Some(user_id)),
        display_name: Set(name.into()),
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
    task_results::ActiveModel {
        id: Set(Uuid::new_v4()),
        session_id_fk: Set(session_id),
        player_id_fk: Set(player_id),
        task_id: Set(None),
        answer: Set(String::new()),
        created_at: Set(Utc::now()),
        point_delta: Set(points),
        is_bonus: Set(false),
    }
    .insert(&state.db)
    .await
    .expect("insert task result");
}

async fn insert_user(state: &AppState, name: &str) -> Uuid {
    let id = Uuid::new_v4();
    users::ActiveModel {
        id: Set(id),
        email: Set(format!("{name}@tp.test")),
        password_hash: Set(None),
        display_name: Set(name.into()),
        created_at: Set(Utc::now()),
        updated_at: Set(Utc::now()),
        is_admin: Set(false),
        avatar_url: Set(None),
        email_verified: Set(false),
        username: Set(Some(name.into())),
        plan: Set(arena_core::quota::PLAN_PREMIUM.to_string()),
        judge_run_limit: Set(None),
        judge_run_credits: Set(0),
    }
    .insert(&state.db)
    .await
    .expect("insert user");
    id
}

async fn get_json(
    app: &axum::Router,
    uri: &str,
    cookie: Option<&str>,
) -> (StatusCode, serde_json::Value) {
    let resp = app
        .clone()
        .oneshot(req(Method::GET, uri, cookie, None))
        .await
        .expect("get resp");
    let (s, _, b) = read_body(resp).await;
    (s, b)
}

#[tokio::test]
async fn project_judges_returns_distinct_ordered() {
    let state = test_state().await;
    let app = build_router(state.clone());
    let (admin_id, cookie) =
        register_and_login(app.clone(), "admin@pj.test", "password-12345").await;

    let project_id = insert_project(&state, admin_id, true).await;
    let t0 = insert_task(&state, project_id, 0).await;
    let t1 = insert_task(&state, project_id, 1).await;

    let ja = create_judge(&app, &cookie, "alpha").await;
    let jb = create_judge(&app, &cookie, "bravo").await;

    // task 0 → alpha; task 1 → alpha (dup) + bravo.
    attach_judge(&app, &cookie, project_id, t0, ja, 0).await;
    attach_judge(&app, &cookie, project_id, t1, ja, 0).await;
    attach_judge(&app, &cookie, project_id, t1, jb, 1).await;

    let (status, body) = get_json(&app, &format!("/api/projects/{project_id}/judges"), None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let judges = body["judges"].as_array().expect("judges array");
    assert_eq!(judges.len(), 2, "alpha deduped across two tasks: {body}");
    assert_eq!(judges[0]["slug"], "alpha");
    assert_eq!(judges[1]["slug"], "bravo");
    assert_eq!(judges[0]["name"], "Judge alpha");
    assert_eq!(judges[0]["description"], "desc for alpha");
}

#[tokio::test]
async fn project_judges_empty_when_none_attached() {
    let state = test_state().await;
    let app = build_router(state.clone());
    let (admin_id, _cookie) =
        register_and_login(app.clone(), "admin@pj2.test", "password-12345").await;
    let project_id = insert_project(&state, admin_id, true).await;
    insert_task(&state, project_id, 0).await;

    let (status, body) = get_json(&app, &format!("/api/projects/{project_id}/judges"), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["judges"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn top_players_ranked_by_game_points() {
    let state = test_state().await;
    let app = build_router(state.clone());
    let (admin_id, _cookie) =
        register_and_login(app.clone(), "admin@tp.test", "password-12345").await;

    let project_id = insert_project(&state, admin_id, true).await;
    let alice = insert_user(&state, "alice").await;
    let bob = insert_user(&state, "bob").await;

    let s1 = insert_session(&state, project_id, "TP1").await;
    let s2 = insert_session(&state, project_id, "TP2").await;

    // Ranking is by game points, and by the player's BEST session rather
    // than their total. Alice has the bigger sum (350 > 300), yet Bob's
    // single best run (300) beats Alice's (200), so Bob ranks first.
    // Summing would have rewarded playing often over playing well.
    insert_scored_run(&state, s1, alice, "alice", 200).await;
    insert_scored_run(&state, s2, alice, "alice", 150).await;
    insert_scored_run(&state, s1, bob, "bob", 300).await;

    let (status, body) = get_json(
        &app,
        &format!("/api/projects/{project_id}/top-players"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let players = body["players"].as_array().expect("players array");
    assert_eq!(players.len(), 2);

    assert_eq!(players[0]["rank"], 1);
    assert_eq!(
        players[0]["display_name"], "bob",
        "the better single session ranks first, even against a bigger total"
    );
    assert_eq!(players[0]["game_points"], 300);
    assert_eq!(players[0]["sessions_played"], 1);
    assert_eq!(players[0]["best_placement"], 1);

    assert_eq!(players[1]["rank"], 2);
    assert_eq!(players[1]["display_name"], "alice");
    assert_eq!(
        players[1]["game_points"], 200,
        "her best session, not 200 + 150"
    );
    assert_eq!(
        players[1]["sessions_played"], 2,
        "session count still counts every session played"
    );
    assert_eq!(
        players[1]["best_placement"], 1,
        "min placement across her sessions"
    );
}

/// Judges can take a whole session below zero, so the best score may be
/// negative. It must be reported as such rather than floored at 0.
#[tokio::test]
async fn top_players_best_score_can_be_negative() {
    let state = test_state().await;
    let app = build_router(state.clone());
    let (admin_id, _cookie) =
        register_and_login(app.clone(), "admin@tpneg.test", "password-12345").await;

    let project_id = insert_project(&state, admin_id, true).await;
    let carol = insert_user(&state, "carol").await;
    let s1 = insert_session(&state, project_id, "TN1").await;
    let s2 = insert_session(&state, project_id, "TN2").await;

    insert_scored_run(&state, s1, carol, "carol", -80).await;
    insert_scored_run(&state, s2, carol, "carol", -30).await;

    let (status, body) = get_json(
        &app,
        &format!("/api/projects/{project_id}/top-players"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let players = body["players"].as_array().expect("players array");
    assert_eq!(players.len(), 1);
    assert_eq!(
        players[0]["game_points"], -30,
        "the least-bad session, not 0 and not the sum"
    );
}

/// The page shows both boards: all time, and the current season alone. A
/// season resets on the 1st, so the all-time board is what keeps an active
/// project from reading as abandoned in the first days of a month.
#[tokio::test]
async fn top_players_split_all_time_and_current_season() {
    let state = test_state().await;
    let app = build_router(state.clone());
    let (admin_id, _cookie) =
        register_and_login(app.clone(), "admin@tpseason.test", "password-12345").await;

    let project_id = insert_project(&state, admin_id, true).await;
    let alice = insert_user(&state, "alice").await;
    let bob = insert_user(&state, "bob").await;

    let last_season = arena_core::seasons::season_start(Utc::now()) - chrono::Duration::days(1);
    // Bob's 900 is the project record but that session finished last season;
    // Alice's 400 is the best anyone has managed since the reset.
    let s1 = insert_session_finished_at(&state, project_id, "TPS1", last_season).await;
    let s2 = insert_session(&state, project_id, "TPS2").await;
    insert_scored_run(&state, s1, bob, "bob", 900).await;
    insert_scored_run(&state, s2, alice, "alice", 400).await;

    let (status, body) = get_json(
        &app,
        &format!("/api/projects/{project_id}/top-players"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let all = body["players"].as_array().expect("players array");
    assert_eq!(all.len(), 2, "all time keeps last season's players");
    assert_eq!(all[0]["display_name"], "bob");
    assert_eq!(all[0]["game_points"], 900);

    let season = body["season_players"].as_array().expect("season array");
    assert_eq!(season.len(), 1, "only this season's finishes: {body}");
    assert_eq!(season[0]["display_name"], "alice");
    assert_eq!(season[0]["game_points"], 400);
    assert_eq!(season[0]["rank"], 1, "seasonal board is ranked on its own");

    let season_start = body["season_start"].as_str().expect("season_start");
    assert_eq!(
        chrono::DateTime::parse_from_rfc3339(season_start)
            .expect("rfc3339")
            .with_timezone(&Utc),
        arena_core::seasons::season_start(Utc::now()),
    );
}

#[tokio::test]
async fn top_players_empty_without_scored_sessions() {
    let state = test_state().await;
    let app = build_router(state.clone());
    let (admin_id, _cookie) =
        register_and_login(app.clone(), "admin@tp2.test", "password-12345").await;
    let project_id = insert_project(&state, admin_id, true).await;

    let (status, body) = get_json(
        &app,
        &format!("/api/projects/{project_id}/top-players"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["players"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn private_project_subresources_hidden_from_anon() {
    let state = test_state().await;
    let app = build_router(state.clone());
    let (admin_id, cookie) =
        register_and_login(app.clone(), "admin@pv.test", "password-12345").await;
    let project_id = insert_project(&state, admin_id, false).await;

    // Anonymous callers get 404 (existence not leaked).
    let (s1, _) = get_json(&app, &format!("/api/projects/{project_id}/judges"), None).await;
    assert_eq!(s1, StatusCode::NOT_FOUND);
    let (s2, _) = get_json(
        &app,
        &format!("/api/projects/{project_id}/top-players"),
        None,
    )
    .await;
    assert_eq!(s2, StatusCode::NOT_FOUND);

    // The owner/admin can see them.
    let (s3, _) = get_json(
        &app,
        &format!("/api/projects/{project_id}/judges"),
        Some(&cookie),
    )
    .await;
    assert_eq!(s3, StatusCode::OK);
    let (s4, _) = get_json(
        &app,
        &format!("/api/projects/{project_id}/top-players"),
        Some(&cookie),
    )
    .await;
    assert_eq!(s4, StatusCode::OK);
}
