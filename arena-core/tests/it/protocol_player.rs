use arena_core::protocol::*;
use arena_core::session_status::SessionStatus;

#[test]
fn player_error_round_trips() {
    let frame = PlayerFrame::PlayerError {
        seq: 1,
        message: "oops".to_string(),
    };
    let json = serde_json::to_string(&frame).unwrap();
    let parsed: PlayerFrame = serde_json::from_str(&json).unwrap();
    match parsed {
        PlayerFrame::PlayerError { seq, message } => {
            assert_eq!(seq, 1);
            assert_eq!(message, "oops");
        }
        _ => panic!("wrong variant"),
    }
}

#[test]
fn player_snapshot_round_trips() {
    let payload = PlayerSnapshotPayload {
        player_id: uuid::Uuid::nil(),
        display_name: "Alice".to_string(),
        avatar_url: None,
        agent_display_name: None,
        score: 42,
        rank: 1,
        last_seq: 0,
        probes: vec![],
        tasks: vec![],
        total_tasks: 10,
        next_probe_at: None,
        session_started_at: None,
        session_ends_at: None,
        session_status: SessionStatus::Running,
        agent_connected: None,
        completion_status: None,
        judge_results: vec![],
        judge_statuses: vec![],
        session_report: None,
        evaluations: Vec::new(),
        similarity_adjustment: None,
    };
    let frame = PlayerFrame::PlayerSnapshot(payload);
    let json = serde_json::to_string(&frame).unwrap();
    assert!(json.contains("\"type\":\"player_snapshot\""));
    let _parsed: PlayerFrame = serde_json::from_str(&json).unwrap();
}

#[test]
fn player_snapshot_data_converts() {
    let data = PlayerSnapshotData {
        player_id: uuid::Uuid::nil(),
        display_name: "Bob".to_string(),
        avatar_url: None,
        agent_display_name: None,
        score: 0,
        rank: 2,
        last_seq: 5,
        probes: vec![],
        tasks: vec![],
        total_tasks: 8,
        next_probe_at: None,
        session_started_at: None,
        session_ends_at: None,
        session_status: SessionStatus::Running,
        agent_connected: None,
        completion_status: None,
        judge_results: vec![],
        judge_statuses: vec![],
        session_report: None,
        evaluations: Vec::new(),
        similarity_adjustment: None,
    };
    let payload: PlayerSnapshotPayload = data.into();
    assert_eq!(payload.display_name, "Bob");
    assert_eq!(payload.last_seq, 5);
    assert_eq!(payload.total_tasks, 8);
}

#[test]
fn player_completion_status_serde_values() {
    for (status, wire) in [
        (PlayerCompletionStatus::InProgress, "\"in_progress\""),
        (
            PlayerCompletionStatus::AwaitingJudges,
            "\"awaiting_judges\"",
        ),
        (PlayerCompletionStatus::Completed, "\"completed\""),
    ] {
        let json = serde_json::to_string(&status).unwrap();
        assert_eq!(json, wire);
        let back: PlayerCompletionStatus = serde_json::from_str(&json).unwrap();
        assert_eq!(status, back);
    }
}

#[test]
fn player_snapshot_completion_status_roundtrip() {
    let payload = PlayerSnapshotPayload {
        player_id: uuid::Uuid::nil(),
        display_name: "Carol".to_string(),
        avatar_url: None,
        agent_display_name: None,
        score: 7,
        rank: 2,
        last_seq: 3,
        probes: vec![],
        tasks: vec![],
        total_tasks: 4,
        next_probe_at: None,
        session_started_at: None,
        session_ends_at: None,
        session_status: SessionStatus::Running,
        agent_connected: None,
        completion_status: Some(PlayerCompletionStatus::AwaitingJudges),
        judge_results: vec![],
        judge_statuses: vec![],
        session_report: None,
        evaluations: Vec::new(),
        similarity_adjustment: None,
    };
    let frame = PlayerFrame::PlayerSnapshot(payload);
    let json = serde_json::to_string(&frame).unwrap();
    assert!(
        json.contains("\"completion_status\":\"awaiting_judges\""),
        "missing completion_status: {json}"
    );
    let back: PlayerFrame = serde_json::from_str(&json).unwrap();
    assert_eq!(frame, back);
}

