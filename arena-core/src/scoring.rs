use crate::entities::{judge_results, players, probes, task_results, tests as entity_tests};
use crate::protocol::{LeaderboardEntry, PlayerId, ScoreHistorySample};
use sea_orm::sea_query::Expr;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, EntityTrait, FromQueryResult, QueryFilter, QueryOrder,
    QuerySelect, Set,
};
use std::collections::{BTreeMap, HashMap};
use uuid::Uuid;

pub struct PlayerScoreData {
    pub total_points: i64,
    pub tests_passed: u32,
}

/// Parse the first `ai_agents[].name` from a player's `metadata_json` blob.
/// Returns `None` when metadata is missing, malformed, or has no ai_agents.
/// Kept tolerant: forward-compatible with future ololo versions (unknown
/// fields ignored; missing `ai_agents` → None).
pub fn parse_agent_display_name(metadata_json: Option<&str>) -> Option<String> {
    let raw = metadata_json?;
    let value: serde_json::Value = serde_json::from_str(raw).ok()?;
    value
        .get("ai_agents")
        .and_then(|v| v.as_array())
        .and_then(|arr| arr.first())
        .and_then(|first| first.get("name"))
        .and_then(|n| n.as_str())
        .map(str::to_string)
}

pub async fn aggregate_scores(
    db: &sea_orm::DatabaseConnection,
    session_id: Uuid,
) -> Result<HashMap<Uuid, PlayerScoreData>, sea_orm::DbErr> {
    // Aggregate in SQL (GROUP BY) rather than loading every task_result and
    // judge_result row and folding in Rust — this runs every ~5s per session
    // plus on every score event. SUM is nullable per group; COUNT is not.
    #[derive(FromQueryResult)]
    struct SumRow {
        player_id_fk: Uuid,
        total: Option<i64>,
    }
    #[derive(FromQueryResult)]
    struct CountRow {
        player_id_fk: Uuid,
        cnt: i64,
    }

    let mut map: HashMap<Uuid, PlayerScoreData> = HashMap::new();

    // Total points from task results, per player.
    let task_points = task_results::Entity::find()
        .filter(task_results::Column::SessionIdFk.eq(session_id))
        .select_only()
        .column(task_results::Column::PlayerIdFk)
        .column_as(Expr::col(task_results::Column::PointDelta).sum(), "total")
        .group_by(task_results::Column::PlayerIdFk)
        .into_model::<SumRow>()
        .all(db)
        .await?;
    for r in task_points {
        map.entry(r.player_id_fk)
            .or_insert(PlayerScoreData {
                total_points: 0,
                tests_passed: 0,
            })
            .total_points += r.total.unwrap_or(0);
    }

    // Tests passed: non-bonus, positive-delta task results, per player.
    let passed = task_results::Entity::find()
        .filter(task_results::Column::SessionIdFk.eq(session_id))
        .filter(task_results::Column::IsBonus.eq(false))
        .filter(task_results::Column::PointDelta.gt(0))
        .select_only()
        .column(task_results::Column::PlayerIdFk)
        .column_as(Expr::col(task_results::Column::PlayerIdFk).count(), "cnt")
        .group_by(task_results::Column::PlayerIdFk)
        .into_model::<CountRow>()
        .all(db)
        .await?;
    for r in passed {
        map.entry(r.player_id_fk)
            .or_insert(PlayerScoreData {
                total_points: 0,
                tests_passed: 0,
            })
            .tests_passed += r.cnt as u32;
    }

    // Total points from judge results, per player.
    let judge_points = judge_results::Entity::find()
        .filter(judge_results::Column::SessionIdFk.eq(session_id))
        .select_only()
        .column(judge_results::Column::PlayerIdFk)
        .column_as(Expr::col(judge_results::Column::PointDelta).sum(), "total")
        .group_by(judge_results::Column::PlayerIdFk)
        .into_model::<SumRow>()
        .all(db)
        .await?;
    for r in judge_points {
        map.entry(r.player_id_fk)
            .or_insert(PlayerScoreData {
                total_points: 0,
                tests_passed: 0,
            })
            .total_points += r.total.unwrap_or(0);
    }

    Ok(map)
}

