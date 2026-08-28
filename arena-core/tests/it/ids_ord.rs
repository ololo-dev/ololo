use arena_core::protocol::{PlayerId, SessionId, TaskId};

#[test]
fn player_id_ord_btreemap() {
    let mut m = std::collections::BTreeMap::new();
    let k = PlayerId(uuid::Uuid::nil());
    m.insert(k, 1);
    m.insert(k, 2);
    assert_eq!(m.len(), 1);
    assert_eq!(m[&k], 2);
}

#[test]
fn task_id_ord_btreemap() {
    let mut m = std::collections::BTreeMap::new();
    let k = TaskId(uuid::Uuid::nil());
    m.insert(k, 1);
    m.insert(k, 2);
    assert_eq!(m.len(), 1);
    assert_eq!(m[&k], 2);
}

#[test]
fn session_id_ord_btreemap() {
    let mut m = std::collections::BTreeMap::new();
    let k = SessionId(uuid::Uuid::nil());
    m.insert(k, 1);
    m.insert(k, 2);
    assert_eq!(m.len(), 1);
    assert_eq!(m[&k], 2);
}
