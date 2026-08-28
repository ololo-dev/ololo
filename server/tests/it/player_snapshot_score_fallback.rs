//! Regression: the player snapshot's score/rank must fall back to a DB
//! recompute when the session-registry cache can't answer. Finished
//! sessions are not hydrated into the registry at startup, so after a
//! server restart the cache-only read returned the (0, 0) sentinel —
//! the player page showed score 0 and "rank #0" while the session page
//! (report leaderboard, computed from the DB) showed the real totals.

use axum::http::{Method, StatusCode};
use sea_orm::{ActiveModelTrait, Set};
use server::build_router;
use server::entities::{judge_results, judges, task_judges, task_results, tasks};
use tower::ServiceExt;
use uuid::Uuid;

use crate::common;
use crate::common::{read_body_json, register_and_login_default, req_with_cookie, test_state};

/// Create project + session via the API; returns (join_code, project_id).
async fn create_session(app: &axum::Router, cookie: &str) -> (String, Uuid) {
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
    let project_id: Uuid = pb["id"]
        .as_str()
        .expect("project id")
        .parse()
        .expect("uuid");

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
    (
        sb["join_code"].as_str().expect("join_code").to_string(),
        project_id,
    )
}

async fn join_session(app: &axum::Router, cookie: &str, code: &str) -> Uuid {
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
    body["player_id"]
        .as_str()
        .expect("player_id")
        .parse()
        .expect("uuid")
}

/// Insert task_result (+100) and a scored judge_result (−30) for the player.
async fn seed_scores(
    db: &sea_orm::DatabaseConnection,
    project_id: Uuid,
    session_id: Uuid,
    player_id: Uuid,
) {
    let now = chrono::Utc::now();
    let task_id = Uuid::new_v4();
    tasks::ActiveModel {
        id: Set(task_id),
        project_id_fk: Set(project_id),
        ordinal: Set(0),
        title: Set("T".to_string()),
        content: Set(String::new()),
        test_template: Set(serde_json::json!({ "kind": "shell", "command_template": "echo ok" })),
        created_at: Set(now),
        tags: Set(String::new()),
        point_value: Set(10),
        deadline_secs: Set(Some(300)),
        min_interval_secs: Set(Some(5)),
        interval_increment_secs: Set(Some(0)),
        max_interval_secs: Set(Some(300)),
        fail_points: Set(0),
        no_response_points: Set(0),
        completion_bonus_points: Set(10),
        evaluation: Set(None),
    }
    .insert(db)
    .await
    .expect("task");

    task_results::ActiveModel {
        id: Set(Uuid::new_v4()),
        session_id_fk: Set(session_id),
        player_id_fk: Set(player_id),
        task_id: Set(Some(task_id)),
        answer: Set(String::new()),
        created_at: Set(now),
        point_delta: Set(100),
        is_bonus: Set(false),
    }
    .insert(db)
    .await
    .expect("task_result");

    let judge_id = Uuid::new_v4();
    judges::ActiveModel {
        id: Set(judge_id),
        slug: Set("anti-cheater".to_string()),
        name: Set("Anti Cheater".to_string()),
        description: Set("d".into()),
        prompt: Set("p".into()),
        rating_scale: Set(serde_json::json!({"min":0.0,"max":10.0,"step":1.0})),
        kind: Set("llm".to_string()),
        scope: Set("task".to_string()),
        evidence_mode: Set("tools".to_string()),
        evidence_needs: Set(None),
        created_at: Set(now),
        updated_at: Set(now),
        llm_provider_id_fk: Set(None),
        llm_model: Set(None),
        llm_pool_id_fk: Set(None),
        llm_source_order: Set(arena_core::llm::resolve::SOURCE_ORDER_POOL_FIRST.to_string()),
        criteria: Set(None),
        max_interactive: Set(None),
        avatar_url: Set(None),
        probes_config: Set(None),
        ignore_paths: Set(None),
    }
    .insert(db)
    .await
    .expect("judge");

    let task_judge_id = Uuid::new_v4();
    task_judges::ActiveModel {
        id: Set(task_judge_id),
        task_id: Set(task_id),
        judge_id: Set(judge_id),
        ordinal: Set(0),
        rating_scale_override: Set(None),
        created_at: Set(now),
        updated_at: Set(now),
        weight: Set(None),
    }
    .insert(db)
    .await
    .expect("task_judge");

    judge_results::ActiveModel {
        id: Set(Uuid::new_v4()),
        session_id_fk: Set(session_id),
        player_id_fk: Set(player_id),
        task_judge_id: Set(task_judge_id),
        rating: Set(serde_json::json!(2.0)),
        point_delta: Set(-30),
        feedback: Set("suspicious".to_string()),
        model: Set("m".to_string()),
        provider: Set("ollama".to_string()),
        raw_output: Set(String::new()),
        duration_ms: Set(Some(1000)),
        run_log: Set(None),
        tokens_input: Set(None),
        tokens_output: Set(None),
        tokens_cache_read: Set(None),
        tokens_cache_write: Set(None),
        status: Set("scored".to_string()),
        error: Set(None),
        created_at: Set(now),
        updated_at: Set(now),
        verdict_kind: Set(None),
    }
    .insert(db)
    .await
    .expect("judge_result");
}

