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

#[test]
fn decode_probe_graded_with_and_without_the_next_check_hint() {
    let pid = Uuid::nil();
    let json = format!(
        r#"{{"type":"probe_graded","probe_id":"{pid}","outcome":"pass","point_delta":5,"next_probe_in_secs":12}}"#
    );
    let frame: PlayerAgentFrame = serde_json::from_str(&json).unwrap();
    match frame {
        PlayerAgentFrame::ProbeGraded {
            next_probe_in_secs, ..
        } => assert_eq!(next_probe_in_secs, Some(12)),
        _ => panic!("expected ProbeGraded, got {frame:?}"),
    }
    // Pre-upgrade servers send no hint; the frame still decodes.
    let json = format!(
        r#"{{"type":"probe_graded","probe_id":"{pid}","outcome":"error","point_delta":-1}}"#
    );
    let frame: PlayerAgentFrame = serde_json::from_str(&json).unwrap();
    match frame {
        PlayerAgentFrame::ProbeGraded {
            next_probe_in_secs, ..
        } => assert_eq!(next_probe_in_secs, None),
        _ => panic!("expected ProbeGraded, got {frame:?}"),
    }
}

#[test]
fn decode_judge_lifecycle_frames() {
    let tid = Uuid::nil();
    // The server sends the player page's status payload verbatim — fields
    // the CLI does not read (status, updated_at, judge_result_id) pass by.
    let json = format!(
        r#"{{"type":"judge_started","task_id":"{tid}","judge_slug":"correctness","judge_name":"Correctness","status":"running","error":null,"updated_at":"2026-01-01T00:00:00Z","judge_result_id":null}}"#
    );
    let frame: PlayerAgentFrame = serde_json::from_str(&json).unwrap();
    match frame {
        PlayerAgentFrame::JudgeStarted {
            task_id,
            judge_name,
        } => {
            assert_eq!(task_id, Some(tid));
            assert_eq!(judge_name, "Correctness");
        }
        _ => panic!("expected JudgeStarted, got {frame:?}"),
    }
    let json = format!(
        r#"{{"type":"judge_failed","task_id":"{tid}","judge_slug":"data","judge_name":"Data","status":"failed","error":"The judge could not complete its review."}}"#
    );
    let frame: PlayerAgentFrame = serde_json::from_str(&json).unwrap();
    match frame {
        PlayerAgentFrame::JudgeFailed {
            judge_name, error, ..
        } => {
            assert_eq!(judge_name, "Data");
            assert!(error.unwrap().contains("could not complete"));
        }
        _ => panic!("expected JudgeFailed, got {frame:?}"),
    }
    // A verdict names its task on current servers, and not on old ones.
    let json = format!(
        r#"{{"type":"judge_scored","task_id":"{tid}","judge_slug":"data","judge_name":"Data","rating":7.5,"feedback":"ok","point_delta":9,"created_at":"2026-01-01T00:00:00Z"}}"#
    );
    match serde_json::from_str::<PlayerAgentFrame>(&json).unwrap() {
        PlayerAgentFrame::JudgeScored { task_id, .. } => assert_eq!(task_id, Some(tid)),
        other => panic!("expected JudgeScored, got {other:?}"),
    }
    let json = r#"{"type":"judge_scored","judge_name":"Data","point_delta":9}"#;
    match serde_json::from_str::<PlayerAgentFrame>(json).unwrap() {
        PlayerAgentFrame::JudgeScored { task_id, .. } => assert_eq!(task_id, None),
        other => panic!("expected JudgeScored, got {other:?}"),
    }
}

// ── liveness ─────────────────────────────────────────────────────────

/// Short clocks for the tests: ping every 50 ms, dead after 400 ms of silence.
const QUICK: super::Liveness = super::Liveness {
    ping_every: std::time::Duration::from_millis(50),
    silence_limit: std::time::Duration::from_millis(400),
};

/// A one-connection WebSocket server running `serve` on the accepted socket.
async fn one_shot_server<F, Fut>(serve: F) -> String
where
    F: FnOnce(tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>) -> Fut + Send + 'static,
    Fut: std::future::Future<Output = ()> + Send + 'static,
{
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let ws = tokio_tungstenite::accept_async(stream).await.unwrap();
        serve(ws).await;
    });
    format!("ws://127.0.0.1:{port}/ws/player/agent/TEST")
}

const FINAL: &str =
    r#"{"type":"session_complete","session_id":"s","reason":"all_tasks_completed"}"#;

/// The incident on plum (2026-09-24): the socket stayed open, the server's
/// frames stopped arriving, and the agent waited 16 minutes for probes that
/// were being scored as no response. Silence now ends the connection so the
/// caller dials again.
#[tokio::test]
async fn a_connection_gone_silent_is_dropped_so_the_agent_redials() {
    let url = one_shot_server(|ws| async move {
        // Accept, then neither send nor read: a half-open path.
        tokio::time::sleep(std::time::Duration::from_secs(10)).await;
        drop(ws);
    })
    .await;
    let started = std::time::Instant::now();
    let outcome = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        super::connect_once(&url, "ololo_test_pat", None, None, None, QUICK),
    )
    .await
    .expect("the agent must give up on a silent connection");
    assert!(
        matches!(outcome, Ok(false)),
        "reconnect, not finish: {outcome:?}"
    );
    let waited = started.elapsed();
    assert!(
        waited >= QUICK.silence_limit,
        "not before the limit: {waited:?}"
    );
}

/// A quiet server that answers pings is alive: nothing is dropped before
/// the session's own end arrives.
#[tokio::test]
async fn a_quiet_server_that_answers_pings_keeps_the_connection() {
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::Message;
    let url = one_shot_server(|mut ws| async move {
        let until = tokio::time::Instant::now() + std::time::Duration::from_millis(1_200);
        loop {
            tokio::select! {
                _ = tokio::time::sleep_until(until) => break,
                m = ws.next() => match m {
                    Some(Ok(Message::Ping(data))) => {
                        let _ = ws.send(Message::Pong(data)).await;
                    }
                    Some(Ok(_)) => {}
                    _ => return,
                },
            }
        }
        let _ = ws.send(Message::Text(FINAL.into())).await;
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    })
    .await;
    let outcome = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        super::connect_once(&url, "ololo_test_pat", None, None, None, QUICK),
    )
    .await
    .expect("the session end arrives");
    assert!(
        matches!(outcome, Ok(true)),
        "three silence limits of pongs keep it: {outcome:?}"
    );
}

/// The countdown alone proves the server alive, whether or not it reads.
#[tokio::test]
async fn a_server_that_keeps_counting_down_keeps_the_connection() {
    use futures_util::SinkExt;
    use tokio_tungstenite::tungstenite::Message;
    let url = one_shot_server(|mut ws| async move {
        for left in (0..12).rev() {
            let tick = format!(
                r#"{{"type":"running_countdown","session_id":"s","seconds_remaining":{left},"version":1}}"#
            );
            if ws.send(Message::Text(tick)).await.is_err() {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        let _ = ws.send(Message::Text(FINAL.into())).await;
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    })
    .await;
    let outcome = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        super::connect_once(&url, "ololo_test_pat", None, None, None, QUICK),
    )
    .await
    .expect("the session end arrives");
    assert!(matches!(outcome, Ok(true)), "{outcome:?}");
}
