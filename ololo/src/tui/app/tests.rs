use super::*;
use crate::tui::event::{HeaderDelta, ProbeInfo, ValidationKind};
use crate::tui::header::HeaderState;
use std::sync::atomic::AtomicU64;
use uuid::Uuid;

fn fresh_app() -> TuiApp {
    let header = HeaderState::new("s", "p");
    let parser = Parser::new(24, 80, 0);
    TuiApp::new_for_test(header, parser, Arc::new(AtomicU64::new(0)), 80, 24)
}

#[test]
fn f10_in_tui_focus_sets_user_requested_quit() {
    let mut app = fresh_app();
    app.on_key(
        crossterm::event::KeyCode::F(10),
        crossterm::event::KeyModifiers::NONE,
    );
    assert_eq!(app.should_quit, Some(QuitReason::UserRequested));
}

#[test]
fn f10_in_pty_focus_does_not_quit() {
    let mut app = fresh_app();
    app.input_focus = InputFocus::Pty;
    app.on_key(
        crossterm::event::KeyCode::F(10),
        crossterm::event::KeyModifiers::NONE,
    );
    assert_eq!(app.should_quit, None);
}

#[test]
fn f9_toggles_focus() {
    let mut app = fresh_app();
    assert_eq!(app.input_focus, InputFocus::Tui);
    app.on_key(
        crossterm::event::KeyCode::F(9),
        crossterm::event::KeyModifiers::NONE,
    );
    assert_eq!(app.input_focus, InputFocus::Pty);
    app.on_key(
        crossterm::event::KeyCode::F(9),
        crossterm::event::KeyModifiers::NONE,
    );
    assert_eq!(app.input_focus, InputFocus::Tui);
}

#[test]
fn tab_no_longer_toggles_focus() {
    // Regression guard: Tab must reach the embedded agent (see
    // key_to_pty_bytes + run()'s dispatch order), not get hijacked as
    // a focus toggle the way it used to be.
    let mut app = fresh_app();
    app.input_focus = InputFocus::Pty;
    app.on_key(
        crossterm::event::KeyCode::Tab,
        crossterm::event::KeyModifiers::NONE,
    );
    assert_eq!(app.input_focus, InputFocus::Pty);
}

#[test]
fn ctrl_c_in_pty_focus_pushes_intent_marker() {
    let mut app = fresh_app();
    app.input_focus = InputFocus::Pty;
    app.on_key(
        crossterm::event::KeyCode::Char('c'),
        crossterm::event::KeyModifiers::CONTROL,
    );
    let last = app.probes.back().unwrap();
    assert!(last.stdout.contains("ctrl-c"));
}

#[test]
fn ctrl_c_in_tui_focus_does_nothing() {
    let mut app = fresh_app();
    app.on_key(
        crossterm::event::KeyCode::Char('c'),
        crossterm::event::KeyModifiers::CONTROL,
    );
    assert!(app.probes.is_empty());
}

/// Arrival of a probe for `(task_id, ordinal)` with the given title.
fn arrive(app: &mut TuiApp, task_id: Uuid, ordinal: i32, title: &str) -> Uuid {
    let probe_id = Uuid::new_v4();
    app.on_event(TuiEvent::ProbeArrived(ProbeInfo {
        probe_id,
        rendered_command: "echo hi".to_string(),
        deadline_secs: 30,
        task_id: Some(task_id),
        task_ordinal: ordinal,
        task_title: title.to_string(),
        task_description: "desc".to_string(),
        test_ordinal: 1,
        test_total: 1,
        test_label: String::new(),
        test_description: String::new(),
        expected_answer: None,
        answer_template: String::new(),
        validation_kind: ValidationKind::Minijinja,
    }));
    probe_id
}

fn press(app: &mut TuiApp, code: crossterm::event::KeyCode) {
    app.on_key(code, crossterm::event::KeyModifiers::NONE);
}

#[test]
fn sidebar_down_selects_first_task_then_probe() {
    let mut app = fresh_app();
    app.sidebar_view = SidebarView::Probes;
    let tid = Uuid::new_v4();
    let pid = arrive(&mut app, tid, 0, "Setup");
    press(&mut app, crossterm::event::KeyCode::Down);
    assert_eq!(app.sidebar_cursor, Some(NavTarget::Task(tid)));
    press(&mut app, crossterm::event::KeyCode::Down);
    assert_eq!(app.sidebar_cursor, Some(NavTarget::Probe(pid)));
    // Clamped at the last row.
    press(&mut app, crossterm::event::KeyCode::Down);
    assert_eq!(app.sidebar_cursor, Some(NavTarget::Probe(pid)));
    press(&mut app, crossterm::event::KeyCode::Up);
    assert_eq!(app.sidebar_cursor, Some(NavTarget::Task(tid)));
}

#[test]
fn enter_on_task_toggles_fold() {
    let mut app = fresh_app();
    app.sidebar_view = SidebarView::Probes;
    let tid = Uuid::new_v4();
    arrive(&mut app, tid, 0, "Setup");
    press(&mut app, crossterm::event::KeyCode::Down); // select task
    assert!(!app.task_groups()[0].folded, "active task starts unfolded");
    press(&mut app, crossterm::event::KeyCode::Enter);
    assert!(app.task_groups()[0].folded, "Enter folds the task");
    press(&mut app, crossterm::event::KeyCode::Enter);
    assert!(!app.task_groups()[0].folded, "Enter again unfolds");
}

#[test]
fn unfold_override_survives_task_passing() {
    // Manually unfolded task stays open even after the scheduler
    // advances past it (auto-fold is only the default).
    let mut app = fresh_app();
    app.sidebar_view = SidebarView::Probes;
    let tid = Uuid::new_v4();
    arrive(&mut app, tid, 0, "Setup");
    press(&mut app, crossterm::event::KeyCode::Down);
    press(&mut app, crossterm::event::KeyCode::Enter); // fold
    press(&mut app, crossterm::event::KeyCode::Enter); // unfold (override=false)
    arrive(&mut app, Uuid::new_v4(), 1, "Next"); // task 0 passes
    let groups = app.task_groups();
    let setup = groups.iter().find(|g| g.task_id == tid).unwrap();
    assert!(setup.passed, "task 0 passed once task 1 arrived");
    assert!(!setup.folded, "manual unfold overrides auto-fold");
}

#[test]
fn enter_on_probe_opens_popup_and_esc_closes() {
    let mut app = fresh_app();
    app.sidebar_view = SidebarView::Probes;
    let tid = Uuid::new_v4();
    let pid = arrive(&mut app, tid, 0, "Setup");
    press(&mut app, crossterm::event::KeyCode::Down); // task
    press(&mut app, crossterm::event::KeyCode::Down); // probe
    press(&mut app, crossterm::event::KeyCode::Enter);
    assert_eq!(app.probe_popup, Some(pid));
    // Keys are swallowed while the popup is open.
    press(&mut app, crossterm::event::KeyCode::Down);
    assert_eq!(app.probe_popup, Some(pid));
    press(&mut app, crossterm::event::KeyCode::Esc);
    assert_eq!(app.probe_popup, None);
}

#[test]
fn folded_task_hides_its_probes_from_navigation() {
    let mut app = fresh_app();
    app.sidebar_view = SidebarView::Probes;
    let t0 = Uuid::new_v4();
    arrive(&mut app, t0, 0, "Setup");
    let t1 = Uuid::new_v4();
    let p1 = arrive(&mut app, t1, 1, "Next"); // task 0 auto-folds (passed)
    press(&mut app, crossterm::event::KeyCode::Down); // task 1 (newest first)
    assert_eq!(app.sidebar_cursor, Some(NavTarget::Task(t1)));
    press(&mut app, crossterm::event::KeyCode::Down); // task 1's probe
    assert_eq!(app.sidebar_cursor, Some(NavTarget::Probe(p1)));
    press(&mut app, crossterm::event::KeyCode::Down); // folded task 0 header
    assert_eq!(app.sidebar_cursor, Some(NavTarget::Task(t0)));
    // Folded → its probe is not reachable; cursor clamps here.
    press(&mut app, crossterm::event::KeyCode::Down);
    assert_eq!(app.sidebar_cursor, Some(NavTarget::Task(t0)));
}

#[test]
fn popup_p_queues_paste_and_hands_focus_to_agent() {
    let mut app = fresh_app();
    app.has_pty = true;
    let tid = Uuid::new_v4();
    let pid = arrive(&mut app, tid, 0, "Trivia");
    app.on_event(TuiEvent::ProbeGraded {
        next_probe_in_secs: None,
        probe_id: pid,
        outcome: arena_core::protocol::ProbeOutcome::Error,
        point_delta: -5,
        expected: Some("Paris".to_string()),
        actual: Some("London".to_string()),
    });
    app.probe_popup = Some(pid);
    press(&mut app, crossterm::event::KeyCode::Char('p'));
    let text = app.pty_paste_pending.take().expect("paste queued");
    assert!(text.contains("Trivia"), "paste names the task: {text}");
    assert!(
        text.contains("Status: fail (-5 pts)"),
        "paste has status: {text}"
    );
    assert!(
        text.contains("Expected: Paris"),
        "paste has expected: {text}"
    );
    assert!(text.contains("Actual: London"), "paste has actual: {text}");
    assert_eq!(app.probe_popup, None, "popup closes after paste");
    assert_eq!(app.input_focus, InputFocus::Pty, "focus moves to agent");
}

