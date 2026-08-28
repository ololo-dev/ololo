use crate::state::GameServerState;
use crate::ws::player_agent::interval::apply_no_response_backoff;
use arena_core::entities::{probes, task_results};
use arena_core::protocol::{ArenaFrame, PlayerAgentFrame, ProbeOutcome, ZmqEvent};
use axum::extract::ws::{Message, WebSocket};
use chrono::Utc;
use sea_orm::prelude::Expr;
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, QuerySelect, Set};
use uuid::Uuid;

pub struct NoResponseInput<'a> {
    pub probe_id: Uuid,
    pub task_id: Uuid,
    pub no_response_points: i32,
    pub expected_answer_display: &'a Option<String>,
    pub secret_expected: bool,
    pub interval_increment_secs: i32,
    pub min_interval_secs: i32,
    pub max_interval_secs: i32,
}

pub async fn insert_task_result(
    state: &GameServerState,
    session_id: Uuid,
    task_id: Option<Uuid>,
    point_delta: i32,
    answer: &str,
    player_id: Uuid,
) {
    let am = task_results::ActiveModel {
        id: Set(Uuid::new_v4()),
        session_id_fk: Set(session_id),
        player_id_fk: Set(player_id),
        task_id: Set(task_id),
        answer: Set(answer.to_string()),
        created_at: Set(Utc::now()),
        point_delta: Set(point_delta),
        is_bonus: Set(false),
    };
    if let Err(e) = am.insert(&state.db).await {
        tracing::warn!(session_id = %session_id, error = %e, "player_agent: task_result insert failed");
    }
}

async fn score_no_response(
    state: &GameServerState,
    probe_id: Uuid,
    session_id: Uuid,
    task_id: Option<Uuid>,
    no_response_points: i32,
    player_id: Uuid,
) {
    let delta = no_response_points;
    crate::session_log_store::record(
        crate::session_log_store::base_dir(),
        session_id,
        Some(player_id),
        "probe_no_response",
        serde_json::json!({
            "player_id": player_id,
            "task_id": task_id,
            "probe_id": probe_id,
            "point_delta": delta,
        }),
    )
    .await;
    let _ = probes::Entity::update_many()
        .col_expr(probes::Column::Outcome, Expr::value("no_response"))
        .col_expr(probes::Column::PointDelta, Expr::value(delta))
        .col_expr(probes::Column::UpdatedAt, Expr::value(Utc::now()))
        .filter(probes::Column::Id.eq(probe_id))
        .filter(probes::Column::Outcome.is_null())
        .exec(&state.db)
        .await;
    insert_task_result(state, session_id, task_id, delta, "", player_id).await;
}

/// Send a `ProbeGraded { outcome: NoResponse }` frame to the player so
/// the agent's probes panel reflects the server's recorded outcome
/// (rather than guessing locally). `expected` is the server-stored
/// `expected_answer` for display in the panel detail row, if any.
pub async fn emit_no_response_grade(
    socket: &mut WebSocket,
    probe_id: Uuid,
    point_delta: i32,
    expected: &Option<String>,
    secret_expected: bool,
) {
    let frame = PlayerAgentFrame::ProbeGraded {
        probe_id,
        outcome: ProbeOutcome::NoResponse,
        point_delta,
        // Suppress the expected value when the task marked it secret.
        expected: if secret_expected {
            None
        } else {
            expected.clone()
        },
        actual: None,
    };
    let json = serde_json::to_string(&frame).unwrap_or_default();
    let _ = socket.send(Message::Text(json)).await;
}

/// Record a no-response outcome for a probe: apply backoff, score it, publish
/// the score delta, refresh the leaderboard, and emit the graded frame.
pub async fn record_no_response(
    state: &GameServerState,
    socket: &mut WebSocket,
    session_id: Uuid,
    player_id: Uuid,
    join_code: &str,
    input: &NoResponseInput<'_>,
    current_interval_secs: &mut i32,
) {
    *current_interval_secs = apply_no_response_backoff(
        *current_interval_secs,
        input.interval_increment_secs,
        input.min_interval_secs,
        input.max_interval_secs,
    );
    score_no_response(
        state,
        input.probe_id,
        session_id,
        Some(input.task_id),
        input.no_response_points,
        player_id,
    )
    .await;
    publish_score_change(
        state,
        session_id,
        player_id,
        input.no_response_points as i64,
        join_code,
    )
    .await;
    broadcast_leaderboard(state, session_id, join_code).await;
    emit_no_response_grade(
        socket,
        input.probe_id,
        input.no_response_points,
        input.expected_answer_display,
        input.secret_expected,
    )
    .await;
}

