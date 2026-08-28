use super::*;

#[test]
fn test_header_no_git_segment_when_zero() {
    let app = fresh_app(120, 24);
    let flat = header_flat(&app, 120, 24);
    assert!(
        !flat.contains("[+"),
        "no git segment when diff_stats all zero (got: {flat:?})"
    );
    assert!(!flat.contains("[!"));
    assert!(!flat.contains("[-"));
}

#[test]
fn test_header_shows_git_segment_with_changes() {
    let mut app = fresh_app(120, 24);
    app.diff_stats = crate::tui::git_diff::DiffStats {
        added: 2,
        modified: 1,
        deleted: 1,
    };
    let flat = header_flat(&app, 120, 24);
    assert!(flat.contains("+2"), "header must show added; got: {flat:?}");
    assert!(
        flat.contains("!1"),
        "header must show modified; got: {flat:?}"
    );
    assert!(
        flat.contains("-1"),
        "header must show deleted; got: {flat:?}"
    );
}

#[test]
fn test_header_suppresses_git_segment_on_narrow_terminal() {
    let mut app = fresh_app(65, 24);
    app.diff_stats = crate::tui::git_diff::DiffStats {
        added: 2,
        modified: 1,
        deleted: 1,
    };
    let flat = header_flat(&app, 65, 24);
    assert!(
        !flat.contains("+2"),
        "git segment must be suppressed on narrow terminal; got: {flat:?}"
    );
    assert!(!flat.contains("!1"));
    assert!(!flat.contains("-1"));
}
