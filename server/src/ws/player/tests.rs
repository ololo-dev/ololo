use super::*;

#[test]
fn handler_is_async_fn() {
    let _ = std::any::TypeId::of::<PlayerWsParams>();
}

// CONC-H1: a reconnect supersedes the prior connection, and the superseded
// handler's teardown must not evict the live successor's channel.
#[test]
fn reconnect_supersedes_and_conditional_remove_keeps_successor() {
    use crate::state::{PlayerChannel, PlayerRegistry};
    use dashmap::DashMap;
    use std::sync::Arc;
    use tokio::sync::broadcast;

    let registry: PlayerRegistry = Arc::new(DashMap::new());
    let pid = uuid::Uuid::new_v4();

    let (tx_a, _) = broadcast::channel(4);
    let chan_a = Arc::new(PlayerChannel::new(tx_a));
    registry.insert(pid, chan_a.clone());

    // Reconnect: newest wins.
    let (tx_b, _) = broadcast::channel(4);
    let chan_b = Arc::new(PlayerChannel::new(tx_b));
    registry.insert(pid, chan_b.clone());

    // Old handler tears down: must NOT evict the live successor.
    registry.remove_if(&pid, |_, current| Arc::ptr_eq(current, &chan_a));
    assert!(
        registry.contains_key(&pid),
        "superseded teardown must not evict the reconnected channel"
    );
    assert!(Arc::ptr_eq(&registry.get(&pid).unwrap(), &chan_b));

    // New handler tears down: removes its own entry.
    registry.remove_if(&pid, |_, current| Arc::ptr_eq(current, &chan_b));
    assert!(!registry.contains_key(&pid));
}
