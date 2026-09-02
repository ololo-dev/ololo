use super::*;

#[test]
fn sidebar_renders_probe_deadline_countdown() {
    let mut app = fresh_app(120, 40);
    app.sidebar_view = crate::tui::app::SidebarView::Probes;
    let pid = uuid::Uuid::new_v4();
    app.on_event(crate::tui::event::TuiEvent::ProbeArrived(
        crate::tui::event::ProbeInfo {
            probe_id: pid,
            rendered_command: "echo hi".to_string(),
            deadline_secs: 42,
            task_id: None,
            task_ordinal: 0,
            task_title: "MyTask".to_string(),
            task_description: "desc".to_string(),
            test_ordinal: 0,
            test_total: 0,
            test_label: String::new(),
            test_description: String::new(),
            expected_answer: None,
            answer_template: String::new(),
            validation_kind: ValidationKind::Minijinja,
        },
    ));
    let flat = header_flat(&app, 120, 40);
    assert!(flat.contains("42"), "sidebar must show deadline countdown");
    assert!(flat.contains("MyTask"), "sidebar must show task title");
}

#[test]
fn sidebar_renders_fail_when_server_grades_error() {
    // Server graded this probe as Error (exit 0 but wrong stdout
    // per the server's fixture-aware validation). The panel must
    // show FAIL — the server is the sole grader.
    let mut app = fresh_app(120, 40);
    app.sidebar_view = crate::tui::app::SidebarView::Probes;
    let pid = uuid::Uuid::new_v4();
    app.on_event(crate::tui::event::TuiEvent::ProbeResult(mk_result(
        pid,
        "echo hello",
        "world",
        None,
        0,
    )));
    app.on_event(crate::tui::event::TuiEvent::ProbeGraded {
        next_probe_in_secs: None,
        probe_id: pid,
        outcome: arena_core::protocol::ProbeOutcome::Error,
        point_delta: -5,
        expected: Some("hello".to_string()),
        actual: Some("world".to_string()),
    });
    let flat = header_flat(&app, 120, 40);
    assert!(
        flat.contains("✗"),
        "sidebar must show ✗ when server grades Error (got: {flat:?})"
    );
    assert!(
        flat.contains("Expected: hello"),
        "sidebar must show the expected value (got: {flat:?})"
    );
    assert!(
        flat.contains("Actual: world"),
        "sidebar must show the actual value (got: {flat:?})"
    );
    assert!(
        !flat.contains("✓"),
        "sidebar must NOT show ✓ when server grades Error (got: {flat:?})"
    );
}

#[test]
fn sidebar_renders_pass_when_server_grades_pass() {
    let mut app = fresh_app(120, 40);
    app.sidebar_view = crate::tui::app::SidebarView::Probes;
    let pid = uuid::Uuid::new_v4();
    let tid = uuid::Uuid::new_v4();
    app.on_event(crate::tui::event::TuiEvent::ProbeResult(mk_result(
        pid,
        "echo hello",
        "hello",
        Some(tid),
        1,
    )));
    app.on_event(crate::tui::event::TuiEvent::ProbeGraded {
        next_probe_in_secs: None,
        probe_id: pid,
        outcome: arena_core::protocol::ProbeOutcome::Pass,
        point_delta: 10,
        expected: Some("hello".to_string()),
        actual: Some("hello".to_string()),
    });
    // Scheduler advances to task 2 — marks task 1 as done.
    let pid2 = uuid::Uuid::new_v4();
    let tid2 = uuid::Uuid::new_v4();
    app.on_event(crate::tui::event::TuiEvent::ProbeArrived(
        crate::tui::event::ProbeInfo {
            probe_id: pid2,
            rendered_command: "echo next".to_string(),
            deadline_secs: 30,
            task_id: Some(tid2),
            task_ordinal: 2,
            task_title: "Next".to_string(),
            task_description: String::new(),
            test_ordinal: 0,
            test_total: 0,
            test_label: String::new(),
            test_description: String::new(),
            expected_answer: None,
            answer_template: String::new(),
            validation_kind: ValidationKind::Minijinja,
        },
    ));
    let flat = header_flat(&app, 120, 40);
    // ponytail: passed tasks collapse to a 1-line "✓ <title>" summary,
    // so the per-probe "PASS" badge no longer renders. The summary line
    // is the new signal that the probe (and task) passed.
    assert!(
        flat.contains("✓ T"),
        "sidebar must show ✓ summary when task is done (got: {flat:?})"
    );
    assert!(
        flat.contains("1/2"),
        "sidebar header must show passed/total task count 1/2 (got: {flat:?})"
    );
}

