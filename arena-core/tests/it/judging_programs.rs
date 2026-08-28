//! The judge's own program, run against a real evidence snapshot.
//!
//! The cases here are the two the user asked for by name — an anti-cheat
//! judge that looks deeper instead of penalizing when the work window is
//! empty, and a ladder judge that reads its neighbouring rungs — plus the
//! ways a program can be wrong, which must fail loudly rather than quietly
//! scoring somebody.

use crate::common;
use crate::common::*;

use arena_core::judging::evidence::{Evidence, EvidenceNeeds, build_evidence};
use arena_core::judging::programs::{Decision, Review, run_decide, run_review, split_programs};
use arena_core::validation::judge_results::RatingScale;
use chrono::Utc;
use sea_orm::{ActiveModelTrait, ActiveValue::Set, DatabaseConnection};
use std::path::Path;
use uuid::Uuid;

fn scale(min: f64, max: f64) -> RatingScale {
    RatingScale {
        min,
        max,
        step: 1.0,
    }
}

async fn insert_task(
    db: &DatabaseConnection,
    project_id: Uuid,
    ordinal: i32,
    title: &str,
    tags_json: &str,
) -> Uuid {
    let id = Uuid::new_v4();
    arena_core::entities::tasks::ActiveModel {
        id: Set(id),
        project_id_fk: Set(project_id),
        ordinal: Set(ordinal),
        title: Set(title.to_string()),
        content: Set(format!("Do {title}")),
        test_template: Set(serde_json::json!({"kind":"shell"})),
        created_at: Set(Utc::now()),
        tags: Set(tags_json.to_string()),
        point_value: Set(25),
        deadline_secs: Set(Some(300)),
        min_interval_secs: Set(Some(5)),
        interval_increment_secs: Set(Some(0)),
        max_interval_secs: Set(Some(300)),
        fail_points: Set(0),
        no_response_points: Set(0),
        completion_bonus_points: Set(0),
        evaluation: Set(None),
    }
    .insert(db)
    .await
    .expect("insert task");
    id
}

async fn award(db: &DatabaseConnection, session: Uuid, player: Uuid, task: Uuid, points: i32) {
    arena_core::entities::task_results::ActiveModel {
        id: Set(Uuid::new_v4()),
        session_id_fk: Set(session),
        player_id_fk: Set(player),
        task_id: Set(Some(task)),
        answer: Set("ok".to_string()),
        created_at: Set(Utc::now()),
        point_delta: Set(points),
        is_bonus: Set(false),
    }
    .insert(db)
    .await
    .expect("insert task_result");
}

fn repo_with_a_commit(dir: &Path) -> String {
    make_repo(dir);
    write_file(dir, "solution.sh", "echo hi\n");
    commit(dir, "chore: scaffold")
}

/// An evidence snapshot for one task of a two-rung golf ladder.
async fn ladder_evidence(db: &DatabaseConnection, dir: &Path, judged_rung: usize) -> Evidence {
    let owner = insert_user(db).await;
    let project = insert_project(db, owner).await;
    let session = insert_session(db, project).await;
    let player = insert_player(db, session).await;
    repo_with_a_commit(dir);

    let loose = insert_task(
        db,
        project,
        0,
        "Golf the solution to at most 140 bytes",
        r#"["code-golf"]"#,
    )
    .await;
    let tight = insert_task(
        db,
        project,
        1,
        "Golf the solution to at most 120 bytes",
        r#"["code-golf"]"#,
    )
    .await;
    award(db, session, player, loose, 25).await;
    award(db, session, player, tight, 25).await;

    let judged = if judged_rung == 0 { loose } else { tight };
    // The rung under judgement carries its own commit, the way ololo commits
    // a finished task — a review checking citations has nothing to cite
    // otherwise.
    write_file(dir, "solution.sh", "echo hi;#\n");
    commit(dir, &format!("feat({judged}): golfed"));

    build_evidence(
        db,
        dir,
        session,
        player,
        judged,
        "golf-verify",
        &scale(-25.0, 0.0),
        None,
        EvidenceNeeds::everything(),
        &arena_core::judging::tools::ToolScope::everything(),
    )
    .await
    .expect("build_evidence")
}