#[test]
fn player_snapshot_completion_status_none_skipped_and_absent_parses() {
    let payload = PlayerSnapshotPayload {
        player_id: uuid::Uuid::nil(),
        display_name: "Dave".to_string(),
        avatar_url: None,
        agent_display_name: None,
        score: 0,
        rank: 1,
        last_seq: 0,
        probes: vec![],
        tasks: vec![],
        total_tasks: 1,
        next_probe_at: None,
        session_started_at: None,
        session_ends_at: None,
        session_status: SessionStatus::Running,
        agent_connected: None,
        completion_status: None,
        judge_results: vec![],
        judge_statuses: vec![],
        session_report: None,
        evaluations: Vec::new(),
        similarity_adjustment: None,
    };
    // None means "not computed" and must not appear on the wire.
    let json = serde_json::to_string(&payload).unwrap();
    assert!(
        !json.contains("completion_status"),
        "None must be skipped: {json}"
    );

    // Payloads from servers that predate the field must deserialize to None.
    let old = r#"{
        "player_id": "00000000-0000-0000-0000-000000000000",
        "display_name": "Dave",
        "score": 0,
        "rank": 1,
        "last_seq": 0,
        "probes": [],
        "tasks": [],
        "total_tasks": 1,
        "next_probe_at": null,
        "session_started_at": null,
        "session_ends_at": null,
        "session_status": "running"
    }"#;
    let back: PlayerSnapshotPayload = serde_json::from_str(old).unwrap();
    assert_eq!(back.completion_status, None);
}

#[test]
fn probe_state_dispatched_roundtrip() {
    let state = ProbeState::Dispatched;
    let json = serde_json::to_string(&state).unwrap();
    assert_eq!(json, "\"dispatched\"");
    let back: ProbeState = serde_json::from_str(&json).unwrap();
    assert_eq!(state, back);
}

#[test]
fn probe_state_resolved_roundtrip() {
    let state = ProbeState::Resolved;
    let json = serde_json::to_string(&state).unwrap();
    assert_eq!(json, "\"resolved\"");
    let back: ProbeState = serde_json::from_str(&json).unwrap();
    assert_eq!(state, back);
}

#[test]
fn test_display_label_filters_legacy_placeholders() {
    use arena_core::protocol::test_display_label;
    assert_eq!(test_display_label(""), None);
    assert_eq!(test_display_label("   "), None);
    assert_eq!(test_display_label("Structured markdown test 2"), None);
    assert_eq!(test_display_label("Passthrough test 0"), None);
    assert_eq!(
        test_display_label("Answer the addition question with the correct sum"),
        Some("Answer the addition question with the correct sum"),
    );
    assert_eq!(
        test_display_label("  Definition of done \n"),
        Some("Definition of done")
    );
}

#[test]
fn probe_updated_dispatched_roundtrip() {
    let payload = ProbeUpdatedPayload {
        seq: 1,
        probe_id: uuid::Uuid::new_v4(),
        task_id: uuid::Uuid::new_v4(),
        task_title: "Task A".to_string(),
        task_ordinal: 0,
        adapted_test_id: uuid::Uuid::new_v4(),
        test_ordinal: Some(0),
        label: None,
        description: None,
        test_command: "run test".to_string(),
        attempt: 1,
        rendered_command: "echo hello".to_string(),
        fixture_values: Some("key=val".to_string()),
        expected_answer: Some("42".to_string()),
        state: ProbeState::Dispatched,
        outcome: None,
        actual: None,
        expected: None,
        exit_code: None,
        duration_ms: None,
        dispatched_at: Some(chrono::Utc::now()),
        deadline_at: Some(chrono::Utc::now()),
        resolved_at: None,
        point_delta: 0,
        score: 100,
        rank: 1,
        updated_at: chrono::Utc::now(),
        next_probe_at: None,
    };
    let frame = PlayerFrame::ProbeUpdated(payload);
    let json = serde_json::to_string(&frame).unwrap();
    assert!(json.contains("\"type\":\"probe_updated\""));
    let back: PlayerFrame = serde_json::from_str(&json).unwrap();
    assert_eq!(frame, back);
}

#[test]
fn probe_updated_resolved_roundtrip() {
    let payload = ProbeUpdatedPayload {
        seq: 2,
        probe_id: uuid::Uuid::new_v4(),
        task_id: uuid::Uuid::new_v4(),
        task_title: "Task B".to_string(),
        task_ordinal: 1,
        adapted_test_id: uuid::Uuid::new_v4(),
        test_ordinal: Some(0),
        label: Some("Answer the addition question".to_string()),
        description: None,
        test_command: "run test b".to_string(),
        attempt: 2,
        rendered_command: "echo world".to_string(),
        fixture_values: None,
        expected_answer: None,
        state: ProbeState::Resolved,
        outcome: Some("pass".to_string()),
        actual: Some("42".to_string()),
        expected: Some("42".to_string()),
        exit_code: Some(0),
        duration_ms: Some(150),
        dispatched_at: None,
        deadline_at: None,
        resolved_at: Some(chrono::Utc::now()),
        point_delta: 10,
        score: 110,
        rank: 1,
        updated_at: chrono::Utc::now(),
        next_probe_at: Some(chrono::Utc::now()),
    };
    let frame = PlayerFrame::ProbeUpdated(payload);
    let json = serde_json::to_string(&frame).unwrap();
    assert!(json.contains("\"type\":\"probe_updated\""));
    let back: PlayerFrame = serde_json::from_str(&json).unwrap();
    assert_eq!(frame, back);
}

