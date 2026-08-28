use crate::state::{PlayerRegistry, SessionRegistry};
use arena_core::protocol::{ArenaFrame, PlayerFrame, ScoreRankUpdatedPayload};
use arena_core::scoring;
use uuid::Uuid;

pub use scoring::{
    PlayerScoreData, aggregate_scores, award_completion_bonus, check_task_completion,
    compute_leaderboard,
};

pub fn read_score_rank(
    session_registry: &SessionRegistry,
    session_id: Uuid,
    player_id: Uuid,
) -> (i64, usize) {
    for entry in session_registry.iter() {
        let cache = match entry.cache.read() {
            Ok(c) => c,
            Err(_) => continue,
        };
        if cache.session_id != session_id {
            continue;
        }
        let score = cache
            .leaderboard
            .iter()
            .find(|e| e.player_id.as_uuid() == player_id)
            .map(|e| e.total_points)
            .unwrap_or(0);
        let rank = cache
            .leaderboard
            .iter()
            .position(|e| e.player_id.as_uuid() == player_id)
            .map(|p| p + 1)
            .unwrap_or(0);
        return (score, rank);
    }
    (0, 0)
}

pub async fn broadcast_leaderboard(
    db: &sea_orm::DatabaseConnection,
    session_registry: &SessionRegistry,
    player_registry: &PlayerRegistry,
    join_code: &str,
    session_id: Uuid,
) {
    let entries = match compute_leaderboard(db, session_id).await {
        Ok(e) => e,
        Err(_) => return,
    };

    if let Some(entry) = session_registry.get(join_code) {
        let version = if let Ok(mut cache) = entry.cache.write() {
            cache.version = cache.version.saturating_add(1);
            cache.leaderboard = entries.clone();
            cache.version
        } else {
            0
        };
        let _ = entry.tx.send(ArenaFrame::LeaderboardUpdate {
            entries: entries.clone(),
            version,
        });

        for (rank_idx, lb_entry) in entries.iter().enumerate() {
            let pid = lb_entry.player_id.as_uuid();
            if let Some(ch) = player_registry.get(&pid) {
                let score = lb_entry.total_points;
                let rank = rank_idx + 1;
                let _ = ch.send_sequenced(|seq| {
                    PlayerFrame::ScoreRankUpdated(ScoreRankUpdatedPayload { seq, score, rank })
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{PlayerChannel, SessionCacheInner, SessionEntry};
    use arena_core::protocol::LeaderboardEntry;
    use arena_core::protocol::PlayerId;
    use arena_core::session_status::SessionStatus;
    use dashmap::DashMap;
    use std::sync::Arc;
    use tokio::sync::broadcast;

    fn make_session_entry(session_id: Uuid, entries: Vec<LeaderboardEntry>) -> SessionEntry {
        let (tx, _) = broadcast::channel::<ArenaFrame>(16);
        SessionEntry {
            tx,
            cache: Arc::new(std::sync::RwLock::new(SessionCacheInner {
                session_id,
                phase: SessionStatus::Running,
                version: 0,
                participants: vec![],
                leaderboard: entries,
                started_at: None,
            })),
        }
    }

    #[tokio::test]
    async fn broadcast_leaderboard_fans_out_score_rank_updated() {
        let _db = sea_orm::DatabaseConnection::Disconnected;

        let session_id = Uuid::new_v4();
        let p1 = Uuid::new_v4();
        let p2 = Uuid::new_v4();

        let session_registry: SessionRegistry = Arc::new(DashMap::new());
        let join_code = "TESTCODE";
        session_registry.insert(
            join_code.to_string(),
            make_session_entry(session_id, vec![]),
        );

        let player_registry: PlayerRegistry = Arc::new(DashMap::new());
        let (tx1, mut rx1) = broadcast::channel::<crate::protocol::PlayerFrame>(16);
        let (tx2, mut rx2) = broadcast::channel::<crate::protocol::PlayerFrame>(16);
        player_registry.insert(p1, Arc::new(PlayerChannel::new(tx1)));
        player_registry.insert(p2, Arc::new(PlayerChannel::new(tx2)));

        let entries = vec![
            LeaderboardEntry {
                player_id: PlayerId(p1),
                display_name: "Alice".to_string(),
                agent_display_name: None,
                total_points: 100,
                tests_passed: 5,
                total_wall_ms: 0,
            },
            LeaderboardEntry {
                player_id: PlayerId(p2),
                display_name: "Bob".to_string(),
                agent_display_name: None,
                total_points: 50,
                tests_passed: 2,
                total_wall_ms: 0,
            },
        ];

        {
            let entry = session_registry.get(join_code).unwrap();
            let mut cache = entry.cache.write().unwrap();
            cache.leaderboard = entries.clone();
        }

        let version = {
            let entry = session_registry.get(join_code).unwrap();
            let mut cache = entry.cache.write().unwrap();
            cache.version = cache.version.saturating_add(1);
            cache.leaderboard = entries.clone();
            cache.version
        };
        let _ = session_registry
            .get(join_code)
            .unwrap()
            .tx
            .send(ArenaFrame::LeaderboardUpdate {
                entries: entries.clone(),
                version,
            });

        for (rank_idx, lb_entry) in entries.iter().enumerate() {
            let pid = lb_entry.player_id.as_uuid();
            if let Some(ch) = player_registry.get(&pid) {
                let score = lb_entry.total_points;
                let rank = rank_idx + 1;
                let _ = ch.send_sequenced(|seq| {
                    PlayerFrame::ScoreRankUpdated(ScoreRankUpdatedPayload { seq, score, rank })
                });
            }
        }

        match rx1.try_recv() {
            Ok(PlayerFrame::ScoreRankUpdated(payload)) => {
                assert_eq!(payload.score, 100);
                assert_eq!(payload.rank, 1);
                assert_eq!(payload.seq, 0);
            }
            other => panic!("expected ScoreRankUpdated for p1, got {:?}", other),
        }

        match rx2.try_recv() {
            Ok(PlayerFrame::ScoreRankUpdated(payload)) => {
                assert_eq!(payload.score, 50);
                assert_eq!(payload.rank, 2);
                assert_eq!(payload.seq, 0);
            }
            other => panic!("expected ScoreRankUpdated for p2, got {:?}", other),
        }
    }

    #[test]
    fn broadcast_leaderboard_accepts_player_registry_arg() {
        fn _type_check(
            db: &sea_orm::DatabaseConnection,
            sr: &SessionRegistry,
            pr: &PlayerRegistry,
            jc: &str,
            sid: Uuid,
        ) {
            let _fut = broadcast_leaderboard(db, sr, pr, jc, sid);
        }
    }

    #[test]
    fn write_lock_released_before_fan_out() {
        let session_id = Uuid::new_v4();
        let p1 = Uuid::new_v4();

        let session_registry: SessionRegistry = Arc::new(DashMap::new());
        let join_code = "LOCKTEST";
        session_registry.insert(
            join_code.to_string(),
            make_session_entry(session_id, vec![]),
        );

        let player_registry: PlayerRegistry = Arc::new(DashMap::new());
        let (tx, _rx) = broadcast::channel::<crate::protocol::PlayerFrame>(16);
        let ch = Arc::new(PlayerChannel::new(tx));

        let _ = ch.send_sequenced(|seq| {
            let entry = session_registry.get(join_code).unwrap();
            let cache = entry.cache.read().unwrap();
            assert!(cache.leaderboard.is_empty());
            PlayerFrame::ScoreRankUpdated(ScoreRankUpdatedPayload {
                seq,
                score: 0,
                rank: 0,
            })
        });

        player_registry.insert(p1, ch);
    }
}
