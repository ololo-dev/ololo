use super::*;

#[cfg(test)]
mod pty_inner_rect_tests {
    use super::*;

    #[test]
    fn sidebar_mode_no_tokens_matches_20pct_sidebar_layout() {
        // 100x40, show_sidebar, no tokens -> main_pct 80%, sidebar box has a
        // 1-row header offset + 1-cell border on each side (see render.rs::view).
        let rect = pty_inner_rect(100, 40, true, false, false);
        assert_eq!(rect, Rect::new(1, 2, 78, 37));
    }

    #[test]
    fn sidebar_mode_with_tokens_uses_wider_sidebar() {
        // tokens present -> main_pct shrinks to 65%.
        let rect = pty_inner_rect(100, 40, true, true, false);
        assert_eq!(rect, Rect::new(1, 2, 63, 37));
    }

    #[test]
    fn no_sidebar_mode_matches_full_width_layout() {
        // No sidebar: 1-row top margin + 1-col side margins + border, on
        // top of the 1-row header (see render.rs::view's else branch).
        let rect = pty_inner_rect(100, 40, false, false, false);
        assert_eq!(rect, Rect::new(2, 3, 96, 36));
    }

    #[test]
    fn chat_view_narrows_the_agent_to_55pct() {
        // The chat sidebar takes 45% (render.rs::view) — the PTY must be
        // sized to the 55% that remains, or the agent wraps its output
        // past the pane's real edge.
        let rect = pty_inner_rect(100, 40, true, false, true);
        assert_eq!(rect, Rect::new(1, 2, 53, 37));
    }

    #[test]
    fn tiny_terminal_clamps_to_minimum_size() {
        let rect = pty_inner_rect(10, 5, true, false, false);
        assert_eq!(rect.width, 20);
        assert_eq!(rect.height, 5);
    }
}

#[cfg(test)]
mod apply_pty_layout_tests {
    use super::*;
    use crate::tui::header::HeaderState;
    use std::sync::Arc;
    use std::sync::atomic::AtomicU64;

    fn app_at(cols: u16, rows: u16) -> TuiApp {
        let header = HeaderState::new("s", "p");
        let parser = vt100::Parser::new(rows, cols, 0);
        TuiApp::new_for_test(header, parser, Arc::new(AtomicU64::new(0)), cols, rows)
    }

    #[test]
    fn resizes_vt100_parser_along_with_pty_geometry() {
        let mut app = app_at(80, 24);
        app.sidebar_view = crate::tui::app::SidebarView::Probes;
        // Sidebar shown, no tokens: 100x40 -> inner rect 78x37.
        apply_pty_layout(&mut app, None, 100, 40);
        assert_eq!(app.pty_parser.screen().size(), (37, 78), "(rows, cols)");
        assert_eq!((app.pty_cols, app.pty_rows), (78, 37));
    }

    #[test]
    fn sidebar_toggle_re_fits_parser_to_full_width() {
        // Regression: F4 resized only the OS PTY; the local parser kept the
        // old grid, so the agent's SIGWINCH repaint rendered mangled.
        let mut app = app_at(80, 24);
        app.sidebar_view = crate::tui::app::SidebarView::Probes;
        apply_pty_layout(&mut app, None, 100, 40);
        assert_eq!(app.pty_parser.screen().size(), (37, 78));
        app.show_sidebar = false;
        apply_pty_layout(&mut app, None, 100, 40);
        assert_eq!(app.pty_parser.screen().size(), (36, 96));
        assert_eq!((app.pty_cols, app.pty_rows), (96, 36));
    }

    #[test]
    fn chat_view_fits_the_parser_to_the_narrower_agent_pane() {
        // Regression: the default chat view takes 45% of the terminal, but
        // the PTY kept being sized at 80% — the agent wrapped its output
        // past the pane's real edge (visible as clipped lines).
        let mut app = app_at(80, 24);
        apply_pty_layout(&mut app, None, 100, 40);
        assert_eq!(app.pty_parser.screen().size(), (37, 53), "(rows, cols)");
        assert_eq!((app.pty_cols, app.pty_rows), (53, 37));
    }
}

#[cfg(test)]
mod scrollback_delta_tests {
    use super::*;
    use crossterm::event::{MouseButton, MouseEventKind};

    #[test]
    fn scroll_up_moves_further_back_in_history() {
        assert_eq!(scrollback_delta(MouseEventKind::ScrollUp), Some(3));
    }

    #[test]
    fn scroll_down_moves_toward_live_output() {
        assert_eq!(scrollback_delta(MouseEventKind::ScrollDown), Some(-3));
    }

    #[test]
    fn non_scroll_kinds_have_no_delta() {
        assert_eq!(
            scrollback_delta(MouseEventKind::Down(MouseButton::Left)),
            None
        );
        assert_eq!(
            scrollback_delta(MouseEventKind::Up(MouseButton::Left)),
            None
        );
        assert_eq!(
            scrollback_delta(MouseEventKind::Drag(MouseButton::Left)),
            None
        );
        assert_eq!(scrollback_delta(MouseEventKind::Moved), None);
        assert_eq!(scrollback_delta(MouseEventKind::ScrollLeft), None);
        assert_eq!(scrollback_delta(MouseEventKind::ScrollRight), None);
    }

