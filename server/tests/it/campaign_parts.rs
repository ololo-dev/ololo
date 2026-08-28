//! Campaign projects: the parts list, the session gate, and the carry-over
//! discovery endpoint.
//!
//! The rule under test is one sentence — a part opens once the caller has a
//! *completing* run of the part before it — and it has three sharp edges the
//! obvious implementation gets wrong: a finished session where the player
//! never reached the end does not count, a revoked player does not count, and
//! "most recent" must mean most recently finished, because that is the
//! snapshot the next part continues from.

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use chrono::{Duration as ChronoDuration, Utc};
use sea_orm::{ActiveModelTrait, DatabaseConnection, Set};
use server::entities::{
    cli_tokens, players, projects, session_scheduler_state, sessions, tasks, users,
};
use server::{AppState, build_router};
use tower::ServiceExt;
use uuid::Uuid;

use crate::common::{read_body_json, req, test_state};

async fn seed_user(db: &DatabaseConnection, name: &str) -> Uuid {
    let id = Uuid::new_v4();
    users::ActiveModel {
        id: Set(id),
        email: Set(format!("{name}-{id}@example.com")),
        password_hash: Set(None),
        display_name: Set(name.to_string()),
        created_at: Set(Utc::now()),
        updated_at: Set(Utc::now()),
        is_admin: Set(false),
        avatar_url: Set(None),
        email_verified: Set(true),
        username: Set(None),
        plan: Set(arena_core::quota::PLAN_PREMIUM.to_string()),
        judge_run_limit: Set(None),
        judge_run_credits: Set(0),
    }
    .insert(db)
    .await
    .expect("insert user");
    id
}

async fn mint_pat(db: &DatabaseConnection, user_id: Uuid, filler: char) -> String {
    let token = format!("ololo_{}", filler.to_string().repeat(64));
    cli_tokens::ActiveModel {
        id: Set(Uuid::new_v4()),
        token_hash: Set(server::auth::pat::hash_pat(&token)),
        user_id: Set(user_id),
        created_at: Set(Utc::now().into()),
        expires_at: Set((Utc::now() + ChronoDuration::days(30)).into()),
    }
    .insert(db)
    .await
    .expect("insert cli token");
    token
}

#[allow(clippy::too_many_arguments)]
async fn seed_project(
    db: &DatabaseConnection,
    owner: Uuid,
    slug: &str,
    parent: Option<Uuid>,
    part_ordinal: Option<i32>,
) -> Uuid {
    let id = Uuid::new_v4();
    projects::ActiveModel {
        id: Set(id),
        name: Set(slug.to_string()),
        slug: Set(Some(slug.to_string())),
        description: Set(String::new()),
        category: Set(None),
        tags: Set("[]".into()),
        cover_image_url: Set(None),
        owner_user_id_fk: Set(owner),
        public: Set(true),
        archived_at: Set(None),
        created_at: Set(Utc::now()),
        updated_at: Set(Utc::now()),
        default_value_points: Set(10),
        default_fail_points: Set(-5),
        default_no_response_points: Set(-10),
        default_completion_bonus_points: Set(10),
        default_deadline_secs: Set(60),
        default_session_duration_secs: Set(900),
        idle_timeout_secs: Set(300),
        default_min_interval_secs: Set(5),
        default_interval_increment_secs: Set(5),
        default_max_interval_secs: Set(60),
        memory_schema: Set(None),
        show_tasks: Set(true),
        parent_project_id_fk: Set(parent),
        part_ordinal: Set(part_ordinal),
    }
    .insert(db)
    .await
    .expect("insert project");
    id
}

