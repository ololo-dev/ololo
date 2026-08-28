//! Admin LLM provider CRUD + assignments + operation-model resolution chain.

use axum::http::{Method, StatusCode};
use server::build_router;
use tower::ServiceExt;
use uuid::Uuid;

use crate::common;
use crate::common::{read_body_json, register_and_login_default, req_with_cookie, test_state};

async fn create_provider(
    app: &axum::Router,
    cookie: &str,
    name: &str,
    kind: &str,
    base_url: Option<&str>,
    api_key: Option<&str>,
) -> serde_json::Value {
    let mut body = serde_json::json!({ "name": name, "kind": kind });
    if let Some(u) = base_url {
        body["base_url"] = u.into();
    }
    if let Some(k) = api_key {
        body["api_key"] = k.into();
    }
    let resp = app
        .clone()
        .oneshot(req_with_cookie(
            Method::POST,
            "/api/admin/llm/providers",
            cookie,
            Some(body),
        ))
        .await
        .expect("create provider");
    let (sc, pb) = read_body_json(resp).await;
    assert_eq!(sc, StatusCode::CREATED, "{pb}");
    pb
}

#[tokio::test]
async fn provider_crud_masks_keys_and_validates() {
    let state = test_state().await;
    let app = build_router(state.clone());
    let (_id, cookie) = register_and_login_default(app.clone(), "llm-admin1@x.test").await;

    // openai_compatible without base_url is rejected.
    let resp = app
        .clone()
        .oneshot(req_with_cookie(
            Method::POST,
            "/api/admin/llm/providers",
            &cookie,
            Some(serde_json::json!({ "name": "NoBase", "kind": "openai_compatible" })),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNPROCESSABLE_ENTITY);

    let p = create_provider(
        &app,
        &cookie,
        "Router",
        "openrouter",
        None,
        Some("sk-secret"),
    )
    .await;
    assert_eq!(p["has_api_key"], true);
    assert!(
        p.get("api_key").is_none() && p.get("api_key_enc").is_none(),
        "key material must never be returned: {p}"
    );
    let pid = p["id"].as_str().unwrap().to_string();

    // Duplicate name → conflict.
    let resp = app
        .clone()
        .oneshot(req_with_cookie(
            Method::POST,
            "/api/admin/llm/providers",
            &cookie,
            Some(serde_json::json!({ "name": "Router", "kind": "openrouter" })),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CONFLICT);

    // Update without api_key keeps the stored key; clear_api_key removes it.
    let resp = app
        .clone()
        .oneshot(req_with_cookie(
            Method::PUT,
            &format!("/api/admin/llm/providers/{pid}"),
            &cookie,
            Some(serde_json::json!({ "name": "Router 2" })),
        ))
        .await
        .unwrap();
    let (sc, pb) = read_body_json(resp).await;
    assert_eq!(sc, StatusCode::OK, "{pb}");
    assert_eq!(pb["name"], "Router 2");
    assert_eq!(pb["has_api_key"], true);

    let resp = app
        .clone()
        .oneshot(req_with_cookie(
            Method::PUT,
            &format!("/api/admin/llm/providers/{pid}"),
            &cookie,
            Some(serde_json::json!({ "clear_api_key": true })),
        ))
        .await
        .unwrap();
    let (sc, pb) = read_body_json(resp).await;
    assert_eq!(sc, StatusCode::OK, "{pb}");
    assert_eq!(pb["has_api_key"], false);
}

#[tokio::test]
async fn assignments_roundtrip_and_delete_clears_references() {
    let state = test_state().await;
    let app = build_router(state.clone());
    let (_id, cookie) = register_and_login_default(app.clone(), "llm-admin2@x.test").await;

    let p = create_provider(&app, &cookie, "Local", "ollama", None, None).await;
    let pid = p["id"].as_str().unwrap().to_string();

    // Unknown operation rejected.
    let resp = app
        .clone()
        .oneshot(req_with_cookie(
            Method::PUT,
            "/api/admin/llm/assignments",
            &cookie,
            Some(serde_json::json!({
                "operations": { "nonsense": { "provider_id": pid, "model": "m" } }
            })),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNPROCESSABLE_ENTITY);

    // Set default + memory assignment.
    let resp = app
        .clone()
        .oneshot(req_with_cookie(
            Method::PUT,
            "/api/admin/llm/assignments",
            &cookie,
            Some(serde_json::json!({
                "default": { "provider_id": pid, "model": "llama3.2" },
                "operations": { "memory": { "provider_id": pid, "model": "qwen3" } }
            })),
        ))
        .await
        .unwrap();
    let (sc, body) = read_body_json(resp).await;
    assert_eq!(sc, StatusCode::OK, "{body}");
    assert_eq!(body["default"]["model"], "llama3.2");
    assert_eq!(body["operations"]["memory"]["model"], "qwen3");

    // Deleting the provider clears every assignment referencing it.
    let resp = app
        .clone()
        .oneshot(req_with_cookie(
            Method::DELETE,
            &format!("/api/admin/llm/providers/{pid}"),
            &cookie,
            None,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);

    let resp = app
        .clone()
        .oneshot(req_with_cookie(
            Method::GET,
            "/api/admin/llm/assignments",
            &cookie,
            None,
        ))
        .await
        .unwrap();
    let (sc, body) = read_body_json(resp).await;
    assert_eq!(sc, StatusCode::OK, "{body}");
    assert!(body["default"].is_null(), "{body}");
    assert!(body["operations"].get("memory").is_none(), "{body}");
}

#[tokio::test]
async fn resolution_chain_prefers_override_then_op_then_default() {
    let state = test_state().await;
    let app = build_router(state.clone());
    let (_id, cookie) = register_and_login_default(app.clone(), "llm-admin3@x.test").await;

    let default_p = create_provider(&app, &cookie, "DefaultP", "ollama", None, None).await;
    let op_p = create_provider(
        &app,
        &cookie,
        "OpP",
        "openai_compatible",
        Some("http://op.example/v1"),
        None,
    )
    .await;
    let judge_p = create_provider(&app, &cookie, "JudgeP", "openrouter", None, None).await;
    let default_id = Uuid::parse_str(default_p["id"].as_str().unwrap()).unwrap();
    let op_id = Uuid::parse_str(op_p["id"].as_str().unwrap()).unwrap();
    let judge_id = Uuid::parse_str(judge_p["id"].as_str().unwrap()).unwrap();

    let enc = &state.settings_encryption;
    let db = &state.db;
    use arena_core::llm::resolve::{LlmOverride, SOURCE_ORDER_POOL_FIRST, resolve_operation_model};

    // Nothing configured yet → None (legacy fallback path).
    assert!(
        resolve_operation_model(db, enc, "judge", &LlmOverride::none())
            .await
            .is_none()
    );

    // Default only → every operation resolves to the default.
    let resp = app
        .clone()
        .oneshot(req_with_cookie(
            Method::PUT,
            "/api/admin/llm/assignments",
            &cookie,
            Some(serde_json::json!({
                "default": { "provider_id": default_id, "model": "llama3.2" }
            })),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let cfg = resolve_operation_model(db, enc, "memory", &LlmOverride::none())
        .await
        .expect("default resolves");
    assert_eq!(
        (cfg.provider.as_str(), cfg.model.as_str()),
        ("ollama", "llama3.2")
    );

    // Operation assignment beats the default.
    let resp = app
        .clone()
        .oneshot(req_with_cookie(
            Method::PUT,
            "/api/admin/llm/assignments",
            &cookie,
            Some(serde_json::json!({
                "operations": { "judge": { "provider_id": op_id, "model": "op-model" } }
            })),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let cfg = resolve_operation_model(db, enc, "judge", &LlmOverride::none())
        .await
        .expect("op resolves");
    assert_eq!(
        (cfg.provider.as_str(), cfg.model.as_str()),
        ("custom", "op-model")
    );
    assert_eq!(cfg.base_url.as_deref(), Some("http://op.example/v1"));

    // Per-judge override beats the operation assignment.
    let cfg = resolve_operation_model(
        db,
        enc,
        "judge",
        &LlmOverride::for_judge(
            None,
            Some(judge_id),
            Some("judge-model"),
            SOURCE_ORDER_POOL_FIRST,
        ),
    )
    .await
    .expect("override resolves");
    assert_eq!(
        (cfg.provider.as_str(), cfg.model.as_str()),
        ("openrouter", "judge-model")
    );

    // A model with no provider is NOT an override: it used to be grafted
    // onto whatever provider the chain resolved, which made the effective
    // model depend on unrelated settings. It is now ignored outright and the
    // chain's own assignment stands.
    let cfg = resolve_operation_model(
        db,
        enc,
        "judge",
        &LlmOverride::for_judge(None, None, Some("swapped"), SOURCE_ORDER_POOL_FIRST),
    )
    .await
    .expect("chain still resolves");
    assert_eq!(
        (cfg.provider.as_str(), cfg.model.as_str()),
        ("custom", "op-model"),
        "a bare model must not override the operation assignment"
    );

    // A provider with no model is likewise inert.
    let cfg = resolve_operation_model(
        db,
        enc,
        "judge",
        &LlmOverride::for_judge(None, Some(judge_id), None, SOURCE_ORDER_POOL_FIRST),
    )
    .await
    .expect("chain still resolves");
    assert_eq!(
        (cfg.provider.as_str(), cfg.model.as_str()),
        ("custom", "op-model"),
        "a bare provider must not override the operation assignment"
    );

    // Disabling the op provider falls back to the default assignment.
    let resp = app
        .clone()
        .oneshot(req_with_cookie(
            Method::PUT,
            &format!("/api/admin/llm/providers/{op_id}"),
            &cookie,
            Some(serde_json::json!({ "enabled": false })),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let cfg = resolve_operation_model(db, enc, "judge", &LlmOverride::none())
        .await
        .expect("falls back to default");
    assert_eq!(
        (cfg.provider.as_str(), cfg.model.as_str()),
        ("ollama", "llama3.2")
    );
}

#[tokio::test]
async fn provider_test_validates_before_spending_tokens() {
    // The refusal paths must not depend on a live provider: a blank model
    // and a vanished row are answered locally, and only a well-formed
    // request would ever reach the network.
    let state = test_state().await;
    let app = build_router(state.clone());
    let (_id, cookie) = register_and_login_default(app.clone(), "llm-test-btn@x.test").await;

    let p = create_provider(
        &app,
        &cookie,
        "test-target",
        "openai_compatible",
        Some("http://127.0.0.1:9"),
        Some("k-test"),
    )
    .await;
    let id = p["id"].as_str().expect("id");

    let resp = app
        .clone()
        .oneshot(req_with_cookie(
            Method::POST,
            &format!("/api/admin/llm/providers/{id}/test"),
            &cookie,
            Some(serde_json::json!({ "model": "  " })),
        ))
        .await
        .expect("test call");
    let (sc, body) = read_body_json(resp).await;
    assert_eq!(sc, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert_eq!(body["error"], "model_required");

    let resp = app
        .clone()
        .oneshot(req_with_cookie(
            Method::POST,
            &format!("/api/admin/llm/providers/{}/test", Uuid::new_v4()),
            &cookie,
            Some(serde_json::json!({ "model": "m" })),
        ))
        .await
        .expect("test call");
    let (sc, body) = read_body_json(resp).await;
    assert_eq!(sc, StatusCode::NOT_FOUND, "{body}");

    // A real request against a dead endpoint comes back as a structured
    // failure, not an HTTP error: the admin reads the provider's words.
    let resp = app
        .clone()
        .oneshot(req_with_cookie(
            Method::POST,
            &format!("/api/admin/llm/providers/{id}/test"),
            &cookie,
            Some(serde_json::json!({ "model": "test-model" })),
        ))
        .await
        .expect("test call");
    let (sc, body) = read_body_json(resp).await;
    assert_eq!(sc, StatusCode::OK, "{body}");
    assert_eq!(body["ok"], false);
    assert_eq!(body["model"], "test-model");
    assert!(
        body["error"].as_str().is_some_and(|e| !e.is_empty()),
        "{body}"
    );
}
