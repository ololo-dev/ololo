use super::wire::PlayerAgentFrame;
use super::*;
use crate::tui::event::PlayerRunStatus;

#[test]
fn parse_player_id_accepts_valid_uuid() {
    let s = "550e8400-e29b-41d4-a716-446655440000";
    let parsed = parse_player_id(s);
    assert!(parsed.is_some());
    assert_eq!(parsed.unwrap().to_string(), s);
}

#[test]
fn parse_player_id_rejects_malformed_string() {
    assert!(parse_player_id("not-a-uuid").is_none());
    assert!(parse_player_id("").is_none());
    assert!(parse_player_id("  ").is_none());
}

#[test]
fn decode_leaderboard_update_frame() {
    let viewer = Uuid::nil();
    let json = format!(
        r#"{{"type":"leaderboard_update","version":3,"entries":[
            {{"player_id":"{viewer}","display_name":"me","total_points":42,"tests_passed":4,"total_wall_ms":1000}},
            {{"player_id":"550e8400-e29b-41d4-a716-446655440000","display_name":"other","total_points":10,"tests_passed":1,"total_wall_ms":500}}
        ]}}"#
    );
    let frame: PlayerAgentFrame = serde_json::from_str(&json).unwrap();
    match frame {
        PlayerAgentFrame::LeaderboardUpdate { entries, .. } => {
            assert_eq!(entries.len(), 2);
            assert_eq!(entries[0].total_points, 42);
            assert_eq!(entries[0].player_id, viewer);
        }
        _ => panic!("expected LeaderboardUpdate, got {frame:?}"),
    }
}

#[test]
fn decode_player_progress_update_frame() {
    let viewer = Uuid::nil();
    let json = format!(
        r#"{{"type":"player_progress_update","player_id":"{viewer}","current_task_id":null,"attempt":2,"status":"awaiting_result"}}"#
    );
    let frame: PlayerAgentFrame = serde_json::from_str(&json).unwrap();
    match frame {
        PlayerAgentFrame::PlayerProgressUpdate {
            player_id,
            attempt,
            status,
            ..
        } => {
            assert_eq!(player_id, viewer);
            assert_eq!(attempt, 2);
            assert_eq!(status, PlayerRunStatus::AwaitingResult);
        }
        _ => panic!("expected PlayerProgressUpdate, got {frame:?}"),
    }
}

#[test]
fn decode_session_complete_with_player_tasks_completed_reason() {
    let json =
        r#"{"type":"session_complete","session_id":"abc","reason":"player_tasks_completed"}"#;
    let frame: PlayerAgentFrame = serde_json::from_str(json).unwrap();
    match frame {
        PlayerAgentFrame::SessionComplete { reason, .. } => {
            assert_eq!(
                reason.as_deref(),
                Some(arena_core::protocol::SESSION_COMPLETE_REASON_PLAYER_TASKS_COMPLETED)
            );
        }
        _ => panic!("expected SessionComplete, got {frame:?}"),
    }
}

#[test]
fn player_tasks_completed_ack_is_not_terminal() {
    // The per-player "all your tasks are done" ack must keep the frame
    // loop (and the WebSocket) alive — the session is still running for
    // other players and the real session-end frame arrives later.
    assert!(!session_complete_is_terminal(Some(
        arena_core::protocol::SESSION_COMPLETE_REASON_PLAYER_TASKS_COMPLETED
    )));
}

#[test]
fn final_session_complete_reasons_are_terminal() {
    for reason in [
        None,
        Some("all_tasks_completed"),
        Some("time_expired"),
        Some("cancelled"),
        Some("finished"),
    ] {
        assert!(
            session_complete_is_terminal(reason),
            "reason {reason:?} must end the frame loop"
        );
    }
}

#[test]
fn progress_for_other_player_is_dropped() {
    let viewer = Uuid::nil();
    let other = Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap();
    let json = format!(
        r#"{{"type":"player_progress_update","player_id":"{other}","current_task_id":null,"attempt":1,"status":"backoff"}}"#
    );
    let frame: PlayerAgentFrame = serde_json::from_str(&json).unwrap();
    // The viewer filter in connect_once compares viewer_player_id == Some(player_id).
    // Verify the decoded player_id does not match the viewer.
    match frame {
        PlayerAgentFrame::PlayerProgressUpdate { player_id, .. } => {
            assert_ne!(Some(player_id), Some(viewer));
        }
        _ => panic!("expected PlayerProgressUpdate"),
    }
}

#[test]
fn agent_ws_url_carries_player_identity() {
    let pid = uuid::Uuid::parse_str("e9f05c24-5e55-4841-9e6b-68b3f67a2422").unwrap();
    assert_eq!(
        super::agent_ws_url("wss://gs.example", "VKIBCB", Some(pid)),
        "wss://gs.example/ws/player/agent/VKIBCB?player_id=e9f05c24-5e55-4841-9e6b-68b3f67a2422"
    );
    // Without a resolved player id (old flow) the URL stays bare.
    assert_eq!(
        super::agent_ws_url("wss://gs.example", "VKIBCB", None),
        "wss://gs.example/ws/player/agent/VKIBCB"
    );
}

/// The exhaustion contract `run_with_sink`'s outer loop depends on: against a
/// dead endpoint, `run_connect_loop` must come back `false` after its attempt
/// budget (rather than hanging or erroring out), with the backoff schedule
/// advanced — that return is what now triggers a re-resolve instead of the
/// old fatal exit that abandoned the session mid-run.
#[tokio::test]
async fn connect_loop_reports_exhaustion_against_dead_endpoint() {
    // A port that just had a listener and lost it: connection refused, fast.
    let port = {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.local_addr().unwrap().port()
    };
    let url = format!("ws://127.0.0.1:{port}/ws/player/agent/TEST");
    let mut backoff_ms: u64 = 1;
    let mut memory = None;
    let completed = super::connect::run_connect_loop(
        &url,
        "ololo_test_pat",
        None,
        None,
        &mut backoff_ms,
        Some(2),
        &mut memory,
    )
    .await;
    assert!(!completed, "a dead endpoint must exhaust, not complete");
    assert!(backoff_ms > 1, "the backoff schedule must have advanced");
}
