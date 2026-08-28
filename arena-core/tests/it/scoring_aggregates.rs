//! Aggregate / leaderboard ordering tests for `arena_core::scoring`.

use crate::common;
use crate::common::*;

use arena_core::entities::judge_results;
use arena_core::entities::task_results;
use arena_core::protocol::*;
use arena_core::scoring::*;

use chrono::Utc;
use sea_orm::ActiveModelTrait;
use sea_orm::Set;
use uuid::Uuid;

async fn insert_task_result(
    db: &sea_orm::DatabaseConnection,
    session_id: Uuid,
    player_id: Uuid,
    point_delta: i32,
    is_bonus: bool,
) {
    task_results::ActiveModel {
        id: Set(Uuid::new_v4()),
        session_id_fk: Set(session_id),
        player_id_fk: Set(player_id),
        task_id: Set(None),
        answer: Set(String::new()),
        created_at: Set(Utc::now()),
        point_delta: Set(point_delta),
        is_bonus: Set(is_bonus),
    }
    .insert(db)
    .await
    .expect("insert task_result");
}

static ORDINAL: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);

async fn insert_judge_chain(
    db: &sea_orm::DatabaseConnection,
    project_id: Uuid,
    session_id: Uuid,
    player_id: Uuid,
    point_delta: i32,
) {
    let ordinal = ORDINAL.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let task_id = Uuid::new_v4();
    arena_core::entities::tasks::ActiveModel {
        id: Set(task_id),
        project_id_fk: Set(project_id),
        ordinal: Set(ordinal),
        title: Set("t".to_string()),
        content: Set(String::new()),
        test_template: Set(serde_json::json!({"kind":"shell","command_template":"echo ok"})),
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

    let judge_id = Uuid::new_v4();
    arena_core::entities::judges::ActiveModel {
        id: Set(judge_id),
        slug: Set(format!("j{}", &judge_id.to_string()[..8])),
        name: Set("j".to_string()),
        description: Set(String::new()),
        prompt: Set(String::new()),
        rating_scale: Set(serde_json::json!([{"label":"ok","value":1}])),
        kind: Set("llm".to_string()),
        scope: Set("task".to_string()),
        evidence_mode: Set("tools".to_string()),
        evidence_needs: Set(None),
        created_at: Set(Utc::now()),
        updated_at: Set(Utc::now()),
        llm_provider_id_fk: Set(None),
        llm_model: Set(None),
        llm_pool_id_fk: Set(None),
        llm_source_order: Set(arena_core::llm::resolve::SOURCE_ORDER_POOL_FIRST.to_string()),
        criteria: Set(None),
        probes_config: Set(None),
        max_interactive: Set(None),
        avatar_url: Set(None),
        ignore_paths: Set(None),
    }
    .insert(db)
    .await
    .expect("insert judge");

    let task_judge_id = Uuid::new_v4();
    arena_core::entities::task_judges::ActiveModel {
        id: Set(task_judge_id),
        task_id: Set(task_id),
        judge_id: Set(judge_id),
        ordinal: Set(0),
        rating_scale_override: Set(None),
        weight: Set(None),
        created_at: Set(Utc::now()),
        updated_at: Set(Utc::now()),
    }
    .insert(db)
    .await
    .expect("insert task_judge");

    judge_results::ActiveModel {
        id: Set(Uuid::new_v4()),
        session_id_fk: Set(session_id),
        player_id_fk: Set(player_id),
        task_judge_id: Set(task_judge_id),
        rating: Set(serde_json::json!({"label":"ok","value":1})),
        point_delta: Set(point_delta),
        feedback: Set(String::new()),
        model: Set("m".to_string()),
        provider: Set(String::new()),
        raw_output: Set(String::new()),
        duration_ms: Set(None),
        run_log: Set(None),
        tokens_input: Set(None),
        tokens_output: Set(None),
        tokens_cache_read: Set(None),
        tokens_cache_write: Set(None),
        status: Set("scored".to_string()),
        error: Set(None),
        verdict_kind: Set(None),
        created_at: Set(Utc::now()),
        updated_at: Set(Utc::now()),
    }
    .insert(db)
    .await
    .expect("insert judge_result");
}

#[tokio::test]
async fn aggregate_scores_task_results_only() {
    let db = setup_db().await;
    let owner = insert_user(&db).await;
    let project = insert_project(&db, owner).await;
    let session = insert_session(&db, project).await;
    let player = insert_player(&db, session).await;

    insert_task_result(&db, session, player, 10, false).await;
    insert_task_result(&db, session, player, -5, false).await;
    insert_task_result(&db, session, player, 10, true).await;

    let map = aggregate_scores(&db, session).await.expect("aggregate");
    let data = map.get(&player).expect("player present");
    assert_eq!(data.total_points, 15);
    assert_eq!(data.tests_passed, 1);
}

#[tokio::test]
async fn aggregate_scores_judge_results_only() {
    let db = setup_db().await;
    let owner = insert_user(&db).await;
    let project = insert_project(&db, owner).await;
    let session = insert_session(&db, project).await;
    let player = insert_player(&db, session).await;

    insert_judge_chain(&db, project, session, player, 7).await;
    insert_judge_chain(&db, project, session, player, 3).await;

    let map = aggregate_scores(&db, session).await.expect("aggregate");
    let data = map.get(&player).expect("player present");
    assert_eq!(data.total_points, 10);
    assert_eq!(data.tests_passed, 0);
}

#[tokio::test]
async fn aggregate_scores_both_sources_sum() {
    let db = setup_db().await;
    let owner = insert_user(&db).await;
    let project = insert_project(&db, owner).await;
    let session = insert_session(&db, project).await;
    let player = insert_player(&db, session).await;

    insert_task_result(&db, session, player, 10, false).await;
    insert_task_result(&db, session, player, 20, false).await;
    insert_judge_chain(&db, project, session, player, 5).await;
    insert_judge_chain(&db, project, session, player, -3).await;

    let map = aggregate_scores(&db, session).await.expect("aggregate");
    let data = map.get(&player).expect("player present");
    assert_eq!(data.total_points, 32);
    assert_eq!(data.tests_passed, 2);
}

#[test]
fn sort_order_total_points_desc() {
    let pid1 = Uuid::new_v4();
    let pid2 = Uuid::new_v4();
    let entries = vec![
        LeaderboardEntry {
            player_id: PlayerId(pid1),
            display_name: "Alice".to_string(),
            agent_display_name: None,
            total_points: 50,
            tests_passed: 5,
            total_wall_ms: 0,
        },
        LeaderboardEntry {
            player_id: PlayerId(pid2),
            display_name: "Bob".to_string(),
            agent_display_name: None,
            total_points: 100,
            tests_passed: 10,
            total_wall_ms: 0,
        },
    ];
    let mut sorted = entries;
    sorted.sort_by(|a, b| {
        b.total_points
            .cmp(&a.total_points)
            .then_with(|| b.tests_passed.cmp(&a.tests_passed))
            .then_with(|| a.display_name.cmp(&b.display_name))
    });
    assert_eq!(sorted[0].display_name, "Bob");
    assert_eq!(sorted[1].display_name, "Alice");
}

#[test]
fn sort_order_tiebreak_tests_passed() {
    let pid1 = Uuid::new_v4();
    let pid2 = Uuid::new_v4();
    let entries = vec![
        LeaderboardEntry {
            player_id: PlayerId(pid1),
            display_name: "Alice".to_string(),
            agent_display_name: None,
            total_points: 50,
            tests_passed: 3,
            total_wall_ms: 0,
        },
        LeaderboardEntry {
            player_id: PlayerId(pid2),
            display_name: "Bob".to_string(),
            agent_display_name: None,
            total_points: 50,
            tests_passed: 5,
            total_wall_ms: 0,
        },
    ];
    let mut sorted = entries;
    sorted.sort_by(|a, b| {
        b.total_points
            .cmp(&a.total_points)
            .then_with(|| b.tests_passed.cmp(&a.tests_passed))
            .then_with(|| a.display_name.cmp(&b.display_name))
    });
    assert_eq!(sorted[0].display_name, "Bob");
    assert_eq!(sorted[1].display_name, "Alice");
}

#[test]
fn read_score_rank_defensively_sorts() {
    let pid1 = Uuid::new_v4();
    let pid2 = Uuid::new_v4();
    let pid3 = Uuid::new_v4();
    let leaderboard = vec![
        LeaderboardEntry {
            player_id: PlayerId(pid1),
            display_name: "Alice".to_string(),
            agent_display_name: None,
            total_points: 30,
            tests_passed: 3,
            total_wall_ms: 0,
        },
        LeaderboardEntry {
            player_id: PlayerId(pid2),
            display_name: "Bob".to_string(),
            agent_display_name: None,
            total_points: 100,
            tests_passed: 10,
            total_wall_ms: 0,
        },
        LeaderboardEntry {
            player_id: PlayerId(pid3),
            display_name: "Carol".to_string(),
            agent_display_name: None,
            total_points: 100,
            tests_passed: 5,
            total_wall_ms: 0,
        },
    ];
    let (score, rank) = read_score_rank(&leaderboard, pid2);
    assert_eq!(score, 100);
    assert_eq!(rank, 1);
    let (score, rank) = read_score_rank(&leaderboard, pid3);
    assert_eq!(score, 100);
    assert_eq!(rank, 2);
    let (score, rank) = read_score_rank(&leaderboard, pid1);
    assert_eq!(score, 30);
    assert_eq!(rank, 3);
}

#[test]
fn read_score_rank_missing_player() {
    let pid1 = Uuid::new_v4();
    let missing = Uuid::new_v4();
    let leaderboard = vec![LeaderboardEntry {
        player_id: PlayerId(pid1),
        display_name: "Alice".to_string(),
        agent_display_name: None,
        total_points: 50,
        tests_passed: 5,
        total_wall_ms: 0,
    }];
    let (score, rank) = read_score_rank(&leaderboard, missing);
    assert_eq!(score, 0);
    assert_eq!(rank, 0);
}

#[test]
fn sort_order_tiebreak_display_name() {
    let pid1 = Uuid::new_v4();
    let pid2 = Uuid::new_v4();
    let entries = vec![
        LeaderboardEntry {
            player_id: PlayerId(pid1),
            display_name: "Bob".to_string(),
            agent_display_name: None,
            total_points: 50,
            tests_passed: 5,
            total_wall_ms: 0,
        },
        LeaderboardEntry {
            player_id: PlayerId(pid2),
            display_name: "Alice".to_string(),
            agent_display_name: None,
            total_points: 50,
            tests_passed: 5,
            total_wall_ms: 0,
        },
    ];
    let mut sorted = entries;
    sorted.sort_by(|a, b| {
        b.total_points
            .cmp(&a.total_points)
            .then_with(|| b.tests_passed.cmp(&a.tests_passed))
            .then_with(|| a.display_name.cmp(&b.display_name))
    });
    assert_eq!(sorted[0].display_name, "Alice");
    assert_eq!(sorted[1].display_name, "Bob");
}
