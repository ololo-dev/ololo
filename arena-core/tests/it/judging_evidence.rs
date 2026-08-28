//! `build_evidence`: the one snapshot every judge reads from.
//!
//! The cases here are the reach a judge did not have before — the player's
//! other tasks, the answers its probes recorded, and the session's memory —
//! plus the observation that motivated the whole redesign: an empty work
//! window is a fact to look into, reported as such and never as a verdict.

use crate::common;
use crate::common::*;
use arena_core::judging::tools::ToolScope;

use arena_core::entities::{player_memory, probes, projects, task_results, tests};
use arena_core::judging::evidence::{EvidenceNeeds, build_evidence};
use arena_core::validation::judge_results::RatingScale;
use chrono::Utc;
use sea_orm::{ActiveModelTrait, ActiveValue::Set, DatabaseConnection, EntityTrait};
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

async fn award(db: &DatabaseConnection, session: Uuid, player: Uuid, task: Uuid, points: i32) {
    task_results::ActiveModel {
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

async fn insert_test(db: &DatabaseConnection, session: Uuid, task: Uuid, ordinal: i32) -> Uuid {
    let id = Uuid::new_v4();
    tests::ActiveModel {
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
        initiator: Set("system".to_string()),
        probe_config: Set(None),
        registered_by_judge_id: Set(None),
    }
    .insert(db)
    .await
    .expect("insert test");
    id
}

#[allow(clippy::too_many_arguments)]
async fn insert_probe(
    db: &DatabaseConnection,
    session: Uuid,
    player: Uuid,
    test_id: Uuid,
    outcome: &str,
    answer: &str,
    point_delta: i32,
) {
    probes::ActiveModel {
        id: Set(Uuid::new_v4()),
        test_id: Set(test_id),
        player_id: Set(player),
        session_id: Set(session),
        attempt: Set(1),
        rendered_command: Set("echo ok".to_string()),
        fixture_values: Set("{}".to_string()),
        expected_answer: Set(None),
        outcome: Set(Some(outcome.to_string())),
        dispatched_at: Set(Utc::now()),
        deadline_at: Set(Utc::now()),
        resolved_at: Set(Some(Utc::now())),
        updated_at: Set(Some(Utc::now())),
        output: Set(None),
        exit_code: Set(Some(0)),
        duration_ms: Set(Some(12)),
        point_delta: Set(Some(point_delta)),
        resolved_answer: Set(Some(answer.to_string())),
        secret_meta: Set(None),
        artifact_path: Set(None),
        result_json: Set(None),
    }
    .insert(db)
    .await
    .expect("insert probe");
}

/// A repo with one commit, which is what the dossier needs to read a diff.
fn repo_with_a_commit(dir: &Path) -> String {
    make_repo(dir);
    write_file(dir, "solution.sh", "echo hi\n");
    commit(dir, "chore: scaffold")
}

#[tokio::test]
async fn a_judge_can_see_the_players_other_tasks() {
    // The reach a golf ladder needs: rung 4 has to know that rung 5 — a
    // tighter budget on the same solution — already passed.
    let db = setup_db().await;
    let owner = insert_user(&db).await;
    let project = insert_project(&db, owner).await;
    let session = insert_session(&db, project).await;
    let player = insert_player(&db, session).await;
    let dir = tempfile::tempdir().unwrap();
    repo_with_a_commit(dir.path());

    let rung_140 = insert_task(&db, project, 0, "Golf to 140", r#"["code-golf"]"#).await;
    let rung_120 = insert_task(&db, project, 1, "Golf to 120", r#"["code-golf"]"#).await;
    award(&db, session, player, rung_140, 25).await;
    award(&db, session, player, rung_120, 25).await;

    let ev = build_evidence(
        &db,
        dir.path(),
        session,
        player,
        rung_140,
        "golf-verify",
        &scale(-25.0, 0.0),
        None,
        EvidenceNeeds::everything(),
        &ToolScope::everything(),
    )
    .await
    .expect("build_evidence");

    let titles: Vec<&str> = ev.tasks.iter().map(|t| t.title.as_str()).collect();
    assert!(titles.contains(&"Golf to 140"), "got {titles:?}");
    assert!(titles.contains(&"Golf to 120"), "got {titles:?}");
    let tighter = ev
        .tasks
        .iter()
        .find(|t| t.title == "Golf to 120")
        .expect("the tighter rung");
    assert_eq!(tighter.earned, 25);
    assert_eq!(tighter.tags, vec!["code-golf".to_string()]);
}

#[tokio::test]
async fn an_empty_work_window_is_reported_as_a_fact() {
    // The anti-cheat case: no diff is a reason to look closer, and the
    // evidence says so plainly rather than implying guilt.
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
        &ToolScope::everything(),
    )
    .await
    .expect("build_evidence");

    assert!(ev.task.diff_is_empty);
    assert_eq!(ev.task.earned, 0);
    assert_eq!(ev.task.title, "Implement it");
}

#[tokio::test]
async fn probe_answers_reach_the_judge() {
    // Only the execution judge could read these before. A byte-budget rung is
    // unreadable without the number the probe actually returned.
    let db = setup_db().await;
    let owner = insert_user(&db).await;
    let project = insert_project(&db, owner).await;
    let session = insert_session(&db, project).await;
    let player = insert_player(&db, session).await;
    let dir = tempfile::tempdir().unwrap();
    repo_with_a_commit(dir.path());

    let task = insert_task(&db, project, 0, "Golf to 140", r#"["code-golf"]"#).await;
    let size_test = insert_test(&db, session, task, 2).await;
    insert_probe(&db, session, player, size_test, "pass", "138", 25).await;

    let ev = build_evidence(
        &db,
        dir.path(),
        session,
        player,
        task,
        "golf-verify",
        &scale(-25.0, 0.0),
        None,
        EvidenceNeeds::everything(),
        &ToolScope::everything(),
    )
    .await
    .expect("build_evidence");

    assert_eq!(ev.task.probes.len(), 1);
    let p = &ev.task.probes[0];
    assert_eq!(p.answer.as_deref(), Some("138"));
    assert_eq!(p.outcome.as_deref(), Some("pass"));
    assert_eq!(p.test_ordinal, Some(2));
    assert_eq!(p.point_delta, 25);
}

#[tokio::test]
async fn a_judge_gets_only_what_it_declared() {
    // The cross-task view walks git twice per task the player reached. A
    // judge that never reads it should not pay for it — and, more important,
    // must not receive an empty `tasks[]` it could mistake for "the player
    // reached nothing".
    let db = setup_db().await;
    let owner = insert_user(&db).await;
    let project = insert_project(&db, owner).await;
    let session = insert_session(&db, project).await;
    let player = insert_player(&db, session).await;
    let dir = tempfile::tempdir().unwrap();
    repo_with_a_commit(dir.path());

    let mut p: projects::ActiveModel = projects::Entity::find_by_id(project)
        .one(&db)
        .await
        .unwrap()
        .unwrap()
        .into();
    p.memory_schema = Set(Some(r#"{"run": "sh solution.sh"}"#.to_string()));
    p.update(&db).await.expect("set memory schema");

    let first = insert_task(&db, project, 0, "Golf to 140", r#"["code-golf"]"#).await;
    let second = insert_task(&db, project, 1, "Golf to 120", r#"["code-golf"]"#).await;
    award(&db, session, player, first, 25).await;
    award(&db, session, player, second, 25).await;
    let t = insert_test(&db, session, second, 0).await;
    insert_probe(&db, session, player, t, "pass", "118", 25).await;

    let build = async |needs| {
        build_evidence(
            &db,
            dir.path(),
            session,
            player,
            second,
            "golf-verify",
            &scale(-25.0, 0.0),
            None,
            needs,
            &ToolScope::everything(),
        )
        .await
        .expect("build_evidence")
    };

    let all = build(EvidenceNeeds::everything()).await;
    assert_eq!(all.tasks.len(), 2);
    assert_eq!(all.task.probes.len(), 1);
    assert!(!all.memory.is_empty());

    let nothing = build(EvidenceNeeds::minimal()).await;
    assert!(nothing.tasks.is_empty(), "no cross-task view was asked for");
    assert!(nothing.task.probes.is_empty());
    assert!(nothing.memory.is_empty());
    // The floor stands regardless: there is no verdict to reach without it.
    assert_eq!(nothing.task.title, "Golf to 120");
    assert_eq!(nothing.task.earned, 25);
    assert_eq!(nothing.judge.slug, "golf-verify");

    let probes_only = build(EvidenceNeeds::parse(&["probes".to_string()]).unwrap()).await;
    assert_eq!(probes_only.task.probes.len(), 1);
    assert!(probes_only.tasks.is_empty());
    assert!(probes_only.memory.is_empty());
}

#[test]
fn an_unknown_section_is_rejected_by_name() {
    // Silently ignoring it would hand the judge less evidence than it asked
    // for, and it would conclude from the gap.
    let err = EvidenceNeeds::parse(&["tasks".to_string(), "diffs".to_string()])
        .expect_err("unknown section");
    assert!(err.contains("diffs"), "got {err}");

    // An undeclared judge keeps everything; that is the pre-declaration
    // behaviour and the only safe direction to be wrong in.
    assert_eq!(
        EvidenceNeeds::from_column(None),
        EvidenceNeeds::everything()
    );
    assert_eq!(
        EvidenceNeeds::from_column(Some("not json")),
        EvidenceNeeds::everything()
    );
    assert_eq!(
        EvidenceNeeds::from_column(Some(r#"["probes"]"#)),
        EvidenceNeeds::parse(&["probes".to_string()]).unwrap()
    );
    // A judge that declared nothing at all asked for nothing at all.
    assert_eq!(
        EvidenceNeeds::from_column(Some("[]")),
        EvidenceNeeds::minimal()
    );
}

#[tokio::test]
async fn an_oversized_answer_is_cut_without_splitting_a_character() {
    // Probe answers are whatever the player's program printed. Cutting at a
    // byte offset would panic mid-character on any non-ASCII output, and take
    // the whole judge run down with it.
    let db = setup_db().await;
    let owner = insert_user(&db).await;
    let project = insert_project(&db, owner).await;
    let session = insert_session(&db, project).await;
    let player = insert_player(&db, session).await;
    let dir = tempfile::tempdir().unwrap();
    repo_with_a_commit(dir.path());

    let task = insert_task(&db, project, 0, "Print it", "[]").await;
    let t = insert_test(&db, session, task, 0).await;
    let shout = "ы".repeat(5_000);
    insert_probe(&db, session, player, t, "pass", &shout, 10).await;

    let ev = build_evidence(
        &db,
        dir.path(),
        session,
        player,
        task,
        "task-anti-cheat",
        &scale(-10.0, 0.0),
        None,
        EvidenceNeeds::everything(),
        &ToolScope::everything(),
    )
    .await
    .expect("build_evidence");

    let answer = ev.task.probes[0].answer.as_deref().expect("an answer");
    assert!(answer.ends_with("…[truncated]"));
    assert_eq!(answer.chars().filter(|c| *c == 'ы').count(), 2_000);
}

#[tokio::test]
async fn session_memory_reaches_the_judge() {
    // The `run:` line the player declared. The execution judge has always
    // needed it to render a probe; a judge reading the solution needs it to
    // know what the solution even is.
    let db = setup_db().await;
    let owner = insert_user(&db).await;
    let project = insert_project(&db, owner).await;
    let session = insert_session(&db, project).await;
    let player = insert_player(&db, session).await;
    let dir = tempfile::tempdir().unwrap();
    repo_with_a_commit(dir.path());

    let mut p: projects::ActiveModel = projects::Entity::find_by_id(project)
        .one(&db)
        .await
        .unwrap()
        .unwrap()
        .into();
    p.memory_schema = Set(Some(r#"{"run": "sh solution.sh"}"#.to_string()));
    p.update(&db).await.expect("set memory schema");

    player_memory::ActiveModel {
        id: Set(Uuid::new_v4()),
        session_id_fk: Set(session),
        player_id_fk: Set(player),
        values_json: Set(r#"{"run": "node s.js"}"#.to_string()),
        source_hash: Set(Some(String::new())),
        created_at: Set(Utc::now()),
        updated_at: Set(Utc::now()),
    }
    .insert(&db)
    .await
    .expect("insert player_memory");

    let task = insert_task(&db, project, 0, "Run it", "[]").await;
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
        &ToolScope::everything(),
    )
    .await
    .expect("build_evidence");

    // The extracted value wins over the project default.
    assert_eq!(ev.memory.get("run").map(String::as_str), Some("node s.js"));
}

#[tokio::test]
async fn the_judges_own_bounds_travel_with_the_evidence() {
    // A program deciding whether to spend an LLM call needs to know whether
    // it is even allowed to penalize.
    let db = setup_db().await;
    let owner = insert_user(&db).await;
    let project = insert_project(&db, owner).await;
    let session = insert_session(&db, project).await;
    let player = insert_player(&db, session).await;
    let dir = tempfile::tempdir().unwrap();
    repo_with_a_commit(dir.path());
    let task = insert_task(&db, project, 0, "Anything", "[]").await;

    let penalty = build_evidence(
        &db,
        dir.path(),
        session,
        player,
        task,
        "task-anti-cheat",
        &scale(-50.0, 0.0),
        None,
        EvidenceNeeds::everything(),
        &ToolScope::everything(),
    )
    .await
    .unwrap();
    assert!(penalty.judge.is_penalty);
    assert_eq!(penalty.judge.scale_min, -50.0);
    assert_eq!(penalty.judge.slug, "task-anti-cheat");

    let rating = build_evidence(
        &db,
        dir.path(),
        session,
        player,
        task,
        "architecture",
        &scale(0.0, 10.0),
        None,
        EvidenceNeeds::everything(),
        &ToolScope::everything(),
    )
    .await
    .unwrap();
    assert!(!rating.judge.is_penalty);
}

#[tokio::test]
async fn the_snapshot_serializes_whole() {
    // It has to survive a round trip: the same value feeds a deterministic
    // pre-check, a prompt, and the run record that explains the verdict later.
    let db = setup_db().await;
    let owner = insert_user(&db).await;
    let project = insert_project(&db, owner).await;
    let session = insert_session(&db, project).await;
    let player = insert_player(&db, session).await;
    let dir = tempfile::tempdir().unwrap();
    repo_with_a_commit(dir.path());
    let task = insert_task(&db, project, 0, "Anything", r#"["cli"]"#).await;
    award(&db, session, player, task, 10).await;

    let ev = build_evidence(
        &db,
        dir.path(),
        session,
        player,
        task,
        "task-anti-cheat",
        &scale(-10.0, 0.0),
        None,
        EvidenceNeeds::everything(),
        &ToolScope::everything(),
    )
    .await
    .unwrap();

    let json = ev.to_json();
    let back: serde_json::Value = serde_json::from_str(&json).expect("valid JSON");
    assert_eq!(back["task"]["earned"], 10);
    assert_eq!(back["judge"]["is_penalty"], true);
    assert!(back["tasks"].is_array());
    assert!(back["memory"].is_object());
}
