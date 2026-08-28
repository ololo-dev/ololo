//! Moved from src/recovery.rs `mod tests`.

use arena_core::session_status::SessionStatus;
use game_server::recovery::{RecoveryAction, decide_recovery, restart_stagger_ms};
use game_server::state::compute_remaining;

// ponytail: env is global + tests run in parallel; one test mutates it sequentially.
#[test]
fn stagger_env_cases() {
    unsafe {
        std::env::remove_var("ARENA_RESTART_STAGGER_MS");
    }
    assert_eq!(restart_stagger_ms(), 50);
    unsafe {
        std::env::set_var("ARENA_RESTART_STAGGER_MS", "250");
    }
    assert_eq!(restart_stagger_ms(), 250);
    unsafe {
        std::env::set_var("ARENA_RESTART_STAGGER_MS", "not-a-number");
    }
    assert_eq!(restart_stagger_ms(), 50);
    unsafe {
        std::env::remove_var("ARENA_RESTART_STAGGER_MS");
    }
}

#[test]
fn decide_running_positive_remaining_spawns_timer() {
    let action = decide_recovery(SessionStatus::Running, 120);
    match action {
        RecoveryAction::SpawnRunning { remaining } => assert_eq!(remaining, 120),
        other => panic!(
            "expected SpawnRunning, got {:?}",
            other_discriminant(&other)
        ),
    }
}

#[test]
fn decide_running_zero_remaining_finishes_now() {
    let action = decide_recovery(SessionStatus::Running, 0);
    assert!(matches!(action, RecoveryAction::FinishNow));
}

#[test]
fn decide_running_negative_remaining_finishes_now() {
    let action = decide_recovery(SessionStatus::Running, -50);
    assert!(matches!(action, RecoveryAction::FinishNow));
}

#[test]
fn decide_paused_spawns_parked_poller() {
    let action = decide_recovery(SessionStatus::Paused, 300);
    assert!(matches!(action, RecoveryAction::SpawnParked));
}

#[test]
fn decide_paused_zero_remaining_still_parks() {
    // Paused sessions don't transition to Finished on recovery; the parked
    // poller re-reads DB status and lets the owner resume into running_timer
    // (which itself handles the remaining<=0 → Finished transition).
    let action = decide_recovery(SessionStatus::Paused, 0);
    assert!(matches!(action, RecoveryAction::SpawnParked));
}

#[test]
fn compute_remaining_subtracts_elapsed_adds_paused() {
    let started = chrono::Utc::now() - chrono::Duration::seconds(40);
    let now = chrono::Utc::now();
    // 100s duration, 40s elapsed, 10s paused => 70s remaining
    // (pause window added back so remaining stays frozen across pause).
    assert_eq!(compute_remaining(100, started, now, Some(10)), 70);
}

#[test]
fn compute_remaining_clamps_to_zero() {
    let started = chrono::Utc::now() - chrono::Duration::seconds(200);
    let now = chrono::Utc::now();
    assert_eq!(compute_remaining(100, started, now, None), 0);
    assert_eq!(compute_remaining(100, started, now, Some(50)), 0);
}

#[test]
fn compute_remaining_no_pause_uses_zero() {
    let started = chrono::Utc::now() - chrono::Duration::seconds(10);
    let now = chrono::Utc::now();
    assert_eq!(compute_remaining(100, started, now, None), 90);
}

// ponytail: helper only — keeps match arms exhaustive-friendly for panic msgs.
fn other_discriminant(action: &RecoveryAction) -> &'static str {
    match action {
        RecoveryAction::SpawnRunning { .. } => "SpawnRunning",
        RecoveryAction::SpawnParked => "SpawnParked",
        RecoveryAction::FinishNow => "FinishNow",
    }
}
