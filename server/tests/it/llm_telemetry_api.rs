//! Admin unified LLM telemetry API: record + list + filter + pagination.

use axum::http::{Method, StatusCode};
use chrono::{Duration, Utc};
use sea_orm::{ActiveModelTrait, ActiveValue::Set, DatabaseConnection, EntityTrait};
use server::build_router;
use tower::ServiceExt;
use uuid::Uuid;

use arena_core::entities::llm_requests;
use arena_core::llm::telemetry::{LlmContext, LlmRequestRecord, record_llm_request};

use crate::common;
use crate::common::{read_body_json, register_and_login_default, req_with_cookie, test_state};

fn model_cfg(model: &str) -> arena_core::llm::ModelConfig {
    arena_core::llm::ModelConfig {
        provider_name: None,
        provider: "ollama".to_string(),
        model: model.to_string(),
        base_url: None,
        api_key: None,
    }
}

/// Insert one telemetry row with an explicit timestamp so list ordering and
/// pagination are deterministic (rows land newest-first).
async fn insert_row(
    db: &DatabaseConnection,
    operation: &str,
    status: &str,
    model: &str,
    secs_ago: i64,
) -> Uuid {
    let id = Uuid::new_v4();
    llm_requests::ActiveModel {
        id: Set(id),
        operation: Set(operation.to_string()),
        provider: Set("ollama".to_string()),
        provider_name: Set(Some("Local Ollama".to_string())),
        model: Set(model.to_string()),
        status: Set(status.to_string()),
        error: Set((status == "failed").then(|| "boom".to_string())),
        tokens_input: Set(10),
        tokens_output: Set(20),
        tokens_cache_read: Set(0),
        tokens_cache_write: Set(0),
        duration_ms: Set(123),
        session_id: Set(None),
        player_id: Set(None),
        task_id: Set(None),
        judge_slug: Set(None),
        detail_json: Set(None),
        events_json: Set(Some(
            r#"[{"at_ms":1,"kind":"llm","duration_ms":5,"input":"sys+user","output":"hi"}]"#
                .to_string(),
        )),
        created_at: Set(Utc::now() - Duration::seconds(secs_ago)),
    }
    .insert(db)
    .await
    .expect("insert llm_requests row");
    id
}

#[tokio::test]
async fn telemetry_list_filters_and_paginates() {
    let state = test_state().await;
    let app = build_router(state.clone());
    let (_id, cookie) = register_and_login_default(app.clone(), "llm-tel1@x.test").await;

    // Oldest → newest: judge, memory, memory, adaptation, judge(failed).
    insert_row(&state.db, "judge", "ok", "m-old", 50).await;
    insert_row(&state.db, "memory", "ok", "m-mem1", 40).await;
    insert_row(&state.db, "memory", "failed", "m-mem2", 30).await;
    insert_row(&state.db, "adaptation", "ok", "m-adapt", 20).await;
    let newest = insert_row(&state.db, "judge", "failed", "m-new", 10).await;

    // Full list, newest first.
    let resp = app
        .clone()
        .oneshot(req_with_cookie(
            Method::GET,
            "/api/admin/llm/telemetry",
            &cookie,
            None,
        ))
        .await
        .unwrap();
    let (sc, body) = read_body_json(resp).await;
    assert_eq!(sc, StatusCode::OK, "{body}");
    assert_eq!(body["total"], 5, "{body}");
    let items = body["items"].as_array().expect("items array");
    assert_eq!(items.len(), 5);
    assert_eq!(items[0]["id"], newest.to_string(), "newest first");
    assert_eq!(items[0]["model"], "m-new");
    assert_eq!(items[4]["model"], "m-old", "oldest last");

    // Filter by operation.
    let resp = app
        .clone()
        .oneshot(req_with_cookie(
            Method::GET,
            "/api/admin/llm/telemetry?operation=memory",
            &cookie,
            None,
        ))
        .await
        .unwrap();
    let (sc, body) = read_body_json(resp).await;
    assert_eq!(sc, StatusCode::OK, "{body}");
    assert_eq!(body["total"], 2, "{body}");
    let items = body["items"].as_array().unwrap();
    assert!(items.iter().all(|i| i["operation"] == "memory"), "{body}");
    assert_eq!(items[0]["model"], "m-mem2", "newer memory row first");
    // The per-turn trace is a detail-only field: a list of 50 traces would
    // be megabytes, so list rows must omit it.
    assert!(
        items.iter().all(|i| i["events_json"].is_null()),
        "list must not carry traces: {body}"
    );

    // Filter by status.
    let resp = app
        .clone()
        .oneshot(req_with_cookie(
            Method::GET,
            "/api/admin/llm/telemetry?status=failed",
            &cookie,
            None,
        ))
        .await
        .unwrap();
    let (sc, body) = read_body_json(resp).await;
    assert_eq!(sc, StatusCode::OK, "{body}");
    assert_eq!(body["total"], 2, "{body}");

    // Combined filter.
    let resp = app
        .clone()
        .oneshot(req_with_cookie(
            Method::GET,
            "/api/admin/llm/telemetry?operation=judge&status=failed",
            &cookie,
            None,
        ))
        .await
        .unwrap();
    let (sc, body) = read_body_json(resp).await;
    assert_eq!(sc, StatusCode::OK, "{body}");
    assert_eq!(body["total"], 1, "{body}");
    assert_eq!(body["items"][0]["model"], "m-new");

    // Pagination: total stays the full match count.
    let resp = app
        .clone()
        .oneshot(req_with_cookie(
            Method::GET,
            "/api/admin/llm/telemetry?limit=2&offset=2",
            &cookie,
            None,
        ))
        .await
        .unwrap();
    let (sc, body) = read_body_json(resp).await;
    assert_eq!(sc, StatusCode::OK, "{body}");
    assert_eq!(body["total"], 5, "total is unaffected by paging: {body}");
    let items = body["items"].as_array().unwrap();
    assert_eq!(items.len(), 2);
    assert_eq!(items[0]["model"], "m-mem2", "page 2 starts at the 3rd row");
    assert_eq!(items[1]["model"], "m-mem1");
}

