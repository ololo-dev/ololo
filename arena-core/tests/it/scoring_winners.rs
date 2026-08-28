//! `session_winners` / `player_counts` — the bulk aggregates behind the
//! project page's session rows. These must agree with `compute_leaderboard`
//! about who won, or the project page and the session report contradict each
//! other for the same session.

use crate::common;
use crate::common::*;

use arena_core::entities::{judge_results, players, sessions, task_results};
use arena_core::scoring::*;
use arena_core::session_status::SessionStatus;

use chrono::Utc;
use sea_orm::{ActiveModelTrait, EntityTrait, Set};
use uuid::Uuid;

/// The shared `insert_session` helper hardcodes one join code, so tests that
/// need two sessions in the same DB must supply their own.
async fn a_session(db: &sea_orm::DatabaseConnection, project_id: Uuid, join_code: &str) -> Uuid {
    let id = Uuid::new_v4();
    sessions::ActiveModel {
        id: Set(id),
        name: Set("s".to_string()),
        created_at: Set(Utc::now()),
        owner_id_fk: Set(None),
        status: Set(SessionStatus::Finished),
        join_code: Set(join_code.to_string()),
        started_at: Set(Some(Utc::now())),
        finished_at: Set(Some(Utc::now())),
        paused_at: Set(None),
        paused_duration_secs: Set(None),
        project_id_fk: Set(project_id),
        game_server_id: Set(None),
        cancel_reason: Set(None),
        cancelled_by: Set(None),
    }
    .insert(db)
    .await
    .expect("insert session");
    id
}

async fn named_player(db: &sea_orm::DatabaseConnection, session_id: Uuid, name: &str) -> Uuid {
    let id = Uuid::new_v4();
    players::ActiveModel {
        id: Set(id),
        session_id_fk: Set(session_id),
        user_id_fk: Set(None),
        display_name: Set(name.to_string()),
        fingerprint: Set(None),
        metadata_json: Set(None),
        joined_at: Set(Utc::now()),
        reconnected_at: Set(None),
        revoked_at: Set(None),
        agent_connected: Set(false),
        agent_last_seen_at: Set(None),
    }
    .insert(db)
    .await
    .expect("insert player");
    id
}

async fn score(db: &sea_orm::DatabaseConnection, session_id: Uuid, player_id: Uuid, points: i32) {
    task_results::ActiveModel {
        id: Set(Uuid::new_v4()),
        session_id_fk: Set(session_id),
        player_id_fk: Set(player_id),
        task_id: Set(None),
        point_delta: Set(points),
        answer: Set(String::new()),
        created_at: Set(Utc::now()),
        is_bonus: Set(false),
    }
    .insert(db)
    .await
    .expect("insert task_result");
}

#[tokio::test]
async fn picks_the_highest_scorer_per_session() {
    let db = setup_db().await;
    let owner = insert_user(&db).await;
    let project = insert_project(&db, owner).await;
    let a = a_session(&db, project, "WINA01").await;
    let b = a_session(&db, project, "WINB02").await;

    let winner = named_player(&db, a, "Ada").await;
    let loser = named_player(&db, a, "Bob").await;
    score(&db, a, winner, 120).await;
    score(&db, a, loser, 40).await;

    let other = named_player(&db, b, "Cleo").await;
    score(&db, b, other, 10).await;

    let winners = session_winners(&db, &[a, b]).await.expect("winners");
    assert_eq!(winners[&a].display_name, "Ada");
    assert_eq!(winners[&a].total_points, 120);
    assert_eq!(winners[&b].display_name, "Cleo");
}

#[tokio::test]
async fn judge_deltas_can_flip_the_winner() {
    // A player ahead on probes alone can still lose once judges dock them —
    // the row must reflect the final score, not the probe subtotal.
    let db = setup_db().await;
    let owner = insert_user(&db).await;
    let project = insert_project(&db, owner).await;
    let session = insert_session(&db, project).await;
    let player = named_player(&db, session, "Ada").await;
    let rival = named_player(&db, session, "Bob").await;
    let (_task_id, _judge_id, task_judge_id) = insert_chain(&db, project, session, player).await;

    score(&db, session, player, 200).await;
    score(&db, session, rival, 150).await;

    judge_results::ActiveModel {
        id: Set(Uuid::new_v4()),
        session_id_fk: Set(session),
        player_id_fk: Set(player),
        task_judge_id: Set(task_judge_id),
        rating: Set(serde_json::json!(-100.0)),
        point_delta: Set(-100),
        feedback: Set("pre-implemented solution".to_string()),
        model: Set("test".to_string()),
        provider: Set("test".to_string()),
        raw_output: Set(String::new()),
        duration_ms: Set(None),
        run_log: Set(None),
        tokens_input: Set(None),
        tokens_output: Set(None),
        tokens_cache_read: Set(None),
        tokens_cache_write: Set(None),
        status: Set("scored".to_string()),
        error: Set(None),
        created_at: Set(Utc::now()),
        updated_at: Set(Utc::now()),
        verdict_kind: Set(None),
    }
    .insert(&db)
    .await
    .expect("insert judge_result");

    let winners = session_winners(&db, &[session]).await.expect("winners");
    assert_eq!(winners[&session].display_name, "Bob", "judge delta applied");
    assert_eq!(winners[&session].total_points, 150);
}

#[tokio::test]
async fn agrees_with_compute_leaderboard() {
    let db = setup_db().await;
    let owner = insert_user(&db).await;
    let project = insert_project(&db, owner).await;
    let session = insert_session(&db, project).await;
    for (name, points) in [("Ada", 70), ("Bob", 130), ("Cleo", 90)] {
        let p = named_player(&db, session, name).await;
        score(&db, session, p, points).await;
    }

    let board = compute_leaderboard(&db, session).await.expect("board");
    let winners = session_winners(&db, &[session]).await.expect("winners");
    assert_eq!(winners[&session].display_name, board[0].display_name);
    assert_eq!(winners[&session].total_points, board[0].total_points);
}

#[tokio::test]
async fn revoked_players_cannot_win_and_are_not_counted() {
    let db = setup_db().await;
    let owner = insert_user(&db).await;
    let project = insert_project(&db, owner).await;
    let session = insert_session(&db, project).await;

    let kicked = named_player(&db, session, "Mallory").await;
    let honest = named_player(&db, session, "Ada").await;
    score(&db, session, kicked, 999).await;
    score(&db, session, honest, 10).await;

    let mut am: players::ActiveModel = players::Entity::find_by_id(kicked)
        .one(&db)
        .await
        .expect("find")
        .expect("player")
        .into();
    am.revoked_at = Set(Some(Utc::now()));
    am.update(&db).await.expect("revoke");

    let winners = session_winners(&db, &[session]).await.expect("winners");
    assert_eq!(winners[&session].display_name, "Ada");

    let counts = player_counts(&db, &[session]).await.expect("counts");
    assert_eq!(counts[&session], 1, "revoked players are not counted");
}

#[tokio::test]
async fn sessions_without_players_are_absent_rather_than_zeroed() {
    let db = setup_db().await;
    let owner = insert_user(&db).await;
    let project = insert_project(&db, owner).await;
    let empty = insert_session(&db, project).await;

    assert!(
        session_winners(&db, &[empty])
            .await
            .expect("winners")
            .is_empty()
    );
    assert!(
        player_counts(&db, &[empty])
            .await
            .expect("counts")
            .is_empty()
    );
    // An empty id list must not fan out into an unfiltered query.
    assert!(session_winners(&db, &[]).await.expect("winners").is_empty());
    assert!(player_counts(&db, &[]).await.expect("counts").is_empty());
}