#[test]
fn probe_updated_rejects_unknown_field() {
    let payload = serde_json::json!({
        "type": "probe_updated",
        "seq": 1,
        "probe_id": uuid::Uuid::new_v4(),
        "task_id": uuid::Uuid::new_v4(),
        "task_title": "T",
        "adapted_test_id": uuid::Uuid::new_v4(),
        "attempt": 1,
        "rendered_command": "echo hi",
        "fixture_values": null,
        "expected_answer": null,
        "state": "dispatched",
        "outcome": null,
        "actual": null,
        "expected": null,
        "exit_code": null,
        "duration_ms": null,
        "dispatched_at": null,
        "deadline_at": null,
        "resolved_at": null,
        "point_delta": 0,
        "score": 0,
        "rank": 1,
        "updated_at": "2026-01-01T00:00:00Z",
        "unexpected": "x"
    })
    .to_string();
    let res: Result<PlayerFrame, _> = serde_json::from_str(&payload);
    assert!(
        res.is_err(),
        "expected unknown-field rejection, got {res:?}"
    );
}

#[test]
fn task_revealed_roundtrip() {
    let payload = TaskRevealedPayload {
        seq: 3,
        task: PlayerTaskSummaryEntry {
            task_id: uuid::Uuid::new_v4(),
            ordinal: 2,
            title: "Sort array".to_string(),
            content: "Sort the array".to_string(),
            tags: vec!["sorting".to_string()],
            adapted_content: "Sort the given array".to_string(),
            result: None,
            scheduler_state: None,
            total_points: 0,
            bonus_points: 0,
        },
        total_tasks: 5,
    };
    let frame = PlayerFrame::TaskRevealed(payload.clone());
    let json = serde_json::to_string(&frame).unwrap();
    assert!(json.contains("\"type\":\"task_revealed\""));
    let back: PlayerFrame = serde_json::from_str(&json).unwrap();
    assert_eq!(frame, back);
}

#[test]
fn task_revealed_rejects_unknown_field() {
    let payload = serde_json::json!({
        "type": "task_revealed",
        "seq": 1,
        "task": {
            "task_id": uuid::Uuid::new_v4(),
            "ordinal": 0,
            "title": "T",
            "content": "C",
            "tags": [],
            "adapted_content": "A",
            "result": null,
            "scheduler_state": null
        },
        "total_tasks": 3,
        "unexpected": "x"
    })
    .to_string();
    let res: Result<PlayerFrame, _> = serde_json::from_str(&payload);
    assert!(
        res.is_err(),
        "expected unknown-field rejection, got {res:?}"
    );
}

#[test]
fn score_rank_updated_roundtrip() {
    let payload = ScoreRankUpdatedPayload {
        seq: 7,
        score: 150,
        rank: 3,
    };
    let frame = PlayerFrame::ScoreRankUpdated(payload);
    let json = serde_json::to_string(&frame).unwrap();
    assert!(json.contains("\"type\":\"score_rank_updated\""));
    let back: PlayerFrame = serde_json::from_str(&json).unwrap();
    assert_eq!(frame, back);
}

#[test]
fn score_rank_updated_rejects_unknown_field() {
    let payload = serde_json::json!({
        "type": "score_rank_updated",
        "seq": 1,
        "score": 0,
        "rank": 1,
        "unexpected": "x"
    })
    .to_string();
    let res: Result<PlayerFrame, _> = serde_json::from_str(&payload);
    assert!(
        res.is_err(),
        "expected unknown-field rejection, got {res:?}"
    );
}