#[test]
fn sidebar_collapses_passed_task_and_keeps_active_task_expanded() {
    // Task A (ordinal 1): both probes graded Pass → collapses to 1-line ✓ summary.
    // Task B (ordinal 2): one probe graded Error → stays expanded with FAIL line.
    let mut app = fresh_app(120, 40);
    app.sidebar_view = crate::tui::app::SidebarView::Probes;

    // Task A — probe 1, passed.
    let pid_a1 = uuid::Uuid::new_v4();
    app.on_event(crate::tui::event::TuiEvent::ProbeArrived(
        crate::tui::event::ProbeInfo {
            probe_id: pid_a1,
            rendered_command: "echo a1".to_string(),
            deadline_secs: 30,
            task_id: Some(uuid::Uuid::new_v4()),
            task_ordinal: 1,
            task_title: "Task A".to_string(),
            task_description: String::new(),
            test_ordinal: 0,
            test_total: 0,
            test_label: String::new(),
            test_description: String::new(),
            expected_answer: None,
            answer_template: String::new(),
            validation_kind: ValidationKind::Minijinja,
        },
    ));
    app.on_event(crate::tui::event::TuiEvent::ProbeGraded {
        next_probe_in_secs: None,
        probe_id: pid_a1,
        outcome: arena_core::protocol::ProbeOutcome::Pass,
        point_delta: 10,
        expected: None,
        actual: None,
    });

    // Task B — probe 2, failed.
    let pid_b1 = uuid::Uuid::new_v4();
    app.on_event(crate::tui::event::TuiEvent::ProbeArrived(
        crate::tui::event::ProbeInfo {
            probe_id: pid_b1,
            rendered_command: "echo b1".to_string(),
            deadline_secs: 30,
            task_id: Some(uuid::Uuid::new_v4()),
            task_ordinal: 2,
            task_title: "Task B".to_string(),
            task_description: String::new(),
            test_ordinal: 0,
            test_total: 0,
            test_label: String::new(),
            test_description: String::new(),
            expected_answer: None,
            answer_template: String::new(),
            validation_kind: ValidationKind::Minijinja,
        },
    ));
    app.on_event(crate::tui::event::TuiEvent::ProbeResult(
        crate::tui::event::ProbeResultInfo {
            probe_id: pid_b1,
            command: "echo b1".to_string(),
            stdout: "wrong".to_string(),
            exit_code: Some(1),
            duration_ms: 5,
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
        },
    ));
    app.on_event(crate::tui::event::TuiEvent::ProbeGraded {
        next_probe_in_secs: None,
        probe_id: pid_b1,
        outcome: arena_core::protocol::ProbeOutcome::Error,
        point_delta: -5,
        expected: Some("right".to_string()),
        actual: Some("wrong".to_string()),
    });

    let flat = header_flat(&app, 120, 40);
    // Header shows 1 of 2 tasks passed.
    assert!(
        flat.contains("1/2"),
        "sidebar header must show 1/2 tasks passed (got: {flat:?})"
    );
    // Task A collapsed: ✓ summary line present, per-probe command hidden.
    assert!(
        flat.contains("✓ Task A"),
        "passed task shows ✓ summary (got: {flat:?})"
    );
    assert!(
        !flat.contains("echo a1"),
        "passed task's probe command must be hidden (got: {flat:?})"
    );
    // Task B expanded: ✗ result entry with Expected/Actual values.
    assert!(flat.contains("✗"), "active task shows ✗ (got: {flat:?})");
    assert!(
        flat.contains("Expected: right"),
        "active task shows expected value (got: {flat:?})"
    );
    assert!(
        flat.contains("Actual: wrong"),
        "active task shows actual value (got: {flat:?})"
    );
    assert!(
        flat.contains("Task B"),
        "active task title still rendered (got: {flat:?})"
    );
}

