use arena_core::protocol::*;

#[test]
fn player_agent_frame_test_push_roundtrip_with_expected_answer() {
    let frame = PlayerAgentFrame::TestPush {
        probe_id: uuid::Uuid::new_v4(),
        rendered_command: "echo hello".to_string(),
        deadline_secs: 30,
        task_id: Some(uuid::Uuid::nil()),
        task_ordinal: 1,
        task_title: "Task A".to_string(),
        task_description: "Print hello".to_string(),
        test_ordinal: 1,
        test_total: 3,
        test_label: String::new(),
        test_description: String::new(),
        expected_answer: Some("hello".to_string()),
        answer_template: String::new(),
        validation_kind: ValidationKind::Minijinja,
    };
    let json = serde_json::to_string(&frame).unwrap();
    assert!(json.contains("\"expected_answer\":\"hello\""));
    assert!(
        json.contains("\"task_id\""),
        "task_id must serialize: {json}"
    );
    assert!(
        json.contains("\"task_ordinal\""),
        "task_ordinal must serialize: {json}"
    );
    let back: PlayerAgentFrame = serde_json::from_str(&json).unwrap();
    assert_eq!(frame, back);
}

#[test]
fn player_agent_frame_test_push_roundtrip_with_answer_template() {
    let frame = PlayerAgentFrame::TestPush {
        probe_id: uuid::Uuid::new_v4(),
        rendered_command: "curl http://x".to_string(),
        deadline_secs: 30,
        task_id: None,
        task_ordinal: 0,
        task_title: String::new(),
        task_description: String::new(),
        test_ordinal: 0,
        test_total: 0,
        test_label: String::new(),
        test_description: String::new(),
        expected_answer: None,
        answer_template: "result != \"\"".to_string(),
        validation_kind: ValidationKind::Minijinja,
    };
    let json = serde_json::to_string(&frame).unwrap();
    assert!(json.contains("\"answer_template\":\"result != \\\"\\\"\""));
    let back: PlayerAgentFrame = serde_json::from_str(&json).unwrap();
    assert_eq!(frame, back);
}

#[test]
fn player_agent_frame_test_push_roundtrip_with_js_validation() {
    let frame = PlayerAgentFrame::TestPush {
        probe_id: uuid::Uuid::new_v4(),
        rendered_command: "curl http://x".to_string(),
        deadline_secs: 30,
        task_id: None,
        task_ordinal: 0,
        task_title: String::new(),
        task_description: String::new(),
        test_ordinal: 0,
        test_total: 0,
        test_label: String::new(),
        test_description: String::new(),
        expected_answer: None,
        answer_template: "const code = Number(result.trim()); code >= 200 && code < 500"
            .to_string(),
        validation_kind: ValidationKind::Javascript,
    };
    let json = serde_json::to_string(&frame).unwrap();
    assert!(json.contains("\"validation_kind\":\"javascript\""));
    let back: PlayerAgentFrame = serde_json::from_str(&json).unwrap();
    assert_eq!(frame, back);
}

#[test]
fn player_agent_frame_test_push_back_compat_omits_new_fields() {
    // Old game-servers that don't send expected_answer/answer_template/
    // validation_kind must still deserialize: serde(default) on all new fields.
    let pid = uuid::Uuid::new_v4();
    let json = format!(
        r#"{{"type":"test_push","probe_id":"{pid}","rendered_command":"echo hi","deadline_secs":30,"task_title":"T","task_description":"D"}}"#
    );
    let frame: PlayerAgentFrame = serde_json::from_str(&json).unwrap();
    match frame {
        PlayerAgentFrame::TestPush {
            expected_answer,
            answer_template,
            validation_kind,
            ..
        } => {
            assert_eq!(expected_answer, None);
            assert_eq!(answer_template, String::new());
            assert_eq!(validation_kind, ValidationKind::Minijinja);
        }
        other => panic!("expected TestPush, got {other:?}"),
    }
}

#[test]
fn session_complete_player_tasks_completed_reason_roundtrip() {
    let frame = PlayerAgentFrame::SessionComplete {
        session_id: uuid::Uuid::new_v4(),
        reason: Some(SESSION_COMPLETE_REASON_PLAYER_TASKS_COMPLETED.to_string()),
    };
    let json = serde_json::to_string(&frame).unwrap();
    assert!(
        json.contains("\"type\":\"session_complete\""),
        "missing tag: {json}"
    );
    assert!(
        json.contains("\"reason\":\"player_tasks_completed\""),
        "missing reason: {json}"
    );
    let back: PlayerAgentFrame = serde_json::from_str(&json).unwrap();
    assert_eq!(frame, back);
}

#[test]
fn session_complete_player_tasks_completed_parses_with_old_client_shape() {
    // Replicates the pre-change ololo mirror enum shape
    // (`ololo/src/player_ws/wire.rs`): internally tagged, `session_id` as a
    // plain String, no knowledge of the new reason value. Fielded binaries
    // must still deserialize the per-player acknowledgment — they treat it
    // as a normal session end and exit (graceful degradation).
    #[derive(Debug, serde::Deserialize)]
    #[serde(tag = "type", rename_all = "snake_case")]
    enum OldClientFrame {
        SessionComplete {
            #[allow(dead_code)]
            session_id: String,
            #[serde(default)]
            reason: Option<String>,
        },
    }

    let frame = PlayerAgentFrame::SessionComplete {
        session_id: uuid::Uuid::new_v4(),
        reason: Some(SESSION_COMPLETE_REASON_PLAYER_TASKS_COMPLETED.to_string()),
    };
    let json = serde_json::to_string(&frame).unwrap();
    let old: OldClientFrame =
        serde_json::from_str(&json).expect("old client shape must still parse");
    let OldClientFrame::SessionComplete { reason, .. } = old;
    assert_eq!(reason.as_deref(), Some("player_tasks_completed"));
}

#[test]
fn validate_placeholder_name_accepts_alnum_underscore_64() {
    assert!(validate_placeholder_name("a").is_ok());
    assert!(validate_placeholder_name("Foo_Bar_123").is_ok());
    let max = "a".repeat(64);
    assert!(validate_placeholder_name(&max).is_ok());
}

#[test]
fn validate_placeholder_name_rejects_dash() {
    assert!(validate_placeholder_name("foo-bar").is_err());
}

#[test]
fn validate_placeholder_name_rejects_empty() {
    assert!(validate_placeholder_name("").is_err());
}

#[test]
fn validate_placeholder_name_rejects_65_chars() {
    let s = "a".repeat(65);
    assert!(validate_placeholder_name(&s).is_err());
}
