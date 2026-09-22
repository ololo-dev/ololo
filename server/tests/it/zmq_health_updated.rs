//! A `ZmqEvent::HealthUpdated` reaches both rooms: the session dashboard
//! (re-stamped with the server's version counter, like every bridged frame)
//! and the player's own page (with or without a dashboard open).

use arena_core::protocol::{
    HealthCheckStatus, HealthCheckpointKind, HealthCheckpointView, HealthFlags, Level, PlayerFrame,
    ZmqEvent,
};
use arena_core::session_status::SessionStatus;
use migration::{Migrator, MigratorTrait};
use server::protocol::ArenaFrame;
use server::state::{PlayerChannel, SessionCacheInner, SessionEntry};
use server::{AppState, AuthConfig};
use std::sync::{Arc, RwLock};
use std::time::Duration;
use tokio::sync::broadcast;
use uuid::Uuid;

const JOIN_CODE: &str = "HLTHWS";

async fn test_state() -> AppState {
    let db = sea_orm::Database::connect("sqlite::memory:")
        .await
        .expect("sqlite connect");
    Migrator::up(&db, None).await.expect("migrate up");
    let cfg = AuthConfig {
        jwt_signing_key: b"integration-test-secret-32-bytes-or-more-xxxxxxx".to_vec(),
        frontend_origins: vec!["http://localhost:5173".to_string()],
        access_ttl: Duration::from_secs(900),
        refresh_ttl: Duration::from_secs(30 * 86_400),
        max_agents_per_session: 16,
    };
    AppState::new(db, cfg)
}

fn checkpoint() -> HealthCheckpointView {
    HealthCheckpointView {
        id: Uuid::new_v4(),
        kind: HealthCheckpointKind::Probe,
        probe_id: Some(Uuid::new_v4()),
        probe_seq: 4,
        task_id: Some(Uuid::new_v4()),
        task_title: Some("Widget".into()),
        commit: "abc123".into(),
        created_at: chrono::Utc::now(),
        t: Some(12.0),
        client: None,
        server: None,
        server_status: HealthCheckStatus::Pending,
        flags: HealthFlags::default(),
        score: Some(74.3),
        level: Level::Green,
    }
}

#[tokio::test]
async fn a_health_update_reaches_the_dashboard_and_the_player_page() {
    let state = test_state().await;
    let player_id = Uuid::new_v4();
    let session_id = Uuid::new_v4();

    let (tx, mut dash_rx) = broadcast::channel::<ArenaFrame>(16);
    state.session_registry.insert(
        JOIN_CODE.to_string(),
        SessionEntry {
            tx,
            cache: Arc::new(RwLock::new(SessionCacheInner {
                session_id,
                phase: SessionStatus::Running,
                version: 7,
                participants: vec![],
                leaderboard: vec![],
                started_at: None,
            })),
        },
    );
    let (ptx, mut player_rx) = broadcast::channel::<PlayerFrame>(8);
    state
        .player_registry
        .insert(player_id, Arc::new(PlayerChannel::new(ptx)));

    let view = checkpoint();
    server::zmq_sub::route_event(
        &state,
        &ZmqEvent::HealthUpdated {
            join_code: JOIN_CODE.to_string(),
            player_id,
            checkpoint: Box::new(view.clone()),
            timestamp: chrono::Utc::now(),
            // The game server's counter must not leak into the room.
            version: 99_999,
        },
    )
    .await
    .expect("route");

    match dash_rx.try_recv() {
        Ok(ArenaFrame::HealthUpdated {
            player_id: pid,
            checkpoint,
            version,
        }) => {
            assert_eq!(pid, player_id);
            assert_eq!(*checkpoint, view);
            assert_eq!(version, 8, "the server's counter, advanced once");
        }
        other => panic!("expected HealthUpdated on the dashboard, got {other:?}"),
    }
    match player_rx.try_recv() {
        Ok(PlayerFrame::HealthUpdated { checkpoint }) => assert_eq!(*checkpoint, view),
        other => panic!("expected HealthUpdated on the player page, got {other:?}"),
    }
}

#[tokio::test]
async fn without_a_dashboard_the_player_page_still_hears_it() {
    let state = test_state().await;
    let player_id = Uuid::new_v4();
    let (ptx, mut player_rx) = broadcast::channel::<PlayerFrame>(8);
    state
        .player_registry
        .insert(player_id, Arc::new(PlayerChannel::new(ptx)));
    server::zmq_sub::route_event(
        &state,
        &ZmqEvent::HealthUpdated {
            join_code: "NOROOM".to_string(),
            player_id,
            checkpoint: Box::new(checkpoint()),
            timestamp: chrono::Utc::now(),
            version: 1,
        },
    )
    .await
    .expect("route");
    assert!(matches!(
        player_rx.try_recv(),
        Ok(PlayerFrame::HealthUpdated { .. })
    ));
}
