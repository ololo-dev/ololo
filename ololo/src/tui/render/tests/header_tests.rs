use super::*;
use crate::tui::render::header::{HeaderButton, header_button_at, header_link_range};

#[test]
fn terminal_guard_toggles_tui_active() {
    // Use the global test env lock to serialize ui::* tests so
    // that TUI_ACTIVE mutations don't race.
    // SAFETY: same pattern as config.rs's TEST_ENV_LOCK.
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    crate::ui::set_tui_active(false);
    assert!(!crate::ui::tui_active());
    {
        let _tg = TerminalGuard::enter_for_test();
        assert!(crate::ui::tui_active());
    }
    assert!(!crate::ui::tui_active());
}

#[test]
fn view_renders_three_panes_on_80x24() {
    let app = fresh_app(80, 24);
    let flat = header_flat(&app, 80, 24);
    assert!(
        flat.contains("session=ABC"),
        "render must include session label"
    );
    assert!(
        !flat.contains("project="),
        "the header names the session, not the project"
    );
    assert!(flat.contains("🔌"), "status must render with emoji");
    assert!(flat.contains("Connecting"));
}

#[test]
fn view_renders_three_panes_on_120x40() {
    let app = fresh_app(120, 40);
    let flat = header_flat(&app, 120, 40);
    assert!(flat.contains("session=ABC"));
}

#[test]
fn view_renders_three_panes_on_200x50() {
    let app = fresh_app(200, 50);
    let flat = header_flat(&app, 200, 50);
    assert!(flat.contains("session=ABC"));
}

#[test]
fn header_links_the_session_page_and_not_the_project_page() {
    let mut app = fresh_app(200, 50);
    app.header.session_url = "http://localhost:5173/s/ABC".to_string();
    app.header.project_url = "http://localhost:5173/projects/proj".to_string();
    let flat = header_flat(&app, 200, 50);
    assert!(
        flat.contains("http://localhost:5173/s/ABC"),
        "header must link the session page when session_url is set"
    );
    assert!(
        !flat.contains("/projects/proj"),
        "header must not link the project page"
    );
    assert!(
        !flat.contains("session=ABC"),
        "session={{id}} label must be replaced by the link"
    );
}

#[test]
fn session_link_range_covers_the_url_after_its_icon() {
    let url = "http://localhost:5173/s/ABC";
    let (start, end) = header_link_range(url).expect("a set url is a link");
    assert_eq!(end - start, url.len() as u16);
    assert_eq!(header_link_range(""), None);
}

/// The header's first row, one symbol per terminal column.
fn header_cells(app: &TuiApp, w: u16) -> Vec<String> {
    let backend = TestBackend::new(w, 24);
    let mut terminal = ratatui::Terminal::new(backend).unwrap();
    terminal.draw(|f| view(f, app)).unwrap();
    let buffer = terminal.backend().buffer().clone();
    (0..w)
        .map(|x| buffer[(x, 0)].symbol().to_string())
        .collect()
}

fn header_row(app: &TuiApp, w: u16) -> String {
    header_cells(app, w).concat()
}

/// The terminal column `label` starts at in the header row.
fn column_of(cells: &[String], label: &str) -> u16 {
    (0..cells.len())
        .find(|&i| cells[i..].concat().starts_with(label))
        .unwrap_or_else(|| panic!("{label:?} not in the header")) as u16
}

#[test]
fn help_and_exit_buttons_sit_in_the_top_right_corner() {
    let app = fresh_app(120, 24);
    let row = header_row(&app, 120);
    assert!(
        row.trim_end().ends_with("[Help - F1] [Exit - F10]"),
        "buttons must close the header row: {row:?}"
    );
}

#[test]
fn a_click_on_a_button_names_that_button() {
    let app = fresh_app(120, 24);
    let cells = header_cells(&app, 120);
    let help = column_of(&cells, "[Help - F1]");
    let exit = column_of(&cells, "[Exit - F10]");
    assert_eq!(header_button_at(120, help), Some(HeaderButton::Help));
    assert_eq!(header_button_at(120, help + 10), Some(HeaderButton::Help));
    assert_eq!(header_button_at(120, exit), Some(HeaderButton::Exit));
    assert_eq!(header_button_at(120, exit + 11), Some(HeaderButton::Exit));
    assert_eq!(header_button_at(120, help - 1), None);
    assert_eq!(
        header_button_at(120, exit - 1),
        None,
        "the gap between them"
    );
    assert_eq!(header_button_at(120, 2), None);
}

#[test]
fn a_narrow_header_drops_the_buttons() {
    let app = fresh_app(40, 24);
    let row = header_row(&app, 40);
    assert!(!row.contains("[Help - F1]"), "{row:?}");
    assert_eq!(header_button_at(40, 35), None);
}

#[test]
fn fatal_footer_texts_per_variant() {
    assert_eq!(fatal_footer(&QuitReason::TtyLost), "TTY lost — quitting");
    assert_eq!(
        fatal_footer(&QuitReason::PickerFailed("claude".into())),
        "Agent claude missing — quitting"
    );
}

#[test]
fn header_renders_score_and_rank_when_present() {
    let mut app = fresh_app(120, 24);
    app.score = Some(42);
    app.rank = Some(3);
    let flat = header_flat(&app, 120, 24);
    assert!(flat.contains("score=42"), "header must include score");
    assert!(flat.contains("rank=#3"), "header must include rank");
}

#[test]
fn header_renders_dash_when_score_rank_absent() {
    let app = fresh_app(120, 24);
    let flat = header_flat(&app, 120, 24);
    assert!(
        flat.contains("score=—"),
        "header must show dash for missing score"
    );
    assert!(
        flat.contains("rank=—"),
        "header must show dash for missing rank"
    );
}