/// Build a per-second score-history timeseries for a session.
///
/// Returns `Ok(None)` when `started_at` is `None` or when no `task_results`
/// / `judge_results` rows exist for the session. Otherwise emits one
/// `ScoreHistorySample` per distinct second (same-second events coalesced), ordered by
/// `created_at` ascending, with cumulative per-player totals at each step.
///
/// `t` is clamped to `>= 0` (events recorded before `started_at` due to
/// clock skew collapse onto `t = 0`). Re-judges that UPDATE `point_delta`
/// without altering `created_at` replace the original delta at its original
/// `t` — keeping the timeseries shape stable across re-judges.
pub async fn build_score_history(
    db: &impl sea_orm::ConnectionTrait,
    session_id: Uuid,
    started_at: Option<chrono::DateTime<chrono::Utc>>,
) -> Result<Option<Vec<ScoreHistorySample>>, sea_orm::DbErr> {
    let started_at = match started_at {
        Some(s) => s,
        None => return Ok(None),
    };

    let task_rows = task_results::Entity::find()
        .filter(task_results::Column::SessionIdFk.eq(session_id))
        .order_by_asc(task_results::Column::CreatedAt)
        .all(db)
        .await?;
    let judge_rows = judge_results::Entity::find()
        .filter(judge_results::Column::SessionIdFk.eq(session_id))
        .order_by_asc(judge_results::Column::CreatedAt)
        .all(db)
        .await?;

    // Linear merge of two individually-sorted streams by created_at.
    let mut events: Vec<(chrono::DateTime<chrono::Utc>, Uuid, i32)> =
        Vec::with_capacity(task_rows.len() + judge_rows.len());
    let mut ti = 0;
    let mut ji = 0;
    while ti < task_rows.len() && ji < judge_rows.len() {
        if task_rows[ti].created_at <= judge_rows[ji].created_at {
            let r = &task_rows[ti];
            events.push((r.created_at, r.player_id_fk, r.point_delta));
            ti += 1;
        } else {
            let r = &judge_rows[ji];
            events.push((r.created_at, r.player_id_fk, r.point_delta));
            ji += 1;
        }
    }
    while ti < task_rows.len() {
        let r = &task_rows[ti];
        events.push((r.created_at, r.player_id_fk, r.point_delta));
        ti += 1;
    }
    while ji < judge_rows.len() {
        let r = &judge_rows[ji];
        events.push((r.created_at, r.player_id_fk, r.point_delta));
        ji += 1;
    }

    if events.is_empty() {
        return Ok(None);
    }

    let mut cumulative: HashMap<Uuid, i64> = HashMap::new();
    let mut samples: Vec<ScoreHistorySample> = Vec::new();
    for i in 0..events.len() {
        let (created_at, player_id, delta) = events[i];
        *cumulative.entry(player_id).or_insert(0) += delta as i64;
        let t = ((created_at - started_at).num_seconds()).max(0) as f64;
        // Coalesce events in the same second: build the full per-player snapshot
        // only once per distinct `t`, not once per row. The chart is per-second,
        // so this preserves its shape while avoiding O(rows * players) work and
        // memory for bursty sessions.
        let next_t = events
            .get(i + 1)
            .map(|(ca, _, _)| ((*ca - started_at).num_seconds()).max(0) as f64);
        if next_t != Some(t) {
            let scores = BTreeMap::from_iter(cumulative.iter().map(|(k, v)| (PlayerId(*k), *v)));
            samples.push(ScoreHistorySample { t, scores });
        }
    }

    Ok(Some(samples))
}