#[test]
fn sidebar_folds_task_zero_when_scheduler_advances() {
    // Projects number tasks from 0 (`0.setup.md` → ordinal 0). When the
    // scheduler advances to task 1, task 0 must fold to a ✓ summary —
    // ordinal 0 is a real task, not the "unknown" sentinel.
    let mut app = fresh_app(120, 40);
    app.sidebar_view = crate::tui::app::SidebarView::Probes;
    let pid0 = uuid::Uuid::new_v4();
    app.on_event(crate::tui::event::TuiEvent::ProbeArrived(
        crate::tui::event::ProbeInfo {
            probe_id: pid0,
            rendered_command: "echo setup".to_string(),
            deadline_secs: 30,
            task_id: Some(uuid::Uuid::new_v4()),
            task_ordinal: 0,
            task_title: "Setup".to_string(),
            task_description: "bring up serve.sh".to_string(),
            test_ordinal: 1,
            test_total: 1,
            test_label: String::new(),
            test_description: String::new(),
            expected_answer: None,
            answer_template: String::new(),
            validation_kind: ValidationKind::Minijinja,
        },
    ));
    app.on_event(crate::tui::event::TuiEvent::ProbeGraded {
        next_probe_in_secs: None,
        probe_id: pid0,
        outcome: arena_core::protocol::ProbeOutcome::Pass,
        point_delta: 10,
        expected: None,
        actual: None,
    });
    // Scheduler advances to task 1.
    app.on_event(crate::tui::event::TuiEvent::ProbeArrived(
        crate::tui::event::ProbeInfo {
            probe_id: uuid::Uuid::new_v4(),
            rendered_command: "echo add".to_string(),
            deadline_secs: 30,
            task_id: Some(uuid::Uuid::new_v4()),
            task_ordinal: 1,
            task_title: "Addition".to_string(),
            task_description: "answer addition questions".to_string(),
            test_ordinal: 1,
            test_total: 1,
            test_label: String::new(),
            test_description: String::new(),
            expected_answer: None,
            answer_template: String::new(),
            validation_kind: ValidationKind::Minijinja,
        },
    ));
    let flat = header_flat(&app, 120, 40);
    assert!(
        flat.contains("✓ Setup"),
        "task 0 must fold to ✓ summary once the scheduler advances (got: {flat:?})"
    );
    assert!(
        !flat.contains("bring up serve.sh"),
        "folded task 0 must hide its probe entries (got: {flat:?})"
    );
    assert!(
        flat.contains("1/2"),
        "header must count task 0 as passed (got: {flat:?})"
    );
}

#[test]
fn sidebar_excludes_synthetic_probes_from_task_count() {
    // A MemberJoined synthetic probe (task_id: None) must NOT count
    // as a task in the passed/total header.
    let mut app = fresh_app(120, 40);
    app.sidebar_view = crate::tui::app::SidebarView::Probes;
    app.on_event(crate::tui::event::TuiEvent::MemberJoined {
        name: "alice".to_string(),
    });
    let flat = header_flat(&app, 120, 40);
    // No real tasks → header shows no task count segment, just "probes".
    assert!(
        !flat.contains("0/1"),
        "synthetic probe must not count as a task (got: {flat:?})"
    );
    assert!(
        !flat.contains("1/1"),
        "synthetic probe must not count as a passed task (got: {flat:?})"
    );
    // The synthetic probe is still rendered as a standalone block
    // (its join message is visible as the description line).
    assert!(
        flat.contains("joined"),
        "synthetic probe body still rendered (got: {flat:?})"
    );
}

