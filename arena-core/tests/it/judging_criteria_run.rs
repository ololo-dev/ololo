//! Criteria judges through `run_judge`: the sheet is validated, weight-
//! averaged, mapped onto the panel-share scale, and stored whole.

use crate::common;
use crate::common::*;

use arena_core::entities::judge_results;
use arena_core::judging::criteria::CriteriaContext;
use arena_core::judging::{AgentResponse, run_judge};
use arena_core::validation::judge_results::RatingScale;
use sea_orm::EntityTrait;
use std::collections::BTreeMap;

fn ctx() -> CriteriaContext {
    let mut weights = BTreeMap::new();
    weights.insert("product".to_string(), 0.6);
    weights.insert("tests".to_string(), 0.4);
    CriteriaContext {
        keys: vec!["product".to_string(), "tests".to_string()],
        weights,
    }
}

/// The judge's panel share of a 200-point open-ended task: 0..80, step 1.
fn share_scale() -> RatingScale {
    RatingScale {
        min: 0.0,
        max: 80.0,
        step: 1.0,
    }
}

#[tokio::test]
async fn criteria_sheet_is_scored_mapped_and_stored_whole() {
    let db = setup_db().await;
    let owner = insert_user(&db).await;
    let project = insert_project(&db, owner).await;
    let session = insert_session(&db, project).await;
    let player = insert_player(&db, session).await;
    let (task_id, judge_id, task_judge_id) = insert_chain(&db, project, session, player).await;
    let (tj, judge, task) = row_snapshots(task_id, judge_id, task_judge_id);

    let dir = tempfile::tempdir().unwrap();
    make_repo(dir.path());
    write_file(dir.path(), "a.txt", "x");
    commit(dir.path(), "init");

    // product 10.0 (w 0.6) + tests 5.0 (w 0.4) → overall 8.0 → 80% of the
    // 0..80 share = 64 points.
    let llm = FakeJudgeLlm::new(vec![AgentResponse::Final {
        text: r#"{"criteria": [
            {"key": "product", "score": 10.0, "rationale": "matches the brief",
             "evidence": ["commit:abc123"]},
            {"key": "tests", "score": 5.0, "rationale": "half the paths covered"}
        ], "feedback": "solid build"}"#
            .to_string(),
    }]);

    let out = run_judge(
        &db,
        &llm,
        dir.path(),
        session,
        player,
        task_id,
        None,
        None,
        &tj,
        &judge,
        &task,
        &[],
        None,
        &[],
        "llama3.2",
        "ollama",
        None,
        &share_scale(),
        None,
        None,
        Some(&ctx()),
        None,
        &[],
    )
    .await
    .expect("run_judge");

    assert_eq!(out.rating, 64.0, "8.0/10 of the 0..80 share");
    assert_eq!(out.point_delta, 64);
    assert_eq!(out.feedback, "solid build");

    let row = judge_results::Entity::find_by_id(out.judge_result_id)
        .one(&db)
        .await
        .expect("query")
        .expect("row");
    assert_eq!(row.point_delta, 64);
    // The sheet is stored whole, not collapsed to a number.
    assert_eq!(row.rating["overall"], serde_json::json!(64.0));
    assert_eq!(row.rating["criteria"][0]["key"], "product");
    assert_eq!(row.rating["criteria"][0]["evidence"][0], "commit:abc123");
    assert_eq!(row.verdict_kind.as_deref(), Some("full"));
}

#[tokio::test]
async fn null_criteria_renormalize_and_all_null_scores_zero() {
    let db = setup_db().await;
    let owner = insert_user(&db).await;
    let project = insert_project(&db, owner).await;
    let session = insert_session(&db, project).await;
    let player = insert_player(&db, session).await;
    let (task_id, judge_id, task_judge_id) = insert_chain(&db, project, session, player).await;
    let (tj, judge, task) = row_snapshots(task_id, judge_id, task_judge_id);

    let dir = tempfile::tempdir().unwrap();
    make_repo(dir.path());
    write_file(dir.path(), "a.txt", "x");
    commit(dir.path(), "init");

    // tests is null with a rationale → excluded; product 5.0 alone → overall
    // 5.0 → 50% of 0..80 = 40.
    let llm = FakeJudgeLlm::new(vec![AgentResponse::Final {
        text: r#"{"criteria": [
            {"key": "product", "score": 5.0, "rationale": "half done"},
            {"key": "tests", "score": null, "rationale": "no tests reachable"}
        ], "feedback": "partial coverage"}"#
            .to_string(),
    }]);

    let out = run_judge(
        &db,
        &llm,
        dir.path(),
        session,
        player,
        task_id,
        None,
        None,
        &tj,
        &judge,
        &task,
        &[],
        None,
        &[],
        "llama3.2",
        "ollama",
        None,
        &share_scale(),
        None,
        None,
        Some(&ctx()),
        None,
        &[],
    )
    .await
    .expect("run_judge");
    assert_eq!(out.rating, 40.0, "null weight leaves the denominator");
    assert_eq!(out.point_delta, 40);
}

#[tokio::test]
async fn a_sheet_with_undeclared_keys_is_retried_then_rejected() {
    let db = setup_db().await;
    let owner = insert_user(&db).await;
    let project = insert_project(&db, owner).await;
    let session = insert_session(&db, project).await;
    let player = insert_player(&db, session).await;
    let (task_id, judge_id, task_judge_id) = insert_chain(&db, project, session, player).await;
    let (tj, judge, task) = row_snapshots(task_id, judge_id, task_judge_id);

    let dir = tempfile::tempdir().unwrap();
    make_repo(dir.path());
    write_file(dir.path(), "a.txt", "x");
    commit(dir.path(), "init");

    let bad = r#"{"criteria": [{"key": "vibes", "score": 9.0, "rationale": "great vibes"}],
                  "feedback": "?"}"#;
    let llm = FakeJudgeLlm::new(vec![
        AgentResponse::Final {
            text: bad.to_string(),
        },
        AgentResponse::Final {
            text: bad.to_string(),
        },
        AgentResponse::Final {
            text: bad.to_string(),
        },
    ]);

    let err = run_judge(
        &db,
        &llm,
        dir.path(),
        session,
        player,
        task_id,
        None,
        None,
        &tj,
        &judge,
        &task,
        &[],
        None,
        &[],
        "llama3.2",
        "ollama",
        None,
        &share_scale(),
        None,
        None,
        Some(&ctx()),
        None,
        &[],
    )
    .await
    .expect_err("undeclared keys never become a verdict");
    assert!(matches!(err, arena_core::judging::JudgeError::AiParseError));
}
