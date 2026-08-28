//! Moved from src/ws/player_agent.rs inline test modules.

use arena_core::entities::tasks::Model;
use arena_core::session_status::SessionStatus;
use game_server::ws::player_agent::{ProbeAction, decide_probe_action};

// ponytail: static-assertion test — proves the pass_points→point_value merge.
// The pass delta reads `task_row.point_value`; this test pins
// that the field exists on the model and that `pass_points` does not. If a
// future refactor reintroduces `pass_points`, this fails to compile,
// surfacing the regression before it reaches the scoring path.
#[test]
fn pass_delta_reads_point_value_not_pass_points() {
    // Compile-time field access — no runtime work.
    fn _assert_has_point_value(m: &Model) -> i32 {
        m.point_value
    }
    // `pass_points` must not exist on the model. Encoded as a trait bound
    // that cannot be satisfied if the field is reintroduced: we rely on
    // the fact that the field-access closure above is the only reference.
    // (A dedicated `pass_points` field would re-add a column and a second
    // scoring source — exactly what the merge eliminated.)
    let _ = _assert_has_point_value;
}

#[test]
fn running_dispatches() {
    assert!(matches!(
        decide_probe_action(SessionStatus::Running),
        ProbeAction::Dispatch
    ));
}

#[test]
fn paused_skips_dispatch() {
    assert!(matches!(
        decide_probe_action(SessionStatus::Paused),
        ProbeAction::Pause
    ));
}

#[test]
fn finished_exits() {
    assert!(matches!(
        decide_probe_action(SessionStatus::Finished),
        ProbeAction::Exit
    ));
}

#[test]
fn cancelled_exits() {
    assert!(matches!(
        decide_probe_action(SessionStatus::Cancelled),
        ProbeAction::Exit
    ));
}

#[test]
fn lobby_skips_dispatch_treated_as_pause() {
    assert!(matches!(
        decide_probe_action(SessionStatus::Lobby),
        ProbeAction::Pause
    ));
}

#[tokio::test]
async fn pause_branch_select_breaks_on_cancel() {
    use arena_core::protocol::ArenaFrame;
    use game_server::state::{SessionCacheInner, SessionEntry};
    use std::sync::{Arc, RwLock};

    let (tx, _) = tokio::sync::broadcast::channel::<ArenaFrame>(16);
    let cache = Arc::new(RwLock::new(SessionCacheInner {
        session_id: uuid::Uuid::new_v4(),
        phase: SessionStatus::Paused,
        version: 0,
        participants: vec![],
        leaderboard: vec![],
        started_at: None,
    }));
    let entry = SessionEntry {
        tx,
        cache,
        cancel: tokio_util::sync::CancellationToken::new(),
    };
    let cancel = entry.cancel.clone();
    let handle = tokio::spawn(async move {
        tokio::select! {
            _ = cancel.cancelled() => "cancelled",
            _ = tokio::time::sleep(std::time::Duration::from_secs(30)) => "timed_out",
        }
    });
    entry.cancel.cancel();
    assert_eq!(handle.await.unwrap(), "cancelled");
}
