//! `record_judge_run_status` must not leave a previous run's score standing.
//!
//! `compute_player_scores` sums `point_delta` across every `judge_results` row
//! regardless of `status`, so a stale delta sitting under a `failed` row keeps
//! moving the player's score with nothing in the UI to explain it. It is also
//! the repair path: re-running a judge that can no longer score has to undo the
//! score it replaces.

use crate::common;
use crate::common::*;

use arena_core::entities::judge_results;
use arena_core::judging::record_judge_run_status;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};

#[tokio::test]
async fn failed_status_clears_a_previous_scores_point_delta() {
    let db = setup_db().await;
    let owner = insert_user(&db).await;
    let project = insert_project(&db, owner).await;
    let session = insert_session(&db, project).await;
    let player = insert_player(&db, session).await;
    let (_task_id, _judge_id, task_judge_id) = insert_chain(&db, project, session, player).await;

    // A prior run scored a penalty (the golf-verify shape: negative delta).
    record_judge_run_status(
        &db,
        session,
        player,
        task_judge_id,
        "execution:bubblewrap",
        "execution",
        "running",
        None,
        None,
    )
    .await
    .expect("record running");
    let row = judge_results::Entity::find()
        .filter(judge_results::Column::TaskJudgeId.eq(task_judge_id))
        .one(&db)
        .await
        .unwrap()
        .expect("row exists");
    let mut scored: judge_results::ActiveModel = row.into();
    use sea_orm::{ActiveModelTrait, Set};
    scored.point_delta = Set(-67);
    scored.rating = Set(serde_json::json!(-67.0));
    scored.feedback = Set("Server-side re-run: 1/3 probes passed.".to_string());
    scored.status = Set("scored".to_string());
    scored.update(&db).await.expect("persist prior score");

    // The re-run cannot score (broken sandbox) and is recorded as failed.
    record_judge_run_status(
        &db,
        session,
        player,
        task_judge_id,
        "execution:bubblewrap",
        "execution",
        "failed",
        Some("execution failed: sandbox cannot execute commands"),
        None,
    )
    .await
    .expect("record failed");

    let row = judge_results::Entity::find()
        .filter(judge_results::Column::TaskJudgeId.eq(task_judge_id))
        .filter(judge_results::Column::PlayerIdFk.eq(player))
        .one(&db)
        .await
        .unwrap()
        .expect("row still exists");
    assert_eq!(row.status, "failed");
    assert_eq!(
        row.point_delta, 0,
        "a failed run must not keep charging the player the old penalty"
    );
    assert_eq!(row.rating, serde_json::Value::Null, "stale rating cleared");
    assert!(row.feedback.is_empty(), "stale feedback cleared");
}
