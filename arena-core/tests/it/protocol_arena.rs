use arena_core::protocol::*;
use arena_core::session_status::SessionStatus;

#[test]
fn arena_frame_test_push_roundtrip() {
    let frame = ArenaFrame::TestPush {
        task_id: TaskId(uuid::Uuid::new_v4()),
        player_id: PlayerId(uuid::Uuid::new_v4()),
        attempt: 2,
        rendered_command: "echo hello".to_string(),
        deadline_secs: 30,
    };
    let json = serde_json::to_string(&frame).unwrap();
    let back: ArenaFrame = serde_json::from_str(&json).unwrap();
    assert_eq!(frame, back);
}

#[test]
fn arena_frame_test_result_roundtrip() {
    let frame = ArenaFrame::TestResult {
        task_id: TaskId(uuid::Uuid::new_v4()),
        attempt: 3,
        pass: true,
        duration_ms: 1234,
        exit_code: 0,
        stdout_tail: "ok\n".to_string(),
    };
    let json = serde_json::to_string(&frame).unwrap();
    let back: ArenaFrame = serde_json::from_str(&json).unwrap();
    assert_eq!(frame, back);
}

#[test]
fn arena_frame_heartbeat_roundtrip() {
    let frame = ArenaFrame::Heartbeat;
    let json = serde_json::to_string(&frame).unwrap();
    assert_eq!(json, r#"{"type":"heartbeat"}"#);
    let back: ArenaFrame = serde_json::from_str(&json).unwrap();
    assert_eq!(frame, back);
}

#[test]
fn arena_frame_rejects_unknown_field() {
    let payload = serde_json::json!({
        "type": "test_push",
        "task_id": uuid::Uuid::new_v4(),
        "player_id": uuid::Uuid::new_v4(),
        "attempt": 1,
        "rendered_command": "echo hi",
        "deadline_secs": 30,
        "unexpected": "x"
    })
    .to_string();
    let res: Result<ArenaFrame, _> = serde_json::from_str(&payload);
    assert!(
        res.is_err(),
        "expected unknown-field rejection, got {res:?}"
    );
}

#[test]
fn player_handshake_roundtrip() {
    let msg = ClientMessage::PlayerHandshake {
        join_code: "ABCD12".to_string(),
        display_name: "TestBot".to_string(),
        fingerprint: Some("sha256:abc".to_string()),
        metadata_json: None,
    };
    let json = serde_json::to_string(&msg).unwrap();
    assert!(json.contains("\"type\":\"player_handshake\""));
    let back: ClientMessage = serde_json::from_str(&json).unwrap();
    assert_eq!(msg, back);
}

#[test]
fn player_handshake_rejects_kind_field() {
    let payload =
        r#"{"type":"player_handshake","join_code":"XY","display_name":"Bot","kind":"human"}"#;
    let res: Result<ClientMessage, _> = serde_json::from_str(payload);
    assert!(
        res.is_err(),
        "expected unknown-field rejection, got {res:?}"
    );
}

#[test]
fn lobby_countdown_roundtrip() {
    let frame = ArenaFrame::LobbyCountdown {
        session_id: uuid::Uuid::nil(),
        seconds_remaining: 45,
        version: 1,
    };
    let json = serde_json::to_string(&frame).unwrap();
    assert!(
        json.contains("\"type\":\"lobby_countdown\""),
        "missing type tag in: {json}"
    );
    let back: ArenaFrame = serde_json::from_str(&json).unwrap();
    assert_eq!(frame, back);
}

#[test]
fn session_started_roundtrip() {
    let frame = ArenaFrame::SessionStarted {
        session_id: uuid::Uuid::nil(),
        version: 2,
        total_tasks: Some(17),
    };
    let json = serde_json::to_string(&frame).unwrap();
    assert!(
        json.contains("\"type\":\"session_started\""),
        "missing type tag in: {json}"
    );
    let back: ArenaFrame = serde_json::from_str(&json).unwrap();
    assert_eq!(frame, back);
}

#[test]
fn session_started_back_compat_omits_total_tasks() {
    // Old game-servers that don't send total_tasks must still
    // deserialize: serde(default) on the new field.
    let json = r#"{"type":"session_started","session_id":"00000000-0000-0000-0000-000000000000","version":2}"#;
    let frame: ArenaFrame = serde_json::from_str(json).unwrap();
    match frame {
        ArenaFrame::SessionStarted { total_tasks, .. } => {
            assert_eq!(total_tasks, None);
        }
        _ => panic!("wrong variant"),
    }
}

#[test]
fn admin_adapted_tasks_snapshot_roundtrip() {
    let frame = ArenaFrame::AdminAdaptedTasksSnapshot {
        session_id: uuid::Uuid::nil(),
        players: vec![AdminAdaptedTaskView {
            player_id: uuid::Uuid::nil(),
            player_display_name: "Bot".to_string(),
            tasks: vec![AdminTaskEntry {
                task_id: uuid::Uuid::nil(),
                task_order: 1,
                title: "T".to_string(),
                adapted_content: "".to_string(),
                status: AdaptedTaskStatus::Pending,
                adaptation_attempts: 0,
            }],
        }],
    };
    let json = serde_json::to_string(&frame).unwrap();
    assert!(
        json.contains("\"type\":\"admin_adapted_tasks_snapshot\""),
        "missing type tag in: {json}"
    );
    let back: ArenaFrame = serde_json::from_str(&json).unwrap();
    assert_eq!(frame, back);
}

#[test]
fn admin_adapted_task_updated_roundtrip() {
    let player_id = uuid::Uuid::new_v4();
    let frame = ArenaFrame::AdminAdaptedTaskUpdated {
        session_id: uuid::Uuid::nil(),
        player_id,
        entry: AdminTaskEntry {
            task_id: uuid::Uuid::nil(),
            task_order: 1,
            title: "T".to_string(),
            adapted_content: "".to_string(),
            status: AdaptedTaskStatus::Ready,
            adaptation_attempts: 1,
        },
    };
    let json = serde_json::to_string(&frame).unwrap();
    assert!(
        json.contains("\"type\":\"admin_adapted_task_updated\""),
        "missing type tag in: {json}"
    );
    let back: ArenaFrame = serde_json::from_str(&json).unwrap();
    assert_eq!(frame, back);
}

#[test]
fn adapted_task_status_serde() {
    assert_eq!(
        serde_json::to_string(&AdaptedTaskStatus::Pending).unwrap(),
        "\"pending\""
    );
    assert_eq!(
        serde_json::to_string(&AdaptedTaskStatus::Ready).unwrap(),
        "\"ready\""
    );
    assert_eq!(
        serde_json::to_string(&AdaptedTaskStatus::Failed).unwrap(),
        "\"failed\""
    );
}

#[test]
fn project_session_update_paused_roundtrip() {
    let frame = ArenaFrame::ProjectSessionUpdate {
        session_id: uuid::Uuid::nil(),
        name: "Test Session".to_string(),
        status: SessionStatus::Paused,
        project_id: uuid::Uuid::nil(),
        join_code: Some("ABCD12".to_string()),
        created_at: chrono::Utc::now(),
        player_count: 2,
        cancel_reason: None,
        cancelled_by: None,
    };
    let json = serde_json::to_string(&frame).unwrap();
    assert!(
        json.contains("\"status\":\"paused\""),
        "expected paused status in: {json}"
    );
    let back: ArenaFrame = serde_json::from_str(&json).unwrap();
    assert_eq!(frame, back);
    match back {
        ArenaFrame::ProjectSessionUpdate { status, .. } => {
            assert_eq!(status, SessionStatus::Paused);
        }
        other => panic!("wrong variant: {other:?}"),
    }
}

#[test]
fn session_snapshot_paused_phase_roundtrip() {
    let payload = SessionSnapshotPayload {
        session_id: uuid::Uuid::nil(),
        phase: SessionStatus::Paused,
        version: 7,
        participants: vec![],
        leaderboard: vec![],
        started_at: None,
        timeline: None,
        activity: None,
        score_history: None,
    };
    let frame = ArenaFrame::SessionSnapshot(payload);
    let json = serde_json::to_string(&frame).unwrap();
    assert!(
        json.contains("\"phase\":\"paused\""),
        "expected paused phase in: {json}"
    );
    let back: ArenaFrame = serde_json::from_str(&json).unwrap();
    assert_eq!(frame, back);
}

#[test]
fn member_info_completion_status_roundtrip_and_absent() {
    let member = MemberInfo {
        user_id: "u-1".to_string(),
        player_id: None,
        display_name: "Alice".to_string(),
        joined_at: "2026-01-01T00:00:00Z".to_string(),
        avatar_url: None,
        fingerprint: None,
        username: None,
        agent_display_name: None,
        completion_status: Some(PlayerCompletionStatus::Completed),
    };
    let json = serde_json::to_string(&member).unwrap();
    assert!(
        json.contains("\"completion_status\":\"completed\""),
        "missing completion_status: {json}"
    );
    let back: MemberInfo = serde_json::from_str(&json).unwrap();
    assert_eq!(member, back);

    // None must be skipped on the wire (list paths don't compute it).
    let none_member = MemberInfo {
        completion_status: None,
        ..member
    };
    let json = serde_json::to_string(&none_member).unwrap();
    assert!(
        !json.contains("completion_status"),
        "None must be skipped: {json}"
    );

    // Frames from servers that predate the field must deserialize to None.
    let old = r#"{
        "user_id": "u-1",
        "display_name": "Alice",
        "joined_at": "2026-01-01T00:00:00Z",
        "avatar_url": null,
        "fingerprint": null,
        "username": null
    }"#;
    let back: MemberInfo = serde_json::from_str(old).unwrap();
    assert_eq!(back.completion_status, None);
}