async fn seed_task(db: &DatabaseConnection, project_id: Uuid, ordinal: i32) {
    tasks::ActiveModel {
        id: Set(Uuid::new_v4()),
        project_id_fk: Set(project_id),
        ordinal: Set(ordinal),
        title: Set(format!("task {ordinal}")),
        content: Set(String::new()),
        test_template: Set(serde_json::json!({"kind":"shell","command_template":"echo hi"})),
        created_at: Set(Utc::now()),
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
    .insert(db)
    .await
    .expect("insert task");
}

/// A finished session of `project_id` that `user` played. `completed` writes
/// the scheduler row the unlock predicate looks for; `revoked` marks the
/// player row revoked. `finished_offset_secs` orders competing runs.
async fn seed_finished_session(
    db: &DatabaseConnection,
    project_id: Uuid,
    user: Uuid,
    completed: bool,
    revoked: bool,
    finished_offset_secs: i64,
) -> (Uuid, Uuid) {
    let session_id = Uuid::new_v4();
    let finished_at = Utc::now() - ChronoDuration::seconds(3600 - finished_offset_secs);
    sessions::ActiveModel {
        id: Set(session_id),
        name: Set("s".into()),
        created_at: Set(finished_at - ChronoDuration::seconds(900)),
        owner_id_fk: Set(Some(user)),
        status: Set(arena_core::session_status::SessionStatus::Finished),
        join_code: Set(format!("C{:05}", finished_offset_secs)),
        started_at: Set(Some(finished_at - ChronoDuration::seconds(900))),
        finished_at: Set(Some(finished_at)),
        paused_at: Set(None),
        paused_duration_secs: Set(None),
        project_id_fk: Set(project_id),
        game_server_id: Set(None),
        cancel_reason: Set(None),
        cancelled_by: Set(None),
    }
    .insert(db)
    .await
    .expect("insert session");

    let player_id = Uuid::new_v4();
    players::ActiveModel {
        id: Set(player_id),
        session_id_fk: Set(session_id),
        user_id_fk: Set(Some(user)),
        display_name: Set("p".into()),
        fingerprint: Set(None),
        metadata_json: Set(None),
        joined_at: Set(finished_at - ChronoDuration::seconds(900)),
        reconnected_at: Set(None),
        revoked_at: Set(revoked.then(Utc::now)),
        agent_connected: Set(false),
        agent_last_seen_at: Set(None),
    }
    .insert(db)
    .await
    .expect("insert player");

    if completed {
        session_scheduler_state::ActiveModel {
            id: Set(Uuid::new_v4()),
            session_id_fk: Set(session_id),
            player_id_fk: Set(player_id),
            task_id: Set(None),
            state: Set(arena_core::session_completion::SCHEDULER_STATE_COMPLETED.to_string()),
            next_probe_at: Set(None),
            created_at: Set(finished_at),
            updated_at: Set(finished_at),
        }
        .insert(db)
        .await
        .expect("insert scheduler state");
        award_completion_bonuses(db, session_id, player_id).await;
    }

    (session_id, player_id)
}

/// A campaign with two parts, owned by a fresh user.
async fn seed_campaign(db: &DatabaseConnection) -> (Uuid, Uuid, Uuid, Uuid) {
    let owner = seed_user(db, "owner").await;
    let parent = seed_project(db, owner, "campaign", None, None).await;
    let part_one = seed_project(db, owner, "campaign-1", Some(parent), Some(0)).await;
    let part_two = seed_project(db, owner, "campaign-2", Some(parent), Some(1)).await;
    seed_task(db, part_one, 0).await;
    seed_task(db, part_two, 0).await;
    (owner, parent, part_one, part_two)
}

async fn parts_of(state: &AppState, project_id: Uuid) -> serde_json::Value {
    let resp = build_router(state.clone())
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri(format!("/api/projects/{project_id}/parts"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("request");
    let (status, body) = read_body_json(resp).await;
    assert_eq!(status, StatusCode::OK, "parts list: {body}");
    body
}

async fn previous_part_source(
    state: &AppState,
    project_id: Uuid,
    pat: Option<&str>,
) -> (StatusCode, serde_json::Value) {
    let mut builder = Request::builder()
        .method(Method::GET)
        .uri(format!("/api/projects/{project_id}/previous-part-source"));
    if let Some(pat) = pat {
        builder = builder.header("X-API-Key", pat);
    }
    let resp = build_router(state.clone())
        .oneshot(builder.body(Body::empty()).unwrap())
        .await
        .expect("request");
    read_body_json(resp).await
}

/// Create a session as `user`, returning the status and body.
async fn create_session(
    state: &AppState,
    cookie: &str,
    project_id: Uuid,
) -> (StatusCode, serde_json::Value) {
    let resp = build_router(state.clone())
        .oneshot(req(
            Method::POST,
            "/api/sessions",
            Some(cookie),
            Some(serde_json::json!({ "name": "run", "project_id": project_id })),
        ))
        .await
        .expect("request");
    read_body_json(resp).await
}

#[tokio::test]
async fn an_anonymous_visitor_sees_the_first_part_open_and_the_rest_locked() {
    let state = test_state().await;
    let (_owner, parent, part_one, part_two) = seed_campaign(&state.db).await;

    let body = parts_of(&state, parent).await;
    let parts = body["parts"].as_array().expect("parts array");
    assert_eq!(parts.len(), 2, "{body}");
    assert_eq!(parts[0]["id"], part_one.to_string());
    assert_eq!(parts[0]["state"], "available");
    assert_eq!(parts[1]["id"], part_two.to_string());
    assert_eq!(
        parts[1]["state"], "locked",
        "a visitor we don't know has cleared nothing"
    );
    assert_eq!(parts[0]["task_count"], 1, "{body}");
    // A part goes out as the same summary every listing carries, so the
    // campaign page can render the catalog's own card for it.
    for field in [
        "name",
        "slug",
        "description",
        "tags",
        "session_duration_secs",
        "points",
        "intervals",
    ] {
        assert!(
            !parts[0][field].is_null(),
            "a part must carry '{field}' like any other project summary: {body}"
        );
    }
    assert_eq!(
        parts[0]["parent_project_slug"], "campaign",
        "a part names its campaign: {body}"
    );
}

#[tokio::test]
async fn an_ordinary_project_reports_no_parts() {
    let state = test_state().await;
    let owner = seed_user(&state.db, "owner").await;
    let solo = seed_project(&state.db, owner, "solo", None, None).await;

    let body = parts_of(&state, solo).await;
    assert_eq!(
        body["parts"].as_array().expect("parts array").len(),
        0,
        "the frontend asks unconditionally, so this must not 404: {body}"
    );
}

#[tokio::test]
async fn clearing_a_part_unlocks_the_next_one() {
    let state = test_state().await;
    let (_owner, parent, part_one, _part_two) = seed_campaign(&state.db).await;
    let player = seed_user(&state.db, "player").await;

    seed_finished_session(&state.db, part_one, player, true, false, 100).await;

    let completed =
        arena_core::campaign::user_completed_projects(&state.db, &[part_one], player).await;
    assert!(
        completed.expect("query").contains(&part_one),
        "a finished run with a completed scheduler row is what unlocking is made of"
    );

    let body = parts_of(&state, parent).await;
    // The public list is per-caller and this request is anonymous, so it must
    // NOT leak another player's progress.
    assert_eq!(body["parts"][0]["state"], "available");
}

#[tokio::test]
async fn a_finished_session_that_was_never_completed_does_not_unlock() {
    let state = test_state().await;
    let (_owner, _parent, part_one, _part_two) = seed_campaign(&state.db).await;
    let player = seed_user(&state.db, "player").await;

    seed_finished_session(&state.db, part_one, player, false, false, 100).await;

    let completed = arena_core::campaign::user_completed_projects(&state.db, &[part_one], player)
        .await
        .expect("query");
    assert!(
        completed.is_empty(),
        "playing a part and abandoning it leaves nothing for the next part to continue"
    );
}

#[tokio::test]
async fn a_revoked_player_does_not_unlock_anything() {
    let state = test_state().await;
    let (_owner, _parent, part_one, _part_two) = seed_campaign(&state.db).await;
    let player = seed_user(&state.db, "player").await;

    seed_finished_session(&state.db, part_one, player, true, true, 100).await;

    let completed = arena_core::campaign::user_completed_projects(&state.db, &[part_one], player)
        .await
        .expect("query");
    assert!(
        completed.is_empty(),
        "a revoked player is ineligible everywhere else; campaigns are no exception"
    );
}

#[tokio::test]
async fn the_carry_over_source_points_at_the_most_recent_completing_run() {
    let state = test_state().await;
    let (_owner, _parent, part_one, part_two) = seed_campaign(&state.db).await;
    let player = seed_user(&state.db, "player").await;
    let pat = mint_pat(&state.db, player, 'a').await;

    let (_old_session, _old_player) =
        seed_finished_session(&state.db, part_one, player, true, false, 100).await;
    let (new_session, new_player) =
        seed_finished_session(&state.db, part_one, player, true, false, 500).await;

    let (status, body) = previous_part_source(&state, part_two, Some(&pat)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body["session_id"],
        new_session.to_string(),
        "the newest completing run is the one whose code the next part continues: {body}"
    );
    assert_eq!(body["player_id"], new_player.to_string());
    assert_eq!(body["prev_project_slug"], "campaign-1");
    assert_eq!(
        body["git_remote_path"],
        format!("/git/{new_session}/{new_player}.git"),
        "the CLI appends this to the server base to clone"
    );
}

#[tokio::test]
async fn the_carry_over_source_needs_a_pat_and_a_completed_predecessor() {
    let state = test_state().await;
    let (_owner, _parent, part_one, part_two) = seed_campaign(&state.db).await;
    let player = seed_user(&state.db, "player").await;
    let pat = mint_pat(&state.db, player, 'b').await;

    let (status, body) = previous_part_source(&state, part_two, None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");

    let (status, body) = previous_part_source(&state, part_two, Some(&pat)).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(body["error"], "not_completed");

    // The first part has nothing before it.
    let (status, body) = previous_part_source(&state, part_one, Some(&pat)).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(body["error"], "no_previous_part");
}

#[tokio::test]
async fn session_creation_refuses_a_locked_part_and_the_campaign_itself() {
    let state = test_state().await;
    let (_owner, parent, part_one, part_two) = seed_campaign(&state.db).await;

    let app = build_router(state.clone());
    let (user_id, cookie) =
        crate::common::register_and_login_default(app, "player@example.com").await;

    // The campaign parent is a table of contents, not a playable project.
    let (status, body) = create_session(&state, &cookie, parent).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["error"], "campaign_project");

    // Part two is gated on part one.
    let (status, body) = create_session(&state, &cookie, part_two).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["error"], "part_locked");
    assert_eq!(
        body["required_project"], "campaign-1",
        "the refusal must name what to finish: {body}"
    );
    assert_eq!(body["required_part_ordinal"], 0);

    // Part one is always open.
    let (status, body) = create_session(&state, &cookie, part_one).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");

    // Clearing part one opens part two. The live session from the line above
    // has to end first — one live session per player — which is exactly the
    // shape of real play.
    let session_id: Uuid = body["id"]
        .as_str()
        .expect("session id")
        .parse()
        .expect("uuid");
    finish_and_complete(&state.db, session_id, user_id).await;

    let (status, body) = create_session(&state, &cookie, part_two).await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "part two must open once part one is cleared: {body}"
    );
}

/// Mark a live session finished with its player completed — what the game
/// server does at the end of a run everyone got through.
async fn finish_and_complete(db: &DatabaseConnection, session_id: Uuid, user_id: Uuid) {
    use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};

    let session = sessions::Entity::find_by_id(session_id)
        .one(db)
        .await
        .expect("query")
        .expect("session");
    let mut am: sessions::ActiveModel = session.into();
    am.status = Set(arena_core::session_status::SessionStatus::Finished);
    am.finished_at = Set(Some(Utc::now()));
    am.update(db).await.expect("finish session");

    // The owner is auto-joined as a player only once the CLI joins, so make
    // sure the row exists before marking it complete.
    let player = players::Entity::find()
        .filter(players::Column::SessionIdFk.eq(session_id))
        .filter(players::Column::UserIdFk.eq(user_id))
        .one(db)
        .await
        .expect("query");
    let player_id = match player {
        Some(p) => p.id,
        None => {
            let id = Uuid::new_v4();
            players::ActiveModel {
                id: Set(id),
                session_id_fk: Set(session_id),
                user_id_fk: Set(Some(user_id)),
                display_name: Set("p".into()),
                fingerprint: Set(None),
                metadata_json: Set(None),
                joined_at: Set(Utc::now()),
                reconnected_at: Set(None),
                revoked_at: Set(None),
                agent_connected: Set(false),
                agent_last_seen_at: Set(None),
            }
            .insert(db)
            .await
            .expect("insert player");
            id
        }
    };

    session_scheduler_state::ActiveModel {
        id: Set(Uuid::new_v4()),
        session_id_fk: Set(session_id),
        player_id_fk: Set(player_id),
        task_id: Set(None),
        state: Set(arena_core::session_completion::SCHEDULER_STATE_COMPLETED.to_string()),
        next_probe_at: Set(None),
        created_at: Set(Utc::now()),
        updated_at: Set(Utc::now()),
    }
    .insert(db)
    .await
    .expect("insert scheduler state");

    award_completion_bonuses(db, session_id, player_id).await;
}

