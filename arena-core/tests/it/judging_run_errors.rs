//! `run_judge` tests: parse / validation error paths.

use crate::common;
use crate::common::*;

use arena_core::judging::{AgentResponse, JudgeError, PriorTaskResult, run_judge};

/// The scale `run_judge` now expects from its caller. Production resolves it
/// through `gate_task_judge`, so the tests do too; a generous payout leaves
/// the judge's own scale intact, which is what these cases are about.
fn gated_scale(
    judge: &arena_core::judging::JudgeRow,
    tj: &arena_core::judging::TaskJudgeRow,
) -> arena_core::validation::judge_results::RatingScale {
    match arena_core::judging::gate_task_judge(&judge.rating_scale, &tj.rating_scale_override, 1000)
    {
        arena_core::judging::TaskJudgeGate::Run(scale) => scale,
        arena_core::judging::TaskJudgeGate::Skip { reason } => {
            panic!("unexpected gate skip: {reason}")
        }
    }
}

#[tokio::test]
async fn run_judge_parse_error_on_non_json() {
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

    // Three non-JSON finals: the initial reply plus both nudge retries, so
    // the loop gives up and surfaces the parse error.
    let llm = FakeJudgeLlm::new(vec![
        AgentResponse::Final {
            text: "I think the code is great".to_string(),
        },
        AgentResponse::Final {
            text: "Still prose, not a verdict".to_string(),
        },
        AgentResponse::Final {
            text: "More prose".to_string(),
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
        "m",
        "test-provider",
        None,
        &gated_scale(&judge, &tj),
        None,
        None,
        None,
        None,
        &[],
    )
    .await
    .expect_err("persistently non-JSON finals → parse error");

    assert!(matches!(err, JudgeError::AiParseError), "got {err:?}");
}

#[tokio::test]
async fn run_judge_nudge_recovers_from_prose_final() {
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

    // Prose final, then a valid verdict on the nudge retry.
    let llm = FakeJudgeLlm::new(vec![
        AgentResponse::Final {
            text: "Let me summarise my analysis first".to_string(),
        },
        AgentResponse::Final {
            text: r#"{"rating": 5.0, "feedback": "ok"}"#.to_string(),
        },
    ]);

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
        "m",
        "test-provider",
        None,
        &gated_scale(&judge, &tj),
        None,
        None,
        None,
        None,
        &[],
    )
    .await
    .expect("nudge retry should recover");

    assert_eq!(out.rating, 5.0);
    assert_eq!(out.feedback, "ok");
}

#[tokio::test]
async fn run_judge_rating_out_of_range_clamps_to_the_nearest_bound() {
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

    let llm = FakeJudgeLlm::new(vec![AgentResponse::Final {
        text: r#"{"rating": 99.0, "feedback": "over"}"#.to_string(),
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
        "m",
        "test-provider",
        None,
        &gated_scale(&judge, &tj),
        None,
        None,
        None,
        None,
        &[],
    )
    .await
    .expect("out-of-range clamps instead of failing (and re-billing) the run");

    // 99 on a 0-10 scale is the cap, not a failure: the run is not re-run.
    assert_eq!(out.rating, 10.0);
    assert_eq!(out.point_delta, 10);
}

#[tokio::test]
async fn run_judge_feedback_too_long() {
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

    let long = "a".repeat(10_001);
    let llm = FakeJudgeLlm::new(vec![AgentResponse::Final {
        text: format!(r#"{{"rating": 5.0, "feedback": "{long}"}}"#),
    }]);

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
        "m",
        "test-provider",
        None,
        &gated_scale(&judge, &tj),
        None,
        None,
        None,
        None,
        &[],
    )
    .await
    .expect_err("feedback over 10k chars");

    assert!(matches!(err, JudgeError::FeedbackTooLong), "got {err:?}");
}

#[tokio::test]
async fn run_judge_strips_markdown_fences() {
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

    let llm = FakeJudgeLlm::new(vec![AgentResponse::Final {
        text: "```json\n{\"rating\": 7.0, \"feedback\": \"fenced\"}\n```".to_string(),
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
        "m",
        "test-provider",
        None,
        &gated_scale(&judge, &tj),
        None,
        None,
        None,
        None,
        &[],
    )
    .await
    .expect("fenced JSON parses");

    assert_eq!(out.rating, 7.0);
    assert_eq!(out.feedback, "fenced");
}

#[tokio::test]
async fn run_judge_ignores_extra_json_fields() {
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

    let llm = FakeJudgeLlm::new(vec![AgentResponse::Final {
        text: r#"{"rating": 5.0, "feedback": "ok", "confidence": 0.9, "extra": true}"#.to_string(),
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
        "m",
        "test-provider",
        None,
        &gated_scale(&judge, &tj),
        None,
        None,
        None,
        None,
        &[],
    )
    .await
    .expect("extra fields ignored");

    assert_eq!(out.rating, 5.0);
}

#[tokio::test]
async fn run_judge_with_prior_results_in_context() {
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

    let llm = FakeJudgeLlm::new(vec![AgentResponse::Final {
        text: r#"{"rating": 6.0, "feedback": "ok"}"#.to_string(),
    }]);

    let prior = vec![PriorTaskResult {
        point_delta: 10,
        answer: "reverse('abc') => 'cba'".to_string(),
    }];

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
        &prior,
        None,
        &[],
        "m",
        "test-provider",
        None,
        &gated_scale(&judge, &tj),
        None,
        None,
        None,
        None,
        &[],
    )
    .await
    .expect("run_judge with prior");

    assert_eq!(out.rating, 6.0);
}