#[test]
fn popup_p_without_pty_does_nothing() {
    let mut app = fresh_app();
    let pid = arrive(&mut app, Uuid::new_v4(), 0, "Trivia");
    app.probe_popup = Some(pid);
    press(&mut app, crossterm::event::KeyCode::Char('p'));
    assert_eq!(app.pty_paste_pending, None);
    assert_eq!(app.probe_popup, Some(pid), "popup stays open");
}

#[test]
fn task_group_sums_graded_points() {
    let mut app = fresh_app();
    let tid = Uuid::new_v4();
    let p1 = arrive(&mut app, tid, 0, "Trivia");
    assert_eq!(
        app.task_groups()[0].points,
        None,
        "no points before grading"
    );
    app.on_event(TuiEvent::ProbeGraded {
        next_probe_in_secs: None,
        probe_id: p1,
        outcome: arena_core::protocol::ProbeOutcome::Error,
        point_delta: -5,
        expected: None,
        actual: None,
    });
    let p2 = arrive(&mut app, tid, 0, "Trivia");
    app.on_event(TuiEvent::ProbeGraded {
        next_probe_in_secs: None,
        probe_id: p2,
        outcome: arena_core::protocol::ProbeOutcome::Pass,
        point_delta: 10,
        expected: None,
        actual: None,
    });
    assert_eq!(app.task_groups()[0].points, Some(5));
}

#[test]
fn focus_switch_away_clears_sidebar_selection_and_popup() {
    let mut app = fresh_app();
    app.sidebar_view = SidebarView::Probes;
    let tid = Uuid::new_v4();
    arrive(&mut app, tid, 0, "Setup");
    press(&mut app, crossterm::event::KeyCode::Down); // select task
    assert_eq!(app.sidebar_cursor, Some(NavTarget::Task(tid)));
    press(&mut app, crossterm::event::KeyCode::F(9)); // focus → Pty
    assert_eq!(app.input_focus, InputFocus::Pty);
    assert_eq!(app.sidebar_cursor, None, "selection cleared on focus loss");

    // Same for an open popup: leaving the pane closes it.
    press(&mut app, crossterm::event::KeyCode::F(9)); // back to Tui
    press(&mut app, crossterm::event::KeyCode::Down); // task
    press(&mut app, crossterm::event::KeyCode::Down); // probe
    press(&mut app, crossterm::event::KeyCode::Enter); // open popup
    assert!(app.probe_popup.is_some());
    press(&mut app, crossterm::event::KeyCode::F(9));
    assert_eq!(app.probe_popup, None, "popup closed on focus loss");
    assert_eq!(app.sidebar_cursor, None);
}

#[test]
fn sidebar_keys_ignored_in_pty_focus() {
    let mut app = fresh_app();
    arrive(&mut app, Uuid::new_v4(), 0, "Setup");
    app.input_focus = InputFocus::Pty;
    press(&mut app, crossterm::event::KeyCode::Down);
    assert_eq!(app.sidebar_cursor, None);
}

#[test]
fn on_event_applies_header_delta() {
    let mut app = fresh_app();
    app.on_event(TuiEvent::Header(HeaderDelta::Lobby { seconds: 5 }));
    assert_eq!(app.header.status, crate::tui::header::Status::Lobby);
    assert_eq!(app.header.countdown_secs, Some(5));
}

#[test]
fn on_event_started_clears_countdown() {
    let mut app = fresh_app();
    app.on_event(TuiEvent::Header(HeaderDelta::Lobby { seconds: 30 }));
    app.on_event(TuiEvent::Header(HeaderDelta::Started));
    assert_eq!(app.header.countdown_secs, None);
}

#[test]
fn on_event_probe_arrived_pushes_entry() {
    let mut app = fresh_app();
    let probe_id = Uuid::new_v4();
    app.on_event(TuiEvent::ProbeArrived(ProbeInfo {
        probe_id,
        rendered_command: "echo hi".to_string(),
        deadline_secs: 5,
        task_id: None,
        task_ordinal: 0,
        task_title: "T".to_string(),
        task_description: "D".to_string(),
        test_ordinal: 0,
        test_total: 0,
        test_label: String::new(),
        test_description: String::new(),
        expected_answer: None,
        answer_template: String::new(),
        validation_kind: ValidationKind::Minijinja,
    }));
    assert_eq!(app.probes.len(), 1);
    assert_eq!(app.probes[0].probe_id, probe_id);
}

#[test]
fn on_event_should_quit_propagates() {
    let mut app = fresh_app();
    app.on_event(TuiEvent::ShouldQuit(QuitReason::SessionComplete));
    assert_eq!(app.should_quit, Some(QuitReason::SessionComplete));
}

#[test]
fn on_session_started_validates_agent() {
    let mut app = fresh_app();
    app.on_session_started("/no/such/path");
    assert!(matches!(app.should_quit, Some(QuitReason::PickerFailed(_))));
}

#[test]
fn on_session_started_accepts_valid_agent() {
    let mut app = fresh_app();
    #[cfg(unix)]
    app.on_session_started("/bin/sh");
    #[cfg(not(unix))]
    app.on_session_started("C:\\Windows\\System32\\cmd.exe");
    assert!(app.should_quit.is_none());
}

#[test]
fn evict_oldest_keeps_at_most_200() {
    let mut app = fresh_app();
    for i in 0..300 {
        app.on_event(TuiEvent::ProbeResult(ProbeResultInfo {
            probe_id: Uuid::new_v4(),
            command: format!("cmd-{i}"),
            stdout: format!("p{i}"),
            exit_code: Some(0),
            duration_ms: 0,
            error: None,
            task_id: None,
            task_ordinal: 0,
            task_title: String::new(),
            task_description: String::new(),
            test_ordinal: 0,
            test_total: 0,
            test_label: String::new(),
            test_description: String::new(),
            deadline_secs: None,
            expected_answer: None,
            answer_template: String::new(),
            validation_kind: ValidationKind::Minijinja,
            outcome: None,
            point_delta: None,
            graded_expected: None,
        }));
    }
    assert_eq!(app.probes.len(), 200);
}

#[test]
fn probe_arrived_retains_deadline_secs() {
    let mut app = fresh_app();
    let pid = Uuid::new_v4();
    app.on_event(TuiEvent::ProbeArrived(ProbeInfo {
        probe_id: pid,
        rendered_command: "echo hi".to_string(),
        deadline_secs: 45,
        task_id: None,
        task_ordinal: 0,
        task_title: "T".to_string(),
        task_description: "D".to_string(),
        test_ordinal: 0,
        test_total: 0,
        test_label: String::new(),
        test_description: String::new(),
        expected_answer: None,
        answer_template: String::new(),
        validation_kind: ValidationKind::Minijinja,
    }));
    let stored = app.probes.iter().find(|p| p.probe_id == pid).unwrap();
    assert_eq!(stored.deadline_secs, Some(45));
}

#[test]
fn leaderboard_update_stores_viewer_score_and_rank() {
    let mut app = fresh_app();
    app.viewer_player_id = Some(Uuid::nil());
    let viewer = Uuid::nil();
    let other = Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap();
    let entries = vec![
        crate::tui::event::LeaderboardEntry {
            player_id: other,
            display_name: "other".to_string(),
            agent_display_name: None,
            total_points: 100,
            tests_passed: 5,
            total_wall_ms: 0,
        },
        crate::tui::event::LeaderboardEntry {
            player_id: viewer,
            display_name: "me".to_string(),
            agent_display_name: None,
            total_points: 42,
            tests_passed: 4,
            total_wall_ms: 0,
        },
    ];
    app.on_event(TuiEvent::LeaderboardUpdate { entries });
    assert_eq!(app.score, Some(42));
    assert_eq!(app.rank, Some(2));
}

#[test]
fn leaderboard_update_with_viewer_absent_clears_score_rank() {
    let mut app = fresh_app();
    app.viewer_player_id = Some(Uuid::nil());
    let other = Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap();
    let entries = vec![crate::tui::event::LeaderboardEntry {
        player_id: other,
        display_name: "other".to_string(),
        agent_display_name: None,
        total_points: 100,
        tests_passed: 5,
        total_wall_ms: 0,
    }];
    app.on_event(TuiEvent::LeaderboardUpdate { entries });
    assert_eq!(app.score, None);
    assert_eq!(app.rank, None);
}