#[test]
fn player_snapshot_total_tasks_roundtrip() {
    let payload = PlayerSnapshotPayload {
        player_id: uuid::Uuid::nil(),
        display_name: "Eve".to_string(),
        avatar_url: None,
        agent_display_name: None,
        score: 10,
        rank: 5,
        last_seq: 2,
        probes: vec![],
        tasks: vec![],
        total_tasks: 20,
        next_probe_at: None,
        session_started_at: None,
        session_ends_at: None,
        session_status: SessionStatus::Running,
        agent_connected: None,
        completion_status: None,
        judge_results: vec![],
        judge_statuses: vec![],
        session_report: None,
        evaluations: Vec::new(),
        similarity_adjustment: None,
    };
    let frame = PlayerFrame::PlayerSnapshot(payload);
    let json = serde_json::to_string(&frame).unwrap();
    assert!(json.contains("\"total_tasks\":20"));
    let back: PlayerFrame = serde_json::from_str(&json).unwrap();
    assert_eq!(frame, back);
}

#[test]
fn score_rank_updated_payload_no_feedback_leak() {
    let payload = ScoreRankUpdatedPayload {
        seq: 9,
        score: 200,
        rank: 2,
    };
    let frame = PlayerFrame::ScoreRankUpdated(payload);
    let json = serde_json::to_string(&frame).unwrap();
    let v: serde_json::Value = serde_json::from_str(&json).unwrap();
    let body = &v;
    assert_eq!(body["seq"], 9);
    assert_eq!(body["score"], 200);
    assert_eq!(body["rank"], 2);
    assert!(body.get("feedback").is_none(), "feedback leaked: {json}");
    assert!(
        body.get("raw_output").is_none(),
        "raw_output leaked: {json}"
    );
}

#[test]
fn player_agent_judge_scored_roundtrip() {
    let payload = JudgeScoredPayload {
        task_id: uuid::Uuid::nil(),
        judge_slug: "style".to_string(),
        judge_name: "Style".to_string(),
        rating: 0.9,
        feedback: "clean".to_string(),
        point_delta: 3,
        created_at: chrono::Utc::now(),
    };
    let frame = PlayerAgentFrame::JudgeScored(payload.clone());
    let json = serde_json::to_string(&frame).unwrap();
    assert!(
        json.contains("\"type\":\"judge_scored\""),
        "missing tag: {json}"
    );
    let back: PlayerAgentFrame = serde_json::from_str(&json).unwrap();
    assert_eq!(frame, back);
}

#[test]
fn player_agent_judge_scored_rejects_unknown_field() {
    let v = serde_json::json!({
        "type": "judge_scored",
        "task_id": uuid::Uuid::nil(),
        "judge_slug": "s",
        "judge_name": "n",
        "rating": 0.5,
        "feedback": "",
        "point_delta": 1,
        "created_at": "2026-01-01T00:00:00Z",
        "unexpected": "x"
    })
    .to_string();
    let res: Result<PlayerAgentFrame, _> = serde_json::from_str(&v);
    assert!(
        res.is_err(),
        "expected unknown-field rejection, got {res:?}"
    );
}

#[test]
fn player_frame_judge_scored_roundtrip() {
    let payload = PlayerJudgeScoredPayload {
        task_id: uuid::Uuid::nil(),
        judge_slug: "style".to_string(),
        judge_name: "Style".to_string(),
        rating: 0.7,
        feedback: "ok".to_string(),
        point_delta: 2,
        created_at: chrono::Utc::now(),
        duration_ms: Some(3100),
    };
    let frame = PlayerFrame::JudgeScored(payload.clone());
    let json = serde_json::to_string(&frame).unwrap();
    assert!(
        json.contains("\"type\":\"judge_scored\""),
        "missing tag: {json}"
    );
    let back: PlayerFrame = serde_json::from_str(&json).unwrap();
    assert_eq!(frame, back);
}

#[test]
fn player_frame_judge_scored_rejects_unknown_field() {
    let v = serde_json::json!({
        "type": "judge_scored",
        "task_id": uuid::Uuid::nil(),
        "judge_slug": "s",
        "judge_name": "n",
        "rating": 0.5,
        "feedback": "",
        "point_delta": 1,
        "created_at": "2026-01-01T00:00:00Z",
        "unexpected": "x"
    })
    .to_string();
    let res: Result<PlayerFrame, _> = serde_json::from_str(&v);
    assert!(
        res.is_err(),
        "expected unknown-field rejection, got {res:?}"
    );
}

#[test]
fn the_report_cue_is_a_bare_tagged_frame() {
    // The page keys on the tag alone — the report itself comes from the
    // snapshot — so the wire shape is the whole contract.
    let json = serde_json::to_string(&PlayerFrame::SessionReportReady).unwrap();
    assert_eq!(json, r#"{"type":"session_report_ready"}"#);
    let parsed: PlayerFrame = serde_json::from_str(&json).unwrap();
    assert!(matches!(parsed, PlayerFrame::SessionReportReady));
}
