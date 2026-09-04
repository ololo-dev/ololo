//! `run_judge` tests: parse / validation error paths.

use crate::common;
use crate::common::*;

use arena_core::judging::{
    AgentResponse, JudgeError, PriorJudgeResult, PriorRequest, PriorTaskResult, RequestFate,
    ResumeFrom, TurnsOutcome, run_judge,
};

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

    // Two non-JSON finals: the judge's reply, then the extractor's — so
    // the run gives up and surfaces the parse error.
    let llm = FakeJudgeLlm::new(vec![
        AgentResponse::Final {
            text: "I think the code is great".to_string(),
        },
        AgentResponse::Final {
            text: "Still prose, not a verdict".to_string(),
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

/// Records every call so a test can see what the extractor was handed.
struct ExtractorSpy {
    responses: std::sync::Mutex<Vec<AgentResponse>>,
    calls: std::sync::Mutex<Vec<(String, String, usize)>>,
}

#[async_trait::async_trait]
impl arena_core::judging::JudgeLlm for ExtractorSpy {
    async fn run_agent(
        &self,
        system: &str,
        user: &str,
        tools: Vec<arena_core::judging::ToolDef>,
        _prior_tool_result: Option<&str>,
    ) -> Result<AgentResponse, JudgeError> {
        self.calls
            .lock()
            .unwrap()
            .push((system.to_string(), user.to_string(), tools.len()));
        Ok(self.responses.lock().unwrap().remove(0))
    }
}

/// A prose final is not re-judged: the analysis goes to the extractor — a
/// tool-less call with its own small brief — and the JSON it returns is the
/// verdict. Before this, the judge was re-run twice with its prose fed back
/// as a "tool result" and a request for a different JSON shape than its
/// system prompt demanded.
#[tokio::test]
async fn run_judge_extracts_the_verdict_from_a_prose_final() {
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

    let llm = ExtractorSpy {
        responses: std::sync::Mutex::new(vec![
            AgentResponse::Final {
                text: "Let me summarise my analysis first: the map { a: 1 } is fine, 5 of 10."
                    .to_string(),
            },
            AgentResponse::Final {
                text: r#"{"rating": 5.0, "feedback": "ok"}"#.to_string(),
            },
        ]),
        calls: std::sync::Mutex::new(Vec::new()),
    };

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
    .expect("the extractor recovers the verdict");

    assert_eq!(out.rating, 5.0);
    assert_eq!(out.feedback, "ok");

    let calls = llm.calls.lock().unwrap();
    assert_eq!(calls.len(), 2, "judge, then extractor");
    let (system, user, tools) = &calls[1];
    assert_eq!(*tools, 0, "the extractor gets no tools");
    assert!(system.contains("You are not the judge"), "{system}");
    assert!(user.contains("the map { a: 1 } is fine, 5 of 10"), "{user}");
    assert!(
        !user.contains("Tool result"),
        "the analysis is the prompt, not a tool result"
    );
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

/// Drives the turn loop like the rig judge: pauses on the first run,
/// resumes on the second, and records what it was handed.
struct PausingTurns {
    resumes: std::sync::Mutex<Vec<Option<ResumeFrom>>>,
}

#[async_trait::async_trait]
impl arena_core::judging::JudgeLlm for PausingTurns {
    async fn run_agent(
        &self,
        _system: &str,
        _user: &str,
        _tools: Vec<arena_core::judging::ToolDef>,
        _prior_tool_result: Option<&str>,
    ) -> Result<AgentResponse, JudgeError> {
        panic!("a turn-driven judge is never driven through run_agent");
    }

    fn supports_turns(&self) -> bool {
        true
    }

    async fn run_turns(
        &self,
        _system: &str,
        _user: &str,
        _tools: Vec<arena_core::judging::ToolDef>,
        _images: &[arena_core::judging::JudgeImage],
        resume: Option<ResumeFrom>,
    ) -> Result<TurnsOutcome, JudgeError> {
        let resumed = resume.is_some();
        self.resumes.lock().unwrap().push(resume);
        Ok(if resumed {
            TurnsOutcome::Final {
                text: r#"{"rating": 6.0, "feedback": "seen the capture"}"#.to_string(),
            }
        } else {
            TurnsOutcome::Suspended {
                transcript: serde_json::json!([{"role": "user", "content": "the brief"}]),
            }
        })
    }
}

/// A judge that registers a participant request does not write a
/// provisional verdict and get re-run from scratch: its run PAUSES with
/// its transcript, and the re-drive resumes that conversation with the
/// request's fate as the next turn.
#[tokio::test]
async fn run_judge_pauses_on_a_request_and_resumes_the_conversation() {
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

    let llm = PausingTurns {
        resumes: std::sync::Mutex::new(Vec::new()),
    };
    let scale = gated_scale(&judge, &tj);

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
        &scale,
        None,
        None,
        None,
        None,
        &[],
    )
    .await
    .expect_err("a paused run is not a verdict");
    let transcript = match err {
        JudgeError::Suspended(t) => *t,
        other => panic!("expected Suspended, got {other:?}"),
    };
    assert_eq!(transcript[0]["content"], "the brief");
    assert!(
        <arena_core::entities::judge_results::Entity as sea_orm::EntityTrait>::find()
            .all(&db)
            .await
            .unwrap()
            .is_empty(),
        "no provisional verdict is written"
    );

    // The re-drive: the waiting row carries the transcript and the fate of
    // the request; the judge picks up where it stopped.
    let prior = PriorJudgeResult {
        rating: 0.0,
        feedback: String::new(),
        requests: vec![PriorRequest {
            instruction: "Capture rome.png".into(),
            fate: RequestFate::Delivered { files: 2 },
        }],
        transcript: Some(transcript.clone()),
    };
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
        Some(&prior),
        &[],
        "m",
        "test-provider",
        None,
        &scale,
        None,
        None,
        None,
        None,
        &[],
    )
    .await
    .expect("the resumed run scores");
    assert_eq!(out.rating, 6.0);
    assert_eq!(out.feedback, "seen the capture");

    let resumes = llm.resumes.lock().unwrap();
    assert_eq!(resumes.len(), 2);
    assert!(resumes[0].is_none(), "the first run starts fresh");
    let resumed = resumes[1].as_ref().expect("the second run resumes");
    assert_eq!(
        resumed.transcript, transcript,
        "the same conversation continues"
    );
    assert!(
        resumed.update.starts_with("Your run paused"),
        "{}",
        resumed.update
    );
    assert!(
        resumed
            .update
            .contains("\"Capture rome.png\" → DELIVERED — 2 file(s)"),
        "{}",
        resumed.update
    );
}