#[test]
fn score_history_sample_roundtrip() {
    let sample = ScoreHistorySample {
        t: 5.0,
        scores: std::collections::BTreeMap::from([(PlayerId(uuid::Uuid::nil()), 10)]),
    };
    let json = serde_json::to_string(&sample).unwrap();
    assert!(
        json.contains("\"scores\""),
        "expected scores key in: {json}"
    );
    assert!(json.contains("\"t\""), "expected t key in: {json}");
    let back: ScoreHistorySample = serde_json::from_str(&json).unwrap();
    assert_eq!(sample, back);

    let with_unknown = r#"{"t":5.0,"scores":{},"bogus":true}"#;
    let err = serde_json::from_str::<ScoreHistorySample>(with_unknown);
    assert!(err.is_err(), "expected deny_unknown_fields to reject bogus");
}

#[test]
fn session_snapshot_score_history_roundtrip() {
    let player_a = PlayerId(uuid::Uuid::nil());
    let player_b = PlayerId(uuid::Uuid::new_v4());

    let samples = vec![
        ScoreHistorySample {
            t: 0.0,
            scores: std::collections::BTreeMap::from([(player_a, 5), (player_b, 0)]),
        },
        ScoreHistorySample {
            t: 10.0,
            scores: std::collections::BTreeMap::from([(player_a, 5), (player_b, 3)]),
        },
        ScoreHistorySample {
            t: 20.0,
            scores: std::collections::BTreeMap::from([(player_a, 8), (player_b, 3)]),
        },
    ];

    let payload = SessionSnapshotPayload {
        session_id: uuid::Uuid::nil(),
        phase: SessionStatus::Paused,
        version: 9,
        participants: vec![],
        leaderboard: vec![],
        started_at: None,
        timeline: None,
        activity: None,
        score_history: Some(samples.clone()),
    };
    let frame = ArenaFrame::SessionSnapshot(payload);
    let json = serde_json::to_string(&frame).unwrap();
    assert!(
        json.contains("\"score_history\""),
        "expected score_history key in: {json}"
    );

    let back: ArenaFrame = serde_json::from_str(&json).unwrap();
    assert_eq!(frame, back);

    let back_samples = match back {
        ArenaFrame::SessionSnapshot(p) => p
            .score_history
            .expect("score_history must be Some after roundtrip"),
        other => panic!("wrong variant: {other:?}"),
    };
    assert_eq!(back_samples.len(), 3, "expected 3 samples");

    // BTreeMap iterates keys in sorted order; verify that invariant holds.
    let first_keys: Vec<uuid::Uuid> = back_samples[0].scores.keys().map(|p| p.0).collect();
    let mut sorted = first_keys.clone();
    sorted.sort();
    assert_eq!(
        first_keys, sorted,
        "BTreeMap keys must be in sorted (non-decreasing) order"
    );

    // Verify t values preserved and ordered 0.0, 10.0, 20.0.
    let ts: Vec<f64> = back_samples.iter().map(|s| s.t).collect();
    assert_eq!(
        ts,
        vec![0.0, 10.0, 20.0],
        "t values must be preserved & ordered"
    );

    // Negative: a ScoreHistorySample with an extra field must be rejected.
    let player_a_str = player_a.0.to_string();
    let player_b_str = player_b.0.to_string();
    let bad = format!(
        r#"{{"type":"session_snapshot","session_id":"00000000-0000-0000-0000-000000000000","phase":"paused","version":9,"participants":[],"leaderboard":[],"score_history":[{{"t":0.0,"scores":{{"{player_a_str}":5,"{player_b_str}":0}},"extra":1}}]}}"#
    );
    let res: Result<ArenaFrame, _> = serde_json::from_str(&bad);
    assert!(
        res.is_err(),
        "expected deny_unknown_fields to reject extra field in ScoreHistorySample, got {res:?}"
    );
}

