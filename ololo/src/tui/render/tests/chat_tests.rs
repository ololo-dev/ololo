//! Chat view (F5) rendering: transcript content and pane chrome.

use super::*;
use crate::tui::app::SidebarView;

fn task_probe(ordinal: i32, title: &str, desc: &str) -> crate::tui::event::ProbeResultInfo {
    let mut p = mk_result(
        uuid::Uuid::new_v4(),
        "echo hi",
        "done-note: present",
        Some(uuid::Uuid::new_v4()),
        ordinal,
    );
    p.task_title = title.to_string();
    p.task_description = desc.to_string();
    p.test_ordinal = 1;
    p.test_total = 1;
    p.outcome = Some(arena_core::protocol::ProbeOutcome::Pass);
    p
}

#[test]
fn chat_view_renders_task_header_check_and_verdict() {
    let mut app = fresh_app(120, 40);
    app.sidebar_view = SidebarView::Chat;
    // The sidebar is ~20% of the terminal — keep fixture strings short
    // enough to survive truncation at that width.
    app.on_event(crate::tui::event::TuiEvent::ProbeResult(task_probe(
        0,
        "Wx",
        "done-note",
    )));
    app.on_event(crate::tui::event::TuiEvent::JudgeScored {
        task_id: None,
        judge_name: "Creativity".to_string(),
        point_delta: 17,
        feedback: "goes beyond".to_string(),
    });
    let flat = header_flat(&app, 120, 40);
    assert!(flat.contains("chat"), "pane title must say chat");
    assert!(flat.contains("TASK #0"), "task header missing");
    assert!(flat.contains("Wx"), "task title missing");
    assert!(flat.contains("done-note"), "check line missing");
    assert!(flat.contains("Creativity"), "judge verdict missing");
    assert!(flat.contains("+17"), "verdict points missing");
    assert!(flat.contains("goes beyond"), "verdict feedback missing");
}

#[test]
fn chat_view_is_the_default() {
    let mut app = fresh_app(120, 40);
    app.on_event(crate::tui::event::TuiEvent::ProbeResult(task_probe(
        0,
        "Wx",
        "done-note",
    )));
    let flat = header_flat(&app, 120, 40);
    assert!(flat.contains("chat"), "default pane is the chat transcript");
    assert!(flat.contains("TASK #0"), "chat task header present");
}

#[test]
fn a_failed_check_tells_what_it_verifies_and_shows_expected_and_got() {
    let mut app = fresh_app(120, 40);
    app.sidebar_view = SidebarView::Chat;
    let mut p = task_probe(0, "Wx", "brief");
    p.test_label = "Current weather line".to_string();
    p.test_description = "The widget must print the city's weather line.".to_string();
    p.stdout = "London: 10".to_string();
    p.graded_expected = Some("Paris: 20".to_string());
    p.outcome = Some(arena_core::protocol::ProbeOutcome::Error);
    app.on_event(crate::tui::event::TuiEvent::ProbeResult(p));

    let flat = header_flat(&app, 120, 40);
    assert!(
        flat.contains("Current weather line"),
        "the test's own label heads the bubble: {flat}"
    );
    assert!(
        flat.contains("must print the city's"),
        "the author's explanation of the check is shown: {flat}"
    );
    assert!(
        flat.contains("↳ London: 10"),
        "the answer the check got is shown: {flat}"
    );
    assert!(
        flat.contains("expected: Paris: 20"),
        "the expected value is shown on a fail: {flat}"
    );
}

#[test]
fn a_failed_check_without_output_still_owes_its_got_side() {
    let mut app = fresh_app(120, 40);
    app.sidebar_view = SidebarView::Chat;
    let mut p = task_probe(0, "Wx", "brief");
    p.test_label = "Forecast endpoint".to_string();
    p.stdout = String::new();
    p.expected_answer = Some("ok".to_string());
    p.outcome = Some(arena_core::protocol::ProbeOutcome::Error);
    app.on_event(crate::tui::event::TuiEvent::ProbeResult(p));

    let flat = header_flat(&app, 120, 40);
    assert!(
        flat.contains("(no answer)"),
        "empty output is named: {flat}"
    );
    assert!(
        flat.contains("expected: ok"),
        "expected shown even without an answer: {flat}"
    );
}