#[tokio::test]
async fn snapshot_score_rank_recomputed_when_registry_cache_absent() {
    let state = test_state().await;
    let app = build_router(state.clone());

    let (_owner, cookie_owner) =
        register_and_login_default(app.clone(), "owner-scorefb@x.test").await;
    let (_joiner, cookie_joiner) =
        register_and_login_default(app.clone(), "joiner-scorefb@x.test").await;

    let (join_code, project_id) = create_session(&app, &cookie_owner).await;
    let player_id = join_session(&app, &cookie_joiner, &join_code).await;

    let session_id = {
        let entry = state
            .session_registry
            .get(&join_code)
            .expect("registry entry");
        let cache = entry.cache.read().expect("cache");
        cache.session_id
    };
    seed_scores(&state.db, project_id, session_id, player_id).await;

    // Simulate a post-restart server: no registry entry for the session
    // (startup hydration skips finished sessions).
    state.session_registry.clear();

    let resp = app
        .clone()
        .oneshot(req_with_cookie(
            Method::GET,
            &format!("/api/sessions/{join_code}/players/{player_id}"),
            &cookie_joiner,
            None,
        ))
        .await
        .expect("snapshot resp");
    let (sc, body) = read_body_json(resp).await;
    assert_eq!(sc, StatusCode::OK, "snapshot: {body}");
    assert_eq!(
        body["score"].as_i64(),
        Some(70),
        "task (+100) and judge (−30) deltas from the DB: {body}"
    );
    assert_eq!(
        body["rank"].as_u64(),
        Some(1),
        "rank is 1-based, never 0: {body}"
    );
}

#[tokio::test]
async fn snapshot_prefers_live_cache_when_player_is_ranked_there() {
    let state = test_state().await;
    let app = build_router(state.clone());

    let (_owner, cookie_owner) =
        register_and_login_default(app.clone(), "owner-scorefb2@x.test").await;
    let (_joiner, cookie_joiner) =
        register_and_login_default(app.clone(), "joiner-scorefb2@x.test").await;

    let (join_code, _project_id) = create_session(&app, &cookie_owner).await;
    let player_id = join_session(&app, &cookie_joiner, &join_code).await;

    // Seed the cached leaderboard with a live total for the player.
    {
        let entry = state
            .session_registry
            .get(&join_code)
            .expect("registry entry");
        let mut cache = entry.cache.write().expect("cache");
        cache.leaderboard = vec![arena_core::protocol::LeaderboardEntry {
            player_id: arena_core::protocol::PlayerId(player_id),
            display_name: "Player".to_string(),
            agent_display_name: None,
            total_points: 55,
            tests_passed: 1,
            total_wall_ms: 0,
        }];
    }

    let resp = app
        .clone()
        .oneshot(req_with_cookie(
            Method::GET,
            &format!("/api/sessions/{join_code}/players/{player_id}"),
            &cookie_joiner,
            None,
        ))
        .await
        .expect("snapshot resp");
    let (sc, body) = read_body_json(resp).await;
    assert_eq!(sc, StatusCode::OK, "snapshot: {body}");
    assert_eq!(
        body["score"].as_i64(),
        Some(55),
        "cache wins when it has the player: {body}"
    );
    assert_eq!(body["rank"].as_u64(), Some(1), "{body}");
}