#[test]
fn cli_auth_challenge_roundtrip() {
    let frame = CliAuthFrame::CliAuthChallenge {
        cli_token: "abc123".to_string(),
    };
    let json = serde_json::to_string(&frame).unwrap();
    assert!(json.contains("\"type\":\"cli_auth_challenge\""));
    let back: CliAuthFrame = serde_json::from_str(&json).unwrap();
    assert_eq!(frame, back);
}

#[test]
fn cli_auth_success_roundtrip() {
    let frame = CliAuthFrame::CliAuthSuccess {
        token: "ololo_deadbeef".to_string(),
    };
    let json = serde_json::to_string(&frame).unwrap();
    assert!(json.contains("\"type\":\"cli_auth_success\""));
    let back: CliAuthFrame = serde_json::from_str(&json).unwrap();
    assert_eq!(frame, back);
}

#[test]
fn cli_auth_error_roundtrip() {
    let frame = CliAuthFrame::CliAuthError {
        code: "session_expired".to_string(),
        message: "The session has expired.".to_string(),
    };
    let json = serde_json::to_string(&frame).unwrap();
    assert!(json.contains("\"type\":\"cli_auth_error\""));
    let back: CliAuthFrame = serde_json::from_str(&json).unwrap();
    assert_eq!(frame, back);
}