/// The program this repo actually ships for a judge, straight out of its
/// markdown — a copy in the test would drift from the file that runs.
/// `None` when this checkout does not carry that judge: these are gates on
/// the judges that happen to ship here, not a required inventory, so the
/// caller early-returns instead of failing.
fn shipped_program(slug: &str) -> Option<String> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root")
        .join("judges")
        .join(format!("{slug}.md"));
    let body = std::fs::read_to_string(&path).ok()?;
    Some(
        split_programs(&body)
            .1
            .decide
            .unwrap_or_else(|| panic!("{slug} carries no program")),
    )
}

async fn insert_test_row(db: &DatabaseConnection, session: Uuid, task: Uuid, ordinal: i32) -> Uuid {
    let id = Uuid::new_v4();
    arena_core::entities::tests::ActiveModel {
        id: Set(id),
        command_template: Set("echo ok".to_string()),
        answer_template: Set(String::new()),
        fixture_definitions: Set("[]".to_string()),
        created_at: Set(Utc::now()),
        session_id: Set(session),
        task_id: Set(task),
        ordinal: Set(ordinal),
        prompt: Set(String::new()),
        description: Set(None),
        probe_config: Set(None),
        initiator: Set("system".to_string()),
        registered_by_judge_id: Set(None),
    }
    .insert(db)
    .await
    .expect("insert test");
    id
}

async fn insert_passing_probe(db: &DatabaseConnection, session: Uuid, player: Uuid, test_id: Uuid) {
    arena_core::entities::probes::ActiveModel {
        id: Set(Uuid::new_v4()),
        test_id: Set(test_id),
        player_id: Set(player),
        session_id: Set(session),
        attempt: Set(1),
        rendered_command: Set("echo ok".to_string()),
        fixture_values: Set("{}".to_string()),
        expected_answer: Set(None),
        outcome: Set(Some("pass".to_string())),
        dispatched_at: Set(Utc::now()),
        deadline_at: Set(Utc::now()),
        resolved_at: Set(Some(Utc::now())),
        updated_at: Set(Some(Utc::now())),
        output: Set(None),
        exit_code: Set(Some(0)),
        duration_ms: Set(Some(12)),
        point_delta: Set(Some(25)),
        resolved_answer: Set(Some("100".to_string())),
        secret_meta: Set(None),
        result_json: Set(None),
        artifact_path: Set(None),
    }
    .insert(db)
    .await
    .expect("insert probe");
}

#[tokio::test]
async fn winning_a_golf_rung_early_is_not_evidence_against_the_later_rungs() {
    // The player wrote a solution compact enough at the first rung. Every rung
    // below it needs no change, so its commit is empty — and an anti-cheat
    // judge reading empty commits would penalize the strongest player on the
    // ladder, repeatedly. Winning early is the game; it cannot also be the
    // evidence.
    let db = setup_db().await;
    let owner = insert_user(&db).await;
    let project = insert_project(&db, owner).await;
    let session = insert_session(&db, project).await;
    let player = insert_player(&db, session).await;
    let dir = tempfile::tempdir().unwrap();
    repo_with_a_commit(dir.path());

    let won_early = insert_task(
        &db,
        project,
        0,
        "Golf the solution to at most 140 bytes",
        r#"["code-golf"]"#,
    )
    .await;
    let nothing_to_do = insert_task(
        &db,
        project,
        1,
        "Golf the solution to at most 120 bytes",
        r#"["code-golf"]"#,
    )
    .await;
    award(&db, session, player, won_early, 25).await;
    award(&db, session, player, nothing_to_do, 25).await;

    // The later rung passed its probes without a commit of its own.
    let t = insert_test_row(&db, session, nothing_to_do, 0).await;
    insert_passing_probe(&db, session, player, t).await;

    let ev = build_evidence(
        &db,
        dir.path(),
        session,
        player,
        nothing_to_do,
        "task-anti-cheat",
        &scale(-25.0, 0.0),
        None,
        EvidenceNeeds::everything(),
        &arena_core::judging::tools::ToolScope::everything(),
    )
    .await
    .expect("build_evidence");
    assert!(ev.task.diff_is_empty, "the later rung committed nothing");

    let Some(program) = shipped_program("task-anti-cheat") else {
        return;
    };
    match run_decide(&program, &ev).expect("program runs") {
        Decision::Skip { reason } => assert!(
            reason.contains("code-golf rung"),
            "skipped for the wrong reason: {reason}"
        ),
        other => panic!("an empty rung below a won rung must not be judged: {other:?}"),
    }
}

