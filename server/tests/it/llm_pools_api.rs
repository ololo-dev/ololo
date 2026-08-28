//! Admin LLM pool CRUD + pool-backed candidate resolution (tiers, the
//! round-robin split within a tier, and what gets skipped).

use arena_core::llm::resolve::{LlmOverride, resolve_operation_candidates};
use axum::http::{Method, StatusCode};
use server::build_router;
use tower::ServiceExt;
use uuid::Uuid;

use crate::common;
use crate::common::{read_body_json, register_and_login_default, req_with_cookie, test_state};

async fn create_provider(app: &axum::Router, cookie: &str, name: &str) -> Uuid {
    let resp = app
        .clone()
        .oneshot(req_with_cookie(
            Method::POST,
            "/api/admin/llm/providers",
            cookie,
            Some(serde_json::json!({ "name": name, "kind": "ollama" })),
        ))
        .await
        .expect("create provider");
    let (sc, body) = read_body_json(resp).await;
    assert_eq!(sc, StatusCode::CREATED, "{body}");
    Uuid::parse_str(body["id"].as_str().unwrap()).unwrap()
}

async fn create_pool(
    app: &axum::Router,
    cookie: &str,
    body: serde_json::Value,
) -> (StatusCode, serde_json::Value) {
    let resp = app
        .clone()
        .oneshot(req_with_cookie(
            Method::POST,
            "/api/admin/llm/pools",
            cookie,
            Some(body),
        ))
        .await
        .expect("create pool");
    read_body_json(resp).await
}

/// Model ids of the resolved candidates, in order — the whole point of pool
/// resolution is that this order is meaningful.
async fn candidate_models(state: &server::state::AppState, operation: &str) -> Vec<String> {
    resolve_operation_candidates(
        &state.db,
        &state.settings_encryption,
        operation,
        &LlmOverride::none(),
    )
    .await
    .into_iter()
    .map(|c| c.model)
    .collect()
}

async fn assign_pool_as_default(app: &axum::Router, cookie: &str, pool_id: Uuid) {
    let resp = app
        .clone()
        .oneshot(req_with_cookie(
            Method::PUT,
            "/api/admin/llm/assignments",
            cookie,
            Some(serde_json::json!({ "default": { "pool_id": pool_id } })),
        ))
        .await
        .unwrap();
    let (sc, body) = read_body_json(resp).await;
    assert_eq!(sc, StatusCode::OK, "{body}");
    assert_eq!(
        body["default"]["pool_id"].as_str(),
        Some(pool_id.to_string().as_str()),
        "assignment must round-trip as a pool: {body}"
    );
}

