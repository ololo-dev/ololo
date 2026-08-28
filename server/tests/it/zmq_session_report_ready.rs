//! Regression: the session report reached the player page only on a manual
//! reload. It is written after the session finished — the page's reconcile
//! poll has stopped by then, and the report judge deliberately publishes no
//! verdict — so nothing told the page to look. `SessionReportReady` is that
//! cue, and it must reach the player's own channel whether or not a session
//! dashboard is open.

use arena_core::protocol::{PlayerFrame, ZmqEvent};
use migration::{Migrator, MigratorTrait};
use server::state::PlayerChannel;
use server::{AppState, AuthConfig};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::broadcast;
use uuid::Uuid;

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

#[tokio::test]
async fn the_report_cue_reaches_the_player_page() {
    let state = test_state().await;
    let player_id = Uuid::new_v4();
    let (tx, mut rx) = broadcast::channel::<PlayerFrame>(8);
    state
        .player_registry
        .insert(player_id, Arc::new(PlayerChannel::new(tx)));

    server::zmq_sub::route_event(
        &state,
        &ZmqEvent::SessionReportReady {
            join_code: "REPORT".to_string(),
            player_id,
            timestamp: chrono::Utc::now(),
        },
    )
    .await
    .expect("route");

    assert!(
        matches!(rx.try_recv(), Ok(PlayerFrame::SessionReportReady)),
        "the player's channel must carry the cue"
    );
}

#[tokio::test]
async fn another_players_report_is_not_announced_here() {
    let state = test_state().await;
    let mine = Uuid::new_v4();
    let (tx, mut rx) = broadcast::channel::<PlayerFrame>(8);
    state
        .player_registry
        .insert(mine, Arc::new(PlayerChannel::new(tx)));

    server::zmq_sub::route_event(
        &state,
        &ZmqEvent::SessionReportReady {
            join_code: "REPORT".to_string(),
            player_id: Uuid::new_v4(),
            timestamp: chrono::Utc::now(),
        },
    )
    .await
    .expect("route");

    assert!(rx.try_recv().is_err(), "a report is one player's own");
}
