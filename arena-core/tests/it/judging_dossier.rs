//! `build_session_dossier` — the deterministic evidence a session-scoped judge
//! reasons over.
//!
//! The whole point of assembling this in Rust is that the joins are exact: a
//! judge that re-derived them from git tools could get them wrong and had no
//! way to be checked. These tests pin the joins and the objective flags.

use crate::common;
use crate::common::*;
use arena_core::judging::tools::ToolScope;

use arena_core::entities::{probes, task_agent_stats, tasks, tests as entity_tests};
use arena_core::judging::dossier::{
    FLAG_EMPTY_TASK_COMMIT, FLAG_NO_AGENT_STATS, FLAG_NO_TASK_COMMIT, FLAG_PASSED_WITHOUT_CHANGES,
    FLAG_ZERO_AGENT_ACTIVITY, build_session_dossier,
};
use arena_core::judging::task_dossier::build_task_dossier;
use chrono::Utc;
use sea_orm::{ActiveModelTrait, DatabaseConnection, Set};
use uuid::Uuid;

async fn insert_task_at(
    db: &DatabaseConnection,
    project_id: Uuid,
    ordinal: i32,
    title: &str,
) -> Uuid {
    let id = Uuid::new_v4();
    tasks::ActiveModel {
        id: Set(id),
        project_id_fk: Set(project_id),
        ordinal: Set(ordinal),
        title: Set(title.to_string()),
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

/// A `tests` row for the task — `probes` reference this, not the task, so the
/// dossier has to join through it.
async fn insert_test_for(db: &DatabaseConnection, session_id: Uuid, task_id: Uuid) -> Uuid {
    let id = Uuid::new_v4();
    entity_tests::ActiveModel {
        id: Set(id),
        command_template: Set("echo hi".to_string()),
        answer_template: Set(String::new()),
        fixture_definitions: Set("[]".to_string()),
        created_at: Set(Utc::now()),
        session_id: Set(session_id),
        task_id: Set(task_id),
        ordinal: Set(0),
        prompt: Set("probe".to_string()),
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

async fn insert_probe(
    db: &DatabaseConnection,
    session_id: Uuid,
    player_id: Uuid,
    test_id: Uuid,
    attempt: i32,
    outcome: &str,
) {
    probes::ActiveModel {
        id: Set(Uuid::new_v4()),
        test_id: Set(test_id),
        player_id: Set(player_id),
        session_id: Set(session_id),
        attempt: Set(attempt),
        rendered_command: Set("echo hi".to_string()),
        fixture_values: Set("{}".to_string()),
        expected_answer: Set(None),
        resolved_answer: Set(None),
        secret_meta: Set(None),
        outcome: Set(Some(outcome.to_string())),
        dispatched_at: Set(Utc::now()),
        deadline_at: Set(Utc::now()),
        resolved_at: Set(Some(Utc::now())),
        updated_at: Set(Some(Utc::now())),
        output: Set(None),
        exit_code: Set(Some(0)),
        duration_ms: Set(Some(5)),
        point_delta: Set(Some(10)),
        artifact_path: Set(None),
        result_json: Set(None),
    }
    .insert(db)
    .await
    .expect("insert probe");
}

#[allow(clippy::too_many_arguments)]
async fn insert_stats(
    db: &DatabaseConnection,
    session_id: Uuid,
    player_id: Uuid,
    task_id: Uuid,
    ordinal: i32,
    tool_calls: i64,
    assistant_messages: i64,
) {
    task_agent_stats::ActiveModel {
        id: Set(Uuid::new_v4()),
        session_id_fk: Set(session_id),
        player_id_fk: Set(player_id),
        task_id_fk: Set(Some(task_id)),
        task_ordinal: Set(ordinal),
        window_started_at: Set(Some(Utc::now())),
        window_ended_at: Set(Some(Utc::now())),
        input_tokens: Set(100),
        output_tokens: Set(50),
        cache_read_tokens: Set(0),
        cache_write_tokens: Set(0),
        reasoning_tokens: Set(0),
        cost: Set(None),
        user_messages: Set(1),
        assistant_messages: Set(assistant_messages),
        tool_calls: Set(tool_calls),
        agents_json: Set("[]".to_string()),
        created_at: Set(Utc::now()),
        updated_at: Set(Utc::now()),
    }
    .insert(db)
    .await
    .expect("insert stats");
}

#[tokio::test]
async fn dossier_correlates_commits_probes_and_agent_activity() {
    let db = setup_db().await;
    let owner = insert_user(&db).await;
    let project = insert_project(&db, owner).await;
    let session = insert_session(&db, project).await;
    let player = insert_player(&db, session).await;

    let t0 = insert_task_at(&db, project, 0, "Implement it").await;
    let t1 = insert_task_at(&db, project, 1, "Refine it").await;

    // t0: two probe attempts, one pass one fail, with real agent activity.
    let test0 = insert_test_for(&db, session, t0).await;
    insert_probe(&db, session, player, test0, 1, "error").await;
    insert_probe(&db, session, player, test0, 2, "pass").await;
    insert_stats(&db, session, player, t0, 0, 12, 8).await;

    // t1: one pass, stats reported but showing no activity at all.
    let test1 = insert_test_for(&db, session, t1).await;
    insert_probe(&db, session, player, test1, 1, "pass").await;
    insert_stats(&db, session, player, t1, 1, 0, 0).await;

    // A repo where t0 has a real commit and t1's commit changes nothing.
    let repo = tempfile::tempdir().unwrap();
    make_repo(repo.path());
    write_file(repo.path(), "README.md", "scaffold\n");
    commit(repo.path(), "initial scaffold");
    write_file(repo.path(), "solution.py", "print('hello')\n");
    commit(repo.path(), &format!("feat({t0}): Implement it"));
    commit_allow_empty(repo.path(), &format!("feat({t1}): Refine it"));

    let dossier = build_session_dossier(
        &db,
        repo.path(),
        session,
        player,
        &[t0, t1],
        &ToolScope::everything(),
    )
    .await
    .expect("dossier");

    assert_eq!(dossier.tasks.len(), 2, "one entry per requested task");
    assert_eq!(dossier.tasks[0].ordinal, 0, "ordinal order");

    // Root commit is the scaffold, not the solution.
    let root = dossier.root_commit.as_ref().expect("root commit found");
    assert_eq!(
        root.total_files, 1,
        "only the scaffold at the root: {root:?}"
    );
    assert_eq!(root.files[0].path, "README.md");

    let e0 = dossier.task(t0).expect("t0 evidence");
    assert!(e0.commit_sha.is_some(), "t0 commit resolved by task id");
    let d0 = e0.diff.as_ref().expect("t0 diffstat");
    assert_eq!(d0.files_changed, 1, "solution.py added");
    assert_eq!(d0.insertions, 1);
    assert!(d0.files.contains(&"solution.py".to_string()));
    assert_eq!(e0.probes.attempts, 2, "probes joined through tests.task_id");
    assert_eq!(e0.probes.passed, 1);
    assert_eq!(e0.probes.failed, 1);
    assert_eq!(e0.agent.as_ref().expect("t0 stats").tool_calls, 12);
    assert!(
        e0.flags.is_empty(),
        "nothing anomalous about t0: {:?}",
        e0.flags
    );

    // t1: an empty commit that probes nevertheless passed, and no activity.
    let e1 = dossier.task(t1).expect("t1 evidence");
    assert!(e1.commit_sha.is_some(), "the empty commit still resolves");
    assert_eq!(e1.diff.as_ref().expect("t1 diffstat").files_changed, 0);
    assert!(
        e1.flags.contains(&FLAG_EMPTY_TASK_COMMIT.to_string()),
        "{:?}",
        e1.flags
    );
    assert!(
        e1.flags.contains(&FLAG_PASSED_WITHOUT_CHANGES.to_string()),
        "a pass with an empty commit is the signal worth surfacing: {:?}",
        e1.flags
    );
    assert!(
        e1.flags.contains(&FLAG_ZERO_AGENT_ACTIVITY.to_string()),
        "{:?}",
        e1.flags
    );
}

#[tokio::test]
async fn work_committed_under_flag_and_artifact_counts_for_the_task() {
    // The exact shape the CLI produces for an honest delivery (plum, every
    // Money Tracker part of 2026-08-23): the work rides in `flag(<task_id>)`
    // and `artifact(<task_id>): sync` commits, and the `feat` snapshot that
    // follows them is empty. Diffing the feat commit alone flagged all of it
    // as empty_task_commit + passed_without_changes.
    let db = setup_db().await;
    let owner = insert_user(&db).await;
    let project = insert_project(&db, owner).await;
    let session = insert_session(&db, project).await;
    let player = insert_player(&db, session).await;

    let t0 = insert_task_at(&db, project, 0, "Build it").await;
    let t1 = insert_task_at(&db, project, 1, "Grow it").await;
    for (ordinal, task) in [(0, t0), (1, t1)] {
        let test = insert_test_for(&db, session, task).await;
        insert_probe(&db, session, player, test, 1, "pass").await;
        insert_stats(&db, session, player, task, ordinal, 5, 5).await;
    }

    let repo = tempfile::tempdir().unwrap();
    make_repo(repo.path());
    write_file(repo.path(), "README.md", "scaffold\n");
    commit(repo.path(), "ololo snapshot: session start");

    // t0: work in the done-flag commit, then an empty feat snapshot.
    std::fs::create_dir_all(repo.path().join("src")).unwrap();
    std::fs::create_dir_all(repo.path().join(".ololo/artifacts/req1")).unwrap();
    write_file(repo.path(), "src/app.py", "print('v1')\n");
    write_file(repo.path(), ".ololo/done.md", "built it\n");
    commit(repo.path(), &format!("flag({t0}): done.md"));
    commit_allow_empty(repo.path(), &format!("feat({t0}): Build it"));

    // t1: work in an artifact sync, then an empty feat snapshot.
    write_file(repo.path(), "src/app.py", "print('v2')\n");
    write_file(repo.path(), ".ololo/artifacts/req1/shot.png", "png-bytes\n");
    commit(repo.path(), &format!("artifact({t1}): sync"));
    commit_allow_empty(repo.path(), &format!("feat({t1}): Grow it"));

    let dossier = build_session_dossier(
        &db,
        repo.path(),
        session,
        player,
        &[t0, t1],
        &ToolScope::everything(),
    )
    .await
    .expect("dossier");

    let e0 = dossier.task(t0).expect("t0 evidence");
    let d0 = e0.diff.as_ref().expect("t0 diffstat");
    assert!(
        d0.files.contains(&"src/app.py".to_string()),
        "the flag commit's work belongs to t0's window: {d0:?}"
    );
    assert!(
        !e0.flags.contains(&FLAG_EMPTY_TASK_COMMIT.to_string()),
        "an honest delivery must not read as an empty commit: {:?}",
        e0.flags
    );
    assert!(!e0.flags.contains(&FLAG_PASSED_WITHOUT_CHANGES.to_string()));

    // t1's window starts at t0's snapshot, not at the artifact commit.
    let e1 = dossier.task(t1).expect("t1 evidence");
    let d1 = e1.diff.as_ref().expect("t1 diffstat");
    assert!(
        d1.files.contains(&"src/app.py".to_string())
            && d1
                .files
                .contains(&".ololo/artifacts/req1/shot.png".to_string()),
        "the artifact sync's work belongs to t1's window: {d1:?}"
    );
    assert!(
        !e1.flags.contains(&FLAG_EMPTY_TASK_COMMIT.to_string()),
        "{:?}",
        e1.flags
    );
    // ...and t0's changes are not double-counted into t1.
    assert_eq!(d1.files_changed, 2, "only t1's window: {d1:?}");
}

#[tokio::test]
async fn dossier_flags_missing_commit_and_missing_stats() {
    let db = setup_db().await;
    let owner = insert_user(&db).await;
    let project = insert_project(&db, owner).await;
    let session = insert_session(&db, project).await;
    let player = insert_player(&db, session).await;
    let task = insert_task_at(&db, project, 0, "Never attempted").await;

    // A repo with no commit mentioning the task, and no stats row at all.
    let repo = tempfile::tempdir().unwrap();
    make_repo(repo.path());
    write_file(repo.path(), "README.md", "scaffold\n");
    commit(repo.path(), "initial scaffold");

    let dossier = build_session_dossier(
        &db,
        repo.path(),
        session,
        player,
        &[task],
        &ToolScope::everything(),
    )
    .await
    .expect("dossier");

    let e = dossier
        .task(task)
        .expect("evidence even for an untouched task");
    assert!(e.commit_sha.is_none());
    assert!(
        e.diff.is_none(),
        "no commit means no diffstat, distinct from an empty one"
    );
    assert!(
        e.flags.contains(&FLAG_NO_TASK_COMMIT.to_string()),
        "{:?}",
        e.flags
    );
    assert!(
        e.flags.contains(&FLAG_NO_AGENT_STATS.to_string()),
        "{:?}",
        e.flags
    );
    assert!(
        !e.flags.contains(&FLAG_PASSED_WITHOUT_CHANGES.to_string()),
        "no probes passed, so that flag must not fire: {:?}",
        e.flags
    );
    assert_eq!(e.probes.attempts, 0);

    // Every requested task gets an entry — the caller must be able to write a
    // terminal judge_results row for each, or the settle poll waits forever.
    assert_eq!(dossier.tasks.len(), 1);
}

#[tokio::test]
async fn task_window_base_skips_the_tasks_own_flag_and_artifact_commits() {
    // Task-scoped twin of the fix above: the window's base used to be "the
    // commit immediately before the feat snapshot" — which is the CLI's own
    // flag/artifact commit that already carries the work, so the window
    // diffed empty for exactly the players who delivered properly.
    let task_id = Uuid::new_v4();
    let repo = tempfile::tempdir().unwrap();
    make_repo(repo.path());
    write_file(repo.path(), "README.md", "scaffold\n");
    commit(repo.path(), "ololo snapshot: session start");
    write_file(repo.path(), "app.py", "print('work')\n");
    write_file(repo.path(), "done.md", "done\n");
    commit(repo.path(), &format!("flag({task_id}): done.md"));
    write_file(repo.path(), "shot.png", "png\n");
    commit(repo.path(), &format!("artifact({task_id}): sync"));
    let feat = commit_allow_empty(repo.path(), &format!("feat({task_id}): Build it"));

    let pack = build_task_dossier(
        repo.path(),
        &task_id.to_string(),
        Some(&feat),
        None,
        &ToolScope::everything(),
    )
    .await;

    // The diff body itself, not a mention in the commit list: on the old
    // base (the artifact commit) the window diffed empty and neither of
    // these lines could appear.
    assert!(
        pack.contains("print('work')"),
        "the flag commit's work is inside the window diff:\n{pack}"
    );
    assert!(
        !pack.contains("(empty:"),
        "the window must not diff empty for an honest delivery:\n{pack}"
    );
}