#[tokio::test]
async fn the_first_golf_rung_is_still_judged_when_its_commit_is_empty() {
    // Nothing was won earlier, so an empty commit on the first rung is the
    // case the judge exists for: the solution may predate the session.
    let db = setup_db().await;
    let owner = insert_user(&db).await;
    let project = insert_project(&db, owner).await;
    let session = insert_session(&db, project).await;
    let player = insert_player(&db, session).await;
    let dir = tempfile::tempdir().unwrap();
    repo_with_a_commit(dir.path());

    let first = insert_task(
        &db,
        project,
        0,
        "Golf the solution to at most 140 bytes",
        r#"["code-golf"]"#,
    )
    .await;
    award(&db, session, player, first, 25).await;
    let t = insert_test_row(&db, session, first, 0).await;
    insert_passing_probe(&db, session, player, t).await;

    let ev = build_evidence(
        &db,
        dir.path(),
        session,
        player,
        first,
        "task-anti-cheat",
        &scale(-25.0, 0.0),
        None,
        EvidenceNeeds::everything(),
        &arena_core::judging::tools::ToolScope::everything(),
    )
    .await
    .expect("build_evidence");

    let Some(program) = shipped_program("task-anti-cheat") else {
        return;
    };
    match run_decide(&program, &ev).expect("program runs") {
        Decision::Ask { focus } => assert!(
            focus
                .expect("the model is pointed at the empty window")
                .contains("changed no lines"),
        ),
        other => panic!("the first rung must still reach the model: {other:?}"),
    }
}

#[tokio::test]
async fn a_ladder_program_reads_its_neighbouring_rungs() {
    // The reach the user asked for: judging the 140-byte rung, the program
    // can see that the same player banked the 120-byte rung — the solution
    // was demonstrably shrunk further.
    let db = setup_db().await;
    let dir = tempfile::tempdir().unwrap();
    let ev = ladder_evidence(&db, dir.path(), 0).await;

    let program = r#"
const budget = t => { const m = /at most (\d+) bytes/.exec(t.title); return m ? Number(m[1]) : null; };
const mine = budget(task);
const tighter = tasks.filter(t => t.id !== task.id && t.earned > 0 && budget(t) !== null && budget(t) < mine);
if (tighter.length > 0) return skip("a tighter rung of the same ladder was reached");
return ask();
"#;

    assert_eq!(
        run_decide(program, &ev).expect("program runs"),
        Decision::Skip {
            reason: "a tighter rung of the same ladder was reached".to_string()
        }
    );
}

#[tokio::test]
async fn the_tightest_rung_still_gets_judged() {
    // The same program on the tightest rung: nothing below it, so it must
    // not skip itself — otherwise the ladder verifies nothing at all.
    let db = setup_db().await;
    let dir = tempfile::tempdir().unwrap();
    let ev = ladder_evidence(&db, dir.path(), 1).await;

    let program = r#"
const budget = t => { const m = /at most (\d+) bytes/.exec(t.title); return m ? Number(m[1]) : null; };
const mine = budget(task);
const tighter = tasks.filter(t => t.id !== task.id && t.earned > 0 && budget(t) !== null && budget(t) < mine);
if (tighter.length > 0) return skip("a tighter rung of the same ladder was reached");
return ask();
"#;

    assert_eq!(
        run_decide(program, &ev).expect("program runs"),
        Decision::Ask { focus: None }
    );
}