#[test]
fn player_progress_stores_attempt_and_status() {
    let mut app = fresh_app();
    app.on_event(TuiEvent::PlayerProgress {
        attempt: 3,
        status: crate::tui::event::PlayerRunStatus::Backoff,
    });
    assert_eq!(app.progress_attempt, Some(3));
    assert_eq!(
        app.progress_status,
        Some(crate::tui::event::PlayerRunStatus::Backoff)
    );
}

#[test]
fn tick_decrements_pending_probe_deadline() {
    let mut app = fresh_app();
    let pid = Uuid::new_v4();
    app.on_event(TuiEvent::ProbeArrived(ProbeInfo {
        probe_id: pid,
        rendered_command: "echo hi".to_string(),
        deadline_secs: 30,
        task_id: None,
        task_ordinal: 0,
        task_title: "T".to_string(),
        task_description: "D".to_string(),
        test_ordinal: 0,
        test_total: 0,
        test_label: String::new(),
        test_description: String::new(),
        expected_answer: None,
        answer_template: String::new(),
        validation_kind: ValidationKind::Minijinja,
    }));
    app.on_event(TuiEvent::Tick);
    app.on_event(TuiEvent::Tick);
    let stored = app.probes.iter().find(|p| p.probe_id == pid).unwrap();
    assert_eq!(stored.deadline_secs, Some(28));
}

#[test]
fn tick_does_not_decrement_resolved_probe_deadline() {
    let mut app = fresh_app();
    let pid = Uuid::new_v4();
    app.on_event(TuiEvent::ProbeArrived(ProbeInfo {
        probe_id: pid,
        rendered_command: "echo hi".to_string(),
        deadline_secs: 30,
        task_id: None,
        task_ordinal: 0,
        task_title: "T".to_string(),
        task_description: "D".to_string(),
        test_ordinal: 0,
        test_total: 0,
        test_label: String::new(),
        test_description: String::new(),
        expected_answer: None,
        answer_template: String::new(),
        validation_kind: ValidationKind::Minijinja,
    }));
    app.on_event(TuiEvent::ProbeResult(ProbeResultInfo {
        probe_id: pid,
        command: "echo hi".to_string(),
        stdout: "hi".to_string(),
        exit_code: Some(0),
        duration_ms: 5,
        error: None,
        task_id: None,
        task_ordinal: 0,
        task_title: "T".to_string(),
        task_description: "D".to_string(),
        test_ordinal: 0,
        test_total: 0,
        test_label: String::new(),
        test_description: String::new(),
        deadline_secs: Some(30),
        expected_answer: None,
        answer_template: String::new(),
        validation_kind: ValidationKind::Minijinja,
        outcome: None,
        point_delta: None,
        graded_expected: None,
    }));
    app.on_event(TuiEvent::Tick);
    let stored = app.probes.iter().find(|p| p.probe_id == pid).unwrap();
    assert_eq!(stored.deadline_secs, Some(30));
}

#[test]
fn probe_result_preserves_task_metadata_from_arrival() {
    let mut app = fresh_app();
    let pid = Uuid::new_v4();
    app.on_event(TuiEvent::ProbeArrived(ProbeInfo {
        probe_id: pid,
        rendered_command: "echo hi".to_string(),
        deadline_secs: 30,
        task_id: None,
        task_ordinal: 0,
        task_title: "Task A".to_string(),
        task_description: "Describe A".to_string(),
        test_ordinal: 0,
        test_total: 0,
        test_label: String::new(),
        test_description: String::new(),
        expected_answer: None,
        answer_template: String::new(),
        validation_kind: ValidationKind::Minijinja,
    }));
    // Result frame omits task metadata (empty strings) — must not clobber.
    app.on_event(TuiEvent::ProbeResult(ProbeResultInfo {
        probe_id: pid,
        command: "echo hi".to_string(),
        stdout: "hi".to_string(),
        exit_code: Some(1),
        duration_ms: 7,
        error: None,
        task_id: None,
        task_ordinal: 0,
        task_title: String::new(),
        task_description: String::new(),
        test_ordinal: 0,
        test_total: 0,
        test_label: String::new(),
        test_description: String::new(),
        deadline_secs: None,
        expected_answer: None,
        answer_template: String::new(),
        validation_kind: ValidationKind::Minijinja,
        outcome: None,
        point_delta: None,
        graded_expected: None,
    }));
    let stored = app.probes.iter().find(|p| p.probe_id == pid).unwrap();
    assert_eq!(stored.task_title, "Task A");
    assert_eq!(stored.task_description, "Describe A");
    assert_eq!(stored.exit_code, Some(1));
}

#[test]
fn probe_result_with_explicit_metadata_overrides_arrival() {
    let mut app = fresh_app();
    let pid = Uuid::new_v4();
    app.on_event(TuiEvent::ProbeArrived(ProbeInfo {
        probe_id: pid,
        rendered_command: "echo hi".to_string(),
        deadline_secs: 30,
        task_id: None,
        task_ordinal: 0,
        task_title: "Old".to_string(),
        task_description: "Old desc".to_string(),
        test_ordinal: 0,
        test_total: 0,
        test_label: String::new(),
        test_description: String::new(),
        expected_answer: None,
        answer_template: String::new(),
        validation_kind: ValidationKind::Minijinja,
    }));
    app.on_event(TuiEvent::ProbeResult(ProbeResultInfo {
        probe_id: pid,
        command: "echo hi".to_string(),
        stdout: "hi".to_string(),
        exit_code: Some(0),
        duration_ms: 3,
        error: None,
        task_id: None,
        task_ordinal: 0,
        task_title: "New".to_string(),
        task_description: "New desc".to_string(),
        test_ordinal: 0,
        test_total: 0,
        test_label: String::new(),
        test_description: String::new(),
        deadline_secs: Some(30),
        expected_answer: None,
        answer_template: String::new(),
        validation_kind: ValidationKind::Minijinja,
        outcome: None,
        point_delta: None,
        graded_expected: None,
    }));
    let stored = app.probes.iter().find(|p| p.probe_id == pid).unwrap();
    assert_eq!(stored.task_title, "New");
    assert_eq!(stored.task_description, "New desc");
}

#[test]
fn viewer_identified_sets_viewer_player_id() {
    let mut app = fresh_app();
    let vid = Uuid::new_v4();
    app.on_event(TuiEvent::ViewerIdentified(vid));
    assert_eq!(app.viewer_player_id, Some(vid));
}

/// Grade a probe with the given outcome (server-side judgement).
fn grade(app: &mut TuiApp, pid: Uuid, outcome: arena_core::protocol::ProbeOutcome) {
    app.on_event(TuiEvent::ProbeGraded {
        next_probe_in_secs: None,
        probe_id: pid,
        outcome,
        point_delta: if outcome == arena_core::protocol::ProbeOutcome::Pass {
            5
        } else {
            -5
        },
        expected: None,
        actual: None,
    });
}

#[test]
fn f1_toggles_help_popup() {
    let mut app = fresh_app();
    press(&mut app, crossterm::event::KeyCode::F(1));
    assert!(app.show_help);
    press(&mut app, crossterm::event::KeyCode::F(1));
    assert!(!app.show_help);
}

#[test]
fn f1_from_pty_focus_opens_help_and_pulls_focus_to_tui() {
    let mut app = fresh_app();
    app.has_pty = true;
    app.input_focus = InputFocus::Pty;
    press(&mut app, crossterm::event::KeyCode::F(1));
    assert!(app.show_help);
    assert_eq!(
        app.input_focus,
        InputFocus::Tui,
        "Esc/q must reach the popup"
    );
}

#[test]
fn question_mark_opens_help_in_tui_focus() {
    let mut app = fresh_app();
    press(&mut app, crossterm::event::KeyCode::Char('?'));
    assert!(app.show_help);
}

#[test]
fn help_swallows_keys_and_esc_closes() {
    let mut app = fresh_app();
    arrive(&mut app, Uuid::new_v4(), 0, "Setup");
    press(&mut app, crossterm::event::KeyCode::F(1));
    press(&mut app, crossterm::event::KeyCode::Down);
    assert_eq!(
        app.sidebar_cursor, None,
        "sidebar keys swallowed while help open"
    );
    press(&mut app, crossterm::event::KeyCode::Esc);
    assert!(!app.show_help);
}

#[test]
fn focus_switch_away_closes_help() {
    let mut app = fresh_app();
    press(&mut app, crossterm::event::KeyCode::F(1));
    app.set_input_focus(InputFocus::Pty);
    assert!(!app.show_help);
}

#[test]
fn last_failed_probe_picks_newest_failure_skipping_passes() {
    let mut app = fresh_app();
    let tid = Uuid::new_v4();
    let p_old_fail = arrive(&mut app, tid, 0, "Trivia");
    grade(
        &mut app,
        p_old_fail,
        arena_core::protocol::ProbeOutcome::Error,
    );
    let p_new_fail = arrive(&mut app, tid, 0, "Trivia");
    grade(
        &mut app,
        p_new_fail,
        arena_core::protocol::ProbeOutcome::Error,
    );
    let p_pass = arrive(&mut app, tid, 0, "Trivia");
    grade(&mut app, p_pass, arena_core::protocol::ProbeOutcome::Pass);
    assert_eq!(
        app.last_failed_probe().map(|p| p.probe_id),
        Some(p_new_fail),
        "newest failure wins; a newer pass is skipped"
    );
}