/// The completion bonuses a genuinely completed run carries — the game
/// server writes one per bonus-paying task when it completes via its own
/// probe, and the campaign predicate requires them: an open-ended task sat
/// out to its deadline exhausts the scheduler without earning any.
async fn award_completion_bonuses(db: &DatabaseConnection, session_id: Uuid, player_id: Uuid) {
    use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
    let session = sessions::Entity::find_by_id(session_id)
        .one(db)
        .await
        .expect("query")
        .expect("session");
    let task_ids: Vec<Uuid> = tasks::Entity::find()
        .filter(tasks::Column::ProjectIdFk.eq(session.project_id_fk))
        .all(db)
        .await
        .expect("tasks")
        .into_iter()
        .filter(|t| t.completion_bonus_points != 0)
        .map(|t| t.id)
        .collect();
    for task_id in task_ids {
        arena_core::entities::task_results::ActiveModel {
            id: Set(Uuid::new_v4()),
            session_id_fk: Set(session_id),
            player_id_fk: Set(player_id),
            task_id: Set(Some(task_id)),
            answer: Set("completion bonus".into()),
            created_at: Set(Utc::now()),
            point_delta: Set(10),
            is_bonus: Set(true),
        }
        .insert(db)
        .await
        .expect("insert bonus row");
    }
}

