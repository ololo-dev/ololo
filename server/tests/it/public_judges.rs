//! `GET /api/public/judges` — the landing page's judge-panel block.
//! Unauthenticated; must list point-awarding judges only. Penalty judges
//! (negative rating scales — anti-cheat, golf-verify, from-scratch) are
//! fair-play controls, not reviews, and must never appear.

use crate::common;

use crate::common::{read_body_json, req, test_state};
use axum::http::{Method, StatusCode};
use sea_orm::{ActiveModelTrait, ActiveValue::Set};
use server::build_router;
use tower::ServiceExt;
use uuid::Uuid;

fn judge_row(slug: &str, name: &str, min: f64, max: f64) -> server::entities::judges::ActiveModel {
    let now = chrono::Utc::now();
    server::entities::judges::ActiveModel {
        id: Set(Uuid::new_v4()),
        slug: Set(slug.to_string()),
        name: Set(name.to_string()),
        description: Set(format!("{name} description")),
        prompt: Set("p".into()),
        rating_scale: Set(serde_json::json!({"min": min, "max": max, "step": 1.0})),
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
}

#[tokio::test]
async fn lists_quality_judges_and_skips_penalty_judges() {
    let state = test_state().await;
    let db = state.db.clone();
    let app = build_router(state);

    judge_row("correctness", "Correctness", 0.0, 10.0)
        .insert(&db)
        .await
        .expect("quality judge");
    judge_row("agentic", "Agentic", 0.0, 10.0)
        .insert(&db)
        .await
        .expect("second quality judge");
    judge_row("task-anti-cheat", "Task Anti-Cheat", -50.0, 0.0)
        .insert(&db)
        .await
        .expect("penalty judge");

    let resp = app
        .oneshot(req(Method::GET, "/api/public/judges", None, None))
        .await
        .expect("public judges resp");
    let (status, body) = read_body_json(resp).await;
    assert_eq!(status, StatusCode::OK, "public judges: {body}");

    let names: Vec<&str> = body["judges"]
        .as_array()
        .expect("judges array")
        .iter()
        .map(|j| j["name"].as_str().expect("name"))
        .collect();
    assert!(
        names.contains(&"Correctness"),
        "quality judge listed: {names:?}"
    );
    assert!(
        names.contains(&"Agentic"),
        "quality judge listed: {names:?}"
    );
    assert!(
        !names.contains(&"Task Anti-Cheat"),
        "penalty judge must be skipped: {names:?}"
    );
    // Sorted by name, and each entry carries slug + description for tooltips.
    let mut sorted = names.clone();
    sorted.sort();
    assert_eq!(names, sorted, "sorted by name");
    assert_eq!(body["judges"][0]["slug"], "agentic");
    assert_eq!(body["judges"][0]["description"], "Agentic description");
}