#[test]
fn f2_opens_popup_on_last_failed_probe_from_pty_focus() {
    let mut app = fresh_app();
    app.has_pty = true;
    app.input_focus = InputFocus::Pty;
    let pid = arrive(&mut app, Uuid::new_v4(), 0, "Trivia");
    grade(&mut app, pid, arena_core::protocol::ProbeOutcome::Error);
    press(&mut app, crossterm::event::KeyCode::F(2));
    assert_eq!(app.probe_popup, Some(pid));
    assert_eq!(app.input_focus, InputFocus::Tui, "popup keys must work");
}

#[test]
fn f2_without_failures_does_nothing() {
    let mut app = fresh_app();
    let pid = arrive(&mut app, Uuid::new_v4(), 0, "Trivia");
    grade(&mut app, pid, arena_core::protocol::ProbeOutcome::Pass);
    press(&mut app, crossterm::event::KeyCode::F(2));
    assert_eq!(app.probe_popup, None);
}

#[test]
fn f3_pastes_last_failed_probe_and_focuses_agent() {
    let mut app = fresh_app();
    app.has_pty = true;
    let pid = arrive(&mut app, Uuid::new_v4(), 0, "Trivia");
    app.on_event(TuiEvent::ProbeGraded {
        next_probe_in_secs: None,
        probe_id: pid,
        outcome: arena_core::protocol::ProbeOutcome::Error,
        point_delta: -5,
        expected: Some("Paris".to_string()),
        actual: Some("London".to_string()),
    });
    press(&mut app, crossterm::event::KeyCode::F(3));
    let text = app.pty_paste_pending.take().expect("paste queued");
    assert!(text.contains("Trivia"), "paste names the task: {text}");
    assert!(
        text.contains("Expected: Paris"),
        "paste has expected: {text}"
    );
    assert_eq!(app.input_focus, InputFocus::Pty, "focus moves to agent");
}

#[test]
fn f3_without_pty_or_without_failures_does_nothing() {
    let mut app = fresh_app();
    let pid = arrive(&mut app, Uuid::new_v4(), 0, "Trivia");
    grade(&mut app, pid, arena_core::protocol::ProbeOutcome::Error);
    press(&mut app, crossterm::event::KeyCode::F(3));
    assert_eq!(app.pty_paste_pending, None, "no PTY -> no paste");

    let mut app = fresh_app();
    app.has_pty = true;
    press(&mut app, crossterm::event::KeyCode::F(3));
    assert_eq!(app.pty_paste_pending, None, "no failures -> no paste");
    assert_eq!(app.input_focus, InputFocus::Tui, "focus untouched");
}

#[test]
fn player_tasks_done_ack_commits_last_task_and_does_not_quit() {
    use crate::test_util::test_util::{HOME_LOCK, HomeGuard};
    let _g = HOME_LOCK.lock().unwrap();
    let home = tempfile::tempdir().expect("home tempdir");
    let worktree = tempfile::tempdir().expect("worktree tempdir");
    let _h = HomeGuard::set(home.path().to_str().unwrap());
    std::fs::write(worktree.path().join("solution.txt"), "answer").unwrap();

    let snap = crate::snapshot::SnapshotRepo::new("default", "ACK1", worktree.path(), None, None)
        .expect("snapshot repo");
    let snap = Arc::new(std::sync::Mutex::new(snap));

    let mut app = fresh_app();
    app.snapshot = Some(snap.clone());
    let tid = Uuid::new_v4();
    arrive(&mut app, tid, 0, "Last task");

    // Per-player ack: this player is done, session keeps running for
    // the others. The ack is the "scheduler moved on" signal for the
    // player's LAST task — its snapshot commit must happen right here,
    // not at the final session end, or the judge stalls waiting for
    // the feat(<task_id>) commit.
    app.on_event(TuiEvent::Header(HeaderDelta::PlayerTasksDone));

    assert_eq!(app.should_quit, None, "ack must not quit the TUI");
    assert_eq!(app.header.status, crate::tui::header::Status::TasksDone);
    let msg = snap
        .lock()
        .unwrap()
        .head_commit_message()
        .expect("head commit exists after ack");
    assert_eq!(msg, format!("feat({tid}): Last task"));
}

#[test]
fn final_complete_after_ack_does_not_duplicate_task_commit() {
    use crate::test_util::test_util::{HOME_LOCK, HomeGuard};
    let _g = HOME_LOCK.lock().unwrap();
    let home = tempfile::tempdir().expect("home tempdir");
    let worktree = tempfile::tempdir().expect("worktree tempdir");
    let _h = HomeGuard::set(home.path().to_str().unwrap());
    std::fs::write(worktree.path().join("solution.txt"), "answer").unwrap();

    let snap = crate::snapshot::SnapshotRepo::new("default", "ACK2", worktree.path(), None, None)
        .expect("snapshot repo");
    let snap = Arc::new(std::sync::Mutex::new(snap));

    let mut app = fresh_app();
    app.snapshot = Some(snap.clone());
    let tid = Uuid::new_v4();
    arrive(&mut app, tid, 0, "Last task");
    app.on_event(TuiEvent::Header(HeaderDelta::PlayerTasksDone));
    let msg_after_ack = snap.lock().unwrap().head_commit_message();

    // The real session end arrives later — committed tasks are skipped.
    app.on_event(TuiEvent::Header(HeaderDelta::Complete {
        session_id: "abc".into(),
    }));
    assert_eq!(app.header.status, crate::tui::header::Status::Complete);
    let msg_after_complete = snap.lock().unwrap().head_commit_message();
    assert_eq!(
        msg_after_ack, msg_after_complete,
        "already-committed task must not be re-committed on session end"
    );
}

#[test]
fn frames_after_ack_are_still_processed() {
    let mut app = fresh_app();
    app.viewer_player_id = Some(Uuid::nil());
    app.on_event(TuiEvent::Header(HeaderDelta::PlayerTasksDone));
    app.on_event(TuiEvent::LeaderboardUpdate {
        entries: vec![crate::tui::event::LeaderboardEntry {
            player_id: Uuid::nil(),
            display_name: "me".to_string(),
            agent_display_name: None,
            total_points: 42,
            tests_passed: 4,
            total_wall_ms: 0,
        }],
    });
    assert_eq!(app.score, Some(42), "leaderboard updates keep flowing");
    assert_eq!(app.rank, Some(1));
    assert_eq!(app.should_quit, None);
}

#[test]
fn f4_toggles_sidebar_and_requests_pty_resize() {
    let mut app = fresh_app();
    app.has_pty = true;
    app.input_focus = InputFocus::Pty;
    assert!(app.show_sidebar);
    press(&mut app, crossterm::event::KeyCode::F(4));
    assert!(!app.show_sidebar);
    assert!(
        app.pty_resize_pending,
        "PTY must be resized to the new rect"
    );
    app.pty_resize_pending = false;
    press(&mut app, crossterm::event::KeyCode::F(4));
    assert!(app.show_sidebar);
    assert!(app.pty_resize_pending);
}

#[test]
fn artifact_sweep_needs_no_bookkeeping_state() {
    // The artifacts sweep is fingerprint-driven: with no snapshot repo and
    // no files it does nothing and keeps the zero fingerprint.
    let mut app = fresh_app();
    app.commit_arrived_artifacts();
    assert_eq!(app.artifacts_fingerprint, 0);
}

// ---------- probe-permission popup ----------

fn permission_prompt(
    probe_id: Uuid,
) -> (
    crate::tui::event::PermissionPrompt,
    tokio::sync::oneshot::Receiver<crate::permissions::Decision>,
) {
    let (tx, rx) = tokio::sync::oneshot::channel();
    (
        crate::tui::event::PermissionPrompt {
            probe_id,
            command: "sh answer.sh -q \"5 plus 3\"".into(),
            always_rule: "sh answer.sh *".into(),
            deadline_secs: 60,
            responder: Arc::new(std::sync::Mutex::new(Some(tx))),
        },
        rx,
    )
}

#[test]
fn permission_popup_is_modal_and_allows() {
    use crossterm::event::{KeyCode, KeyModifiers};
    let mut app = fresh_app();
    app.input_focus = InputFocus::Pty;
    let (prompt, mut rx) = permission_prompt(Uuid::new_v4());
    app.on_event(crate::tui::event::TuiEvent::PermissionRequest(prompt));
    assert!(app.permission_popup.is_some());
    assert_eq!(app.input_focus, InputFocus::Tui, "popup pulls focus");

    // Navigation and focus keys are swallowed while the question is open.
    app.on_key(KeyCode::Char('j'), KeyModifiers::NONE);
    app.on_key(KeyCode::F(9), KeyModifiers::NONE);
    assert!(app.permission_popup.is_some());
    assert_eq!(app.input_focus, InputFocus::Tui);

    app.on_key(KeyCode::Char('a'), KeyModifiers::NONE);
    assert!(app.permission_popup.is_none());
    assert_eq!(rx.try_recv().unwrap(), crate::permissions::Decision::Allow);
}