#[test]
fn a_judge_registered_check_names_the_judge_instead_of_the_machine_label() {
    let mut app = fresh_app(120, 40);
    app.sidebar_view = SidebarView::Chat;
    let mut p = task_probe(0, "Wx", "brief");
    p.test_label = "registered: correctness".to_string();
    app.on_event(crate::tui::event::TuiEvent::ProbeResult(p));

    let flat = header_flat(&app, 120, 40);
    assert!(
        flat.contains("extra check from the correctness judge"),
        "the machine label is retold in plain words: {flat}"
    );
    assert!(
        !flat.contains("registered:"),
        "the raw label stays out of the chat: {flat}"
    );
}

#[test]
fn a_click_selects_a_bubble_and_a_second_click_sends_it() {
    use crate::tui::render::chat::{bubble_at, chat_area, compose_bar_row};
    let mut app = fresh_app(120, 40);
    app.has_pty = true;
    app.sidebar_view = SidebarView::Chat;
    let mut p = task_probe(0, "Wx", "brief");
    p.test_label = "Current weather line".to_string();
    p.outcome = Some(arena_core::protocol::ProbeOutcome::Error);
    app.on_event(crate::tui::event::TuiEvent::ProbeResult(p));

    let area = chat_area(&app, 120, 40).expect("chat pane is on screen");
    let col = area.x + area.width / 2;
    // Sweep the pane's rows: the transcript is bottom-anchored, so bubbles
    // sit above the compose bar; every hit maps to a message index.
    let hits: Vec<(u16, usize)> = (0..40u16)
        .filter_map(|row| bubble_at(&app, 120, 40, col, row).map(|i| (row, i)))
        .collect();
    assert!(!hits.is_empty(), "some row must hit a bubble");
    let last_idx = app.chat_transcript().len() - 1;
    let (row, idx) = *hits
        .iter()
        .rev()
        .find(|(_, i)| *i == last_idx)
        .expect("the newest bubble is clickable");
    assert_ne!(
        compose_bar_row(&app, 120, 40),
        Some(row),
        "a bubble row is not the compose bar"
    );

    app.chat_click_bubble(idx);
    assert_eq!(
        app.chat_cursor,
        Some(0),
        "first click selects (cursor counts from the newest)"
    );
    assert_eq!(
        app.input_focus,
        crate::tui::app::InputFocus::Tui,
        "a click pulls TUI focus so keys work without F9"
    );
    assert!(app.pty_paste_pending.is_none(), "one click does not send");

    app.chat_click_bubble(idx);
    let text = app.pty_paste_pending.take().expect("second click sends");
    assert!(
        text.contains("Current weather line"),
        "the bubble's own retelling travels: {text}"
    );
    assert_eq!(
        app.input_focus,
        crate::tui::app::InputFocus::Pty,
        "sending hands focus to the agent"
    );
}

#[test]
fn clicking_the_compose_bar_still_opens_the_compose_line() {
    use crate::tui::render::chat::compose_bar_row;
    let mut app = fresh_app(120, 40);
    app.has_pty = true;
    app.sidebar_view = SidebarView::Chat;
    app.on_event(crate::tui::event::TuiEvent::ProbeResult(task_probe(
        0, "Wx", "brief",
    )));
    let row = compose_bar_row(&app, 120, 40).expect("compose bar exists with a pty");
    // The run loop routes a click on this row to the compose line, and the
    // hit-test must not claim it as a bubble.
    assert_eq!(
        crate::tui::render::chat::bubble_at(&app, 120, 40, 100, row),
        None,
        "the compose bar row is not a bubble"
    );
}

#[test]
fn chat_view_renders_artifact_requests_in_plain_words() {
    let mut app = fresh_app(120, 40);
    let mut p = task_probe(0, "Wx", "brief text");
    p.command = "# ARTIFACT REQUEST from creativity: Capture the game as screenshot.png\n\
                 # Save the file(s) (up to 5) under .ololo/artifacts/abc/; the ololo CLI commits and pushes them.\n\
                 test -n \"$(ls -A .ololo/artifacts/abc/ 2>/dev/null)\" && echo present || echo missing"
        .to_string();
    p.outcome = None;
    app.on_event(crate::tui::event::TuiEvent::ProbeResult(p));

    let flat = header_flat(&app, 120, 40);
    assert!(
        flat.contains("creativity asks:"),
        "the judge speaks by name: {flat}"
    );
    assert!(
        flat.contains("[waiting]"),
        "undelivered request shows its state: {flat}"
    );
    assert!(
        !flat.contains("ls -A"),
        "the polling shell stays out of the chat: {flat}"
    );
}