pub async fn publish_score_change(
    state: &GameServerState,
    session_id: Uuid,
    player_id: Uuid,
    delta: i64,
    join_code: &str,
) {
    let total: i64 = task_results::Entity::find()
        .select_only()
        .column_as(task_results::Column::PointDelta.sum(), "total")
        .filter(task_results::Column::SessionIdFk.eq(session_id))
        .filter(task_results::Column::PlayerIdFk.eq(player_id))
        .into_tuple::<Option<i64>>()
        .one(&state.db)
        .await
        .ok()
        .flatten()
        .flatten()
        .unwrap_or(0);

    let version = state
        .session_registry
        .get(join_code)
        .and_then(|e| e.cache.read().ok().map(|c| c.version))
        .unwrap_or(0);

    let event = ZmqEvent::ScoreChange {
        join_code: join_code.to_string(),
        player_id,
        delta,
        total,
        version,
    };
    state.event_publisher.publish(&event).await;
}

pub(crate) async fn broadcast_leaderboard(
    state: &GameServerState,
    session_id: Uuid,
    join_code: &str,
) {
    let entries = match arena_core::scoring::compute_leaderboard(&state.db, session_id).await {
        Ok(e) => e,
        Err(e) => {
            tracing::warn!(session_id = %session_id, error = %e, "broadcast_leaderboard: compute failed");
            return;
        }
    };

    if let Some(entry) = state.session_registry.get(join_code) {
        let version = if let Ok(mut cache) = entry.cache.write() {
            cache.version = cache.version.saturating_add(1);
            cache.leaderboard = entries.clone();
            cache.version
        } else {
            0
        };
        let _ = entry
            .tx
            .send(ArenaFrame::LeaderboardUpdate { entries, version });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_fixtures::{
        insert_probe, mem_db, session_with_player, task_with_test, test_state,
    };
    use sea_orm::QueryFilter;

    #[tokio::test]
    async fn no_response_grades_the_probe_and_writes_the_result_row() {
        let db = mem_db().await;
        let fx = session_with_player(&db).await;
        let (task_id, test_id) = task_with_test(&db, &fx).await;
        let probe_id = insert_probe(&db, &fx, test_id, None).await;
        let state = test_state(db.clone());

        score_no_response(
            &state,
            probe_id,
            fx.session_id,
            Some(task_id),
            -10,
            fx.player_id,
        )
        .await;

        let probe = probes::Entity::find_by_id(probe_id)
            .one(&db)
            .await
            .expect("query")
            .expect("probe");
        assert_eq!(probe.outcome.as_deref(), Some("no_response"));
        assert_eq!(probe.point_delta, Some(-10));
        assert!(probe.updated_at.is_some(), "grade must bump updated_at");

        let results = task_results::Entity::find()
            .filter(task_results::Column::PlayerIdFk.eq(fx.player_id))
            .all(&db)
            .await
            .expect("query results");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].point_delta, -10);
        assert_eq!(results[0].task_id, Some(task_id));
        assert!(!results[0].is_bonus);
    }

    #[tokio::test]
    async fn no_response_never_overwrites_an_already_graded_probe() {
        // The guard: a pass that landed just before the deadline sweep must
        // survive — no_response only claims probes with no outcome yet.
        let db = mem_db().await;
        let fx = session_with_player(&db).await;
        let (task_id, test_id) = task_with_test(&db, &fx).await;
        let probe_id = insert_probe(&db, &fx, test_id, Some("pass")).await;
        let state = test_state(db.clone());

        score_no_response(
            &state,
            probe_id,
            fx.session_id,
            Some(task_id),
            -10,
            fx.player_id,
        )
        .await;

        let probe = probes::Entity::find_by_id(probe_id)
            .one(&db)
            .await
            .expect("query")
            .expect("probe");
        assert_eq!(
            probe.outcome.as_deref(),
            Some("pass"),
            "a graded probe keeps its verdict"
        );
        assert_eq!(probe.point_delta, None, "the pass's points are untouched");
    }

    #[tokio::test]
    async fn insert_task_result_persists_the_row() {
        let db = mem_db().await;
        let fx = session_with_player(&db).await;
        let (task_id, _test_id) = task_with_test(&db, &fx).await;
        let state = test_state(db.clone());

        insert_task_result(
            &state,
            fx.session_id,
            Some(task_id),
            7,
            "answer",
            fx.player_id,
        )
        .await;

        let rows = task_results::Entity::find()
            .filter(task_results::Column::SessionIdFk.eq(fx.session_id))
            .all(&db)
            .await
            .expect("query");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].answer, "answer");
        assert_eq!(rows[0].point_delta, 7);
    }
}