#[test]
fn permission_esc_declines() {
    use crossterm::event::{KeyCode, KeyModifiers};
    let mut app = fresh_app();
    let (prompt, mut rx) = permission_prompt(Uuid::new_v4());
    app.on_event(crate::tui::event::TuiEvent::PermissionRequest(prompt));
    app.on_key(KeyCode::Esc, KeyModifiers::NONE);
    assert!(app.permission_popup.is_none());
    assert_eq!(
        rx.try_recv().unwrap(),
        crate::permissions::Decision::Decline
    );
}

#[test]
fn permission_resolved_closes_only_matching_popup() {
    let mut app = fresh_app();
    let pid = Uuid::new_v4();
    let (prompt, _rx) = permission_prompt(pid);
    app.on_event(crate::tui::event::TuiEvent::PermissionRequest(prompt));

    app.on_event(crate::tui::event::TuiEvent::PermissionResolved {
        probe_id: Uuid::new_v4(),
    });
    assert!(app.permission_popup.is_some(), "unrelated probe id ignored");

    app.on_event(crate::tui::event::TuiEvent::PermissionResolved { probe_id: pid });
    assert!(app.permission_popup.is_none());
}

#[test]
fn permission_arrows_move_cursor_and_enter_confirms() {
    use crossterm::event::{KeyCode, KeyModifiers};
    let mut app = fresh_app();
    let (prompt, mut rx) = permission_prompt(Uuid::new_v4());
    app.on_event(crate::tui::event::TuiEvent::PermissionRequest(prompt));
    assert_eq!(app.permission_cursor, 0, "opens on allow-once");

    // Down three times lands on decline; a fourth Down clamps.
    app.on_key(KeyCode::Down, KeyModifiers::NONE);
    app.on_key(KeyCode::Down, KeyModifiers::NONE);
    app.on_key(KeyCode::Down, KeyModifiers::NONE);
    app.on_key(KeyCode::Down, KeyModifiers::NONE);
    assert_eq!(app.permission_cursor, 3);
    app.on_key(KeyCode::Up, KeyModifiers::NONE);
    // Up moves back; Left/Right work too.
    app.on_key(KeyCode::Up, KeyModifiers::NONE);
    assert_eq!(app.permission_cursor, 1);
    app.on_key(KeyCode::Right, KeyModifiers::NONE);
    assert_eq!(app.permission_cursor, 2);
    app.on_key(KeyCode::Left, KeyModifiers::NONE);
    assert_eq!(app.permission_cursor, 1);

    app.on_key(KeyCode::Enter, KeyModifiers::NONE);
    assert!(app.permission_popup.is_none());
    assert_eq!(
        rx.try_recv().unwrap(),
        crate::permissions::Decision::AlwaysAllow
    );
}

#[test]
fn permission_enter_defaults_to_allow_once() {
    use crossterm::event::{KeyCode, KeyModifiers};
    let mut app = fresh_app();
    let (prompt, mut rx) = permission_prompt(Uuid::new_v4());
    app.on_event(crate::tui::event::TuiEvent::PermissionRequest(prompt));
    app.on_key(KeyCode::Enter, KeyModifiers::NONE);
    assert_eq!(rx.try_recv().unwrap(), crate::permissions::Decision::Allow);

    // A fresh prompt resets the cursor.
    let (prompt2, _rx2) = permission_prompt(Uuid::new_v4());
    app.permission_cursor = 2;
    app.on_event(crate::tui::event::TuiEvent::PermissionRequest(prompt2));
    assert_eq!(app.permission_cursor, 0);
}

// ── Chat view (F5) ────────────────────────────────────────────────────────

/// A graded task probe with just the fields the chat transcript reads.
fn chat_probe(task_id: Uuid, ordinal: i32, test_ordinal: i32, desc: &str) -> ProbeResultInfo {
    ProbeResultInfo {
        probe_id: Uuid::new_v4(),
        command: "cmd".to_string(),
        stdout: "out".to_string(),
        exit_code: Some(0),
        task_id: Some(task_id),
        task_ordinal: ordinal,
        task_title: format!("Task {ordinal}"),
        task_description: desc.to_string(),
        test_ordinal,
        test_total: 2,
        test_label: String::new(),
        test_description: String::new(),
        outcome: Some(arena_core::protocol::ProbeOutcome::Pass),
        ..Default::default()
    }
}

#[test]
fn f5_toggles_sidebar_view_and_unhides_the_pane() {
    use crossterm::event::{KeyCode, KeyModifiers};
    let mut app = fresh_app();
    assert_eq!(app.sidebar_view, SidebarView::Chat, "chat is the default");
    app.show_sidebar = false;
    app.on_key(KeyCode::F(5), KeyModifiers::NONE);
    assert_eq!(app.sidebar_view, SidebarView::Probes);
    assert!(app.show_sidebar, "toggling the view must reveal the pane");
    app.chat_scroll = 7;
    app.on_key(KeyCode::F(5), KeyModifiers::NONE);
    assert_eq!(app.sidebar_view, SidebarView::Chat);
    assert_eq!(app.chat_scroll, 0, "entering chat re-follows the latest");
}

#[test]
fn f5_works_from_pty_focus_too() {
    use crossterm::event::{KeyCode, KeyModifiers};
    let mut app = fresh_app();
    app.input_focus = InputFocus::Pty;
    app.on_key(KeyCode::F(5), KeyModifiers::NONE);
    assert_eq!(app.sidebar_view, SidebarView::Probes);
}

#[test]
fn judge_scored_is_stored_with_the_current_task_ordinal() {
    let mut app = fresh_app();
    app.on_event(TuiEvent::ProbeResult(chat_probe(Uuid::new_v4(), 1, 1, "d")));
    app.on_event(TuiEvent::JudgeScored {
        task_id: None,
        judge_name: "Creativity".to_string(),
        point_delta: 17,
        feedback: "nice".to_string(),
    });
    assert_eq!(app.judge_verdicts.len(), 1);
    let v = &app.judge_verdicts[0];
    assert_eq!(v.judge_name, "Creativity");
    assert_eq!(v.point_delta, 17);
    assert_eq!(v.task_ordinal, Some(1));
}

#[test]
fn chat_transcript_orders_tasks_collapses_reruns_and_pins_verdicts() {
    let mut app = fresh_app();
    let t0 = Uuid::new_v4();
    let t1 = Uuid::new_v4();
    // Task 0: the same test polled twice, then a verdict.
    app.on_event(TuiEvent::ProbeResult(chat_probe(t0, 0, 1, "done-note")));
    app.on_event(TuiEvent::ProbeResult(chat_probe(t0, 0, 1, "done-note")));
    app.on_event(TuiEvent::JudgeScored {
        task_id: None,
        judge_name: "Data".to_string(),
        point_delta: 19,
        feedback: String::new(),
    });
    // Task 1 begins.
    app.on_event(TuiEvent::ProbeResult(chat_probe(t1, 1, 1, "forecast")));
    app.on_event(TuiEvent::MemberJoined {
        name: "Ololoster".to_string(),
    });

    let msgs = app.chat_transcript();
    let kinds: Vec<String> = msgs
        .iter()
        .map(|m| match m {
            ChatMsg::TaskHeader { ordinal, .. } => format!("task{ordinal}"),
            ChatMsg::Brief { .. } => "brief".to_string(),
            ChatMsg::Check { runs, .. } => format!("check×{runs}"),
            ChatMsg::Request { judge, .. } => format!("request:{judge}"),
            ChatMsg::DoneNote(_) => "done-note".to_string(),
            ChatMsg::Verdict(v) => format!("judge:{}", v.judge_name),
            ChatMsg::System { .. } => "system".to_string(),
        })
        .collect();
    assert_eq!(
        kinds,
        vec![
            "task0",
            "brief",
            "check×2",
            "judge:Data",
            "task1",
            "brief",
            "check×1",
            "system"
        ],
        "chat must read oldest-first: task → brief → collapsed checks → verdicts"
    );
}