pub fn read_score_rank(leaderboard: &[LeaderboardEntry], player_id: Uuid) -> (i64, usize) {
    let mut sorted: Vec<&LeaderboardEntry> = leaderboard.iter().collect();
    sorted.sort_by(|a, b| {
        b.total_points
            .cmp(&a.total_points)
            .then_with(|| b.tests_passed.cmp(&a.tests_passed))
            .then_with(|| a.display_name.cmp(&b.display_name))
    });
    match sorted
        .iter()
        .position(|e| e.player_id.as_uuid() == player_id)
    {
        Some(idx) => (sorted[idx].total_points, idx + 1),
        None => (0, 0),
    }
}

pub async fn compute_leaderboard(
    db: &sea_orm::DatabaseConnection,
    session_id: Uuid,
) -> Result<Vec<LeaderboardEntry>, sea_orm::DbErr> {
    let player_rows = players::Entity::find()
        .filter(players::Column::SessionIdFk.eq(session_id))
        .filter(players::Column::RevokedAt.is_null())
        .all(db)
        .await?;

    let scores = aggregate_scores(db, session_id).await?;

    let mut entries: Vec<LeaderboardEntry> = player_rows
        .iter()
        .map(|p| {
            let data = scores.get(&p.id);
            LeaderboardEntry {
                player_id: PlayerId(p.id),
                display_name: p.display_name.clone(),
                agent_display_name: parse_agent_display_name(p.metadata_json.as_deref()),
                total_points: data.map(|d| d.total_points).unwrap_or(0),
                tests_passed: data.map(|d| d.tests_passed).unwrap_or(0),
                total_wall_ms: 0,
            }
        })
        .collect();

    entries.sort_by(|a, b| {
        b.total_points
            .cmp(&a.total_points)
            .then_with(|| b.tests_passed.cmp(&a.tests_passed))
            .then_with(|| a.display_name.cmp(&b.display_name))
    });

    Ok(entries)
}

/// Winner of one session: the top row of the leaderboard.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionWinner {
    pub player_id: Uuid,
    pub display_name: String,
    pub total_points: i64,
    pub tests_passed: u32,
}

/// One row of a session's final standings.
#[derive(Debug, Clone)]
pub struct SessionStanding {
    pub player_id: Uuid,
    /// Account behind the player row; `None` for anonymous joins. Anonymous
    /// players still occupy a placement — boards that aggregate by account
    /// filter them out *after* ranking, not before.
    pub user_id: Option<Uuid>,
    pub display_name: String,
    pub total_points: i64,
    pub tests_passed: u32,
    /// 1-based rank within the session under the leaderboard sort rule.
    pub placement: i32,
}