#[test]
fn artifact_request_block_sits_on_its_own_background() {
    let mut app = fresh_app(120, 40);
    let mut p = task_probe(0, "Wx", "brief text");
    p.command = "# ARTIFACT REQUEST from creativity: Capture the game as screenshot.png\n\
                 # Save the file(s) (up to 5) under .ololo/artifacts/abc/; the ololo CLI commits and pushes them.\n\
                 test -n \"$(ls -A .ololo/artifacts/abc/ 2>/dev/null)\" && echo present || echo missing"
        .to_string();
    p.outcome = None;
    app.on_event(crate::tui::event::TuiEvent::ProbeResult(p));

    let backend = ratatui::backend::TestBackend::new(120, 40);
    let mut terminal = ratatui::Terminal::new(backend).unwrap();
    terminal.draw(|f| view(f, &app)).unwrap();
    let buffer = terminal.backend().buffer().clone();
    let tinted = (0..40u16).any(|y| {
        (0..120u16)
            .any(|x| buffer[(x, y)].style().bg == Some(ratatui::style::Color::Rgb(56, 44, 16)))
    });
    assert!(
        tinted,
        "the request bubble must render on its amber-ish background"
    );
}

#[test]
fn chat_pane_shows_the_message_button_and_the_input_line() {
    let mut app = fresh_app(120, 40);
    app.has_pty = true;
    app.on_event(crate::tui::event::TuiEvent::ProbeResult(task_probe(
        0, "Wx", "brief",
    )));
    let flat = header_flat(&app, 120, 40);
    assert!(
        flat.contains("message the agent"),
        "the compose button is visible: {flat}"
    );

    app.open_chat_compose();
    for c in "hello agent".chars() {
        app.on_key(
            crossterm::event::KeyCode::Char(c),
            crossterm::event::KeyModifiers::NONE,
        );
    }
    let flat = header_flat(&app, 120, 40);
    assert!(
        flat.contains("hello agent"),
        "typed text is visible in the input line: {flat}"
    );
    assert!(
        !flat.contains("message the agent"),
        "the button yields to the input line: {flat}"
    );
}

#[test]
fn chat_view_renders_the_players_done_note() {
    let mut app = fresh_app(120, 40);
    app.on_event(crate::tui::event::TuiEvent::ProbeResult(task_probe(
        0, "Wx", "brief",
    )));
    app.on_event(crate::tui::event::TuiEvent::CompletionFlagPublished {
        path: ".ololo/wx-done.md".to_string(),
        text: "Built the widget".to_string(),
    });

    let flat = header_flat(&app, 120, 40);
    assert!(
        flat.contains("you"),
        "the player's message carries their voice: {flat}"
    );
    assert!(
        flat.contains("Built the widget"),
        "the note's words survive: {flat}"
    );
}

/// A chat that ends sentences in `…` sends the reader to a popup to finish
/// them. These guard the "nothing is cut" contract for each message kind.
mod nothing_is_truncated {
    use super::*;

    /// The rendered pane as one string per row, so wrapped text can be
    /// searched across rows after joining.
    fn pane_rows(app: &TuiApp, w: u16, h: u16) -> Vec<String> {
        let backend = ratatui::backend::TestBackend::new(w, h);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        terminal.draw(|f| view(f, app)).unwrap();
        let buffer = terminal.backend().buffer().clone();
        (0..h)
            .map(|y| {
                (0..w)
                    .map(|x| buffer[(x, y)].symbol().to_string())
                    .collect()
            })
            .collect()
    }