#[tokio::test]
async fn an_empty_work_window_points_the_model_somewhere() {
    // The anti-cheat case: an empty diff is not guilt, it is a reason to
    // look before the window. The program says where to look; the model
    // still decides.
    let db = setup_db().await;
    let owner = insert_user(&db).await;
    let project = insert_project(&db, owner).await;
    let session = insert_session(&db, project).await;
    let player = insert_player(&db, session).await;
    let dir = tempfile::tempdir().unwrap();
    make_repo(dir.path());
    let task = insert_task(&db, project, 0, "Implement it", "[]").await;

    let ev = build_evidence(
        &db,
        dir.path(),
        session,
        player,
        task,
        "task-anti-cheat",
        &scale(-50.0, 0.0),
        None,
        EvidenceNeeds::everything(),
        &arena_core::judging::tools::ToolScope::everything(),
    )
    .await
    .expect("build_evidence");

    let program = r#"
if (task.diff_is_empty) return ask({ focus: "whether the behaviour predates the window" });
return ask();
"#;

    assert_eq!(
        run_decide(program, &ev).expect("program runs"),
        Decision::Ask {
            focus: Some("whether the behaviour predates the window".to_string())
        }
    );
}

/// A session with `n` tasks that all passed probes and banked points. The
/// repo always opens with a session-start snapshot carrying the solution —
/// the replay shape. When `commit_one` is set, one task gets a real
/// in-session commit that adds lines — one written line is enough to break
/// the replay premise.
async fn vacuum_session_evidence(
    db: &DatabaseConnection,
    dir: &Path,
    n: usize,
    commit_one: bool,
) -> Evidence {
    let owner = insert_user(db).await;
    let project = insert_project(db, owner).await;
    let session = insert_session(db, project).await;
    let player = insert_player(db, session).await;
    make_repo(dir);
    write_file(dir, "answer.sh", "echo prebaked\n");
    commit(dir, "ololo snapshot: session start");

    let mut judged = Uuid::nil();
    for i in 0..n {
        let task = insert_task(db, project, i as i32, &format!("Task {i}"), "[]").await;
        award(db, session, player, task, 25).await;
        let test_id = insert_test_row(db, session, task, 0).await;
        insert_passing_probe(db, session, player, test_id).await;
        if commit_one && i == 0 {
            write_file(dir, "solution.sh", "echo built live\n");
            commit(dir, &format!("feat({task}): task 0"));
        }
        judged = task;
    }

    build_evidence(
        db,
        dir,
        session,
        player,
        judged,
        "task-anti-cheat",
        &scale(-50.0, 0.0),
        None,
        EvidenceNeeds::everything(),
        &arena_core::judging::tools::ToolScope::everything(),
    )
    .await
    .expect("build_evidence")
}

#[tokio::test]
async fn a_session_that_wrote_nothing_after_start_is_penalized_without_a_model() {
    // The 2GHT4B / 6MTAHS case: a pre-baked solution sweeps every task — with
    // no commits at all, or with task commits that are all empty markers. What
    // both shapes share is the replay signature: zero lines written by any
    // in-session commit after the session-start snapshot, zero reported agent
    // activity, probes passing. Decided deterministically, before the model is
    // paid to flip a coin over it (6MTAHS: three -50s and thirteen 0s on
    // identical evidence).
    let db = setup_db().await;
    let dir = tempfile::tempdir().unwrap();
    let ev = vacuum_session_evidence(&db, dir.path(), 5, false).await;

    let Some(program) = shipped_program("task-anti-cheat") else {
        return;
    };
    match run_decide(&program, &ev).expect("program runs") {
        Decision::Score { rating, feedback } => {
            assert_eq!(rating, -50.0);
            assert!(feedback.contains("replay"), "{feedback}");
        }
        other => panic!("expected a deterministic penalty, got {other:?}"),
    }
}