#[tokio::test]
async fn the_parts_list_shows_a_signed_in_players_own_progress() {
    let state = test_state().await;
    let (_owner, parent, part_one, _part_two) = seed_campaign(&state.db).await;

    let app = build_router(state.clone());
    let (user_id, cookie) =
        crate::common::register_and_login_default(app, "progress@example.com").await;
    seed_finished_session(&state.db, part_one, user_id, true, false, 100).await;

    let resp = build_router(state.clone())
        .oneshot(req(
            Method::GET,
            &format!("/api/projects/{parent}/parts"),
            Some(&cookie),
            None,
        ))
        .await
        .expect("request");
    let (status, body) = read_body_json(resp).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["parts"][0]["state"], "completed", "{body}");
    assert_eq!(
        body["parts"][1]["state"], "available",
        "clearing part one opens part two for this caller: {body}"
    );
}

/// A finished, completed session of `project_id` whose scoring rows for
/// `user` sum to exactly `game_points`. The board derives standings from
/// the scoring tables, so the score is seeded as a task result — topped up
/// past the +10 completion bonus `seed_finished_session` already writes.
async fn seed_awarded_session(
    db: &DatabaseConnection,
    project_id: Uuid,
    user: Uuid,
    game_points: i64,
    offset_secs: i64,
) {
    let (session_id, player_id) =
        seed_finished_session(db, project_id, user, true, false, offset_secs).await;
    arena_core::entities::task_results::ActiveModel {
        id: Set(Uuid::new_v4()),
        session_id_fk: Set(session_id),
        player_id_fk: Set(player_id),
        task_id: Set(None),
        answer: Set("seed score".into()),
        created_at: Set(Utc::now()),
        point_delta: Set(i32::try_from(game_points - 10).expect("score fits i32")),
        is_bonus: Set(false),
    }
    .insert(db)
    .await
    .expect("insert score");
}

