//! Moved from src/ws/session_lifecycle.rs `mod tests`.

use arena_core::protocol::ArenaFrame;
use arena_core::session_status::SessionStatus;
use game_server::state::{SessionCacheInner, SessionEntry, compute_remaining};
use std::sync::{Arc, RwLock};
use tokio::sync::broadcast;
use uuid::Uuid;

fn make_entry() -> SessionEntry {
    let (tx, _) = broadcast::channel::<ArenaFrame>(16);
    let cache = Arc::new(RwLock::new(SessionCacheInner::new(
        Uuid::new_v4(),
        SessionStatus::Running,
        None,
    )));
    SessionEntry {
        tx,
        cache,
        cancel: tokio_util::sync::CancellationToken::new(),
    }
}

#[test]
fn session_entry_has_cancel_field() {
    let entry = make_entry();
    // ponytail: field existence proven by construction; cancel is live.
    let _ = entry.cancel.clone();
}

#[tokio::test]
async fn cancelled_token_wakes_select() {
    // ponytail: proves cancel.cancelled() arms a select! branch and wakes
    // a sleeping timer. Mirrors the lobby_timer/running_timer pattern.
    let entry = make_entry();
    let cancel = entry.cancel.clone();
    let handle = tokio::spawn(async move {
        tokio::select! {
            _ = cancel.cancelled() => "cancelled",
            _ = tokio::time::sleep(std::time::Duration::from_secs(30)) => "timed_out",
        }
    });
    entry.cancel.cancel();
    let result = handle.await.unwrap();
    assert_eq!(result, "cancelled");
}

#[test]
fn recompute_remaining_subtracts_elapsed_adds_paused() {
    let started = chrono::Utc::now() - chrono::Duration::seconds(40);
    let now = chrono::Utc::now();
    // 100s duration, 40s elapsed, 10s paused => 70s remaining
    // (pause window added back so remaining stays frozen across pause).
    assert_eq!(compute_remaining(100, started, now, Some(10)), 70);
}

#[test]
fn recompute_remaining_clamps_to_zero() {
    let started = chrono::Utc::now() - chrono::Duration::seconds(200);
    let now = chrono::Utc::now();
    // 100s duration, 200s elapsed => clamped to 0 (no underflow).
    assert_eq!(compute_remaining(100, started, now, None), 0);
    // Even with 50s pause credit, elapsed exceeds duration.
    assert_eq!(compute_remaining(100, started, now, Some(50)), 0);
}

#[test]
fn recompute_remaining_no_pause_uses_zero() {
    let started = chrono::Utc::now() - chrono::Duration::seconds(10);
    let now = chrono::Utc::now();
    // 100s duration, 10s elapsed, no pause => 90s remaining.
    assert_eq!(compute_remaining(100, started, now, None), 90);
}

#[test]
fn recompute_remaining_zero_on_resume_transitions_to_finished() {
    // ponytail: contract — remaining <= 0 on resume => Finished.
    // Encoded as the arithmetic producing 0; running_timer breaks the loop
    // and runs the completion path when remaining <= 0.
    let started = chrono::Utc::now() - chrono::Duration::seconds(100);
    let now = chrono::Utc::now();
    let remaining = compute_remaining(100, started, now, None);
    assert!(remaining <= 0, "remaining must be <= 0 to trigger Finished");
}
