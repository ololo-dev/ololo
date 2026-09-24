//! `GET /api/sessions/:id/health` and the snapshots: the same loader, the
//! session's own visibility rule, and a payload shaped for the chart.

use arena_core::entities::{app_settings, health_checkpoints, sessions};
use arena_core::protocol::SessionHealthPayload;
use axum::http::{Method, StatusCode};
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, Set};
use server::build_router;
use tower::ServiceExt;
use uuid::Uuid;

use crate::common::{read_body_json, register_and_login_default, req_with_cookie, test_state};

async fn session_with_player(
    app: &axum::Router,
    creator: &str,
    joiner: &str,
) -> (Uuid, String, Uuid) {
    let resp = app
        .clone()
        .oneshot(req_with_cookie(
            Method::POST,
            "/api/projects",
            creator,
            Some(serde_json::json!({ "name": format!("proj-{}", Uuid::new_v4()), "public": true })),
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
            Some(serde_json::json!({ "name": "s", "project_id": project_id })),
        ))
        .await
        .expect("create session");
    let (sc, sb) = read_body_json(resp).await;
    assert_eq!(sc, StatusCode::CREATED, "session: {sb}");
    let code = sb["join_code"].as_str().expect("join_code").to_string();
    let session_id: Uuid = sb["id"].as_str().expect("id").parse().unwrap();
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
    let player_id: Uuid = jb["player_id"]
        .as_str()
        .expect("player_id")
        .parse()
        .unwrap();
    (session_id, code, player_id)
}

async fn insert_checkpoint(
    db: &sea_orm::DatabaseConnection,
    session_id: Uuid,
    player_id: Uuid,
    seq: i32,
    score: f64,
) {
    let now = chrono::Utc::now();
    health_checkpoints::ActiveModel {
        id: Set(Uuid::new_v4()),
        session_id_fk: Set(session_id),
        player_id_fk: Set(player_id),
        task_id_fk: Set(None),
        probe_id_fk: Set(None),
        kind: Set("probe".into()),
        probe_seq: Set(seq),
        commit_sha: Set(format!("{seq:040}")),
        derived_task_id: Set(None),
        score_mismatch: Set(false),
        version_mismatch: Set(false),
        task_mismatch: Set(false),
        late: Set(false),
        history_rewritten: Set(false),
        client_status: Set(Some("ok".into())),
        client_score: Set(Some(score)),
        client_grade: Set(Some("B".into())),
        client_result: Set(None),
        client_duration_ms: Set(Some(10)),
        client_jscpd_version: Set(Some(ololo_health_version())),
        client_error: Set(None),
        client_reported_at: Set(Some(now)),
        server_status: Set("ok".into()),
        server_score: Set(Some(score)),
        server_grade: Set(Some("B".into())),
        server_result: Set(None),
        server_duration_ms: Set(Some(20)),
        server_jscpd_version: Set(Some(ololo_health_version())),
        server_error: Set(None),
        server_verified_at: Set(Some(now)),
        tests_status: Set(None),
        tests_result: Set(None),
        tests_reported_at: Set(None),
        created_at: Set(now),
        updated_at: Set(now),
    }
    .insert(db)
    .await
    .expect("checkpoint");
}

fn ololo_health_version() -> String {
    "0.1.16".to_string()
}

