// ── Tests ─────────────────────────────────────────────────────────────────────
// `tests` is a directory module (declared as `#[cfg(test)] mod tests;` in
// `render.rs`). `super` is the `render` module, which re-exports `view`,
// `TerminalGuard`, and `fatal_footer`.

use super::*;
use crate::tui::app::TuiApp;
use crate::tui::event::ValidationKind;
use crate::tui::header::HeaderState;
use ratatui::backend::TestBackend;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;
use vt100::Parser;

fn fresh_app(cols: u16, rows: u16) -> TuiApp {
    let header = HeaderState::new("ABC", "proj");
    let parser = Parser::new(rows, cols, 0);
    let dropped = Arc::new(AtomicU64::new(0));
    TuiApp::new_for_test(header, parser, dropped, cols, rows)
}

/// Build a `ProbeResultInfo`, defaulting the boilerplate fields that the
/// renderer ignores, so tests stay focused on the values that matter.
fn mk_result(
    pid: uuid::Uuid,
    command: &str,
    stdout: &str,
    task_id: Option<uuid::Uuid>,
    task_ordinal: i32,
) -> crate::tui::event::ProbeResultInfo {
    crate::tui::event::ProbeResultInfo {
        probe_id: pid,
        command: command.to_string(),
        stdout: stdout.to_string(),
        exit_code: Some(0),
        duration_ms: 5,
        error: None,
        task_id,
        task_ordinal,
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
    }
}

fn header_flat(app: &TuiApp, w: u16, h: u16) -> String {
    let backend = TestBackend::new(w, h);
    let mut terminal = ratatui::Terminal::new(backend).unwrap();
    terminal.draw(|f| view(f, app)).unwrap();
    let buffer = terminal.backend().buffer().clone();
    (0..h)
        .flat_map(|y| {
            let buf = buffer.clone();
            (0..w).map(move |x| buf[(x, y)].symbol().to_string())
        })
        .collect()
}

mod chat_tests;
mod desktop_agent_tests;
mod git_tests;
mod header_tests;
mod sidebar_tests;
mod tokens_tests;

#[test]
fn help_popup_renders_hotkey_list() {
    let mut app = fresh_app(80, 24);
    app.show_help = true;
    let flat = header_flat(&app, 80, 24);
    assert!(flat.contains("hotkeys"), "help popup title missing");
    assert!(flat.contains("F2"), "last-failed hotkey missing");
    assert!(flat.contains("F10"), "quit hotkey missing");
}

#[test]
fn help_popup_hidden_by_default() {
    let app = fresh_app(80, 24);
    let flat = header_flat(&app, 80, 24);
    assert!(!flat.contains("hotkeys"));
}