/// Final standings of every session in `session_ids`, keyed by session id.
///
/// Rows are ranked by the same rule as [`compute_leaderboard`] — points,
/// then tests passed, then name — so project boards and the session report
/// never disagree about the order.
///
/// Aggregated in bulk (a fixed four queries, not four per session) because
/// the project page asks about every finished session at once. Sessions with
/// no (unrevoked) players are absent from the map.
pub async fn session_standings(
    db: &sea_orm::DatabaseConnection,
    session_ids: &[Uuid],
) -> Result<HashMap<Uuid, Vec<SessionStanding>>, sea_orm::DbErr> {
    if session_ids.is_empty() {
        return Ok(HashMap::new());
    }

    #[derive(FromQueryResult)]
    struct SumRow {
        session_id_fk: Uuid,
        player_id_fk: Uuid,
        total: Option<i64>,
    }
    #[derive(FromQueryResult)]
    struct CountRow {
        session_id_fk: Uuid,
        player_id_fk: Uuid,
        cnt: i64,
    }

    let ids = session_ids.to_vec();
    // (session, player) -> (points, tests_passed)
    let mut tally: HashMap<(Uuid, Uuid), (i64, u32)> = HashMap::new();

    let task_points = task_results::Entity::find()
        .filter(task_results::Column::SessionIdFk.is_in(ids.clone()))
        .select_only()
        .column(task_results::Column::SessionIdFk)
        .column(task_results::Column::PlayerIdFk)
        .column_as(Expr::col(task_results::Column::PointDelta).sum(), "total")
        .group_by(task_results::Column::SessionIdFk)
        .group_by(task_results::Column::PlayerIdFk)
        .into_model::<SumRow>()
        .all(db)
        .await?;
    for r in task_points {
        tally
            .entry((r.session_id_fk, r.player_id_fk))
            .or_insert((0, 0))
            .0 += r.total.unwrap_or(0);
    }

    let judge_points = judge_results::Entity::find()
        .filter(judge_results::Column::SessionIdFk.is_in(ids.clone()))
        .select_only()
        .column(judge_results::Column::SessionIdFk)
        .column(judge_results::Column::PlayerIdFk)
        .column_as(Expr::col(judge_results::Column::PointDelta).sum(), "total")
        .group_by(judge_results::Column::SessionIdFk)
        .group_by(judge_results::Column::PlayerIdFk)
        .into_model::<SumRow>()
        .all(db)
        .await?;
    for r in judge_points {
        tally
            .entry((r.session_id_fk, r.player_id_fk))
            .or_insert((0, 0))
            .0 += r.total.unwrap_or(0);
    }

    let passed = task_results::Entity::find()
        .filter(task_results::Column::SessionIdFk.is_in(ids.clone()))
        .filter(task_results::Column::IsBonus.eq(false))
        .filter(task_results::Column::PointDelta.gt(0))
        .select_only()
        .column(task_results::Column::SessionIdFk)
        .column(task_results::Column::PlayerIdFk)
        .column_as(Expr::col(task_results::Column::PlayerIdFk).count(), "cnt")
        .group_by(task_results::Column::SessionIdFk)
        .group_by(task_results::Column::PlayerIdFk)
        .into_model::<CountRow>()
        .all(db)
        .await?;
    for r in passed {
        tally
            .entry((r.session_id_fk, r.player_id_fk))
            .or_insert((0, 0))
            .1 += r.cnt as u32;
    }

    // Revoked players are off the leaderboard.
    let player_rows = players::Entity::find()
        .filter(players::Column::SessionIdFk.is_in(ids))
        .filter(players::Column::RevokedAt.is_null())
        .all(db)
        .await?;

    let mut by_session: HashMap<Uuid, Vec<SessionStanding>> = HashMap::new();
    for p in player_rows {
        let (total_points, tests_passed) = tally
            .get(&(p.session_id_fk, p.id))
            .copied()
            .unwrap_or((0, 0));
        by_session
            .entry(p.session_id_fk)
            .or_default()
            .push(SessionStanding {
                player_id: p.id,
                user_id: p.user_id_fk,
                display_name: p.display_name,
                total_points,
                tests_passed,
                placement: 0,
            });
    }
    for rows in by_session.values_mut() {
        rows.sort_by(|a, b| {
            b.total_points
                .cmp(&a.total_points)
                .then_with(|| b.tests_passed.cmp(&a.tests_passed))
                .then_with(|| a.display_name.cmp(&b.display_name))
        });
        for (i, r) in rows.iter_mut().enumerate() {
            r.placement = i as i32 + 1;
        }
    }
    Ok(by_session)
}

/// Winner of each session in `session_ids`, keyed by session id: the top row
/// of [`session_standings`]. Sessions with no players are simply absent.
pub async fn session_winners(
    db: &sea_orm::DatabaseConnection,
    session_ids: &[Uuid],
) -> Result<HashMap<Uuid, SessionWinner>, sea_orm::DbErr> {
    Ok(session_standings(db, session_ids)
        .await?
        .into_iter()
        .filter_map(|(sid, rows)| {
            rows.into_iter().next().map(|r| {
                (
                    sid,
                    SessionWinner {
                        player_id: r.player_id,
                        display_name: r.display_name,
                        total_points: r.total_points,
                        tests_passed: r.tests_passed,
                    },
                )
            })
        })
        .collect())
}