    #[test]
    fn applying_delta_clamps_at_zero_and_never_panics() {
        let current: usize = 1;
        let new =
            current.saturating_add_signed(scrollback_delta(MouseEventKind::ScrollDown).unwrap());
        assert_eq!(new, 0, "must clamp at 0, not underflow/panic");
    }
}

#[cfg(test)]
mod mouse_scroll_fallback_bytes_tests {
    use super::*;
    use crossterm::event::{MouseButton, MouseEventKind};

    #[test]
    fn scroll_up_sends_up_arrow_repeated() {
        let bytes = mouse_scroll_fallback_bytes(MouseEventKind::ScrollUp);
        assert_eq!(bytes, Some(b"\x1b[A\x1b[A\x1b[A".to_vec()));
    }

    #[test]
    fn scroll_down_sends_down_arrow_repeated() {
        let bytes = mouse_scroll_fallback_bytes(MouseEventKind::ScrollDown);
        assert_eq!(bytes, Some(b"\x1b[B\x1b[B\x1b[B".to_vec()));
    }

    #[test]
    fn non_scroll_kinds_have_no_fallback() {
        assert_eq!(
            mouse_scroll_fallback_bytes(MouseEventKind::Down(MouseButton::Left)),
            None
        );
        assert_eq!(
            mouse_scroll_fallback_bytes(MouseEventKind::Up(MouseButton::Left)),
            None
        );
        assert_eq!(
            mouse_scroll_fallback_bytes(MouseEventKind::Drag(MouseButton::Left)),
            None
        );
        assert_eq!(mouse_scroll_fallback_bytes(MouseEventKind::Moved), None);
        assert_eq!(
            mouse_scroll_fallback_bytes(MouseEventKind::ScrollLeft),
            None
        );
        assert_eq!(
            mouse_scroll_fallback_bytes(MouseEventKind::ScrollRight),
            None
        );
    }
}

#[cfg(test)]
mod encode_paste_tests {
    use super::*;

    #[test]
    fn bracketed_wraps_text_in_paste_markers() {
        let bytes = encode_paste("hello\tworld", true);
        assert_eq!(bytes, b"\x1b[200~hello\tworld\x1b[201~".to_vec());
    }

    #[test]
    fn unbracketed_sends_raw_text_only() {
        let bytes = encode_paste("hello\tworld", false);
        assert_eq!(bytes, b"hello\tworld".to_vec());
    }

    #[test]
    fn bracketed_empty_string_still_wraps() {
        let bytes = encode_paste("", true);
        assert_eq!(bytes, b"\x1b[200~\x1b[201~".to_vec());
    }
}

#[cfg(test)]
mod mouse_event_to_pty_bytes_tests {
    use super::*;
    use crossterm::event::{KeyModifiers, MouseButton, MouseEventKind};
    use vt100::{MouseProtocolEncoding, MouseProtocolMode};

    #[test]
    fn mode_none_never_forwards_anything() {
        let bytes = mouse_event_to_pty_bytes(
            MouseEventKind::ScrollUp,
            KeyModifiers::NONE,
            3,
            7,
            MouseProtocolMode::None,
            MouseProtocolEncoding::Sgr,
        );
        assert_eq!(bytes, None);
    }

    #[test]
    fn sgr_scroll_up_encodes_button_64() {
        let bytes = mouse_event_to_pty_bytes(
            MouseEventKind::ScrollUp,
            KeyModifiers::NONE,
            3,
            7,
            MouseProtocolMode::PressRelease,
            MouseProtocolEncoding::Sgr,
        );
        assert_eq!(bytes, Some(b"\x1b[<64;4;8M".to_vec()));
    }

    #[test]
    fn sgr_down_left_at_origin() {
        let bytes = mouse_event_to_pty_bytes(
            MouseEventKind::Down(MouseButton::Left),
            KeyModifiers::NONE,
            0,
            0,
            MouseProtocolMode::PressRelease,
            MouseProtocolEncoding::Sgr,
        );
        assert_eq!(bytes, Some(b"\x1b[<0;1;1M".to_vec()));
    }

    #[test]
    fn sgr_up_uses_lowercase_m_terminator() {
        let bytes = mouse_event_to_pty_bytes(
            MouseEventKind::Up(MouseButton::Left),
            KeyModifiers::NONE,
            0,
            0,
            MouseProtocolMode::PressRelease,
            MouseProtocolEncoding::Sgr,
        );
        assert_eq!(bytes, Some(b"\x1b[<0;1;1m".to_vec()));
    }