#[tokio::test]
async fn a_single_written_line_breaks_the_replay_premise() {
    // One in-session commit that adds lines means the player worked live, so
    // per-task evidence gaps go back to being judged case by case (the judged
    // task still has an empty window, so the model is asked).
    let db = setup_db().await;
    let dir = tempfile::tempdir().unwrap();
    let ev = vacuum_session_evidence(&db, dir.path(), 5, true).await;

    let Some(program) = shipped_program("task-anti-cheat") else {
        return;
    };
    match run_decide(&program, &ev).expect("program runs") {
        Decision::Ask { focus } => {
            assert!(focus.is_some(), "the empty window still directs the model");
        }
        other => panic!("expected Ask, got {other:?}"),
    }
}

#[tokio::test]
async fn a_program_can_score_without_a_model() {
    let db = setup_db().await;
    let dir = tempfile::tempdir().unwrap();
    let ev = ladder_evidence(&db, dir.path(), 1).await;

    let program = r#"return score(-3, "two claimed probes did not re-run");"#;
    assert_eq!(
        run_decide(program, &ev).expect("program runs"),
        Decision::Score {
            rating: -3.0,
            feedback: "two claimed probes did not re-run".to_string()
        }
    );
}

#[tokio::test]
async fn the_program_sees_the_bounds_it_must_answer_within() {
    // A program deciding whether to spend a model call has to know whether
    // it is even allowed to penalize.
    let db = setup_db().await;
    let dir = tempfile::tempdir().unwrap();
    let ev = ladder_evidence(&db, dir.path(), 1).await;

    let program = r#"
if (!judge.is_penalty) return skip("nothing to take away");
return score(judge.scale_min, "floored");
"#;
    assert_eq!(
        run_decide(program, &ev).expect("program runs"),
        Decision::Score {
            rating: -25.0,
            feedback: "floored".to_string()
        }
    );
}

#[tokio::test]
async fn a_review_can_drop_a_penalty_that_cites_nothing_real() {
    // The YC4T6K post-mortem: a penalty quoted the task's UUID where a commit
    // sha belonged. No wording of a prompt prevents that; three lines here do.
    let db = setup_db().await;
    let dir = tempfile::tempdir().unwrap();
    let ev = ladder_evidence(&db, dir.path(), 1).await;

    let program = r#"
if (verdict.rating >= 0) return accept();
const shas = tasks.map(t => t.commit_sha).filter(Boolean);
const cited = shas.some(sha => verdict.feedback.indexOf(sha.slice(0, 7)) !== -1);
if (!cited) return revise(0, "penalty dropped: the verdict cites no commit of this session");
return accept();
"#;

    // A penalty quoting a uuid rather than a sha loses its evidence.
    assert_eq!(
        run_review(program, &ev, -10.0, "cheated in task 7b625fc6-a481-4eb2").expect("runs"),
        Review::Revise {
            rating: 0.0,
            feedback: "penalty dropped: the verdict cites no commit of this session".to_string()
        }
    );

    // A penalty quoting a real commit of this session stands.
    let sha = ev
        .task
        .commit_sha
        .clone()
        .expect("the judged task has a commit");
    assert_eq!(
        run_review(program, &ev, -10.0, &format!("see {sha}")).expect("runs"),
        Review::Accept
    );

    // And a clean verdict is never second-guessed.
    assert_eq!(
        run_review(program, &ev, 0.0, "nothing wrong here").expect("runs"),
        Review::Accept
    );
}