#[test]
fn arena_frame_untouched() {
    let json = r#"{"type":"heartbeat"}"#;
    let _frame: ArenaFrame = serde_json::from_str(json).unwrap();
}

#[test]
fn task_started_roundtrip() {
    let frame = ArenaFrame::TaskStarted {
        player_id: uuid::Uuid::new_v4(),
        player_display_name: "Alpha".to_string(),
        task_id: uuid::Uuid::new_v4(),
        task_ordinal: 3,
        task_title: "Build API".to_string(),
        timestamp: chrono::Utc::now(),
        version: 7,
    };
    let json = serde_json::to_string(&frame).unwrap();
    assert!(
        json.contains("\"type\":\"task_started\""),
        "missing type tag in: {json}"
    );
    let back: ArenaFrame = serde_json::from_str(&json).unwrap();
    assert_eq!(frame, back);
}

#[test]
fn task_started_rejects_unknown_field() {
    let v = serde_json::json!({
        "type": "task_started",
        "player_id": uuid::Uuid::nil(),
        "player_display_name": "Alpha",
        "task_id": uuid::Uuid::nil(),
        "task_ordinal": 1,
        "task_title": "T",
        "timestamp": "2026-07-16T00:00:00Z",
        "version": 1,
        "unexpected": "x"
    })
    .to_string();
    let res: Result<ArenaFrame, _> = serde_json::from_str(&v);
    assert!(
        res.is_err(),
        "expected unknown-field rejection, got {res:?}"
    );
}