    #[test]
    fn shift_modifier_adds_4_to_button_code() {
        let bytes = mouse_event_to_pty_bytes(
            MouseEventKind::ScrollUp,
            KeyModifiers::SHIFT,
            0,
            0,
            MouseProtocolMode::PressRelease,
            MouseProtocolEncoding::Sgr,
        );
        assert_eq!(bytes, Some(b"\x1b[<68;1;1M".to_vec()));
    }

    #[test]
    fn press_release_mode_rejects_drag_and_moved() {
        for kind in [
            MouseEventKind::Drag(MouseButton::Left),
            MouseEventKind::Moved,
        ] {
            let bytes = mouse_event_to_pty_bytes(
                kind,
                KeyModifiers::NONE,
                0,
                0,
                MouseProtocolMode::PressRelease,
                MouseProtocolEncoding::Sgr,
            );
            assert_eq!(bytes, None, "{kind:?} must not forward under PressRelease");
        }
    }

    #[test]
    fn button_motion_mode_allows_drag_but_rejects_moved() {
        let drag = mouse_event_to_pty_bytes(
            MouseEventKind::Drag(MouseButton::Left),
            KeyModifiers::NONE,
            0,
            0,
            MouseProtocolMode::ButtonMotion,
            MouseProtocolEncoding::Sgr,
        );
        assert_eq!(drag, Some(b"\x1b[<32;1;1M".to_vec()));

        let moved = mouse_event_to_pty_bytes(
            MouseEventKind::Moved,
            KeyModifiers::NONE,
            0,
            0,
            MouseProtocolMode::ButtonMotion,
            MouseProtocolEncoding::Sgr,
        );
        assert_eq!(moved, None);
    }

    #[test]
    fn any_motion_mode_allows_plain_moved() {
        let bytes = mouse_event_to_pty_bytes(
            MouseEventKind::Moved,
            KeyModifiers::NONE,
            0,
            0,
            MouseProtocolMode::AnyMotion,
            MouseProtocolEncoding::Sgr,
        );
        assert_eq!(bytes, Some(b"\x1b[<35;1;1M".to_vec()));
    }

    #[test]
    fn press_mode_x10_rejects_release() {
        let bytes = mouse_event_to_pty_bytes(
            MouseEventKind::Up(MouseButton::Left),
            KeyModifiers::NONE,
            0,
            0,
            MouseProtocolMode::Press,
            MouseProtocolEncoding::Sgr,
        );
        assert_eq!(bytes, None);
    }

    #[test]
    fn default_x10_encoding_down_left_at_origin() {
        let bytes = mouse_event_to_pty_bytes(
            MouseEventKind::Down(MouseButton::Left),
            KeyModifiers::NONE,
            0,
            0,
            MouseProtocolMode::PressRelease,
            MouseProtocolEncoding::Default,
        );
        assert_eq!(bytes, Some(vec![0x1b, b'[', b'M', 32, 1 + 32, 1 + 32]));
    }

    #[test]
    fn default_x10_encoding_release_always_reports_button_3() {
        let bytes = mouse_event_to_pty_bytes(
            MouseEventKind::Up(MouseButton::Right),
            KeyModifiers::NONE,
            0,
            0,
            MouseProtocolMode::PressRelease,
            MouseProtocolEncoding::Default,
        );
        assert_eq!(bytes, Some(vec![0x1b, b'[', b'M', 3 + 32, 1 + 32, 1 + 32]));
    }
}

#[cfg(test)]
mod mouse_to_pty_local_tests {
    use super::*;

    #[test]
    fn point_inside_rect_returns_zero_based_local_coords() {
        let rect = Rect::new(2, 3, 96, 36);
        // Absolute (2,3) is the rect's top-left corner -> local (0,0).
        assert_eq!(mouse_to_pty_local(2, 3, rect), Some((0, 0)));
        // Absolute (5,10) -> local (3,7).
        assert_eq!(mouse_to_pty_local(5, 10, rect), Some((3, 7)));
    }

    #[test]
    fn point_on_far_edge_is_still_inside() {
        let rect = Rect::new(2, 3, 96, 36);
        // Last valid column/row inside the rect.
        assert_eq!(
            mouse_to_pty_local(2 + 96 - 1, 3 + 36 - 1, rect),
            Some((95, 35))
        );
    }

    #[test]
    fn point_outside_rect_returns_none() {
        let rect = Rect::new(2, 3, 96, 36);
        assert_eq!(mouse_to_pty_local(1, 3, rect), None, "left of rect");
        assert_eq!(mouse_to_pty_local(2, 2, rect), None, "above rect");
        assert_eq!(mouse_to_pty_local(2 + 96, 3, rect), None, "right of rect");
        assert_eq!(mouse_to_pty_local(2, 3 + 36, rect), None, "below rect");
    }

    #[test]
    fn zero_sized_rect_never_contains_a_point() {
        let rect = Rect::new(5, 5, 0, 0);
        assert_eq!(mouse_to_pty_local(5, 5, rect), None);
    }
}
