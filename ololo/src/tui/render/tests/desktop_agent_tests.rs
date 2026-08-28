//! The agent pane when the chosen agent runs in its own window.
//!
//! A desktop agent (Zed, Cursor, Antigravity) has no PTY to mirror. Left
//! alone the pane renders blank, which reads as a crashed agent — these pin
//! that it says where the agent went and what is still being tracked.

use super::*;

/// An app configured the way `run_tui_start` configures a desktop agent.
fn desktop_app(cols: u16, rows: u16, label: &str) -> TuiApp {
    let mut app = fresh_app(cols, rows);
    app.has_pty = false;
    app.agent_is_desktop = true;
    app.agent_label = label.to_string();
    app
}

#[test]
fn names_the_agent_and_says_it_runs_in_its_own_window() {
    let app = desktop_app(120, 40, "zed");
    let flat = header_flat(&app, 120, 40);
    assert!(flat.contains("zed"), "the agent must be named: {flat}");
    assert!(
        flat.contains("ownwindow") || flat.contains("its own window"),
        "must explain where the agent went: {flat}"
    );
}

#[test]
fn states_what_is_still_tracked_so_the_pane_is_not_read_as_a_failure() {
    let app = desktop_app(120, 40, "zed");
    let flat = header_flat(&app, 120, 40);
    for claim in ["probes", "scoring", "token"] {
        assert!(
            flat.contains(claim),
            "pane must say {claim} still works: {flat}"
        );
    }
}

#[test]
fn admits_the_conversation_is_not_visible() {
    // The honest half: ololo cannot mirror a desktop agent's conversation,
    // and the pane should say so rather than implying full observation.
    let app = desktop_app(120, 40, "cursor");
    let flat = header_flat(&app, 120, 40);
    assert!(
        flat.contains("conversation"),
        "pane must be explicit about what it cannot see: {flat}"
    );
}

#[test]
fn a_terminal_agent_without_a_pty_keeps_the_old_placeholder() {
    // Only the desktop flag switches panes; a terminal agent that has not
    // spawned its PTY yet must not claim to be running in a window.
    let mut app = fresh_app(120, 40);
    app.has_pty = false;
    app.agent_is_desktop = false;
    let flat = header_flat(&app, 120, 40);
    assert!(
        !flat.contains("its own window") && !flat.contains("ownwindow"),
        "terminal agent must not get the desktop pane: {flat}"
    );
}

#[test]
fn renders_without_panicking_on_a_narrow_terminal() {
    // The pane is a fixed block of prose; a small terminal must clip it
    // rather than bring the TUI down mid-session.
    let app = desktop_app(40, 12, "antigravity");
    let _ = header_flat(&app, 40, 12);
}
