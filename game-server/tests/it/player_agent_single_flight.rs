//! One player, one dispatching connection.
//!
//! `player_agent_registry` is keyed by `player_id` alone, so a reconnect leaves
//! two live handlers for the same player. Both dispatch every probe, the
//! duplicates execute concurrently on the player's machine and race over the
//! shared `.ololo/tmp` state, and the answers cross between them — observed in
//! session VKIBCB, where duplicates handed out +40 points and took away -30 on
//! the same tasks. These tests pin the two mechanisms that prevent it.

use std::sync::Arc;

use arena_core::protocol::PlayerAgentFrame;
use dashmap::DashMap;
use game_server::ws::player_agent::registry_guard::{RegistryGuard, is_current_connection};
use tokio::sync::mpsc;
use uuid::Uuid;

fn registry() -> Arc<DashMap<Uuid, mpsc::Sender<PlayerAgentFrame>>> {
    Arc::new(DashMap::new())
}

#[tokio::test]
async fn a_superseded_handler_sees_that_it_is_no_longer_current() {
    let reg = registry();
    let player = Uuid::new_v4();

    let (first_tx, _first_rx) = mpsc::channel::<PlayerAgentFrame>(4);
    reg.insert(player, first_tx.clone());
    assert!(
        is_current_connection(&reg, player, &first_tx),
        "the only connection is the current one"
    );

    // The player reconnects: a second handler overwrites the entry.
    let (second_tx, _second_rx) = mpsc::channel::<PlayerAgentFrame>(4);
    reg.insert(player, second_tx.clone());

    assert!(
        !is_current_connection(&reg, player, &first_tx),
        "the older handler must be able to tell it was superseded, or it keeps \
         dispatching probes alongside the new one"
    );
    assert!(is_current_connection(&reg, player, &second_tx));
}

#[tokio::test]
async fn an_old_handlers_teardown_does_not_unregister_the_live_one() {
    let reg = registry();
    let player = Uuid::new_v4();

    let (first_tx, _first_rx) = mpsc::channel::<PlayerAgentFrame>(4);
    reg.insert(player, first_tx.clone());
    let first_guard = RegistryGuard::new(reg.clone(), player, first_tx.clone());

    // Reconnect: the newer handler registers and holds its own guard.
    let (second_tx, _second_rx) = mpsc::channel::<PlayerAgentFrame>(4);
    reg.insert(player, second_tx.clone());
    let _second_guard = RegistryGuard::new(reg.clone(), player, second_tx.clone());

    // The stale handler finally tears down.
    drop(first_guard);

    assert!(
        reg.contains_key(&player),
        "the live connection must stay registered — an unconditional remove here \
         silently stopped JudgeScored delivery to a healthy socket"
    );
    assert!(
        is_current_connection(&reg, player, &second_tx),
        "and the surviving registration must be the newer handler's"
    );
}

#[tokio::test]
async fn the_last_handler_standing_cleans_up_after_itself() {
    let reg = registry();
    let player = Uuid::new_v4();

    let (tx, _rx) = mpsc::channel::<PlayerAgentFrame>(4);
    reg.insert(player, tx.clone());
    let guard = RegistryGuard::new(reg.clone(), player, tx.clone());
    drop(guard);

    assert!(
        !reg.contains_key(&player),
        "a handler that is still current on teardown must remove its own entry, \
         or the registry leaks channels for departed players"
    );
}