    /// The pane's text with the box-drawing frame removed and whitespace
    /// normalised, so a sentence wrapped across rows can be searched as one
    /// string.
    fn flow(rows: &[String]) -> String {
        rows.join(" ")
            .chars()
            .map(|c| {
                if "│─┌┐└┘▌".contains(c) {
                    ' '
                } else {
                    c
                }
            })
            .collect::<String>()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// Only the sidebar pane's own columns, frame stripped — the agent pane
    /// has its own copy ("Waiting for lobby…") that must not be mistaken for
    /// a truncated chat message.
    fn sidebar_rows(rows: &[String]) -> Vec<String> {
        let left = sidebar_left(rows);
        rows.iter()
            .map(|r| {
                r.chars()
                    .skip(left)
                    .filter(|c| !"│─┌┐└┘▌".contains(*c))
                    .collect::<String>()
                    .trim_end()
                    .to_string()
            })
            .collect()
    }

    /// Column where the sidebar pane starts — its own top-left corner, not
    /// the agent pane's.
    fn sidebar_left(rows: &[String]) -> usize {
        rows.iter()
            .find_map(|r| r.rfind('┌'))
            .expect("sidebar pane is drawn")
    }

    #[test]
    fn a_long_check_description_wraps_instead_of_ending_in_an_ellipsis() {
        let mut app = fresh_app(120, 40);
        app.sidebar_view = SidebarView::Chat;
        let mut p = task_probe(0, "Wx", "");
        p.task_description =
            "Verify the shipped extra interactions and usability paths are actually \
             working in the repository snapshot"
                .to_string();
        app.on_event(crate::tui::event::TuiEvent::ProbeResult(p));

        let text = flow(&pane_rows(&app, 120, 40));
        assert!(
            text.contains("Verify the shipped extra interactions and usability paths are actually working in the repository snapshot"),
            "the whole description must survive wrapping: {text}"
        );
        assert!(
            sidebar_rows(&pane_rows(&app, 120, 40))
                .iter()
                .all(|r| !r.ends_with('…')),
            "no chat row may end in an ellipsis: {text}"
        );
    }

    #[test]
    fn a_judges_reasoning_is_shown_whole() {
        let mut app = fresh_app(120, 40);
        app.sidebar_view = SidebarView::Chat;
        app.on_event(crate::tui::event::TuiEvent::ProbeResult(task_probe(
            0, "Wx", "d",
        )));
        app.on_event(crate::tui::event::TuiEvent::JudgeScored {
            task_id: None,
            judge_name: "Creativity".to_string(),
            point_delta: 17,
            feedback: "The build goes well beyond a plain weather card: quick picks, \
                       a units toggle and an honest error page all ship, and the \
                       forecast reads from the pinned dataset."
                .to_string(),
        });

        let text = flow(&pane_rows(&app, 120, 40));
        assert!(
            text.contains("The build goes well beyond a plain weather card: quick picks, a units toggle and an honest error page all ship, and the forecast reads from the pinned dataset."),
            "a verdict cut mid-sentence is the one thing a player cannot act on: {text}"
        );
    }

    #[test]
    fn a_long_task_title_wraps_under_the_marker() {
        let mut app = fresh_app(120, 40);
        app.sidebar_view = SidebarView::Chat;
        app.on_event(crate::tui::event::TuiEvent::ProbeResult(task_probe(
            0,
            "Switch cities and open the full three-day forecast",
            "d",
        )));

        let rows = pane_rows(&app, 120, 40);
        let text = flow(&rows);
        assert!(text.contains("TASK #0"), "{text}");
        assert!(
            text.contains("Switch cities and open the full three-day forecast"),
            "the title survives in full: {text}"
        );
    }

    #[test]
    fn the_chat_view_takes_a_wider_pane_than_the_probe_list() {
        // Prose in a 20% ribbon wraps into two-word lines; the chat earns
        // the extra columns.
        let mut app = fresh_app(120, 40);
        app.has_pty = false;
        app.sidebar_view = SidebarView::Probes;
        app.on_event(crate::tui::event::TuiEvent::ProbeResult(task_probe(
            0, "Wx", "d",
        )));

        let probes_left = sidebar_left(&pane_rows(&app, 120, 40));
        app.sidebar_view = SidebarView::Chat;
        let chat_left = sidebar_left(&pane_rows(&app, 120, 40));
        assert!(
            chat_left < probes_left,
            "the chat pane must start further left (be wider): chat at {chat_left}, probes at {probes_left}"
        );
    }
}

#[test]
fn chat_pane_keeps_a_status_row_above_the_compose_bar() {
    let mut app = fresh_app(120, 40);
    app.sidebar_view = SidebarView::Chat;
    app.has_pty = true;
    app.header.status = crate::tui::header::Status::Running;
    app.on_event(crate::tui::event::TuiEvent::ProbeResult(task_probe(
        0,
        "Wx",
        "done-note",
    )));
    app.on_event(crate::tui::event::TuiEvent::JudgeStarted {
        task_id: None,
        judge_name: "Data".to_string(),
    });
    app.next_probe_due = Some(std::time::Instant::now() + std::time::Duration::from_secs(45));
    let flat = header_flat(&app, 120, 40);
    assert!(flat.contains("TASK #0"), "transcript still renders");
    assert!(
        flat.contains("Data reviewing"),
        "status row names the judge: {flat}"
    );
    assert!(
        flat.contains("in 4") && flat.contains("next check"),
        "status row counts down: {flat}"
    );
    assert!(
        flat.contains("message the agent"),
        "compose bar survives under the status row"
    );
    // Without anything to say, the row yields its line to the transcript.
    app.judge_runs.clear();
    app.next_probe_due = None;
    let flat = header_flat(&app, 120, 40);
    assert!(!flat.contains("next check"), "no status, no row");
}