#[test]
fn chat_transcript_retells_artifact_requests_as_judge_messages() {
    let mut app = fresh_app();
    let tid = Uuid::new_v4();
    app.on_event(TuiEvent::ProbeResult(chat_probe(tid, 0, 1, "brief")));
    let mut req = chat_probe(tid, 0, 2, "brief");
    req.command = "# ARTIFACT REQUEST from creativity: Capture the game as screenshot.png\n\
                   # Save the file(s) (up to 5) under .ololo/artifacts/abc/; the ololo CLI commits and pushes them.\n\
                   test -n \"$(ls -A .ololo/artifacts/abc/ 2>/dev/null)\" && echo present || echo missing"
        .to_string();
    req.outcome = None;
    app.on_event(TuiEvent::ProbeResult(req));

    let msgs = app.chat_transcript();
    assert!(
        msgs.iter().any(|m| matches!(
            m,
            ChatMsg::Request { judge, instruction, path, delivered }
                if judge == "creativity"
                    && instruction.contains("Capture the game")
                    && path == ".ololo/artifacts/abc/"
                    && !delivered
        )),
        "the request probe must speak as the judge, not as a check"
    );
}

#[test]
fn chat_transcript_quotes_the_quiz_question() {
    let mut app = fresh_app();
    let mut p = chat_probe(Uuid::new_v4(), 0, 1, "brief");
    p.command =
        r#"curl -s --data-urlencode "q=ab12: What is 2 plus 2?" http://localhost:3000"#.to_string();
    p.stdout = "4".to_string();
    app.on_event(TuiEvent::ProbeResult(p));

    let msgs = app.chat_transcript();
    assert!(
        msgs.iter().any(|m| matches!(
            m,
            ChatMsg::Check { question: Some(q), .. } if q == "What is 2 plus 2?"
        )),
        "the quiz question must be quoted, qid prefix stripped"
    );
}

#[test]
fn chat_transcript_turns_the_completion_probe_into_guidance() {
    let mut app = fresh_app();
    let mut p = chat_probe(Uuid::new_v4(), 0, 1, "brief");
    p.command = "test -f .ololo/weather-widget-done.md && cat .ololo/weather-widget-done.md || echo not-done"
        .to_string();
    p.stdout = "not-done".to_string();
    p.outcome = Some(arena_core::protocol::ProbeOutcome::Error);
    app.on_event(TuiEvent::ProbeResult(p));

    let msgs = app.chat_transcript();
    assert!(
        msgs.iter().any(|m| matches!(
            m,
            ChatMsg::System { text } if text.contains(".ololo/weather-widget-done.md")
        )),
        "the completion contract reads as guidance naming the flag file"
    );
    assert!(
        !msgs.iter().any(|m| matches!(m, ChatMsg::Check { .. })),
        "the polling probe must not read as a failing check"
    );
}

#[test]
fn done_note_lands_in_the_chat_pinned_to_the_current_task() {
    let mut app = fresh_app();
    app.on_event(TuiEvent::ProbeResult(chat_probe(
        Uuid::new_v4(),
        0,
        1,
        "brief",
    )));
    app.on_event(TuiEvent::CompletionFlagPublished {
        path: ".ololo/wx-done.md".to_string(),
        text: "Built the widget with quick picks".to_string(),
    });

    let msgs = app.chat_transcript();
    assert!(
        msgs.iter().any(|m| matches!(
            m,
            ChatMsg::DoneNote(n)
                if n.text == "Built the widget with quick picks" && n.task_ordinal == Some(0)
        )),
        "the done-note is the player's message on the task in play"
    );
}

#[test]
fn session_status_closes_the_transcript() {
    let mut app = fresh_app();
    app.on_event(TuiEvent::ProbeResult(chat_probe(
        Uuid::new_v4(),
        0,
        1,
        "brief",
    )));
    app.header.status = crate::tui::header::Status::Complete;

    let msgs = app.chat_transcript();
    assert!(
        matches!(
            msgs.last(),
            Some(ChatMsg::System { text }) if text.contains("session complete")
        ),
        "the session status is the transcript's last word"
    );
}

#[test]
fn chat_selection_sends_the_bubbles_text_to_the_agent() {
    use crossterm::event::{KeyCode, KeyModifiers};
    let mut app = fresh_app();
    app.has_pty = true;
    app.on_event(TuiEvent::ProbeResult(chat_probe(
        Uuid::new_v4(),
        0,
        1,
        "brief",
    )));
    app.on_event(TuiEvent::JudgeScored {
        task_id: None,
        judge_name: "Creativity".to_string(),
        point_delta: 17,
        feedback: "goes well beyond the brief".to_string(),
    });

    // ↑ selects the newest bubble; the transcript's last word is the
    // status line, one more ↑ reaches the verdict.
    app.on_key(KeyCode::Up, KeyModifiers::NONE);
    assert_eq!(app.chat_cursor, Some(0));
    while {
        let msgs = app.chat_transcript();
        let idx = msgs.len() - 1 - app.chat_cursor.unwrap();
        !matches!(msgs[idx], ChatMsg::Verdict(_))
    } {
        app.on_key(KeyCode::Up, KeyModifiers::NONE);
    }
    app.on_key(KeyCode::Enter, KeyModifiers::NONE);

    let text = app
        .pty_paste_pending
        .take()
        .expect("bubble queued as paste");
    assert!(
        text.contains("Creativity") && text.contains("goes well beyond the brief"),
        "the verdict's own words travel to the agent: {text}"
    );
    assert!(text.contains("+17"), "the points travel too: {text}");
    assert_eq!(
        app.input_focus,
        InputFocus::Pty,
        "focus lands in the agent so the player can add words and submit"
    );
}

#[test]
fn chat_selection_steps_back_to_follow_mode_and_esc_clears() {
    use crossterm::event::{KeyCode, KeyModifiers};
    let mut app = fresh_app();
    app.has_pty = true;
    app.on_event(TuiEvent::ProbeResult(chat_probe(
        Uuid::new_v4(),
        0,
        1,
        "brief",
    )));

    app.on_key(KeyCode::Up, KeyModifiers::NONE);
    app.on_key(KeyCode::Up, KeyModifiers::NONE);
    assert_eq!(app.chat_cursor, Some(1));
    app.on_key(KeyCode::Down, KeyModifiers::NONE);
    assert_eq!(app.chat_cursor, Some(0));
    app.on_key(KeyCode::Down, KeyModifiers::NONE);
    assert_eq!(app.chat_cursor, None, "stepping past the newest re-follows");

    app.on_key(KeyCode::Up, KeyModifiers::NONE);
    app.on_key(KeyCode::Esc, KeyModifiers::NONE);
    assert_eq!(app.chat_cursor, None, "Esc clears the selection");
}

#[test]
fn enter_without_a_selection_still_opens_the_compose_line() {
    use crossterm::event::{KeyCode, KeyModifiers};
    let mut app = fresh_app();
    app.has_pty = true;
    app.on_event(TuiEvent::ProbeResult(chat_probe(
        Uuid::new_v4(),
        0,
        1,
        "brief",
    )));
    app.on_key(KeyCode::Enter, KeyModifiers::NONE);
    assert!(
        app.chat_input.is_some(),
        "no selection: Enter is the compose shortcut"
    );
    assert!(app.pty_paste_pending.is_none());
}

#[test]
fn chat_compose_pastes_into_the_agent_like_f3() {
    use crossterm::event::{KeyCode, KeyModifiers};
    let mut app = fresh_app();
    app.has_pty = true;
    app.on_event(TuiEvent::ProbeResult(chat_probe(
        Uuid::new_v4(),
        0,
        1,
        "brief",
    )));

    app.open_chat_compose();
    assert_eq!(app.chat_input.as_deref(), Some(""));
    assert_eq!(app.input_focus, InputFocus::Tui, "compose pulls focus");

    for c in "fix it".chars() {
        app.on_key(KeyCode::Char(c), KeyModifiers::NONE);
    }
    app.on_key(KeyCode::Backspace, KeyModifiers::NONE);
    app.on_key(KeyCode::Char('t'), KeyModifiers::NONE);
    app.on_key(KeyCode::Enter, KeyModifiers::NONE);

    assert!(app.chat_input.is_none(), "submit closes the compose line");
    assert_eq!(
        app.pty_paste_pending.as_deref(),
        Some("fix it"),
        "the message is queued as a paste — same mechanism as F3/probe 'p'"
    );
    assert_eq!(
        app.input_focus,
        InputFocus::Pty,
        "focus lands in the agent so the player can edit and submit"
    );
}

#[test]
fn chat_compose_esc_cancels_without_sending() {
    use crossterm::event::{KeyCode, KeyModifiers};
    let mut app = fresh_app();
    app.has_pty = true;
    app.open_chat_compose();
    app.on_key(KeyCode::Char('x'), KeyModifiers::NONE);
    app.on_key(KeyCode::Esc, KeyModifiers::NONE);
    assert!(app.chat_input.is_none());
    assert!(app.pty_paste_pending.is_none());
}

#[test]
fn chat_compose_needs_a_hosted_agent() {
    let mut app = fresh_app();
    app.has_pty = false;
    app.open_chat_compose();
    assert!(app.chat_input.is_none(), "no agent — nowhere to send");
}

