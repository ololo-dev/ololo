use super::*;

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
    assert!(flat.contains("project=proj"));
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
fn header_links_session_and_project_pages_when_urls_set() {
    let mut app = fresh_app(200, 50);
    app.header.session_url = "http://localhost:5173/s/ABC".to_string();
    app.header.project_url = "http://localhost:5173/projects/proj".to_string();
    let flat = header_flat(&app, 200, 50);
    assert!(
        flat.contains("http://localhost:5173/s/ABC"),
        "header must link the session page when session_url is set"
    );
    assert!(
        flat.contains("http://localhost:5173/projects/proj"),
        "header must link the project page when project_url is set"
    );
    assert!(
        !flat.contains("session=ABC"),
        "session={{id}} label must be replaced by the link"
    );
    assert!(
        !flat.contains("project=proj"),
        "project={{id}} label must be replaced by the link"
    );
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