#[test]
fn task_scored_roundtrip() {
    let frame = ArenaFrame::TaskScored {
        player_id: uuid::Uuid::new_v4(),
        player_display_name: "Beta".to_string(),
        task_id: uuid::Uuid::new_v4(),
        task_ordinal: 5,
        task_title: "Write tests".to_string(),
        point_delta: 10,
        judge_name: "Code Quality".to_string(),
        timestamp: chrono::Utc::now(),
        version: 12,
        detail: None,
    };
    let json = serde_json::to_string(&frame).unwrap();
    assert!(
        json.contains("\"type\":\"task_scored\""),
        "missing type tag in: {json}"
    );
    let back: ArenaFrame = serde_json::from_str(&json).unwrap();
    assert_eq!(frame, back);
}

#[test]
fn task_scored_rejects_unknown_field() {
    let v = serde_json::json!({
        "type": "task_scored",
        "player_id": uuid::Uuid::nil(),
        "player_display_name": "Beta",
        "task_id": uuid::Uuid::nil(),
        "task_ordinal": 2,
        "task_title": "T",
        "point_delta": 5,
        "judge_name": "Judge",
        "timestamp": "2026-07-16T00:00:00Z",
        "version": 1,
        "unexpected": "x"
    })
    .to_string();
    let res: Result<ArenaFrame, _> = serde_json::from_str(&v);
    assert!(
        res.is_err(),
        "expected unknown-field rejection, got {res:?}"
    );
}

#[test]
fn zmq_event_judge_scored_roundtrip() {
    let event = ZmqEvent::JudgeScored {
        join_code: "ABCD12".to_string(),
        player_id: uuid::Uuid::new_v4(),
        player_display_name: "Gamma".to_string(),
        task_id: uuid::Uuid::new_v4(),
        task_ordinal: 4,
        task_title: "Refactor".to_string(),
        point_delta: 7,
        judge_slug: "ai-judge-qwen3".to_string(),
        judge_name: "AI Judge qwen3".to_string(),
        rating: 7.0,
        feedback: "Well structured.".to_string(),
        duration_ms: Some(4200),
        timestamp: chrono::Utc::now(),
        version: 3,
        detail: None,
    };
    let json = serde_json::to_string(&event).unwrap();
    assert!(
        json.contains("\"type\":\"judge_scored\""),
        "missing type tag in: {json}"
    );
    let back: ZmqEvent = serde_json::from_str(&json).unwrap();
    assert_eq!(event, back);
}

#[test]
fn zmq_event_judge_scored_join_code_accessor() {
    let event = ZmqEvent::JudgeScored {
        join_code: "WXYZ99".to_string(),
        player_id: uuid::Uuid::nil(),
        player_display_name: "Bot".to_string(),
        task_id: uuid::Uuid::nil(),
        task_ordinal: 1,
        task_title: "T".to_string(),
        point_delta: 1,
        judge_slug: "j".to_string(),
        judge_name: "J".to_string(),
        rating: 1.0,
        feedback: String::new(),
        duration_ms: None,
        timestamp: chrono::Utc::now(),
        version: 1,
        detail: None,
    };
    assert_eq!(event.join_code(), "WXYZ99");
}