async fn top_players(state: &AppState, project_id: Uuid) -> serde_json::Value {
    let resp = build_router(state.clone())
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri(format!("/api/projects/{project_id}/top-players"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("request");
    let (status, body) = read_body_json(resp).await;
    assert_eq!(status, StatusCode::OK, "top players: {body}");
    body
}

#[tokio::test]
async fn a_campaign_board_adds_up_a_players_best_run_of_each_part() {
    // A campaign hosts no sessions of its own, so its board is the sum over
    // its parts. Best-run-per-part is what gets added: replaying part one
    // must never out-score getting further through the campaign.
    let state = test_state().await;
    let (_owner, parent, part_one, part_two) = seed_campaign(&state.db).await;

    let grinder = seed_user(&state.db, "grinder").await;
    let finisher = seed_user(&state.db, "finisher").await;

    // The grinder replays part one three times, best run 100.
    seed_awarded_session(&state.db, part_one, grinder, 60, 100).await;
    seed_awarded_session(&state.db, part_one, grinder, 100, 200).await;
    seed_awarded_session(&state.db, part_one, grinder, 80, 300).await;
    // The finisher clears both parts once, 70 + 50.
    seed_awarded_session(&state.db, part_one, finisher, 70, 400).await;
    seed_awarded_session(&state.db, part_two, finisher, 50, 500).await;

    let body = top_players(&state, parent).await;
    let players = body["players"].as_array().expect("players array");
    assert_eq!(players.len(), 2, "{body}");

    let by_user = |uid: Uuid| -> i64 {
        players
            .iter()
            .find(|p| p["user_id"] == uid.to_string())
            .and_then(|p| p["game_points"].as_i64())
            .unwrap_or_else(|| panic!("no board row for {uid}: {body}"))
    };
    assert_eq!(
        by_user(grinder),
        100,
        "three runs of one part count once, at their best: {body}"
    );
    assert_eq!(
        by_user(finisher),
        120,
        "clearing two parts sums their best runs: {body}"
    );
    assert_eq!(
        players[0]["user_id"],
        finisher.to_string(),
        "getting further through the campaign outranks grinding part one: {body}"
    );
}

async fn campaign_of(state: &AppState, code: &str) -> (StatusCode, serde_json::Value) {
    let resp = build_router(state.clone())
        .oneshot(req(
            Method::GET,
            &format!("/api/sessions/by-code/{code}/campaign"),
            None,
            None,
        ))
        .await
        .expect("request");
    if resp.status() == StatusCode::NO_CONTENT {
        return (StatusCode::NO_CONTENT, serde_json::Value::Null);
    }
    read_body_json(resp).await
}

#[tokio::test]
async fn a_session_dashboard_shows_the_parts_its_players_already_cleared() {
    // A part continues the codebase of the one before it, so "what came
    // before" is the first thing a spectator of part two needs — and the
    // answer has to name the session it happened in, which is where the work
    // can be read.
    let state = test_state().await;
    let (_owner, _parent, part_one, part_two) = seed_campaign(&state.db).await;
    let ada = seed_user(&state.db, "ada").await;

    seed_finished_session(&state.db, part_one, ada, true, false, 100).await;
    seed_finished_session(&state.db, part_two, ada, false, false, 200).await;

    let (status, body) = campaign_of(&state, "C00200").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["current_part_ordinal"], 1, "{body}");

    let parts = body["parts"].as_array().expect("parts");
    assert_eq!(parts.len(), 2, "{body}");
    assert_eq!(parts[0]["current"], false, "{body}");
    assert_eq!(parts[1]["current"], true, "{body}");

    let cleared = parts[0]["cleared_by"].as_array().expect("cleared_by");
    assert_eq!(cleared.len(), 1, "{body}");
    assert_eq!(cleared[0]["display_name"], "p", "{body}");
    assert_eq!(
        cleared[0]["join_code"], "C00100",
        "the run is a link to the session it was cleared in: {body}"
    );
    assert!(
        parts[1]["cleared_by"].as_array().expect("array").is_empty(),
        "the part being played now is not a past achievement: {body}"
    );
}

#[tokio::test]
async fn the_part_being_played_is_answered_by_this_session_alone() {
    // Clearing part two in an earlier run says nothing about how *this* run
    // of it went, and a chip linking to that other session would answer a
    // question nobody on this page asked.
    let state = test_state().await;
    let (_owner, _parent, _part_one, part_two) = seed_campaign(&state.db).await;
    let ada = seed_user(&state.db, "ada").await;

    seed_finished_session(&state.db, part_two, ada, true, false, 100).await;
    seed_finished_session(&state.db, part_two, ada, false, false, 200).await;

    let (_status, body) = campaign_of(&state, "C00200").await;
    assert_eq!(body["session_status"], "finished", "{body}");
    let current = &body["parts"][1];
    assert_eq!(current["current"], true, "{body}");
    assert!(
        current["cleared_by"].as_array().expect("array").is_empty(),
        "this run did not clear it, whatever an earlier one did: {body}"
    );

    // The run that did clear it says so, against its own join code.
    let (_status, body) = campaign_of(&state, "C00100").await;
    let current = &body["parts"][1];
    assert_eq!(current["cleared_by"][0]["join_code"], "C00100", "{body}");
}

#[tokio::test]
async fn sitting_out_the_deadline_does_not_clear_a_part() {
    // An open-ended task force-completes when its work window expires, so the
    // judges can score whatever exists — which exhausts the scheduler without
    // a single check passing. Session TJQJPJ unlocked part two of its
    // campaign that way, on 0/1 tasks. The mark of an earned task is its
    // completion bonus, which a forced completion never gets.
    let state = test_state().await;
    let (_owner, parent, part_one, _part_two) = seed_campaign(&state.db).await;
    let app = build_router(state.clone());
    let (ada, cookie) = crate::common::register_and_login_default(app, "sitter@example.com").await;

    use sea_orm::EntityTrait;
    // Scheduler exhausted, session finished — and no bonus row anywhere,
    // with the task created before the session ended so it is required.
    let (session_id, player_id) =
        seed_finished_session(&state.db, part_one, ada, false, false, 100).await;
    let mut session: sessions::ActiveModel = sessions::Entity::find_by_id(session_id)
        .one(&state.db)
        .await
        .expect("query")
        .expect("session")
        .into();
    session.finished_at = Set(Some(Utc::now() + ChronoDuration::seconds(60)));
    session.update(&state.db).await.expect("age forward");
    session_scheduler_state::ActiveModel {
        id: Set(Uuid::new_v4()),
        session_id_fk: Set(session_id),
        player_id_fk: Set(player_id),
        task_id: Set(None),
        state: Set(arena_core::session_completion::SCHEDULER_STATE_COMPLETED.to_string()),
        next_probe_at: Set(None),
        created_at: Set(Utc::now()),
        updated_at: Set(Utc::now()),
    }
    .insert(&state.db)
    .await
    .expect("insert scheduler state");

    let resp = build_router(state.clone())
        .oneshot(req(
            Method::GET,
            &format!("/api/projects/{parent}/parts"),
            Some(&cookie),
            None,
        ))
        .await
        .expect("request");
    let (_status, body) = read_body_json(resp).await;
    assert_eq!(
        body["parts"][0]["state"], "available",
        "an unearned run leaves the part uncleared: {body}"
    );
    assert_eq!(
        body["parts"][1]["state"], "locked",
        "and the next part stays locked: {body}"
    );
}

#[tokio::test]
async fn a_played_but_unfinished_part_is_not_shown_as_cleared() {
    let state = test_state().await;
    let (_owner, _parent, part_one, part_two) = seed_campaign(&state.db).await;
    let ada = seed_user(&state.db, "ada").await;

    // Played part one and stopped halfway: no scheduler completion row.
    seed_finished_session(&state.db, part_one, ada, false, false, 100).await;
    seed_finished_session(&state.db, part_two, ada, false, false, 200).await;

    let (_status, body) = campaign_of(&state, "C00200").await;
    assert!(
        body["parts"][0]["cleared_by"]
            .as_array()
            .expect("array")
            .is_empty(),
        "{body}"
    );
}

#[tokio::test]
async fn an_ordinary_session_has_no_campaign_to_show() {
    // 204, not an empty object: every other dashboard must not grow a card.
    let state = test_state().await;
    let owner = seed_user(&state.db, "owner").await;
    let solo = seed_project(&state.db, owner, "solo", None, None).await;
    seed_task(&state.db, solo, 0).await;
    seed_finished_session(&state.db, solo, owner, true, false, 300).await;

    let (status, _body) = campaign_of(&state, "C00300").await;
    assert_eq!(status, StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn a_campaign_board_says_how_far_each_player_got() {
    // Points alone cannot answer "who finished this campaign" — a big score
    // on part one outranks a modest run of all five. The board carries the
    // progression next to the score.
    let state = test_state().await;
    let (_owner, parent, part_one, part_two) = seed_campaign(&state.db).await;

    let finisher = seed_user(&state.db, "finisher").await;
    let starter = seed_user(&state.db, "starter").await;
    let quitter = seed_user(&state.db, "quitter").await;

    seed_awarded_session(&state.db, part_one, finisher, 10, 100).await;
    seed_awarded_session(&state.db, part_two, finisher, 10, 200).await;
    // Replaying a part cannot inflate the count.
    seed_awarded_session(&state.db, part_one, starter, 10, 300).await;
    seed_awarded_session(&state.db, part_one, starter, 10, 400).await;
    // Played and scored, but never reached the end of the task list.
    let (session_id, player) =
        seed_finished_session(&state.db, part_one, quitter, false, false, 500).await;
    arena_core::entities::task_results::ActiveModel {
        id: Set(Uuid::new_v4()),
        session_id_fk: Set(session_id),
        player_id_fk: Set(player),
        task_id: Set(None),
        answer: Set("seed score".into()),
        created_at: Set(Utc::now()),
        point_delta: Set(10),
        is_bonus: Set(false),
    }
    .insert(&state.db)
    .await
    .expect("insert score");

    let body = top_players(&state, parent).await;
    assert_eq!(body["parts_total"], 2, "{body}");

    let parts_done = |uid: Uuid| -> i64 {
        body["players"]
            .as_array()
            .expect("players")
            .iter()
            .find(|p| p["user_id"] == uid.to_string())
            .and_then(|p| p["parts_completed"].as_i64())
            .unwrap_or_else(|| panic!("no parts_completed for {uid}: {body}"))
    };
    assert_eq!(parts_done(finisher), 2, "cleared both parts: {body}");
    assert_eq!(
        parts_done(starter),
        1,
        "two runs of one part is one: {body}"
    );
    assert_eq!(
        parts_done(quitter),
        0,
        "an abandoned run clears nothing: {body}"
    );
}

#[tokio::test]
async fn an_ordinary_board_says_nothing_about_parts() {
    // "0 of 0 parts" on a standalone project would be a fact about nothing.
    let state = test_state().await;
    let (_owner, _parent, part_one, _part_two) = seed_campaign(&state.db).await;
    let player = seed_user(&state.db, "player").await;
    seed_awarded_session(&state.db, part_one, player, 60, 100).await;

    let body = top_players(&state, part_one).await;
    assert!(body["parts_total"].is_null(), "{body}");
    assert!(body["players"][0]["parts_completed"].is_null(), "{body}");
}

#[tokio::test]
async fn a_part_keeps_its_own_board() {
    // Only the campaign sums; a part is an ordinary project whose board is
    // the player's best single run.
    let state = test_state().await;
    let (_owner, _parent, part_one, _part_two) = seed_campaign(&state.db).await;
    let player = seed_user(&state.db, "player").await;

    seed_awarded_session(&state.db, part_one, player, 60, 100).await;
    seed_awarded_session(&state.db, part_one, player, 90, 200).await;

    let body = top_players(&state, part_one).await;
    assert_eq!(
        body["players"][0]["game_points"], 90,
        "a part still reports the best run, not the sum: {body}"
    );
}

#[tokio::test]
async fn the_session_report_counts_toward_a_project_s_review_estimate() {
    // Two requirements meet here: every project carries the report, and its
    // run shows up in the "~N reviews" a player reads before starting — the
    // same number their judge-run balance is spent against.
    let state = test_state().await;
    let owner = seed_user(&state.db, "owner").await;
    let project = seed_project(&state.db, owner, "solo", None, None).await;
    seed_task(&state.db, project, 0).await;
    seed_task(&state.db, project, 1).await;

    let before = review_estimate(&state, "solo").await;

    // A report judge exists in the catalogue…
    let judge_id = Uuid::new_v4();
    let now = Utc::now();
    arena_core::entities::judges::ActiveModel {
        id: Set(judge_id),
        slug: Set("general".into()),
        name: Set("The Debrief".into()),
        description: Set(String::new()),
        prompt: Set("write".into()),
        rating_scale: Set(serde_json::json!({"min": 0, "max": 1, "step": 1})),
        kind: Set(arena_core::judging::JUDGE_KIND_REPORT.to_string()),
        scope: Set("session".into()),
        evidence_mode: Set("tools".into()),
        evidence_needs: Set(None),
        criteria: Set(None),
        max_interactive: Set(None),
        avatar_url: Set(None),
        ignore_paths: Set(None),
        llm_provider_id_fk: Set(None),
        llm_model: Set(None),
        llm_pool_id_fk: Set(None),
        llm_source_order: Set("pool_first".into()),
        probes_config: Set(None),
        created_at: Set(now),
        updated_at: Set(now),
    }
    .insert(&state.db)
    .await
    .expect("insert report judge");

    // …and the sweep puts it on every project.
    server::seed::report_judge::ensure_report_judges_everywhere(&state.db).await;

    let after = review_estimate(&state, "solo").await;
    assert_eq!(
        after,
        before + 1,
        "the report is one more run a session pays for, so the estimate must say so"
    );
}

/// The `judge_review_count` a player reads on the project page.
async fn review_estimate(state: &AppState, slug: &str) -> i64 {
    let resp = build_router(state.clone())
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri(format!("/api/projects/by-slug/{slug}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("request");
    let (status, body) = read_body_json(resp).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body["judge_review_count"]
        .as_i64()
        .expect("estimate present")
}
