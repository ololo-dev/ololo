//! `run_judge` tests: verdict upsert, tool-call loop, and limits.

use crate::common;
use crate::common::*;
use arena_core::judging::tools::ToolScope;

use arena_core::entities::judge_results;
use arena_core::judging::{AgentResponse, MAX_TOOL_CALLS, PriorJudgeResult, run_judge};
use sea_orm::EntityTrait;

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
async fn a_review_rewrites_the_verdict_before_it_is_persisted() {
    // The second cut point: the model has answered, and the judge's own
    // program gets the last word on whether that answer may stand. What is
    // stored has to be the reviewed verdict, not the model's — anything else
    // scores the player on a verdict the judge itself rejected.
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

    let evidence = arena_core::judging::evidence::build_evidence(
        &db,
        dir.path(),
        session,
        player,
        task_id,
        "code-quality",
        &gated_scale(&judge, &tj),
        None,
        arena_core::judging::evidence::EvidenceNeeds::everything(),
        &ToolScope::everything(),
    )
    .await
    .expect("build_evidence");

    let llm = FakeJudgeLlm::new(vec![AgentResponse::Final {
        text: r#"{"rating": 9.0, "feedback": "flawless"}"#.to_string(),
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
        &gated_scale(&judge, &tj),
        None,
        Some((
            r#"return revise(4, "reviewed down: praise with nothing behind it");"#,
            &evidence,
        )),
        None,
        None,
        &[],
    )
    .await
    .expect("run_judge");

    assert_eq!(out.rating, 4.0, "the returned verdict is the reviewed one");
    assert_eq!(out.point_delta, 4);
    assert_eq!(out.feedback, "reviewed down: praise with nothing behind it");

    let row = judge_results::Entity::find_by_id(out.judge_result_id)
        .one(&db)
        .await
        .expect("query")
        .expect("persisted row");
    assert_eq!(row.point_delta, 4, "the stored row is the reviewed verdict");
    assert_eq!(row.rating, serde_json::json!(4.0));
    assert!(
        row.raw_output.contains("flawless"),
        "the model's own answer is still on the record: {}",
        row.raw_output
    );
}

#[tokio::test]
async fn a_rejected_verdict_is_not_persisted_at_all() {
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

    let evidence = arena_core::judging::evidence::build_evidence(
        &db,
        dir.path(),
        session,
        player,
        task_id,
        "code-quality",
        &gated_scale(&judge, &tj),
        None,
        arena_core::judging::evidence::EvidenceNeeds::everything(),
        &ToolScope::everything(),
    )
    .await
    .expect("build_evidence");

    let llm = FakeJudgeLlm::new(vec![AgentResponse::Final {
        text: r#"{"rating": 9.0, "feedback": "flawless"}"#.to_string(),
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
        "llama3.2",
        "ollama",
        None,
        &gated_scale(&judge, &tj),
        None,
        Some((r#"return reject("cites nothing");"#, &evidence)),
        None,
        None,
        &[],
    )
    .await
    .expect_err("a rejected verdict must not score anybody");

    assert!(
        matches!(err, arena_core::judging::JudgeError::VerdictRejected(_)),
        "got {err:?}"
    );
    let rows = judge_results::Entity::find()
        .all(&db)
        .await
        .expect("query judge_results");
    assert!(
        rows.is_empty(),
        "nothing may be persisted for a rejected verdict, got {rows:?}"
    );
}

#[tokio::test]
async fn run_judge_final_verdict_upserts() {
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
        text: r#"{"rating": 7.5, "feedback": "good"}"#.to_string(),
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
        &gated_scale(&judge, &tj),
        None,
        None,
        None,
        None,
        &[],
    )
    .await
    .expect("run_judge");

    assert_eq!(out.rating, 7.5);
    assert_eq!(out.point_delta, 8); // 7.5 rounds to 8
    assert_eq!(out.feedback, "good");
    assert_eq!(out.model, "llama3.2");

    let rows = judge_results::Entity::find().all(&db).await.expect("find");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].player_id_fk, player);
    assert_eq!(rows[0].point_delta, 8);
}

#[tokio::test]
async fn run_judge_rerun_replaces_row() {
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

    let llm1 = FakeJudgeLlm::new(vec![AgentResponse::Final {
        text: r#"{"rating": 5.0, "feedback": "first"}"#.to_string(),
    }]);
    run_judge(
        &db,
        &llm1,
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
        "m1",
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
    .expect("run1");

    let llm2 = FakeJudgeLlm::new(vec![AgentResponse::Final {
        text: r#"{"rating": 8.0, "feedback": "second"}"#.to_string(),
    }]);
    let out = run_judge(
        &db,
        &llm2,
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
        Some(&PriorJudgeResult {
            rating: 5.0,
            feedback: "first".to_string(),
        }),
        &[],
        "m2",
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
    .expect("run2");

    assert_eq!(out.feedback, "second");
    assert_eq!(out.model, "m2");

    let rows = judge_results::Entity::find().all(&db).await.expect("find");
    assert_eq!(rows.len(), 1, "upsert replaces, no history");
    assert_eq!(rows[0].feedback, "second");
    assert_eq!(rows[0].model, "m2");
}

#[tokio::test]
async fn run_judge_tool_call_then_final() {
    let db = setup_db().await;
    let owner = insert_user(&db).await;
    let project = insert_project(&db, owner).await;
    let session = insert_session(&db, project).await;
    let player = insert_player(&db, session).await;
    let (task_id, judge_id, task_judge_id) = insert_chain(&db, project, session, player).await;
    let (tj, judge, task) = row_snapshots(task_id, judge_id, task_judge_id);

    let dir = tempfile::tempdir().unwrap();
    make_repo(dir.path());
    write_file(dir.path(), "main.rs", "fn main() {}");
    commit(dir.path(), "init");

    let llm = FakeJudgeLlm::new(vec![
        AgentResponse::ToolCall {
            name: "list_files".to_string(),
            args: serde_json::json!({}),
        },
        AgentResponse::Final {
            text: r#"{"rating": 6.0, "feedback": "decent"}"#.to_string(),
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
    .expect("run_judge");

    assert_eq!(out.rating, 6.0);
    assert_eq!(out.point_delta, 6);
}

#[tokio::test]
async fn run_judge_too_many_tool_calls() {
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

    // MAX_TOOL_CALLS + 1 tool calls, never a Final → TooManyToolCalls
    let mut seq = Vec::new();
    for _ in 0..(MAX_TOOL_CALLS + 1) {
        seq.push(AgentResponse::ToolCall {
            name: "list_files".to_string(),
            args: serde_json::json!({}),
        });
    }
    let llm = FakeJudgeLlm::new(seq);

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
    .expect_err("should hit tool-call limit");

    assert!(
        matches!(err, arena_core::judging::JudgeError::TooManyToolCalls),
        "got {err:?}"
    );
}

#[tokio::test]
async fn run_judge_empty_repo_does_not_panic() {
    let db = setup_db().await;
    let owner = insert_user(&db).await;
    let project = insert_project(&db, owner).await;
    let session = insert_session(&db, project).await;
    let player = insert_player(&db, session).await;
    let (task_id, judge_id, task_judge_id) = insert_chain(&db, project, session, player).await;
    let (tj, judge, task) = row_snapshots(task_id, judge_id, task_judge_id);

    let dir = tempfile::tempdir().unwrap();
    make_repo(dir.path());
    // no commits — empty repo

    let llm = FakeJudgeLlm::new(vec![
        AgentResponse::ToolCall {
            name: "list_files".to_string(),
            args: serde_json::json!({}),
        },
        AgentResponse::Final {
            text: r#"{"rating": 0.0, "feedback": "no code submitted"}"#.to_string(),
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
    .expect("run_judge on empty repo");

    assert_eq!(out.rating, 0.0);
    assert_eq!(out.point_delta, 0);
}

#[tokio::test]
async fn run_judge_persists_provider_and_tokens_but_not_run_log() {
    use arena_core::judging::{JudgeLogEvent, JudgeRunRecorder, log_now_ms};

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

    // Recorder as the LLM adapter would fill it: a tool call then an LLM
    // turn carrying token usage.
    let recorder = JudgeRunRecorder::default();
    recorder.record(JudgeLogEvent {
        at_ms: log_now_ms(),
        kind: "tool".to_string(),
        name: Some("git_ls".to_string()),
        args: Some("{}".to_string()),
        output_chars: Some(120),
        output: Some("a.txt\nb.txt".to_string()),
        duration_ms: 12,
        ..Default::default()
    });
    recorder.record(JudgeLogEvent {
        at_ms: log_now_ms(),
        kind: "llm".to_string(),
        output_chars: Some(40),
        duration_ms: 900,
        tokens_input: Some(1200),
        tokens_output: Some(80),
        tokens_cache_read: Some(300),
        tokens_cache_write: Some(0),
        model: Some("llama3.2".to_string()),
        input: Some("[system]\nprompt".to_string()),
        output: Some("{\"rating\": 6.0}".to_string()),
        ..Default::default()
    });

    let llm = FakeJudgeLlm::new(vec![AgentResponse::Final {
        text: r#"{"rating": 6.0, "feedback": "ok"}"#.to_string(),
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
        Some(&recorder),
        &gated_scale(&judge, &tj),
        None,
        None,
        None,
        None,
        &[],
    )
    .await
    .expect("run_judge");

    let row = judge_results::Entity::find_by_id(out.judge_result_id)
        .one(&db)
        .await
        .unwrap()
        .expect("row persisted");
    assert_eq!(row.provider, "ollama");
    assert_eq!(row.tokens_input, Some(1200));
    assert_eq!(row.tokens_output, Some(80));
    assert_eq!(row.tokens_cache_read, Some(300));
    assert_eq!(row.tokens_cache_write, Some(0));
    // The bulky event log is written to the game-server's on-disk store
    // ({join_code}/{username}/{task_id}.json), not the DB row.
    assert!(
        row.run_log.is_none(),
        "run_log must not be stored in the DB"
    );
    // The recorder still carries the events for the file store.
    assert_eq!(recorder.events().len(), 2);
    assert_eq!(recorder.token_totals(), (1200, 80));
}

#[tokio::test]
async fn attached_images_reach_the_llm_and_are_announced_in_the_prompt() {
    // A screenshot judge is only as good as its delivery: the images must
    // arrive on the same call as the prompt, and the prompt must say they
    // are there — a vision model silently ignores attachments it was never
    // told to look at.
    use arena_core::judging::{JudgeError, JudgeImage, JudgeLlm, ToolDef};
    use std::sync::Mutex;

    struct ImageRecordingLlm {
        seen: Mutex<Option<(usize, String)>>,
    }

    #[async_trait::async_trait]
    impl JudgeLlm for ImageRecordingLlm {
        async fn run_agent(
            &self,
            _system: &str,
            _user: &str,
            _tools: Vec<ToolDef>,
            _prior: Option<&str>,
        ) -> Result<AgentResponse, JudgeError> {
            panic!("run_judge must route through run_agent_with_images");
        }

        async fn run_agent_with_images(
            &self,
            _system: &str,
            user: &str,
            _tools: Vec<ToolDef>,
            _prior: Option<&str>,
            images: &[JudgeImage],
        ) -> Result<AgentResponse, JudgeError> {
            *self.seen.lock().unwrap() = Some((images.len(), user.to_string()));
            Ok(AgentResponse::Final {
                text: r#"{"rating": 7.0, "feedback": "seen"}"#.to_string(),
            })
        }
    }

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

    let llm = ImageRecordingLlm {
        seen: Mutex::new(None),
    };
    let images = vec![JudgeImage {
        media_type: "image/png".to_string(),
        base64: "aGVsbG8=".to_string(),
        label: "screenshot artifact from probe 42".to_string(),
    }];
    run_judge(
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
        "m1",
        "test-provider",
        None,
        &gated_scale(&judge, &tj),
        None,
        None,
        None,
        None,
        &images,
    )
    .await
    .expect("run_judge");

    let (count, user) = llm.seen.lock().unwrap().take().expect("llm was called");
    assert_eq!(count, 1);
    assert!(user.contains("## Attached screenshots"));
    assert!(user.contains("screenshot artifact from probe 42"));
}