#[test]
fn sidebar_uses_server_total_tasks_in_header() {
    // Server reports 17 total tasks via SessionStarted. Task 1 is done
    // (scheduler advanced to task 2), task 2 is the current task — so
    // the header must show 1/17 (one passed of seventeen).
    let mut app = fresh_app(120, 40);
    app.sidebar_view = crate::tui::app::SidebarView::Probes;
    app.total_tasks = Some(17);
    let tid1 = uuid::Uuid::new_v4();
    let tid2 = uuid::Uuid::new_v4();
    // Task 1 probe — graded Pass, then scheduler moves to task 2.
    let pid1 = uuid::Uuid::new_v4();
    app.on_event(crate::tui::event::TuiEvent::ProbeArrived(
        crate::tui::event::ProbeInfo {
            probe_id: pid1,
            rendered_command: "echo ok".to_string(),
            deadline_secs: 30,
            task_id: Some(tid1),
            task_ordinal: 1,
            task_title: "Done".to_string(),
            task_description: String::new(),
            test_ordinal: 0,
            test_total: 0,
            test_label: String::new(),
            test_description: String::new(),
            expected_answer: None,
            answer_template: String::new(),
            validation_kind: ValidationKind::Minijinja,
        },
    ));
    app.on_event(crate::tui::event::TuiEvent::ProbeGraded {
        next_probe_in_secs: None,
        probe_id: pid1,
        outcome: arena_core::protocol::ProbeOutcome::Pass,
        point_delta: 10,
        expected: None,
        actual: None,
    });
    // Task 2 probe — current task, pending. Its arrival advances
    // max_task_ordinal to 2, marking task 1 as done.
    let pid2 = uuid::Uuid::new_v4();
    app.on_event(crate::tui::event::TuiEvent::ProbeArrived(
        crate::tui::event::ProbeInfo {
            probe_id: pid2,
            rendered_command: "echo current".to_string(),
            deadline_secs: 30,
            task_id: Some(tid2),
            task_ordinal: 2,
            task_title: "Current".to_string(),
            task_description: String::new(),
            test_ordinal: 0,
            test_total: 0,
            test_label: String::new(),
            test_description: String::new(),
            expected_answer: None,
            answer_template: String::new(),
            validation_kind: ValidationKind::Minijinja,
        },
    ));
    let flat = header_flat(&app, 120, 40);
    assert!(
        flat.contains("1/17"),
        "header must use server-reported total (1/17, got: {flat:?})"
    );
    assert!(
        !flat.contains("1/1 tasks"),
        "header must NOT fall back to grouped count (got: {flat:?})"
    );
    // Task 1 (Done) collapses to ✓ summary; task 2 (Current) expands.
    assert!(
        flat.contains("✓ Done"),
        "passed task collapses (got: {flat:?})"
    );
    assert!(
        flat.contains("Current"),
        "current task still rendered (got: {flat:?})"
    );
}

#[test]
fn sidebar_renders_sent_when_outcome_pending_and_exit_zero() {
    // No ProbeGraded frame yet — server hasn't graded. Panel must
    // show SENT (neutral), not PASS, so the user isn't misled into
    // thinking the probe passed before the server says so.
    let mut app = fresh_app(120, 40);
    app.sidebar_view = crate::tui::app::SidebarView::Probes;
    let pid = uuid::Uuid::new_v4();
    app.on_event(crate::tui::event::TuiEvent::ProbeResult(
        crate::tui::event::ProbeResultInfo {
            probe_id: pid,
            command: "echo hi".to_string(),
            stdout: "hi".to_string(),
            exit_code: Some(0),
            duration_ms: 5,
            error: None,
            task_id: None,
            task_ordinal: 0,
            task_title: "T".to_string(),
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
        },
    ));
    let flat = header_flat(&app, 120, 40);
    assert!(
        flat.contains("→"),
        "sidebar must show the neutral → (sent) glyph when outcome pending (got: {flat:?})"
    );
    assert!(
        !flat.contains("✓"),
        "sidebar must NOT show ✓ before the server grades (got: {flat:?})"
    );
    assert!(
        !flat.contains("✗"),
        "sidebar must NOT show ✗ before the server grades (got: {flat:?})"
    );
}

#[test]
fn sidebar_shows_probe_type_progress_in_task_header() {
    // TestPush carries test_ordinal/test_total → the task header line
    // must read "Title (2/4)".
    let mut app = fresh_app(120, 40);
    app.sidebar_view = crate::tui::app::SidebarView::Probes;
    app.on_event(crate::tui::event::TuiEvent::ProbeArrived(
        crate::tui::event::ProbeInfo {
            probe_id: uuid::Uuid::new_v4(),
            rendered_command: "echo hi".to_string(),
            deadline_secs: 30,
            task_id: Some(uuid::Uuid::new_v4()),
            task_ordinal: 1,
            task_title: "Trivia".to_string(),
            task_description: "Answer the question".to_string(),
            test_ordinal: 2,
            test_total: 4,
            test_label: String::new(),
            test_description: String::new(),
            expected_answer: None,
            answer_template: String::new(),
            validation_kind: ValidationKind::Minijinja,
        },
    ));
    let flat = header_flat(&app, 120, 40);
    assert!(
        flat.contains("Trivia (2/4)"),
        "task header must show probe-type progress (got: {flat:?})"
    );
}

