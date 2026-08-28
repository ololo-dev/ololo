//! Board / parse-agent / bonus tests for `arena_core::scoring`.

use crate::common;
use crate::common::*;

use arena_core::entities::task_results;
use arena_core::scoring::*;

use chrono::Utc;
use sea_orm::ActiveModelTrait;
use sea_orm::Set;
use uuid::Uuid;

async fn insert_task(db: &sea_orm::DatabaseConnection, project_id: Uuid) -> Uuid {
    let id = Uuid::new_v4();
    arena_core::entities::tasks::ActiveModel {
        id: Set(id),
        project_id_fk: Set(project_id),
        ordinal: Set(0),
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
    id
}

#[test]
fn parse_agent_display_name_extracts_first_ai_agent() {
    let json = r#"{"ai_agents":[{"name":"opencode","version":"0.1"},{"name":"claude"}]}"#;
    assert_eq!(
        parse_agent_display_name(Some(json)),
        Some("opencode".to_string())
    );
}

#[test]
fn parse_agent_display_name_returns_none_for_missing_field() {
    let json = r#"{"build_tools":[{"name":"cargo"}]}"#;
    assert_eq!(parse_agent_display_name(Some(json)), None);
}

#[test]
fn parse_agent_display_name_returns_none_for_empty_array() {
    let json = r#"{"ai_agents":[]}"#;
    assert_eq!(parse_agent_display_name(Some(json)), None);
}

#[test]
fn parse_agent_display_name_returns_none_for_none_input() {
    assert_eq!(parse_agent_display_name(None), None);
}

#[test]
fn parse_agent_display_name_returns_none_for_invalid_json() {
    assert_eq!(parse_agent_display_name(Some("not json")), None);
}

#[test]
fn parse_agent_display_name_tolerates_unknown_fields() {
    let json = r#"{"ai_agents":[{"name":"pi","version":"1.0","extra":true}],"future_field":42}"#;
    assert_eq!(parse_agent_display_name(Some(json)), Some("pi".to_string()));
}

#[tokio::test]
async fn compute_leaderboard_reflects_scores() {
    let db = setup_db().await;
    let owner = insert_user(&db).await;
    let project = insert_project(&db, owner).await;
    let session = insert_session(&db, project).await;
    let player = insert_player(&db, session).await;

    insert_task_result(&db, session, player, 10, false).await;
    insert_task_result(&db, session, player, 5, false).await;

    let board = compute_leaderboard(&db, session)
        .await
        .expect("leaderboard");
    let entry = board
        .iter()
        .find(|e| e.player_id.as_uuid() == player)
        .expect("player in board");
    assert_eq!(entry.total_points, 15);
    assert_eq!(entry.tests_passed, 2);
}

#[tokio::test]
async fn award_completion_bonus_is_idempotent() {
    let db = setup_db().await;
    let owner = insert_user(&db).await;
    let project = insert_project(&db, owner).await;
    let session = insert_session(&db, project).await;
    let player = insert_player(&db, session).await;
    let task_id = insert_task(&db, project).await;

    let first = award_completion_bonus(&db, session, player, task_id, 10)
        .await
        .expect("award");
    assert!(first, "first award should succeed");

    let second = award_completion_bonus(&db, session, player, task_id, 10)
        .await
        .expect("award");
    assert!(!second, "duplicate award should be rejected");
}

#[tokio::test]
async fn check_task_completion_false_when_no_adapted_tests() {
    let db = setup_db().await;
    let owner = insert_user(&db).await;
    let project = insert_project(&db, owner).await;
    let session = insert_session(&db, project).await;
    let player = insert_player(&db, session).await;
    let task_id = Uuid::new_v4();

    let done = check_task_completion(&db, session, player, task_id)
        .await
        .expect("check");
    assert!(!done, "no adapted tests means task not complete");
}

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