#[tokio::test]
async fn a_review_can_throw_the_verdict_out_entirely() {
    let db = setup_db().await;
    let dir = tempfile::tempdir().unwrap();
    let ev = ladder_evidence(&db, dir.path(), 1).await;

    assert_eq!(
        run_review(r#"return reject("no reasoning at all");"#, &ev, -5.0, "bad").expect("runs"),
        Review::Reject {
            reason: "no reasoning at all".to_string()
        }
    );
}

#[tokio::test]
async fn a_review_reads_the_same_snapshot_the_model_was_judged_against() {
    // Otherwise it could only check the verdict against itself.
    let db = setup_db().await;
    let dir = tempfile::tempdir().unwrap();
    let ev = ladder_evidence(&db, dir.path(), 1).await;

    let program = r#"
if (verdict.rating < -task.earned) {
  return revise(-task.earned, "clamped to what the task paid");
}
return accept();
"#;
    assert_eq!(
        run_review(program, &ev, -100.0, "over the top").expect("runs"),
        Review::Revise {
            rating: -25.0,
            feedback: "clamped to what the task paid".to_string()
        }
    );
}

#[tokio::test]
async fn a_review_that_answers_with_a_decide_verb_is_an_error() {
    // The two programs share a prelude, so `skip()` is callable from a
    // review — and means nothing there.
    let db = setup_db().await;
    let dir = tempfile::tempdir().unwrap();
    let ev = ladder_evidence(&db, dir.path(), 1).await;

    assert!(run_review(r#"return skip("wrong end");"#, &ev, 0.0, "").is_err());
    assert!(run_review("42", &ev, 0.0, "").is_err());
}

#[test]
fn both_programs_are_lifted_out_of_the_prompt() {
    let body = "Judge it.

```js decide
return ask();
```

Be fair.

```js review
return accept();
```
";
    let (prompt, programs) = split_programs(body);
    assert_eq!(programs.decide.as_deref(), Some("return ask();"));
    assert_eq!(programs.review.as_deref(), Some("return accept();"));
    assert!(prompt.contains("Judge it."));
    assert!(prompt.contains("Be fair."));
    assert!(
        !prompt.contains("ask()") && !prompt.contains("accept()"),
        "a program leaked into the prompt: {prompt}"
    );
}

#[tokio::test]
async fn a_program_that_decides_nothing_fails_the_run() {
    // Silence must not read as "no penalty" or as "penalize": a broken
    // program is an authoring fault and has to surface as one.
    let db = setup_db().await;
    let dir = tempfile::tempdir().unwrap();
    let ev = ladder_evidence(&db, dir.path(), 1).await;

    assert!(
        run_decide("42", &ev).is_err(),
        "a bare number decides nothing"
    );
    assert!(
        run_decide("task.no_such_field.length", &ev).is_err(),
        "a throwing program fails the run"
    );
    assert!(
        run_decide("return skip(", &ev).is_err(),
        "a syntax error fails the run"
    );
}

#[tokio::test]
async fn a_runaway_program_is_stopped() {
    // The judge loop is blocking; an unbounded loop in an authored program
    // would hold the thread forever. The probe sandbox's limits apply here.
    let db = setup_db().await;
    let dir = tempfile::tempdir().unwrap();
    let ev = ladder_evidence(&db, dir.path(), 1).await;

    let err = run_decide("while (true) {} return ask();", &ev)
        .expect_err("an infinite loop must not hang the judge");
    assert!(
        err.to_string().to_lowercase().contains("limit"),
        "expected a runtime-limit error, got {err}"
    );
}

#[test]
fn the_program_never_reaches_the_model() {
    // Whatever the program says is harness instruction. A model that reads
    // it may re-enact a decision that was already made without it.
    let body =
        "Judge the work.\n\n```js decide\nreturn skip(\"nothing paid\");\n```\n\nBe conservative.";
    let (prompt, programs) = split_programs(body);
    let program = programs.decide;
    assert!(program.is_some());
    assert!(
        !prompt.contains("skip"),
        "program leaked into prompt: {prompt}"
    );
    assert!(prompt.contains("Judge the work."));
    assert!(prompt.contains("Be conservative."));
}