#[test]
fn chat_transcript_keeps_orphan_verdicts_at_the_end() {
    let mut app = fresh_app();
    // A verdict before any probe arrived: no task to pin it to.
    app.on_event(TuiEvent::JudgeScored {
        task_id: None,
        judge_name: "Agentic".to_string(),
        point_delta: 2,
        feedback: String::new(),
    });
    let msgs = app.chat_transcript();
    assert_eq!(msgs.len(), 1);
    assert!(matches!(&msgs[0], ChatMsg::Verdict(v) if v.judge_name == "Agentic"));
}

#[test]
fn chat_view_arrow_keys_move_the_bubble_cursor_not_the_probe_cursor() {
    use crossterm::event::{KeyCode, KeyModifiers};
    let mut app = fresh_app();
    app.on_event(TuiEvent::ProbeResult(chat_probe(Uuid::new_v4(), 0, 1, "d")));
    app.sidebar_view = SidebarView::Chat;
    app.on_key(KeyCode::Up, KeyModifiers::NONE);
    assert_eq!(app.chat_cursor, Some(0), "↑ selects the newest bubble");
    assert_eq!(app.sidebar_cursor, None, "chat view has no probe cursor");
    app.on_key(KeyCode::Esc, KeyModifiers::NONE);
    assert_eq!(app.chat_cursor, None, "Esc clears the selection");
    assert_eq!(app.chat_scroll, 0, "Esc jumps back to the latest message");
}

#[test]
fn chat_scroll_reaches_the_top_of_a_long_message() {
    use crossterm::event::{KeyCode, KeyModifiers};
    // A single check can carry a whole TAP dump. The scroll ceiling must be
    // derived from the text, not from a per-message guess, or the reader is
    // stranded partway up. (↑ selects bubbles now — PgUp is the line scroll.)
    let mut app = fresh_app();
    let mut p = chat_probe(Uuid::new_v4(), 0, 1, "check");
    p.stdout = "ok 1 - a passing subtest with a reasonably long name".repeat(40);
    app.on_event(TuiEvent::ProbeResult(p));
    app.sidebar_view = SidebarView::Chat;

    for _ in 0..40 {
        app.on_key(KeyCode::PageUp, KeyModifiers::NONE);
    }
    assert!(
        app.chat_scroll > 100,
        "a ~2000-character answer must be scrollable well past a few screens, got {}",
        app.chat_scroll
    );
}

// ── Focus restore after modals ────────────────────────────────────────────

#[test]
fn permission_popup_returns_focus_to_the_agent() {
    use crossterm::event::{KeyCode, KeyModifiers};
    let mut app = fresh_app();
    app.has_pty = true;
    app.input_focus = InputFocus::Pty;
    let (prompt, _rx) = permission_prompt(Uuid::new_v4());
    app.on_event(crate::tui::event::TuiEvent::PermissionRequest(prompt));
    assert_eq!(app.input_focus, InputFocus::Tui, "modal pulls focus");
    app.on_key(KeyCode::Char('a'), KeyModifiers::NONE);
    assert_eq!(
        app.input_focus,
        InputFocus::Pty,
        "answering hands focus back to where the user was typing"
    );
}

#[test]
fn permission_resolved_externally_also_restores_focus() {
    let mut app = fresh_app();
    app.has_pty = true;
    app.input_focus = InputFocus::Pty;
    let pid = Uuid::new_v4();
    let (prompt, _rx) = permission_prompt(pid);
    app.on_event(crate::tui::event::TuiEvent::PermissionRequest(prompt));
    app.on_event(crate::tui::event::TuiEvent::PermissionResolved { probe_id: pid });
    assert_eq!(app.input_focus, InputFocus::Pty);
}

#[test]
fn help_open_close_round_trips_pty_focus() {
    use crossterm::event::{KeyCode, KeyModifiers};
    let mut app = fresh_app();
    app.has_pty = true;
    app.input_focus = InputFocus::Pty;
    app.on_key(KeyCode::F(1), KeyModifiers::NONE);
    assert_eq!(app.input_focus, InputFocus::Tui);
    app.on_key(KeyCode::Esc, KeyModifiers::NONE);
    assert_eq!(
        app.input_focus,
        InputFocus::Pty,
        "closing help restores focus"
    );
}

#[test]
fn f9_choice_cancels_the_pending_modal_restore() {
    use crossterm::event::{KeyCode, KeyModifiers};
    let mut app = fresh_app();
    app.has_pty = true;
    app.input_focus = InputFocus::Pty;
    app.on_key(KeyCode::F(1), KeyModifiers::NONE); // help steals to Tui
    app.on_key(KeyCode::Esc, KeyModifiers::NONE); // restore → Pty
    app.on_key(KeyCode::F(9), KeyModifiers::NONE); // user chooses Tui
    assert_eq!(app.input_focus, InputFocus::Tui);
    assert!(
        app.focus_return.is_none(),
        "explicit choice cancels restore"
    );
}

#[test]
fn f5_keeps_pty_focus_so_typing_still_reaches_the_agent() {
    use crossterm::event::{KeyCode, KeyModifiers};
    let mut app = fresh_app();
    app.has_pty = true;
    app.input_focus = InputFocus::Pty;
    app.on_key(KeyCode::F(5), KeyModifiers::NONE);
    assert_eq!(app.sidebar_view, SidebarView::Probes);
    assert_eq!(
        app.input_focus,
        InputFocus::Pty,
        "view flip must not steal focus"
    );
    app.on_key(KeyCode::F(5), KeyModifiers::NONE);
    assert_eq!(app.sidebar_view, SidebarView::Chat);
    assert_eq!(app.input_focus, InputFocus::Pty);
}

#[test]
fn permission_popup_s_approves_all_for_the_session() {
    use crossterm::event::{KeyCode, KeyModifiers};
    let mut app = fresh_app();
    let (prompt, mut rx) = permission_prompt(Uuid::new_v4());
    app.on_event(crate::tui::event::TuiEvent::PermissionRequest(prompt));
    app.on_key(KeyCode::Char('s'), KeyModifiers::NONE);
    assert_eq!(
        rx.try_recv().unwrap(),
        crate::permissions::Decision::AllowAllSession
    );
}

#[test]
fn permission_cursor_reaches_the_session_wide_option() {
    use crossterm::event::{KeyCode, KeyModifiers};
    let mut app = fresh_app();
    let (prompt, mut rx) = permission_prompt(Uuid::new_v4());
    app.on_event(crate::tui::event::TuiEvent::PermissionRequest(prompt));
    for _ in 0..2 {
        app.on_key(KeyCode::Down, KeyModifiers::NONE);
    }
    app.on_key(KeyCode::Enter, KeyModifiers::NONE);
    assert_eq!(
        rx.try_recv().unwrap(),
        crate::permissions::Decision::AllowAllSession
    );
}

// ── What the agent is handed when a probe lands ────────────────────────────

fn probe_result(command: &str, stdout: &str) -> ProbeResultInfo {
    ProbeResultInfo {
        probe_id: Uuid::new_v4(),
        command: command.to_string(),
        stdout: stdout.to_string(),
        exit_code: Some(0),
        duration_ms: 5,
        error: None,
        task_id: None,
        task_ordinal: 1,
        task_title: "Build the ledger".to_string(),
        task_description: "A page of scenarios the agent already has.".to_string(),
        test_ordinal: 5,
        test_total: 6,
        test_label: String::new(),
        test_description: String::new(),
        deadline_secs: Some(900),
        expected_answer: None,
        answer_template: String::new(),
        validation_kind: ValidationKind::Minijinja,
        outcome: None,
        point_delta: None,
        graded_expected: None,
    }
}

const ARTIFACT_COMMAND: &str = "# ARTIFACT REQUEST from ux-review: Capture the ledger page at 1280px as **desktop.png**, and the same page at 375px as **mobile.png**.\n# Save the file(s) (up to 5) under .ololo/artifacts/92afc917/; the ololo CLI commits and pushes them automatically; do NOT run git.\n# If an earlier delivered capture already shows exactly this, copying that file into this folder is a valid delivery.\ntest -n \"$(ls -A .ololo/artifacts/92afc917/ 2>/dev/null)\" && echo delivered || echo \"waiting-for-file: save the capture into .ololo/artifacts/92afc917/\"";

#[test]
fn an_artifact_request_reads_as_a_request_not_a_failed_check() {
    let text = crate::tui::app::probe_paste_text(&probe_result(
        ARTIFACT_COMMAND,
        "waiting-for-file: save the capture into .ololo/artifacts/92afc917/",
    ));

    assert!(
        text.starts_with("Artifact request from ux-review"),
        "{text}"
    );
    assert!(
        text.contains("**desktop.png**"),
        "the instruction survives: {text}"
    );
    assert!(text.contains(".ololo/artifacts/92afc917/"), "{text}");
    assert!(text.contains("Deliver within 15 min"), "{text}");
    // None of the shell that polls for the file, and no verdict language.
    assert!(!text.contains("test -n"), "{text}");
    assert!(!text.contains("Status: fail"), "{text}");
    assert!(!text.contains("Expected:"), "{text}");
}

