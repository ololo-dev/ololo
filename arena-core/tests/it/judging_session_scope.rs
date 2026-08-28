//! `run_session_judge` — one run per player, one verdict row per reached task.
//!
//! The contract these tests defend: Arena Points are gated on
//! `expired_session_pending_judges` reaching zero, which happens only when
//! every (player, task_judge) pair over the reached tasks has a terminal row.
//! A session-scoped run that skips a task — including one whose verdict call
//! failed — hangs the award flow until its deadline. That failure is silent and
//! looks like "judging is slow", so it gets an explicit test.

use crate::common;
use crate::common::*;

use std::collections::HashMap;

use arena_core::entities::{
    judge_results, judges, session_scheduler_state, task_judges, task_results, tasks,
};
use arena_core::judging::session::run_session_judge;
use arena_core::judging::{AgentResponse, JudgeError, JudgeLlm, JudgeRow, TaskJudgeRow};
use arena_core::session_completion::{
    SCHEDULER_STATE_COMPLETED, expired_session_pending_judges, reached_tasks_for_player,
};
use async_trait::async_trait;
use chrono::Utc;
use sea_orm::{ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, Set};
use uuid::Uuid;

/// Answers every task with a fixed verdict, except the Nth call which errors —
/// so a mid-run failure can be exercised.
struct FlakyJudge {
    fail_on_call: usize,
    calls: std::sync::Mutex<usize>,
}

#[async_trait]
impl JudgeLlm for FlakyJudge {
    async fn run_agent(
        &self,
        _system: &str,
        _user: &str,
        _tools: Vec<arena_core::judging::ToolDef>,
        _prior: Option<&str>,
    ) -> Result<AgentResponse, JudgeError> {
        let n = {
            let mut g = self.calls.lock().unwrap();
            *g += 1;
            *g
        };
        if n == self.fail_on_call {
            return Err(JudgeError::Llm("provider exploded".to_string()));
        }
        Ok(AgentResponse::Final {
            text: r#"{"rating": -25.0, "feedback": "partial evidence of pre-existing code"}"#
                .to_string(),
        })
    }
}

