//! `run_session_appraisal` — one verdict about a session, paid once.
//!
//! The contract: the settle poll needs a terminal row on **every** attached
//! pair, and the player needs the points on exactly **one** of them. A run
//! that pays each pair multiplies a single verdict by the size of the ladder;
//! one that writes only the primary row hangs the award flow. Both failures
//! are silent, so they get a test.

use crate::common;
use crate::common::*;

use std::collections::HashMap;

use arena_core::entities::{judge_results, judges, task_judges, tasks};
use arena_core::judging::appraisal::{
    AppraisalInputs, is_appraisal_judge, primary_pair, run_session_appraisal,
};
use arena_core::judging::criteria::CriteriaContext;
use arena_core::judging::{AgentResponse, JudgeRow, TaskJudgeRow};
use arena_core::validation::judge_results::RatingScale;
use chrono::Utc;
use sea_orm::{ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, Set};
use uuid::Uuid;

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
        point_value: Set(100),
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

/// The workflow judge as the seed defines it: session-scoped, positive scale.
async fn insert_appraisal_judge(
    db: &DatabaseConnection,
    task_ids: &[Uuid],
) -> (JudgeRow, Vec<Uuid>) {
    let judge_id = Uuid::new_v4();
    judges::ActiveModel {
        id: Set(judge_id),
        slug: Set("agentic".to_string()),
        name: Set("Agentic".to_string()),
        description: Set(String::new()),
        prompt: Set("Judge the workflow engineering.".to_string()),
        rating_scale: Set(serde_json::json!({"min": 0.0, "max": 10.0, "step": 0.1})),
        kind: Set("llm".to_string()),
        scope: Set("session".to_string()),
        evidence_mode: Set("tools".to_string()),
        evidence_needs: Set(Some(r#"["agent_setup"]"#.to_string())),
        created_at: Set(Utc::now()),
        updated_at: Set(Utc::now()),
        llm_provider_id_fk: Set(None),
        llm_model: Set(None),
        llm_pool_id_fk: Set(None),
        llm_source_order: Set(arena_core::llm::resolve::SOURCE_ORDER_POOL_FIRST.to_string()),
        criteria: Set(Some(r#"["agentic"]"#.to_string())),
        probes_config: Set(None),
        max_interactive: Set(None),
        avatar_url: Set(None),
        ignore_paths: Set(None),
    }
    .insert(db)
    .await
    .expect("insert judge");

    let mut pair_ids = Vec::new();
    for (ordinal, task_id) in task_ids.iter().enumerate() {
        let id = Uuid::new_v4();
        task_judges::ActiveModel {
            id: Set(id),
            task_id: Set(*task_id),
            judge_id: Set(judge_id),
            ordinal: Set(ordinal as i32),
            rating_scale_override: Set(None),
            weight: Set(Some(1.0)),
            created_at: Set(Utc::now()),
            updated_at: Set(Utc::now()),
        }
        .insert(db)
        .await
        .expect("insert task_judge");
        pair_ids.push(id);
    }

    let judge_row = JudgeRow {
        slug: "agentic".to_string(),
        name: "Agentic".to_string(),
        prompt: "Judge the workflow engineering.".to_string(),
        rating_scale: serde_json::json!({"min": 0.0, "max": 10.0, "step": 0.1}),
        kind: "llm".to_string(),
        scope: "session".to_string(),
        llm_provider_id: None,
        llm_model: None,
        llm_pool_id: None,
        llm_source_order: arena_core::llm::resolve::SOURCE_ORDER_POOL_FIRST.to_string(),
        evidence_mode: "tools".to_string(),
        evidence_needs: Some(r#"["agent_setup"]"#.to_string()),
        criteria: Some(r#"["agentic"]"#.to_string()),
        max_interactive: None,
        ignore_paths: None,
    };
    (judge_row, pair_ids)
}

fn criteria_ctx() -> CriteriaContext {
    CriteriaContext {
        keys: vec!["agentic".to_string()],
        weights: [("agentic".to_string(), 1.0)].into_iter().collect(),
    }
}

#[tokio::test]
async fn one_verdict_is_paid_once_and_closes_every_pair() {
    let db = common::setup_db().await;
    let owner = common::insert_user(&db).await;
    let project = common::insert_project(&db, owner).await;
    let session = common::insert_session(&db, project).await;
    let player = common::insert_player(&db, session).await;
    let t0 = insert_task_at(&db, project, 0).await;
    let t1 = insert_task_at(&db, project, 1).await;
    let (judge_row, pair_ids) = insert_appraisal_judge(&db, &[t0, t1]).await;

    let task_judges_map: HashMap<Uuid, TaskJudgeRow> = [(t0, 0usize), (t1, 1usize)]
        .into_iter()
        .map(|(task_id, i)| {
            (
                task_id,
                TaskJudgeRow {
                    id: pair_ids[i],
                    task_id,
                    judge_id: Uuid::nil(),
                    rating_scale_override: None,
                    weight: Some(1.0),
                },
            )
        })
        .collect();

    let reached = vec![t0, t1];
    let primary = primary_pair(&reached, &task_judges_map).expect("a pair to score");
    assert_eq!(primary.task_id, t0, "the first reached task carries it");

    let tmp = tempfile::tempdir().expect("tempdir");
    common::make_repo(tmp.path());
    common::write_file(tmp.path(), "AGENTS.md", "Run with `make dev`.\n");
    common::commit(tmp.path(), "root: session start");

    let llm = FakeJudgeLlm::new(vec![AgentResponse::Final {
        text: r#"{"criteria": [{"key": "agentic", "score": 8.0, "rationale": "AGENTS.md is accurate", "evidence": ["file:AGENTS.md:1"]}], "feedback": "Solid setup; wire an MCP server next time."}"#
            .to_string(),
    }]);
    let inputs = AppraisalInputs {
        primary_task_judge_id: primary.id,
        scale: RatingScale {
            min: 0.0,
            max: 20.0,
            step: 1.0,
        },
        criteria: Some(&criteria_ctx()),
        agent_setup: true,
    };

    let out = run_session_appraisal(
        &db,
        &llm,
        tmp.path(),
        session,
        player,
        &reached,
        &task_judges_map,
        &judge_row,
        &inputs,
        "test-model",
        "test-provider",
    )
    .await
    .expect("appraisal runs");

    assert_eq!(out.scored, 2, "every attached pair gets a terminal row");
    assert_eq!(out.failed, 0);

    let rows = judge_results::Entity::find()
        .filter(judge_results::Column::PlayerIdFk.eq(player))
        .all(&db)
        .await
        .expect("rows");
    assert_eq!(rows.len(), 2);
    let scored: Vec<_> = rows
        .iter()
        .filter(|r| r.task_judge_id == primary.id)
        .collect();
    let carried = scored.first().expect("primary row");
    // 8.0/10 of a 0..20 share.
    assert_eq!(carried.point_delta, 16);
    assert_eq!(carried.status, "scored");
    assert_eq!(
        carried.rating.get("overall").and_then(|v| v.as_f64()),
        Some(16.0),
        "the sheet is stored whole so the scorecard can render it"
    );
    let secondary = rows
        .iter()
        .find(|r| r.task_judge_id != primary.id)
        .expect("secondary row");
    assert_eq!(secondary.point_delta, 0, "one verdict, one payout");
    assert_eq!(secondary.status, "scored");
    assert!(
        secondary.rating.is_null(),
        "no second scorecard: the sheet belongs to the row that was scored"
    );
    assert!(
        secondary.feedback.contains("session as a whole") && secondary.feedback.contains("task 0"),
        "the zero-point row points at where the verdict lives: {}",
        secondary.feedback
    );
}

#[tokio::test]
async fn the_scale_sign_decides_which_session_runner_applies() {
    let mut row = JudgeRow {
        slug: "agentic".to_string(),
        name: "Agentic".to_string(),
        prompt: String::new(),
        rating_scale: serde_json::json!({"min": 0.0, "max": 10.0, "step": 0.1}),
        kind: "llm".to_string(),
        scope: "session".to_string(),
        llm_provider_id: None,
        llm_model: None,
        llm_pool_id: None,
        llm_source_order: arena_core::llm::resolve::SOURCE_ORDER_POOL_FIRST.to_string(),
        evidence_mode: "tools".to_string(),
        evidence_needs: None,
        criteria: None,
        max_interactive: None,
        ignore_paths: None,
    };
    assert!(is_appraisal_judge(&row), "a positive scale awards");
    // Anti-cheat: a scale that can only remove points stays on the clawback
    // runner, which caps each task at what that task actually paid.
    row.rating_scale = serde_json::json!({"min": -500.0, "max": 0.0, "step": 1.0});
    assert!(!is_appraisal_judge(&row));
}