/// Live player count per session, keyed by session id. One query for the
/// whole list; sessions with no players are absent from the map.
pub async fn player_counts(
    db: &sea_orm::DatabaseConnection,
    session_ids: &[Uuid],
) -> Result<HashMap<Uuid, u32>, sea_orm::DbErr> {
    if session_ids.is_empty() {
        return Ok(HashMap::new());
    }
    #[derive(FromQueryResult)]
    struct CountRow {
        session_id_fk: Uuid,
        cnt: i64,
    }
    let rows = players::Entity::find()
        .filter(players::Column::SessionIdFk.is_in(session_ids.to_vec()))
        .filter(players::Column::RevokedAt.is_null())
        .select_only()
        .column(players::Column::SessionIdFk)
        .column_as(Expr::col(players::Column::Id).count(), "cnt")
        .group_by(players::Column::SessionIdFk)
        .into_model::<CountRow>()
        .all(db)
        .await?;
    Ok(rows
        .into_iter()
        .map(|r| (r.session_id_fk, r.cnt as u32))
        .collect())
}

pub async fn check_task_completion(
    db: &sea_orm::DatabaseConnection,
    session_id: Uuid,
    player_id: Uuid,
    task_id: Uuid,
) -> Result<bool, sea_orm::DbErr> {
    let adapted_rows = entity_tests::Entity::find()
        .filter(entity_tests::Column::SessionId.eq(session_id))
        .filter(entity_tests::Column::TaskId.eq(task_id))
        .all(db)
        .await?;

    if adapted_rows.is_empty() {
        return Ok(false);
    }

    for at in &adapted_rows {
        let has_pass = probes::Entity::find()
            .filter(probes::Column::TestId.eq(at.id))
            .filter(probes::Column::PlayerId.eq(player_id))
            .filter(probes::Column::Outcome.eq("pass"))
            .one(db)
            .await?
            .is_some();
        if !has_pass {
            return Ok(false);
        }
    }
    Ok(true)
}

pub async fn award_completion_bonus(
    db: &sea_orm::DatabaseConnection,
    session_id: Uuid,
    player_id: Uuid,
    task_id: Uuid,
    completion_bonus_points: i32,
) -> Result<bool, sea_orm::DbErr> {
    let existing = task_results::Entity::find()
        .filter(task_results::Column::SessionIdFk.eq(session_id))
        .filter(task_results::Column::PlayerIdFk.eq(player_id))
        .filter(task_results::Column::TaskId.eq(Some(task_id)))
        .filter(task_results::Column::IsBonus.eq(true))
        .one(db)
        .await?;

    if existing.is_some() {
        return Ok(false);
    }

    let am = task_results::ActiveModel {
        id: Set(Uuid::new_v4()),
        session_id_fk: Set(session_id),
        player_id_fk: Set(player_id),
        task_id: Set(Some(task_id)),
        answer: Set(String::new()),
        created_at: Set(chrono::Utc::now()),
        point_delta: Set(completion_bonus_points),
        is_bonus: Set(true),
    };
    // The SELECT above is a fast path; the ux_task_results_completion_bonus
    // partial unique index is the real guard against a concurrent double-award
    // (two reconnect handlers racing). The loser's insert hits the constraint —
    // treat that as "already awarded", not an error.
    match am.insert(db).await {
        Ok(_) => Ok(true),
        Err(e) if is_unique_violation(&e) => Ok(false),
        Err(e) => Err(e),
    }
}

/// Whether `e` is a unique-constraint violation (SQLite / Postgres).
fn is_unique_violation(e: &sea_orm::DbErr) -> bool {
    let msg = e.to_string().to_lowercase();
    msg.contains("unique") || msg.contains("duplicate")
}