#[tokio::test]
async fn telemetry_detail_roundtrips_and_bounds_error() {
    let state = test_state().await;
    let app = build_router(state.clone());
    let (_id, cookie) = register_and_login_default(app.clone(), "llm-tel2@x.test").await;

    // Record through the shared helper — the 5000-char error must be
    // truncated to the 1KB cap at insert time.
    let long_error = "e".repeat(5000);
    record_llm_request(
        &state.db,
        LlmRequestRecord::new(
            "project_ai",
            &model_cfg("qwen3"),
            LlmContext {
                session_id: Some(Uuid::new_v4()),
                ..Default::default()
            },
        )
        .with_tokens(11, 22, 3, 4)
        .with_duration_ms(777)
        .with_detail_json(arena_core::llm::telemetry::completion_detail_json(
            "sys", "user", None,
        ))
        .with_events(&{
            // Exercise the real trace-serialization path rather than a
            // hand-written JSON blob.
            let rec = arena_core::judging::JudgeRunRecorder::default();
            rec.record(arena_core::judging::JudgeLogEvent {
                at_ms: 1,
                kind: "llm".to_string(),
                duration_ms: 5,
                input: Some("sys+user".to_string()),
                output: Some("hi".to_string()),
                ..Default::default()
            });
            rec
        })
        .failed(&long_error),
    )
    .await;

    let row = llm_requests::Entity::find()
        .one(&state.db)
        .await
        .expect("query")
        .expect("row recorded");

    let resp = app
        .clone()
        .oneshot(req_with_cookie(
            Method::GET,
            &format!("/api/admin/llm/telemetry/{}", row.id),
            &cookie,
            None,
        ))
        .await
        .unwrap();
    let (sc, body) = read_body_json(resp).await;
    assert_eq!(sc, StatusCode::OK, "{body}");
    assert_eq!(body["operation"], "project_ai");
    assert_eq!(body["provider"], "ollama");
    assert_eq!(body["model"], "qwen3");
    assert_eq!(body["status"], "failed");
    assert_eq!(body["tokens_input"], 11);
    assert_eq!(body["tokens_output"], 22);
    assert_eq!(body["tokens_cache_read"], 3);
    assert_eq!(body["tokens_cache_write"], 4);
    assert_eq!(body["duration_ms"], 777);
    assert!(body["session_id"].is_string());
    assert!(body["player_id"].is_null());
    // …while the single-row read carries the full trace.
    let events: serde_json::Value =
        serde_json::from_str(body["events_json"].as_str().expect("trace on detail read"))
            .expect("trace parses");
    assert_eq!(events[0]["kind"], "llm", "{events}");
    let err = body["error"].as_str().expect("error text");
    assert!(
        err.chars().count() <= 1024 + "…[truncated]".chars().count(),
        "error bounded to ~1KB, got {} chars",
        err.chars().count()
    );
    assert!(err.starts_with("eee"));
    let detail: serde_json::Value =
        serde_json::from_str(body["detail_json"].as_str().expect("detail")).expect("detail json");
    assert_eq!(detail["system_chars"], 3);
    assert_eq!(detail["user_chars"], 4);

    // Unknown id → 404.
    let resp = app
        .clone()
        .oneshot(req_with_cookie(
            Method::GET,
            &format!("/api/admin/llm/telemetry/{}", Uuid::new_v4()),
            &cookie,
            None,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

// ───────────────── Context labels (session / player / task) ─────────────────

/// A telemetry row carries uuids; the API resolves them to names so the admin
/// UI can show a join code, a username and a task title instead.
#[tokio::test]
async fn telemetry_resolves_context_ids_to_names() {
    use arena_core::entities::{players, projects, sessions, tasks, users};
    use arena_core::session_status::SessionStatus;

    let state = test_state().await;
    let app = build_router(state.clone());
    let (user_id, cookie) = register_and_login_default(app.clone(), "tele-ctx@x.test").await;
    let now = Utc::now();

    // The admin who logged in doubles as the player's account.
    let mut u: users::ActiveModel = users::Entity::find_by_id(user_id)
        .one(&state.db)
        .await
        .expect("find user")
        .expect("user exists")
        .into();
    u.username = Set(Some("racer".to_string()));
    u.update(&state.db).await.expect("set username");

    let project_id = Uuid::new_v4();
    projects::ActiveModel {
        id: Set(project_id),
        name: Set("P".to_string()),
        slug: Set(None),
        description: Set(String::new()),
        category: Set(None),
        tags: Set(String::new()),
        cover_image_url: Set(None),
        owner_user_id_fk: Set(user_id),
        public: Set(true),
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
    .expect("project");

    let session_id = Uuid::new_v4();
    sessions::ActiveModel {
        id: Set(session_id),
        name: Set("S".to_string()),
        created_at: Set(now),
        owner_id_fk: Set(None),
        status: Set(SessionStatus::Running),
        join_code: Set("ZZ9XQ2".to_string()),
        game_server_id: Set(None),
        cancel_reason: Set(None),
        cancelled_by: Set(None),
        started_at: Set(Some(now)),
        finished_at: Set(None),
        paused_at: Set(None),
        paused_duration_secs: Set(None),
        project_id_fk: Set(project_id),
    }
    .insert(&state.db)
    .await
    .expect("session");

    let task_id = Uuid::new_v4();
    tasks::ActiveModel {
        id: Set(task_id),
        project_id_fk: Set(project_id),
        ordinal: Set(0),
        title: Set("Reverse a linked list".to_string()),
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
    .insert(&state.db)
    .await
    .expect("task");

    // Two players: one signed in (linkable), one anonymous (name only).
    let signed_in = Uuid::new_v4();
    let anonymous = Uuid::new_v4();
    for (pid, uid, name) in [
        (signed_in, Some(user_id), "Racer"),
        (anonymous, None, "Guest Bot"),
    ] {
        players::ActiveModel {
            id: Set(pid),
            session_id_fk: Set(session_id),
            user_id_fk: Set(uid),
            display_name: Set(name.to_string()),
            fingerprint: Set(None),
            metadata_json: Set(None),
            joined_at: Set(now),
            reconnected_at: Set(None),
            revoked_at: Set(None),
            agent_connected: Set(false),
            agent_last_seen_at: Set(None),
        }
        .insert(&state.db)
        .await
        .expect("player");
    }

    let mut ids = Vec::new();
    for player_id in [signed_in, anonymous] {
        let id = Uuid::new_v4();
        llm_requests::ActiveModel {
            id: Set(id),
            operation: Set("judge".to_string()),
            provider: Set("custom".to_string()),
            provider_name: Set(Some("OpenCode Zen".to_string())),
            model: Set("glm-5-free".to_string()),
            status: Set("ok".to_string()),
            error: Set(None),
            tokens_input: Set(1),
            tokens_output: Set(1),
            tokens_cache_read: Set(0),
            tokens_cache_write: Set(0),
            duration_ms: Set(5),
            session_id: Set(Some(session_id)),
            player_id: Set(Some(player_id)),
            task_id: Set(Some(task_id)),
            judge_slug: Set(None),
            detail_json: Set(None),
            events_json: Set(None),
            created_at: Set(now),
        }
        .insert(&state.db)
        .await
        .expect("telemetry row");
        ids.push(id);
    }

    let resp = app
        .clone()
        .oneshot(req_with_cookie(
            Method::GET,
            "/api/admin/llm/telemetry",
            &cookie,
            None,
        ))
        .await
        .unwrap();
    let (sc, body) = read_body_json(resp).await;
    assert_eq!(sc, StatusCode::OK, "{body}");

    let rows = body["items"].as_array().expect("items");
    let find = |pid: Uuid| {
        rows.iter()
            .find(|r| r["player_id"].as_str() == Some(pid.to_string().as_str()))
            .unwrap_or_else(|| panic!("row for {pid} missing: {body}"))
    };

    let row = find(signed_in);
    // The registry id alone would read "custom" for every OpenAI-compatible
    // endpoint; the row's own name is what tells them apart.
    assert_eq!(row["provider"].as_str(), Some("custom"));
    assert_eq!(row["provider_name"].as_str(), Some("OpenCode Zen"));
    assert_eq!(row["session_code"].as_str(), Some("ZZ9XQ2"));
    assert_eq!(row["player_name"].as_str(), Some("Racer"));
    assert_eq!(row["player_username"].as_str(), Some("racer"));
    assert_eq!(row["task_title"].as_str(), Some("Reverse a linked list"));

    // An anonymous player has a name to show but no account to link to.
    let anon = find(anonymous);
    assert_eq!(anon["player_name"].as_str(), Some("Guest Bot"));
    assert!(anon["player_username"].is_null(), "{anon}");

    // The single-record read resolves the same way.
    let resp = app
        .clone()
        .oneshot(req_with_cookie(
            Method::GET,
            &format!("/api/admin/llm/telemetry/{}", ids[0]),
            &cookie,
            None,
        ))
        .await
        .unwrap();
    let (sc, one) = read_body_json(resp).await;
    assert_eq!(sc, StatusCode::OK, "{one}");
    assert_eq!(one["session_code"].as_str(), Some("ZZ9XQ2"));
    assert_eq!(one["task_title"].as_str(), Some("Reverse a linked list"));
}

/// Telemetry outlives what it points at, so a vanished target must leave the
/// label empty rather than fail the request.
#[tokio::test]
async fn telemetry_context_labels_tolerate_missing_targets() {
    let state = test_state().await;
    let app = build_router(state.clone());
    let (_id, cookie) = register_and_login_default(app.clone(), "tele-ctx2@x.test").await;

    llm_requests::ActiveModel {
        id: Set(Uuid::new_v4()),
        operation: Set("judge".to_string()),
        provider: Set("ollama".to_string()),
        provider_name: Set(None),
        model: Set("m".to_string()),
        status: Set("ok".to_string()),
        error: Set(None),
        tokens_input: Set(0),
        tokens_output: Set(0),
        tokens_cache_read: Set(0),
        tokens_cache_write: Set(0),
        duration_ms: Set(1),
        session_id: Set(Some(Uuid::new_v4())),
        player_id: Set(Some(Uuid::new_v4())),
        task_id: Set(Some(Uuid::new_v4())),
        judge_slug: Set(None),
        detail_json: Set(None),
        events_json: Set(None),
        created_at: Set(Utc::now()),
    }
    .insert(&state.db)
    .await
    .expect("orphan telemetry row");

    let resp = app
        .clone()
        .oneshot(req_with_cookie(
            Method::GET,
            "/api/admin/llm/telemetry",
            &cookie,
            None,
        ))
        .await
        .unwrap();
    let (sc, body) = read_body_json(resp).await;
    assert_eq!(sc, StatusCode::OK, "{body}");
    let row = &body["items"][0];
    assert!(row["session_code"].is_null(), "{row}");
    assert!(row["player_name"].is_null(), "{row}");
    assert!(row["task_title"].is_null(), "{row}");
    // The raw ids survive so the record is still traceable.
    assert!(row["session_id"].is_string(), "{row}");
}