#[tokio::test]
async fn the_history_is_served_with_the_sessions_visibility_and_omitted_when_off() {
    let state = test_state().await;
    let app = build_router(state.clone());
    let (_, owner) = register_and_login_default(app.clone(), "own-h@x.test").await;
    let (_, joiner) = register_and_login_default(app.clone(), "join-h@x.test").await;
    let (_, stranger) = register_and_login_default(app.clone(), "str-h@x.test").await;
    let (session_id, code, player_id) = session_with_player(&app, &owner, &joiner).await;

    // Health off and no checkpoints: the snapshot loader says nothing, the
    // endpoint answers with an empty payload.
    assert!(
        server::api::sessions::load_session_health(&state.db, session_id, None)
            .await
            .is_none()
    );
    let uri = format!("/api/sessions/{session_id}/health");
    let resp = app
        .clone()
        .oneshot(req_with_cookie(Method::GET, &uri, &stranger, None))
        .await
        .expect("get");
    let (sc, body) = read_body_json(resp).await;
    assert_eq!(sc, StatusCode::OK, "{body}");
    let payload: SessionHealthPayload = serde_json::from_value(body).unwrap();
    assert!(payload.players.is_empty());
    assert_eq!(
        payload.thresholds,
        arena_core::protocol::Thresholds::default()
    );

    // Checkpoints exist: they are served, oldest first, under the session's
    // start-relative clock.
    sessions::Entity::update_many()
        .col_expr(
            sessions::Column::StartedAt,
            sea_orm::sea_query::Expr::value(Some(
                chrono::Utc::now() - chrono::Duration::seconds(60),
            )),
        )
        .filter(sessions::Column::Id.eq(session_id))
        .exec(&state.db)
        .await
        .unwrap();
    insert_checkpoint(&state.db, session_id, player_id, 1, 80.0).await;
    insert_checkpoint(&state.db, session_id, player_id, 2, 66.0).await;
    let resp = app
        .clone()
        .oneshot(req_with_cookie(Method::GET, &uri, &stranger, None))
        .await
        .expect("get");
    let (sc, body) = read_body_json(resp).await;
    assert_eq!(sc, StatusCode::OK, "{body}");
    let payload: SessionHealthPayload = serde_json::from_value(body).unwrap();
    let mine = &payload.players[&arena_core::protocol::PlayerId(player_id)];
    assert_eq!(mine.checkpoints.len(), 2);
    assert_eq!(mine.checkpoints[0].probe_seq, 1);
    assert_eq!(mine.checkpoints[1].score, Some(66.0));
    assert_eq!(
        mine.checkpoints[1].level,
        arena_core::protocol::Level::Amber
    );
    assert!(mine.checkpoints[0].t.unwrap() >= 59.0);
    // A player-narrowed load carries only that player — and nothing at all
    // for a player without checkpoints while health is off.
    let one = server::api::sessions::load_session_health(&state.db, session_id, Some(player_id))
        .await
        .expect("this player has checkpoints");
    assert_eq!(one.players.len(), 1);
    assert!(
        server::api::sessions::load_session_health(&state.db, session_id, Some(Uuid::new_v4()))
            .await
            .is_none()
    );

    // Health on, no rows: the loader still answers (an empty chart is drawn).
    app_settings::ActiveModel {
        key: Set(arena_core::health_settings::HEALTH_ENABLED_KEY.into()),
        value: Set("true".into()),
    }
    .insert(&state.db)
    .await
    .unwrap();
    health_checkpoints::Entity::delete_many()
        .filter(health_checkpoints::Column::SessionIdFk.eq(session_id))
        .exec(&state.db)
        .await
        .unwrap();
    let on = server::api::sessions::load_session_health(&state.db, session_id, None)
        .await
        .expect("health is on");
    assert!(on.players.is_empty());

    // An unknown session is not found; the rule is the session's own.
    let resp = app
        .clone()
        .oneshot(req_with_cookie(
            Method::GET,
            &format!("/api/sessions/{}/health", Uuid::new_v4()),
            &stranger,
            None,
        ))
        .await
        .expect("get");
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    let _ = code;
}

/// A tree's result as a verifier stores it: jscpd's dimensions included,
/// so the suite's dimensions can be composed in.
fn tree_result(score: f64) -> serde_json::Value {
    serde_json::json!({
        "schema": 2, "jscpd_version": "0.1.17", "score": score, "grade": "B", "level": "green",
        "health": {"score": score, "grade": "B", "dimensions": [
            {"id": "duplication", "source": "jscpd", "weight": 1.0, "score": score},
            {"id": "complexity", "source": "jscpd", "weight": 1.0, "score": score}
        ]},
        "metrics": {"files": 2, "code_lines": 40, "clones": 0,
                    "ignore_markers": 0, "jscpd_config_present": false},
        "duration_ms": 5
    })
}