#[test]
fn sidebar_wraps_long_description_without_cutting() {
    // A description longer than the sidebar width must wrap across
    // lines — never be truncated with an ellipsis. Terminal wide enough
    // that no single word exceeds the wrap width (hard-breaking longer
    // words is allowed and covered by wrap_text unit tests).
    let mut app = fresh_app(160, 40);
    app.sidebar_view = crate::tui::app::SidebarView::Probes;
    let desc = "implement the multiplication endpoint so that the server \
                answers product questions correctly every time zanzibar";
    app.on_event(crate::tui::event::TuiEvent::ProbeArrived(
        crate::tui::event::ProbeInfo {
            probe_id: uuid::Uuid::new_v4(),
            rendered_command: "echo hi".to_string(),
            deadline_secs: 30,
            task_id: Some(uuid::Uuid::new_v4()),
            task_ordinal: 1,
            task_title: "Multiply".to_string(),
            task_description: desc.to_string(),
            test_ordinal: 1,
            test_total: 1,
            test_label: String::new(),
            test_description: String::new(),
            expected_answer: None,
            answer_template: String::new(),
            validation_kind: ValidationKind::Minijinja,
        },
    ));
    let flat = header_flat(&app, 160, 40);
    // Every word survives the wrap — nothing is cut. ("…" can't be
    // asserted globally: other panes legitimately use it.)
    for word in desc.split_whitespace() {
        assert!(
            flat.contains(word),
            "description word {word:?} must be visible after wrapping (got: {flat:?})"
        );
    }
}

#[test]
fn probe_popup_renders_details_overlay() {
    let mut app = fresh_app(120, 40);
    app.sidebar_view = crate::tui::app::SidebarView::Probes;
    let pid = uuid::Uuid::new_v4();
    let tid = uuid::Uuid::new_v4();
    app.on_event(crate::tui::event::TuiEvent::ProbeArrived(
        crate::tui::event::ProbeInfo {
            probe_id: pid,
            rendered_command: "curl -s localhost:9999/q".to_string(),
            deadline_secs: 30,
            task_id: Some(tid),
            task_ordinal: 0,
            task_title: "Trivia".to_string(),
            task_description: "Answer the trivia question".to_string(),
            test_ordinal: 2,
            test_total: 4,
            test_label: String::new(),
            test_description: String::new(),
            expected_answer: None,
            answer_template: String::new(),
            validation_kind: ValidationKind::Minijinja,
        },
    ));
    app.on_event(crate::tui::event::TuiEvent::ProbeGraded {
        next_probe_in_secs: None,
        probe_id: pid,
        outcome: arena_core::protocol::ProbeOutcome::Error,
        point_delta: -5,
        expected: Some("Paris".to_string()),
        actual: Some("London".to_string()),
    });
    app.probe_popup = Some(pid);
    let flat = header_flat(&app, 120, 40);
    assert!(
        flat.contains("probe details"),
        "popup frame (got: {flat:?})"
    );
    assert!(
        flat.contains("Trivia — probe 2/4"),
        "popup task context (got: {flat:?})"
    );
    assert!(flat.contains("FAIL"), "popup status (got: {flat:?})");
    assert!(
        flat.contains("curl -s localhost:9999/q"),
        "popup shows the command (got: {flat:?})"
    );
    assert!(
        flat.contains("Expected: Paris"),
        "popup expected (got: {flat:?})"
    );
    assert!(
        flat.contains("Actual: London"),
        "popup actual (got: {flat:?})"
    );
    assert!(
        flat.contains("Esc: close"),
        "popup close hint (got: {flat:?})"
    );
}