async fn insert_task_at(db: &DatabaseConnection, project_id: Uuid, ordinal: i32) -> Uuid {
    let id = Uuid::new_v4();
    tasks::ActiveModel {
        id: Set(id),
        project_id_fk: Set(project_id),
        ordinal: Set(ordinal),
        title: Set(format!("Task {ordinal}")),
        content: Set("do the thing".to_string()),
        test_template: Set(serde_json::json!({"kind": "shell"})),
        created_at: Set(Utc::now()),
        tags: Set(String::new()),
        point_value: Set(10),
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

/// A session-scoped judge attached to every task, as the seed would.
async fn insert_session_judge(db: &DatabaseConnection, task_ids: &[Uuid]) -> (Uuid, JudgeRow) {
    let judge_id = Uuid::new_v4();
    let scale = serde_json::json!({"min": -500.0, "max": 0.0, "step": 1.0});
    judges::ActiveModel {
        id: Set(judge_id),
        slug: Set("anti-cheater".to_string()),
        name: Set("Anti Cheater".to_string()),
        description: Set(String::new()),
        prompt: Set("You are an anti-cheat judge.".to_string()),
        rating_scale: Set(scale.clone()),
        kind: Set("llm".to_string()),
        scope: Set("session".to_string()),
        evidence_mode: Set("tools".to_string()),
        evidence_needs: Set(None),
        created_at: Set(Utc::now()),
        updated_at: Set(Utc::now()),
        llm_provider_id_fk: Set(None),
        llm_model: Set(None),
        llm_pool_id_fk: Set(None),
        llm_source_order: Set(arena_core::llm::resolve::SOURCE_ORDER_POOL_FIRST.to_string()),
        criteria: Set(None),
        max_interactive: Set(None),
        avatar_url: Set(None),
        ignore_paths: Set(None),
        probes_config: Set(None),
    }
    .insert(db)
    .await
    .expect("insert judge");

    for task_id in task_ids {
        task_judges::ActiveModel {
            id: Set(Uuid::new_v4()),
            task_id: Set(*task_id),
            judge_id: Set(judge_id),
            ordinal: Set(0),
            rating_scale_override: Set(None),
            created_at: Set(Utc::now()),
            updated_at: Set(Utc::now()),
            weight: Set(None),
        }
        .insert(db)
        .await
        .expect("attach judge");
    }

    (
        judge_id,
        JudgeRow {
            slug: "anti-cheater".to_string(),
            name: "Anti Cheater".to_string(),
            prompt: "You are an anti-cheat judge.".to_string(),
            rating_scale: scale,
            kind: "llm".to_string(),
            scope: "session".to_string(),
            evidence_mode: "tools".to_string(),
            evidence_needs: None,
            llm_provider_id: None,
            llm_pool_id: None,
            llm_source_order: arena_core::llm::resolve::SOURCE_ORDER_POOL_FIRST.to_string(),
            llm_model: None,
            criteria: None,
            max_interactive: None,
            ignore_paths: None,
        },
    )
}

async fn task_judge_map(
    db: &DatabaseConnection,
    judge_id: Uuid,
    task_ids: &[Uuid],
) -> HashMap<Uuid, TaskJudgeRow> {
    task_judges::Entity::find()
        .filter(task_judges::Column::JudgeId.eq(judge_id))
        .filter(task_judges::Column::TaskId.is_in(task_ids.iter().copied()))
        .all(db)
        .await
        .expect("task_judges")
        .into_iter()
        .map(|tj| {
            (
                tj.task_id,
                TaskJudgeRow {
                    id: tj.id,
                    task_id: tj.task_id,
                    judge_id: tj.judge_id,
                    rating_scale_override: tj.rating_scale_override,
                    weight: None,
                },
            )
        })
        .collect()
}

async fn mark_completed(db: &DatabaseConnection, session_id: Uuid, player_id: Uuid) {
    session_scheduler_state::ActiveModel {
        id: Set(Uuid::new_v4()),
        session_id_fk: Set(session_id),
        player_id_fk: Set(player_id),
        task_id: Set(None),
        state: Set(SCHEDULER_STATE_COMPLETED.to_string()),
        next_probe_at: Set(None),
        created_at: Set(Utc::now()),
        updated_at: Set(Utc::now()),
    }
    .insert(db)
    .await
    .expect("insert scheduler row");
}

#[tokio::test]
async fn one_run_scores_every_reached_task_and_clears_the_settle_poll() {
    let db = setup_db().await;
    let owner = insert_user(&db).await;
    let project = insert_project(&db, owner).await;
    let session = insert_session(&db, project).await;
    let player = insert_player(&db, session).await;

    let tasks: Vec<Uuid> = vec![
        insert_task_at(&db, project, 0).await,
        insert_task_at(&db, project, 1).await,
        insert_task_at(&db, project, 2).await,
    ];
    let (judge_id, judge_row) = insert_session_judge(&db, &tasks).await;
    mark_completed(&db, session, player).await;
    // Each task banked 30, so there is something a verdict could withdraw.
    for t in &tasks {
        award_points(&db, session, player, *t, 20, 10).await;
    }

    let repo = tempfile::tempdir().unwrap();
    make_repo(repo.path());
    write_file(repo.path(), "README.md", "scaffold\n");
    commit(repo.path(), "initial scaffold");
    for t in &tasks {
        write_file(repo.path(), &format!("sol-{t}.py"), "print(1)\n");
        commit(repo.path(), &format!("feat({t}): work"));
    }

    // The settle poll expects one row per task before any run happens.
    let reached = reached_tasks_for_player(&db, session, player)
        .await
        .expect("reached");
    assert_eq!(reached.len(), 3);
    assert_eq!(
        expired_session_pending_judges(&db, session)
            .await
            .expect("pending"),
        3,
        "three pairs pending before the run"
    );

    let map = task_judge_map(&db, judge_id, &reached).await;
    let llm = FlakyJudge {
        fail_on_call: 0, // never fails
        calls: std::sync::Mutex::new(0),
    };
    let out = run_session_judge(
        &db,
        &llm,
        repo.path(),
        session,
        player,
        &reached,
        &map,
        &judge_row,
        "fake-model",
        "fake",
    )
    .await
    .expect("session judge run");

    assert_eq!(out.scored, 3, "one verdict per reached task");
    assert_eq!(out.failed, 0);
    assert!(
        out.dossier_json.contains("root_commit"),
        "the evidence handed to the model is retained for the run detail"
    );

    // One row per (task_judge, player), all scored, and the poll is clear.
    let rows = judge_results::Entity::find()
        .filter(judge_results::Column::SessionIdFk.eq(session))
        .all(&db)
        .await
        .expect("rows");
    assert_eq!(rows.len(), 3, "exactly one row per task_judge");
    assert!(rows.iter().all(|r| r.status == "scored"));
    assert!(rows.iter().all(|r| r.point_delta == -25));
    assert_eq!(
        expired_session_pending_judges(&db, session)
            .await
            .expect("pending"),
        0,
        "awards may proceed"
    );
}

#[tokio::test]
async fn a_failed_task_verdict_still_writes_a_terminal_row() {
    let db = setup_db().await;
    let owner = insert_user(&db).await;
    let project = insert_project(&db, owner).await;
    let session = insert_session(&db, project).await;
    let player = insert_player(&db, session).await;

    let tasks: Vec<Uuid> = vec![
        insert_task_at(&db, project, 0).await,
        insert_task_at(&db, project, 1).await,
        insert_task_at(&db, project, 2).await,
    ];
    let (judge_id, judge_row) = insert_session_judge(&db, &tasks).await;
    mark_completed(&db, session, player).await;
    for t in &tasks {
        award_points(&db, session, player, *t, 20, 10).await;
    }

    let repo = tempfile::tempdir().unwrap();
    make_repo(repo.path());
    write_file(repo.path(), "README.md", "scaffold\n");
    commit(repo.path(), "initial scaffold");

    let reached = reached_tasks_for_player(&db, session, player)
        .await
        .expect("reached");
    let map = task_judge_map(&db, judge_id, &reached).await;

    // The second task's verdict call blows up; the other two must still land.
    let llm = FlakyJudge {
        fail_on_call: 2,
        calls: std::sync::Mutex::new(0),
    };
    let out = run_session_judge(
        &db,
        &llm,
        repo.path(),
        session,
        player,
        &reached,
        &map,
        &judge_row,
        "fake-model",
        "fake",
    )
    .await
    .expect("the run itself must not abort on one task's failure");

    assert_eq!(out.scored, 2);
    assert_eq!(out.failed, 1);

    let rows = judge_results::Entity::find()
        .filter(judge_results::Column::SessionIdFk.eq(session))
        .all(&db)
        .await
        .expect("rows");
    assert_eq!(
        rows.len(),
        3,
        "every reached task got a row, failure included"
    );
    let failed: Vec<_> = rows.iter().filter(|r| r.status == "failed").collect();
    assert_eq!(failed.len(), 1);
    assert_eq!(
        failed[0].point_delta, 0,
        "an unjudgeable task must neither reward nor penalize the player"
    );
    assert!(failed[0].error.is_some(), "the reason is recorded");

    assert_eq!(
        expired_session_pending_judges(&db, session)
            .await
            .expect("pending"),
        0,
        "a failed verdict is terminal: awards are not held hostage to it"
    );
}

/// Records the prompts it is shown, so the evidence scope can be inspected.
struct PromptCapturingJudge {
    seen: std::sync::Mutex<Vec<String>>,
}

#[async_trait]
impl JudgeLlm for PromptCapturingJudge {
    async fn run_agent(
        &self,
        _system: &str,
        user: &str,
        _tools: Vec<arena_core::judging::ToolDef>,
        _prior: Option<&str>,
    ) -> Result<AgentResponse, JudgeError> {
        self.seen.lock().unwrap().push(user.to_string());
        Ok(AgentResponse::Final {
            text: r#"{"rating": 0.0, "feedback": "clean"}"#.to_string(),
        })
    }
}

// Projects attach `anti-cheater` to the FIRST task only, for one overall
// verdict on the -500 scale. That must still be a *session-wide* judgment: the
// evidence scope is every task the player reached, while the scoring scope is
// only what the judge is attached to. Conflating the two would silently reduce
// this judge to inspecting task 0 in isolation — exactly the blindness that
// session scope exists to remove.
#[tokio::test]
async fn evidence_spans_the_session_even_when_only_one_task_is_scored() {
    let db = setup_db().await;
    let owner = insert_user(&db).await;
    let project = insert_project(&db, owner).await;
    let session = insert_session(&db, project).await;
    let player = insert_player(&db, session).await;

    let tasks: Vec<Uuid> = vec![
        insert_task_at(&db, project, 0).await,
        insert_task_at(&db, project, 1).await,
        insert_task_at(&db, project, 2).await,
    ];
    // Attached to task 0 only, as the extreme-startup projects do.
    let (judge_id, judge_row) = insert_session_judge(&db, &tasks[..1]).await;
    mark_completed(&db, session, player).await;
    award_points(&db, session, player, tasks[0], 20, 10).await;

    let repo = tempfile::tempdir().unwrap();
    make_repo(repo.path());
    write_file(repo.path(), "README.md", "scaffold\n");
    commit(repo.path(), "initial scaffold");
    for t in &tasks {
        write_file(repo.path(), &format!("sol-{t}.py"), "print(1)\n");
        commit(repo.path(), &format!("feat({t}): work"));
    }

    let reached = reached_tasks_for_player(&db, session, player)
        .await
        .expect("reached");
    assert_eq!(reached.len(), 3, "the player reached all three tasks");

    let map = task_judge_map(&db, judge_id, &reached).await;
    assert_eq!(map.len(), 1, "but the judge is attached to only one");

    let llm = PromptCapturingJudge {
        seen: std::sync::Mutex::new(Vec::new()),
    };
    let out = run_session_judge(
        &db,
        &llm,
        repo.path(),
        session,
        player,
        &reached,
        &map,
        &judge_row,
        "fake-model",
        "fake",
    )
    .await
    .expect("session judge run");

    assert_eq!(out.scored, 1, "one verdict row, for the attached task");

    // The prompt it was given must nevertheless describe all three tasks.
    let prompts = llm.seen.lock().unwrap().clone();
    assert_eq!(prompts.len(), 1);
    for t in &tasks {
        assert!(
            prompts[0].contains(&t.to_string()),
            "task {t} missing from the evidence — the judge is seeing a slice, not the session"
        );
    }

    // And the settle poll is satisfied by that single row.
    assert_eq!(
        expired_session_pending_judges(&db, session)
            .await
            .expect("pending"),
        0
    );
}

/// Award `points` for a task the way the probe/bonus paths do.
async fn award_points(
    db: &DatabaseConnection,
    session_id: Uuid,
    player_id: Uuid,
    task_id: Uuid,
    from_probes: i32,
    bonus: i32,
) {
    for (delta, is_bonus) in [(from_probes, false), (bonus, true)] {
        if delta == 0 {
            continue;
        }
        task_results::ActiveModel {
            id: Set(Uuid::new_v4()),
            session_id_fk: Set(session_id),
            player_id_fk: Set(player_id),
            task_id: Set(Some(task_id)),
            answer: Set(String::new()),
            created_at: Set(Utc::now()),
            point_delta: Set(delta),
            is_bonus: Set(is_bonus),
        }
        .insert(db)
        .await
        .expect("insert task_result");
    }
}

/// Rates every task at the floor of whatever scale it is offered, by reading
/// the bound out of the prompt. Lets the test assert the ceiling is enforced.
struct MaxClawbackJudge {
    seen: std::sync::Mutex<Vec<String>>,
}

#[async_trait]
impl JudgeLlm for MaxClawbackJudge {
    async fn run_agent(
        &self,
        _system: &str,
        user: &str,
        _tools: Vec<arena_core::judging::ToolDef>,
        _prior: Option<&str>,
    ) -> Result<AgentResponse, JudgeError> {
        self.seen.lock().unwrap().push(user.to_string());
        // The prompt states the floor as "`-N` — withdraw everything".
        let floor: f64 = user
            .split("`-")
            .nth(1)
            .and_then(|rest| rest.split('`').next())
            .and_then(|n| n.parse::<f64>().ok())
            .map(|n| -n)
            .expect("prompt must state the withdrawal floor");
        Ok(AgentResponse::Final {
            text: format!(
                r#"{{"rating": {floor}, "feedback": "solution present at the root commit"}}"#
            ),
        })
    }
}

// A cheating verdict reverses a reward, so it must never take more than the
// task paid. The judge cannot exceed that ceiling even when it tries to.
#[tokio::test]
async fn clawback_is_capped_at_the_points_the_task_paid() {
    let db = setup_db().await;
    let owner = insert_user(&db).await;
    let project = insert_project(&db, owner).await;
    let session = insert_session(&db, project).await;
    let player = insert_player(&db, session).await;

    let t0 = insert_task_at(&db, project, 0).await;
    let t1 = insert_task_at(&db, project, 1).await;
    let (judge_id, judge_row) = insert_session_judge(&db, &[t0, t1]).await;
    mark_completed(&db, session, player).await;

    // t0 banked 30 (20 probes + 10 bonus); t1 banked nothing.
    award_points(&db, session, player, t0, 20, 10).await;

    let repo = tempfile::tempdir().unwrap();
    make_repo(repo.path());
    write_file(repo.path(), "README.md", "scaffold\n");
    commit(repo.path(), "initial scaffold");

    let reached = reached_tasks_for_player(&db, session, player)
        .await
        .expect("reached");
    let map = task_judge_map(&db, judge_id, &reached).await;
    let llm = MaxClawbackJudge {
        seen: std::sync::Mutex::new(Vec::new()),
    };
    let out = run_session_judge(
        &db,
        &llm,
        repo.path(),
        session,
        player,
        &reached,
        &map,
        &judge_row,
        "fake-model",
        "fake",
    )
    .await
    .expect("session judge run");

    // t1 earned nothing, so no verdict was needed and no LLM call was made.
    assert_eq!(out.skipped, 1, "the task that paid nothing is skipped");
    assert_eq!(out.scored, 1, "only the task that paid is judged");
    assert_eq!(
        llm.seen.lock().unwrap().len(),
        1,
        "no tokens spent on a task with nothing to withdraw"
    );

    let rows = judge_results::Entity::find()
        .filter(judge_results::Column::SessionIdFk.eq(session))
        .all(&db)
        .await
        .expect("rows");
    assert_eq!(rows.len(), 2, "both reached tasks still get a terminal row");

    let by_delta: Vec<i32> = {
        let mut v: Vec<i32> = rows.iter().map(|r| r.point_delta).collect();
        v.sort();
        v
    };
    assert_eq!(
        by_delta,
        vec![-30, 0],
        "the full 30 points are withdrawn and no more; the unpaid task stays at 0"
    );

    // Net effect: the cheated task is worth exactly zero, not negative.
    let awarded: i32 = task_results::Entity::find()
        .filter(task_results::Column::SessionIdFk.eq(session))
        .all(&db)
        .await
        .expect("task_results")
        .iter()
        .map(|r| r.point_delta)
        .sum();
    let judged: i32 = rows.iter().map(|r| r.point_delta).sum();
    assert_eq!(
        awarded + judged,
        0,
        "withdrawing everything returns the player to zero, never below it"
    );
}

// An admin override tightens the clawback below the full amount; it must not be
// able to loosen it past what the task paid.
#[tokio::test]
async fn a_task_judge_override_can_only_tighten_the_clawback() {
    let db = setup_db().await;
    let owner = insert_user(&db).await;
    let project = insert_project(&db, owner).await;
    let session = insert_session(&db, project).await;
    let player = insert_player(&db, session).await;

    let t0 = insert_task_at(&db, project, 0).await;
    let (judge_id, judge_row) = insert_session_judge(&db, &[t0]).await;
    mark_completed(&db, session, player).await;
    award_points(&db, session, player, t0, 40, 0).await;

    // Cap the clawback at 10 of the 40 points.
    let tj = task_judges::Entity::find()
        .filter(task_judges::Column::JudgeId.eq(judge_id))
        .one(&db)
        .await
        .expect("query")
        .expect("row");
    let mut am: task_judges::ActiveModel = tj.into();
    am.rating_scale_override = Set(Some(
        serde_json::json!({"min": -10.0, "max": 0.0, "step": 1.0}),
    ));
    am.update(&db).await.expect("set override");

    let repo = tempfile::tempdir().unwrap();
    make_repo(repo.path());
    write_file(repo.path(), "README.md", "scaffold\n");
    commit(repo.path(), "initial scaffold");

    let reached = reached_tasks_for_player(&db, session, player)
        .await
        .expect("reached");
    let map = task_judge_map(&db, judge_id, &reached).await;
    let llm = MaxClawbackJudge {
        seen: std::sync::Mutex::new(Vec::new()),
    };
    run_session_judge(
        &db,
        &llm,
        repo.path(),
        session,
        player,
        &reached,
        &map,
        &judge_row,
        "fake-model",
        "fake",
    )
    .await
    .expect("session judge run");

    let row = judge_results::Entity::find()
        .filter(judge_results::Column::SessionIdFk.eq(session))
        .one(&db)
        .await
        .expect("query")
        .expect("row");
    assert_eq!(
        row.point_delta, -10,
        "the override caps the withdrawal at 10, not the full 40 the task paid"
    );
}
