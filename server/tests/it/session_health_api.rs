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
