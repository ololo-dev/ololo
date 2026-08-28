#![allow(dead_code)]

//! Render layer: layout, widgets, and the terminal-guard lifecycle.
//!
//! The render layer is pure: `view(frame, app)` reads the `App`'s
//! `HeaderState`, sidebar `VecDeque`, and main-block `vt100::Parser`,
//! and emits widgets. It never mutates state and never owns
//! long-lived resources.

pub(crate) mod chat;
mod common;
pub(crate) mod header;
mod help;
mod main;
mod markdown;
mod permission;
mod probes;
mod terminal_guard;
#[cfg(test)]
mod tests;
mod tokens;

pub use terminal_guard::TerminalGuard;

use crate::tui::app::TuiApp;
use crate::tui::event::QuitReason;
use crate::tui::render::header::render_header;
use crate::tui::render::main::render_main;
use crate::tui::render::probes::render_sidebar;
use ratatui::Frame;
#[allow(unused_imports)]
pub use ratatui::Terminal as _Terminal;
use ratatui::layout::{Constraint, Direction, Layout, Rect};

pub fn fatal_footer(reason: &QuitReason) -> String {
    match reason {
        QuitReason::TtyLost => "TTY lost — quitting".to_string(),
        QuitReason::PickerFailed(n) => format!("Agent {n} missing — quitting"),
        _ => format!("Quit: {reason:?}"),
    }
}

/// Layout: header (1) / body (rest). When `has_pty && show_sidebar` is
/// true the body splits 80/20 main+sidebar and the running agent is rendered
/// in the main pane. When `has_pty && !show_sidebar` the agent takes the full
/// body width with a 1-row top margin and 1-column side margins. Otherwise the
/// legacy 80/20 placeholder layout is used.
pub fn view(f: &mut Frame, app: &TuiApp) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(0)])
        .split(f.area());

    render_header(f, chunks[0], app);

    // ponytail: widen sidebar 20%->35% when tokens exist so provider/model fit
    let has_tokens = app.tokens.as_ref().is_some_and(|v| !v.is_empty());
    // The chat view is prose — task briefs, check descriptions, judge
    // reasoning — and none of it is truncated, so a 20% ribbon would wrap
    // every sentence into a column of two-word lines. Give it room; the
    // agent pane keeps the majority either way.
    let sidebar_pct = match (app.sidebar_view, has_tokens) {
        (crate::tui::app::SidebarView::Chat, _) => 45,
        (_, true) => 35,
        (_, false) => 20,
    };
    let main_pct = 100u16 - sidebar_pct;

    if app.has_pty {
        if app.show_sidebar {
            render_main_with_sidebar(f, chunks[1], app, main_pct, sidebar_pct);
        } else {
            let body = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Length(1), Constraint::Min(0)])
                .split(chunks[1]);
            let main = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([
                    Constraint::Length(1),
                    Constraint::Min(0),
                    Constraint::Length(1),
                ])
                .split(body[1]);
            render_main(f, main[1], app);
        }
    } else {
        render_main_with_sidebar(f, chunks[1], app, main_pct, sidebar_pct);
    }

    // Popups overlay everything; help sits on top of probe details, and
    // the permission question sits on top of them all.
    probes::render_probe_popup(f, app);
    help::render_help_popup(f, app);
    permission::render_permission_popup(f, app);
}

/// Split `area` horizontally into main+sidebar panes and render both.
fn render_main_with_sidebar(
    f: &mut Frame,
    area: Rect,
    app: &TuiApp,
    main_pct: u16,
    sidebar_pct: u16,
) {
    let body = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(main_pct),
            Constraint::Percentage(sidebar_pct),
        ])
        .split(area);
    render_main(f, body[0], app);
    render_sidebar(f, body[1], app);
}