#[test]
fn a_probe_result_carries_the_task_brief() {
    // A failing check is unreadable without what the task asked for — the
    // brief may be hundreds of lines up the agent's scrollback by now.
    let text = crate::tui::app::probe_paste_text(&probe_result("npm test", "1 failing"));
    assert!(
        text.contains("Probe result — Build the ledger (probe 5/6)"),
        "{text}"
    );
    assert!(
        text.contains("Task description: A page of scenarios"),
        "{text}"
    );
    assert!(text.contains("Command: npm test"), "{text}");
    assert!(text.contains("Actual: 1 failing"), "{text}");
}

#[test]
fn an_artifact_request_hands_the_agent_the_full_instruction() {
    // The command's header folds the ask onto one shell-safe line; the
    // description is the judge's text as written. The agent reads the
    // latter — the file list, one per line, is the whole point.
    let mut p = probe_result(
        ARTIFACT_COMMAND,
        "waiting-for-file: save the capture into .ololo/artifacts/92afc917/",
    );
    p.test_description = "Capture the ledger page:\n\n1. **desktop.png** at 1280px.\n2. \
                          **mobile.png** at 375px, Bangkok's card visible."
        .to_string();
    let text = crate::tui::app::probe_paste_text(&p);
    assert!(
        text.contains("1. **desktop.png** at 1280px.\n2. **mobile.png** at 375px"),
        "{text}"
    );
    assert!(
        !text.contains("Capture the ledger page at 1280px as"),
        "{text}"
    );
}

#[test]
fn an_artifact_request_still_leaves_the_brief_out() {
    // The one paste where the brief buried the content: the request itself
    // is the instruction, and it names everything the agent must do.
    let text = crate::tui::app::probe_paste_text(&probe_result(
        ARTIFACT_COMMAND,
        "waiting-for-file: save the capture into .ololo/artifacts/92afc917/",
    ));
    assert!(!text.contains("A page of scenarios"), "{text}");
}

// ── The chat's status row: what is happening now, what comes next ────────

fn running(app: &mut TuiApp) {
    app.header.status = crate::tui::header::Status::Running;
}

#[test]
fn live_status_is_silent_until_the_server_says_something() {
    let mut app = fresh_app();
    running(&mut app);
    assert_eq!(app.live_status(), None);
}

#[test]
fn live_status_counts_down_to_the_next_check_after_a_grade() {
    let mut app = fresh_app();
    running(&mut app);
    let tid = Uuid::new_v4();
    let pid = arrive(&mut app, tid, 0, "Setup");
    let status = app.live_status().expect("a probe in flight is news");
    assert!(
        status.text.contains("checking your code now"),
        "in flight: {status:?}"
    );
    assert!(status.busy);

    let mut done = chat_probe(tid, 0, 1, "desc");
    done.probe_id = pid;
    done.outcome = None;
    app.on_event(TuiEvent::ProbeResult(done));
    let status = app.live_status().expect("answered, ungraded");
    assert!(
        status.text.contains("ololo is grading it"),
        "answered: {status:?}"
    );

    app.on_event(TuiEvent::ProbeGraded {
        next_probe_in_secs: Some(30),
        probe_id: pid,
        outcome: arena_core::protocol::ProbeOutcome::Pass,
        point_delta: 5,
        expected: None,
        actual: None,
    });
    let status = app.live_status().expect("the server named the next check");
    assert!(
        status.text.starts_with("next check of your code in "),
        "countdown: {status:?}"
    );
    assert!(matches!(status.countdown, Some(29..=30)), "{status:?}");
    assert!(!status.busy, "waiting is not working");

    // The awaited probe arrives: the clock is gone, the check is in flight.
    arrive(&mut app, tid, 0, "Setup");
    assert_eq!(app.next_probe_due, None);
    assert!(
        app.live_status()
            .expect("in flight again")
            .text
            .contains("checking your code now")
    );
}

#[test]
fn live_status_names_the_judges_at_work_until_they_report() {
    let mut app = fresh_app();
    running(&mut app);
    let t0 = Uuid::new_v4();
    let t1 = Uuid::new_v4();
    // Task #0 is over; task #1 runs while #0's judges read.
    app.on_event(TuiEvent::ProbeResult(chat_probe(t0, 0, 1, "a")));
    app.on_event(TuiEvent::ProbeResult(chat_probe(t1, 1, 1, "b")));
    app.on_event(TuiEvent::JudgeStarted {
        task_id: Some(t0),
        judge_name: "Correctness".to_string(),
    });
    app.on_event(TuiEvent::JudgeStarted {
        task_id: Some(t0),
        judge_name: "Data".to_string(),
    });
    let status = app.live_status().expect("judges at work");
    assert_eq!(
        status.text,
        "evaluation in progress — Correctness and Data reviewing task #0"
    );
    assert!(status.busy);

    // One verdict in: the other judge is still reading.
    app.on_event(TuiEvent::JudgeScored {
        task_id: Some(t0),
        judge_name: "Correctness".to_string(),
        point_delta: 12,
        feedback: String::new(),
    });
    assert_eq!(
        app.live_status().unwrap().text,
        "evaluation in progress — Data reviewing task #0"
    );
    // A failed run lets go of the judge too — and with every run settled,
    // the row says so.
    app.on_event(TuiEvent::JudgeFailed {
        task_id: Some(t0),
        judge_name: "Data".to_string(),
    });
    let status = app.live_status().expect("judges done");
    assert_eq!(status.text, "judges are done with task #0");
    assert!(!status.busy);

    // The countdown joins the judges' whereabouts on one row.
    app.on_event(TuiEvent::JudgeStarted {
        task_id: Some(t1),
        judge_name: "Creativity".to_string(),
    });
    app.next_probe_due = Some(std::time::Instant::now() + std::time::Duration::from_secs(20));
    let status = app.live_status().unwrap();
    assert!(
        status.text.starts_with(
            "evaluation in progress — Creativity reviewing task #1 · next check of your code in "
        ),
        "{status:?}"
    );
    assert!(status.busy);
}

#[test]
fn a_verdict_without_a_task_id_settles_the_oldest_run_of_that_judge() {
    let mut app = fresh_app();
    running(&mut app);
    let t0 = Uuid::new_v4();
    app.on_event(TuiEvent::ProbeResult(chat_probe(t0, 0, 1, "a")));
    app.on_event(TuiEvent::JudgeStarted {
        task_id: Some(t0),
        judge_name: "Data".to_string(),
    });
    // Pre-upgrade servers name no task on the verdict.
    app.on_event(TuiEvent::JudgeScored {
        task_id: None,
        judge_name: "Data".to_string(),
        point_delta: 1,
        feedback: String::new(),
    });
    assert_eq!(
        app.live_status().unwrap().text,
        "judges are done with task #0"
    );
    // The verdict is pinned to the task in play, as before.
    assert_eq!(app.judge_verdicts[0].task_ordinal, Some(0));
}

#[test]
fn the_judge_hold_of_a_delivered_task_is_the_status_rows_story() {
    let mut app = fresh_app();
    running(&mut app);
    let t0 = Uuid::new_v4();
    app.on_event(TuiEvent::ProbeResult(chat_probe(t0, 0, 1, "brief")));
    app.on_event(TuiEvent::SnapshotRequested {
        task_id: t0,
        task_title: "Widget".to_string(),
        reason: "todo_complete".to_string(),
    });
    let status = app.live_status().expect("the panel has the build");
    assert_eq!(
        status.text,
        "task delivered — the judge panel is reviewing your delivery…"
    );
    app.on_event(TuiEvent::JudgeStarted {
        task_id: Some(t0),
        judge_name: "Architecture".to_string(),
    });
    assert_eq!(
        app.live_status().unwrap().text,
        "task delivered — Architecture reviewing task #0…"
    );
    // The transcript no longer repeats it as a system line.
    assert!(
        !app.chat_transcript()
            .iter()
            .any(|m| matches!(m, ChatMsg::System { text } if text.contains("reviewing"))),
        "the status row owns the judge-phase narration"
    );
}

#[test]
fn live_status_pauses_with_the_session_and_ends_with_it() {
    let mut app = fresh_app();
    app.header.status = crate::tui::header::Status::Paused;
    app.next_probe_due = Some(std::time::Instant::now() + std::time::Duration::from_secs(20));
    assert!(
        app.live_status()
            .unwrap()
            .text
            .starts_with("session paused")
    );
    app.header.status = crate::tui::header::Status::Complete;
    assert_eq!(
        app.live_status(),
        None,
        "the transcript's closing line says it"
    );
    // All tasks done: only worth a row while judges are still reading.
    app.header.status = crate::tui::header::Status::TasksDone;
    assert_eq!(app.live_status(), None);
    app.on_event(TuiEvent::JudgeStarted {
        task_id: None,
        judge_name: "Data".to_string(),
    });
    let status = app.live_status().unwrap();
    assert!(
        status
            .text
            .starts_with("all your tasks are done — Data reviewing your code")
    );
}
