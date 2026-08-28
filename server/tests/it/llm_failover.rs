//! `complete_with_failover` — the executor that turns a resolved candidate
//! list into one answer, walking past candidates that fail.
//!
//! Candidates are built directly here rather than through a pool: pool
//! *ordering* is covered in `llm_pools_api.rs`, and what matters here is
//! only what the executor does with an order it is handed.

use arena_core::llm::ModelConfig;
use arena_core::llm::telemetry::LlmContext;
use async_trait::async_trait;
use server::llm::{LlmError, LlmService, complete_with_failover};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::common;
use crate::common::test_state;

/// Fails for the named models, succeeds for anything else, and records the
/// models it was asked for so the walk order can be asserted.
struct FlakyLlm {
    failing: Vec<String>,
    calls: Mutex<Vec<String>>,
}

#[async_trait]
impl LlmService for FlakyLlm {
    async fn complete(
        &self,
        cfg: &ModelConfig,
        _system: &str,
        _user: &str,
    ) -> Result<String, LlmError> {
        self.calls.lock().unwrap().push(cfg.model.clone());
        if self.failing.iter().any(|m| m == &cfg.model) {
            return Err(LlmError::AiError(format!("{} is down", cfg.model)));
        }
        Ok(format!("answered by {}", cfg.model))
    }
}

fn candidate(model: &str) -> ModelConfig {
    ModelConfig {
        provider_name: None,
        provider: "ollama".into(),
        model: model.into(),
        base_url: None,
        api_key: None,
    }
}

async fn run(
    failing: &[&str],
    candidates: &[ModelConfig],
) -> (Result<String, LlmError>, Vec<String>) {
    let llm = Arc::new(FlakyLlm {
        failing: failing.iter().map(|s| s.to_string()).collect(),
        calls: Mutex::new(Vec::new()),
    });
    let state = test_state().await.with_llm_service(llm.clone());
    let result = complete_with_failover(
        &state,
        candidates,
        "project_ai",
        LlmContext::default(),
        "system",
        "user",
        Duration::from_secs(5),
    )
    .await;
    let calls = llm.calls.lock().unwrap().clone();
    (result, calls)
}

#[tokio::test]
async fn stops_at_the_first_candidate_that_answers() {
    let candidates = [candidate("a"), candidate("b"), candidate("c")];
    let (result, calls) = run(&[], &candidates).await;
    assert_eq!(result.unwrap(), "answered by a");
    assert_eq!(
        calls,
        vec!["a"],
        "a healthy first candidate must not cost extra calls"
    );
}

#[tokio::test]
async fn walks_past_failing_candidates_in_order() {
    let candidates = [candidate("a"), candidate("b"), candidate("c")];
    let (result, calls) = run(&["a", "b"], &candidates).await;
    assert_eq!(
        result.unwrap(),
        "answered by c",
        "failover must reach the healthy candidate"
    );
    assert_eq!(
        calls,
        vec!["a", "b", "c"],
        "candidates must be tried in the order resolution produced"
    );
}

#[tokio::test]
async fn surfaces_the_last_error_once_every_candidate_failed() {
    let candidates = [candidate("a"), candidate("b")];
    let (result, calls) = run(&["a", "b"], &candidates).await;
    let err = result.expect_err("a fully-down list must fail");
    // The real provider error survives, rather than being flattened into a
    // generic one — that message is what an admin debugs from.
    assert!(
        err.to_string().contains("b is down"),
        "expected the last candidate's error, got: {err}"
    );
    assert_eq!(calls, vec!["a", "b"], "every candidate must be tried once");
}

#[tokio::test]
async fn an_empty_candidate_list_fails_without_calling_the_provider() {
    let (result, calls) = run(&[], &[]).await;
    let err = result.expect_err("nothing configured must not look like success");
    assert!(
        err.to_string().contains("no_model_configured"),
        "unexpected error: {err}"
    );
    assert!(calls.is_empty());
}

#[tokio::test]
async fn each_attempt_lands_its_own_telemetry_row() {
    let candidates = [candidate("a"), candidate("b"), candidate("c")];
    let llm = Arc::new(FlakyLlm {
        failing: vec!["a".into(), "b".into()],
        calls: Mutex::new(Vec::new()),
    });
    let state = test_state().await.with_llm_service(llm);
    complete_with_failover(
        &state,
        &candidates,
        "project_ai",
        LlmContext::default(),
        "system",
        "user",
        Duration::from_secs(5),
    )
    .await
    .expect("c answers");

    use arena_core::entities::llm_requests;
    use sea_orm::EntityTrait;
    let rows = llm_requests::Entity::find()
        .all(&state.db)
        .await
        .expect("telemetry rows");
    // Failover is only debuggable if the skipped candidates are visible:
    // one row per attempt, two of them failed.
    assert_eq!(rows.len(), 3, "expected one row per attempt");
    let failed = rows.iter().filter(|r| r.status == "failed").count();
    assert_eq!(failed, 2, "the two dead candidates must be recorded");
}