#[tokio::test]
async fn pool_crud_validates_and_replaces_members() {
    let state = test_state().await;
    let app = build_router(state.clone());
    let (_id, cookie) = register_and_login_default(app.clone(), "pool-admin1@x.test").await;

    let p1 = create_provider(&app, &cookie, "P1").await;
    let p2 = create_provider(&app, &cookie, "P2").await;

    // Empty name is rejected.
    let (sc, _) = create_pool(&app, &cookie, serde_json::json!({ "name": "  " })).await;
    assert_eq!(sc, StatusCode::UNPROCESSABLE_ENTITY);

    // A member naming a provider that does not exist is rejected, rather
    // than being accepted and silently dropped at resolve time.
    let (sc, _) = create_pool(
        &app,
        &cookie,
        serde_json::json!({
            "name": "Ghost",
            "members": [{ "provider_id": Uuid::new_v4(), "model": "m" }],
        }),
    )
    .await;
    assert_eq!(sc, StatusCode::UNPROCESSABLE_ENTITY);

    // An empty model id is rejected for the same reason.
    let (sc, _) = create_pool(
        &app,
        &cookie,
        serde_json::json!({
            "name": "Blank",
            "members": [{ "provider_id": p1, "model": "  " }],
        }),
    )
    .await;
    assert_eq!(sc, StatusCode::UNPROCESSABLE_ENTITY);

    let (sc, pool) = create_pool(
        &app,
        &cookie,
        serde_json::json!({
            "name": "Fast",
            "description": "cheap first",
            "members": [
                { "provider_id": p1, "model": "a", "priority": 1 },
                { "provider_id": p2, "model": "b", "priority": 0 },
            ],
        }),
    )
    .await;
    assert_eq!(sc, StatusCode::CREATED, "{pool}");
    let pool_id = Uuid::parse_str(pool["id"].as_str().unwrap()).unwrap();
    // Members come back in resolution order, not insertion order.
    let models: Vec<&str> = pool["members"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["model"].as_str().unwrap())
        .collect();
    assert_eq!(models, vec!["b", "a"], "members must be priority-ordered");

    // Duplicate names would make two pools indistinguishable in the UI.
    let (sc, _) = create_pool(&app, &cookie, serde_json::json!({ "name": "Fast" })).await;
    assert_eq!(sc, StatusCode::CONFLICT);

    // PUT replaces the member list wholesale and leaves absent fields alone.
    let resp = app
        .clone()
        .oneshot(req_with_cookie(
            Method::PUT,
            &format!("/api/admin/llm/pools/{pool_id}"),
            &cookie,
            Some(serde_json::json!({
                "members": [{ "provider_id": p1, "model": "only", "priority": 5 }],
            })),
        ))
        .await
        .unwrap();
    let (sc, body) = read_body_json(resp).await;
    assert_eq!(sc, StatusCode::OK, "{body}");
    assert_eq!(body["name"].as_str(), Some("Fast"), "name must survive");
    assert_eq!(
        body["description"].as_str(),
        Some("cheap first"),
        "description must survive"
    );
    let members = body["members"].as_array().unwrap();
    assert_eq!(members.len(), 1, "member list is replaced, not merged");
    assert_eq!(members[0]["model"].as_str(), Some("only"));

    // Unknown pool → 404.
    let resp = app
        .clone()
        .oneshot(req_with_cookie(
            Method::GET,
            &format!("/api/admin/llm/pools/{}", Uuid::new_v4()),
            &cookie,
            None,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn pool_assignment_walks_tiers_and_rotates_within_one() {
    let state = test_state().await;
    let app = build_router(state.clone());
    let (_id, cookie) = register_and_login_default(app.clone(), "pool-admin2@x.test").await;

    let p1 = create_provider(&app, &cookie, "P1").await;
    let p2 = create_provider(&app, &cookie, "P2").await;
    let p3 = create_provider(&app, &cookie, "P3").await;

    // Two tiers: {a, b} share priority 0 and split the load; c is the
    // fallback tier reached only once tier 0 is exhausted.
    let (sc, pool) = create_pool(
        &app,
        &cookie,
        serde_json::json!({
            "name": "Tiered",
            "members": [
                { "provider_id": p1, "model": "a", "priority": 0 },
                { "provider_id": p2, "model": "b", "priority": 0 },
                { "provider_id": p3, "model": "c", "priority": 1 },
            ],
        }),
    )
    .await;
    assert_eq!(sc, StatusCode::CREATED, "{pool}");
    let pool_id = Uuid::parse_str(pool["id"].as_str().unwrap()).unwrap();
    assign_pool_as_default(&app, &cookie, pool_id).await;

    // Every member is a candidate, tier 0 before tier 1 — a failing tier-0
    // member must fall over to its tier partner before dropping to c.
    let first = candidate_models(&state, "judge").await;
    assert_eq!(first, vec!["a", "b", "c"]);

    // The next resolve starts tier 0 on the other member: that rotation is
    // what splits the load. The lower tier stays last either way.
    let second = candidate_models(&state, "judge").await;
    assert_eq!(second, vec!["b", "a", "c"]);

    let third = candidate_models(&state, "judge").await;
    assert_eq!(
        third,
        vec!["a", "b", "c"],
        "cursor wraps with the tier size"
    );

    // A disabled member drops out; the tier keeps working with what is left.
    let resp = app
        .clone()
        .oneshot(req_with_cookie(
            Method::PUT,
            &format!("/api/admin/llm/pools/{pool_id}"),
            &cookie,
            Some(serde_json::json!({
                "members": [
                    { "provider_id": p1, "model": "a", "priority": 0, "enabled": false },
                    { "provider_id": p2, "model": "b", "priority": 0 },
                    { "provider_id": p3, "model": "c", "priority": 1 },
                ],
            })),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(candidate_models(&state, "judge").await, vec!["b", "c"]);

    // Disabling the provider behind a member removes it too — a member can
    // never resolve through a provider the admin switched off.
    let resp = app
        .clone()
        .oneshot(req_with_cookie(
            Method::PUT,
            &format!("/api/admin/llm/providers/{p2}"),
            &cookie,
            Some(serde_json::json!({ "enabled": false })),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(candidate_models(&state, "judge").await, vec!["c"]);
}

#[tokio::test]
async fn judge_override_composes_pool_and_model_in_the_configured_order() {
    use arena_core::llm::resolve::{SOURCE_ORDER_MODEL_FIRST, SOURCE_ORDER_POOL_FIRST};

    let state = test_state().await;
    let app = build_router(state.clone());
    let (_id, cookie) = register_and_login_default(app.clone(), "pool-admin4@x.test").await;

    let p1 = create_provider(&app, &cookie, "P1").await;
    let pinned = create_provider(&app, &cookie, "Pinned").await;

    let (sc, pool) = create_pool(
        &app,
        &cookie,
        serde_json::json!({
            "name": "JudgePool",
            "members": [{ "provider_id": p1, "model": "pooled", "priority": 0 }],
        }),
    )
    .await;
    assert_eq!(sc, StatusCode::CREATED, "{pool}");
    let pool_id = Uuid::parse_str(pool["id"].as_str().unwrap()).unwrap();

    let models = |over: LlmOverride<'static>| {
        let state = state.clone();
        async move {
            resolve_operation_candidates(&state.db, &state.settings_encryption, "judge", &over)
                .await
                .into_iter()
                .map(|c| c.model)
                .collect::<Vec<_>>()
        }
    };

    // Both halves set: the order field decides which leads, and the other
    // trails as failover rather than being discarded.
    assert_eq!(
        models(LlmOverride::for_judge(
            Some(pool_id),
            Some(pinned),
            Some("pinned-model"),
            SOURCE_ORDER_POOL_FIRST,
        ))
        .await,
        vec!["pooled", "pinned-model"]
    );
    assert_eq!(
        models(LlmOverride::for_judge(
            Some(pool_id),
            Some(pinned),
            Some("pinned-model"),
            SOURCE_ORDER_MODEL_FIRST,
        ))
        .await,
        vec!["pinned-model", "pooled"]
    );

    // Either half alone works on its own.
    assert_eq!(
        models(LlmOverride::for_judge(
            Some(pool_id),
            None,
            None,
            SOURCE_ORDER_POOL_FIRST
        ))
        .await,
        vec!["pooled"]
    );
    assert_eq!(
        models(LlmOverride::for_judge(
            None,
            Some(pinned),
            Some("pinned-model"),
            SOURCE_ORDER_POOL_FIRST,
        ))
        .await,
        vec!["pinned-model"]
    );

    // An override naming a pool that no longer exists is inert, not fatal:
    // resolution falls through to the assignment chain (empty here).
    assert!(
        models(LlmOverride::for_judge(
            Some(Uuid::new_v4()),
            None,
            None,
            SOURCE_ORDER_POOL_FIRST,
        ))
        .await
        .is_empty()
    );
}

#[tokio::test]
async fn empty_pool_is_inert_and_deleting_one_clears_its_assignment() {
    let state = test_state().await;
    let app = build_router(state.clone());
    let (_id, cookie) = register_and_login_default(app.clone(), "pool-admin3@x.test").await;

    let p1 = create_provider(&app, &cookie, "P1").await;

    // An operation assigned to a pool with no usable members must fall
    // through to the default assignment rather than resolving to nothing.
    let (sc, empty) = create_pool(&app, &cookie, serde_json::json!({ "name": "Empty" })).await;
    assert_eq!(sc, StatusCode::CREATED, "{empty}");
    let empty_id = Uuid::parse_str(empty["id"].as_str().unwrap()).unwrap();

    let resp = app
        .clone()
        .oneshot(req_with_cookie(
            Method::PUT,
            "/api/admin/llm/assignments",
            &cookie,
            Some(serde_json::json!({
                "default": { "provider_id": p1, "model": "fallback" },
                "operations": { "judge": { "pool_id": empty_id } },
            })),
        ))
        .await
        .unwrap();
    let (sc, body) = read_body_json(resp).await;
    assert_eq!(sc, StatusCode::OK, "{body}");
    assert_eq!(
        candidate_models(&state, "judge").await,
        vec!["fallback"],
        "an empty pool must be inert, not fatal"
    );

    // Assigning a pool that does not exist is refused up front.
    let resp = app
        .clone()
        .oneshot(req_with_cookie(
            Method::PUT,
            "/api/admin/llm/assignments",
            &cookie,
            Some(serde_json::json!({ "default": { "pool_id": Uuid::new_v4() } })),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNPROCESSABLE_ENTITY);

    // Deleting a pool clears the assignments pointing at it, so no dangling
    // reference is left behind (mirrors provider deletion).
    let resp = app
        .clone()
        .oneshot(req_with_cookie(
            Method::DELETE,
            &format!("/api/admin/llm/pools/{empty_id}"),
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
    assert!(
        body["operations"]["judge"].is_null(),
        "assignment to a deleted pool must be gone: {body}"
    );
    assert_eq!(
        body["default"]["model"].as_str(),
        Some("fallback"),
        "unrelated assignments must survive: {body}"
    );
}