#[test]
fn sidebar_task_header_shows_total_points() {
    let mut app = fresh_app(120, 40);
    app.sidebar_view = crate::tui::app::SidebarView::Probes;
    let pid = uuid::Uuid::new_v4();
    app.on_event(crate::tui::event::TuiEvent::ProbeArrived(
        crate::tui::event::ProbeInfo {
            probe_id: pid,
            rendered_command: "echo hi".to_string(),
            deadline_secs: 30,
            task_id: Some(uuid::Uuid::new_v4()),
            task_ordinal: 0,
            task_title: "Trivia".to_string(),
            task_description: "answer".to_string(),
            test_ordinal: 1,
            test_total: 2,
            test_label: String::new(),
            test_description: String::new(),
            expected_answer: None,
            answer_template: String::new(),
            validation_kind: ValidationKind::Minijinja,
        },
    ));
    app.on_event(crate::tui::event::TuiEvent::ProbeGraded {
        next_probe_in_secs: None,
        probe_id: pid,
        outcome: arena_core::protocol::ProbeOutcome::Pass,
        point_delta: 10,
        expected: None,
        actual: None,
    });
    let flat = header_flat(&app, 120, 40);
    assert!(
        flat.contains("Trivia (1/2) +10"),
        "task header must show the graded point total (got: {flat:?})"
    );
}

#[test]
fn sidebar_unfold_override_expands_passed_task() {
    // A passed task auto-folds, but a manual unfold override must expand
    // its probe entries again.
    let mut app = fresh_app(120, 40);
    app.sidebar_view = crate::tui::app::SidebarView::Probes;
    let tid = uuid::Uuid::new_v4();
    let pid = uuid::Uuid::new_v4();
    app.on_event(crate::tui::event::TuiEvent::ProbeArrived(
        crate::tui::event::ProbeInfo {
            probe_id: pid,
            rendered_command: "echo setup".to_string(),
            deadline_secs: 30,
            task_id: Some(tid),
            task_ordinal: 0,
            task_title: "Setup".to_string(),
            task_description: "bring up serve.sh".to_string(),
            test_ordinal: 1,
            test_total: 1,
            test_label: String::new(),
            test_description: String::new(),
            expected_answer: None,
            answer_template: String::new(),
            validation_kind: ValidationKind::Minijinja,
        },
    ));
    app.on_event(crate::tui::event::TuiEvent::ProbeGraded {
        next_probe_in_secs: None,
        probe_id: pid,
        outcome: arena_core::protocol::ProbeOutcome::Pass,
        point_delta: 10,
        expected: None,
        actual: None,
    });
    // Scheduler advances — task 0 passes and auto-folds.
    app.on_event(crate::tui::event::TuiEvent::ProbeArrived(
        crate::tui::event::ProbeInfo {
            probe_id: uuid::Uuid::new_v4(),
            rendered_command: "echo add".to_string(),
            deadline_secs: 30,
            task_id: Some(uuid::Uuid::new_v4()),
            task_ordinal: 1,
            task_title: "Addition".to_string(),
            task_description: "answer addition".to_string(),
            test_ordinal: 1,
            test_total: 1,
            test_label: String::new(),
            test_description: String::new(),
            expected_answer: None,
            answer_template: String::new(),
            validation_kind: ValidationKind::Minijinja,
        },
    ));
    let folded = header_flat(&app, 120, 40);
    assert!(
        !folded.contains("bring up serve.sh"),
        "auto-folded task hides entries (got: {folded:?})"
    );
    app.fold_overrides.insert(tid, false);
    let unfolded = header_flat(&app, 120, 40);
    assert!(
        unfolded.contains("bring up serve.sh"),
        "unfold override expands entries (got: {unfolded:?})"
    );
    assert!(
        unfolded.contains("▾ ✓ Setup"),
        "unfolded passed task keeps ✓ and shows ▾ marker (got: {unfolded:?})"
    );
}

#[test]
fn sidebar_renders_progress_when_present() {
    let mut app = fresh_app(120, 40);
    app.sidebar_view = crate::tui::app::SidebarView::Probes;
    app.progress_attempt = Some(3);
    app.progress_status = Some(crate::tui::event::PlayerRunStatus::Backoff);
    let flat = header_flat(&app, 120, 40);
    assert!(
        flat.contains("attempt=3"),
        "sidebar must show progress attempt"
    );
}