#[tokio::test]
async fn the_history_composes_the_last_completed_test_run_into_every_later_checkpoint() {
    use arena_core::protocol::{
        PlayerId, SuiteResult, TestCounts, TestReportPayload, TestRunStatus,
    };
    let state = test_state().await;
    let app = build_router(state.clone());
    let (_, owner) = register_and_login_default(app.clone(), "own-t@x.test").await;
    let (_, joiner) = register_and_login_default(app.clone(), "join-t@x.test").await;
    let (session_id, _code, player_id) = session_with_player(&app, &owner, &joiner).await;

    let run = |seq: u32, status: TestRunStatus, failed: u64| TestReportPayload {
        probe_id: Uuid::new_v4(),
        probe_seq: seq,
        task_id: None,
        commit: format!("{seq:040}"),
        status,
        command: "npm test".into(),
        coverage_run: false,
        result: (status == TestRunStatus::Ok).then(|| SuiteResult {
            exit_code: Some(i32::from(failed > 0)),
            counts: Some(TestCounts {
                passed: 10 - failed,
                failed,
                skipped: 0,
            }),
            ..SuiteResult::default()
        }),
        error: (status != TestRunStatus::Ok).then(|| "timed out after 300s".to_string()),
        duration_ms: 900,
        log: Some(format!(".ololo/probes/{seq:04}-tests.log")),
    };
    // #1 before any run, #2 with one test in ten failing, #3 untested
    // (unchanged code), #4 whose own run timed out.
    let base = chrono::Utc::now() - chrono::Duration::seconds(40);
    for (seq, tests) in [
        (1, None),
        (2, Some(run(2, TestRunStatus::Ok, 1))),
        (3, None),
        (4, Some(run(4, TestRunStatus::Timeout, 0))),
    ] {
        let at = base + chrono::Duration::seconds(i64::from(seq) * 5);
        health_checkpoints::ActiveModel {
            id: Set(Uuid::new_v4()),
            session_id_fk: Set(session_id),
            player_id_fk: Set(player_id),
            task_id_fk: Set(None),
            probe_id_fk: Set(None),
            kind: Set("probe".into()),
            probe_seq: Set(seq),
            commit_sha: Set(format!("{seq:040}")),
            derived_task_id: Set(None),
            score_mismatch: Set(false),
            version_mismatch: Set(false),
            task_mismatch: Set(false),
            late: Set(false),
            history_rewritten: Set(false),
            client_status: Set(Some("ok".into())),
            client_score: Set(Some(80.0)),
            client_grade: Set(Some("B".into())),
            client_result: Set(Some(tree_result(80.0))),
            client_duration_ms: Set(Some(10)),
            client_jscpd_version: Set(Some(ololo_health_version())),
            client_error: Set(None),
            client_reported_at: Set(Some(at)),
            server_status: Set("ok".into()),
            server_score: Set(Some(80.0)),
            server_grade: Set(Some("B".into())),
            server_result: Set(Some(tree_result(80.0))),
            server_duration_ms: Set(Some(20)),
            server_jscpd_version: Set(Some(ololo_health_version())),
            server_error: Set(None),
            server_verified_at: Set(Some(at)),
            tests_status: Set(tests.as_ref().map(|t| t.status.as_str().to_string())),
            tests_result: Set(tests.map(|t| serde_json::to_value(t).unwrap())),
            tests_reported_at: Set(None),
            created_at: Set(at),
            updated_at: Set(at),
        }
        .insert(&state.db)
        .await
        .expect("checkpoint");
    }
    arena_core::entities::player_test_commands::ActiveModel {
        id: Set(Uuid::new_v4()),
        session_id_fk: Set(session_id),
        player_id_fk: Set(player_id),
        test_command: Set(Some("npm test".into())),
        coverage_command: Set(None),
        sources: Set(r#"["README.md"]"#.into()),
        source_hash: Set("h".into()),
        created_at: Set(chrono::Utc::now()),
        updated_at: Set(chrono::Utc::now()),
    }
    .insert(&state.db)
    .await
    .expect("commands");

    let payload = server::api::sessions::load_session_health(&state.db, session_id, None)
        .await
        .expect("checkpoints exist");
    let mine = &payload.players[&PlayerId(player_id)];
    let [first, second, third, fourth] = &mine.checkpoints[..] else {
        panic!("four checkpoints: {:?}", mine.checkpoints);
    };
    assert_eq!(first.score, Some(80.0), "no run yet: the tree alone");
    assert!(first.tests.is_none());
    // 80, 80 and the tests' 50 (one in ten failing).
    assert_eq!(second.score, Some(68.4));
    let counted = second.tests.as_ref().unwrap().counted.as_ref().unwrap();
    assert!(!counted.inherited);
    assert_eq!(counted.log.as_deref(), Some(".ololo/probes/0002-tests.log"));
    assert_eq!(
        third.score,
        Some(68.4),
        "the run carries over to untested code"
    );
    assert!(
        third
            .tests
            .as_ref()
            .unwrap()
            .counted
            .as_ref()
            .unwrap()
            .inherited
    );
    let fourth_tests = fourth.tests.as_ref().unwrap();
    assert_eq!(
        fourth_tests.attempt.as_ref().map(|a| a.status),
        Some(TestRunStatus::Timeout)
    );
    assert_eq!(fourth_tests.counted.as_ref().unwrap().probe_seq, 2);
    let commands = mine.test_commands.as_ref().expect("the docs were read");
    assert_eq!(commands.test.as_deref(), Some("npm test"));
    assert_eq!(commands.sources, vec!["README.md".to_string()]);
}
