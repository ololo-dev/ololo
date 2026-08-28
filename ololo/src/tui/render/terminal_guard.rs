//! RAII guard that puts the terminal into raw + alternate-screen mode for the
//! TUI and restores it on drop.

pub struct TerminalGuard {
    active: bool,
}

impl TerminalGuard {
    /// Enter raw mode + alternate screen + hide cursor + mouse capture +
    /// bracketed paste, and flip `ui::TUI_ACTIVE = true`. The Drop restores
    /// all of it. Returns `None` if the platform doesn't support raw mode
    /// (used in tests).
    ///
    /// Mouse capture and bracketed paste are requested unconditionally on
    /// the *outer* terminal; whether bytes actually get forwarded to the
    /// embedded agent PTY still depends on whether the agent itself asked
    /// for mouse reporting / bracketed paste (see `vt100::Screen::
    /// mouse_protocol_mode` / `bracketed_paste`, checked at forward time).
    pub fn enter() -> Option<Self> {
        if crossterm::terminal::enable_raw_mode().is_err() {
            return None;
        }
        let _ = crossterm::execute!(
            std::io::stdout(),
            crossterm::terminal::EnterAlternateScreen,
            crossterm::cursor::Hide,
            crossterm::event::EnableMouseCapture,
            crossterm::event::EnableBracketedPaste
        );
        crate::ui::set_tui_active(true);
        Some(Self { active: true })
    }

    /// Test-only constructor: forces `TUI_ACTIVE = false`, then sets
    /// it to `true` so the test sees a known-good state. Drop sets
    /// it back to `false`. Idempotent under concurrent test runs.
    #[cfg(test)]
    pub fn enter_for_test() -> Self {
        crate::ui::set_tui_active(false);
        crate::ui::set_tui_active(true);
        Self { active: false }
    }

    /// Restore: raw mode off, leave alternate screen, show cursor, disable
    /// mouse capture + bracketed paste, `TUI_ACTIVE = false`. Idempotent.
    pub fn restore(&mut self) {
        if !self.active {
            return;
        }
        let _ = crossterm::terminal::disable_raw_mode();
        let _ = crossterm::execute!(
            std::io::stdout(),
            crossterm::event::DisableBracketedPaste,
            crossterm::event::DisableMouseCapture,
            crossterm::terminal::LeaveAlternateScreen,
            crossterm::cursor::Show
        );
        crate::ui::set_tui_active(false);
        self.active = false;
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        self.restore();
        // For the test constructor, also force the flag off so
        // concurrent tests don't see leaked state. The real
        // `enter()` already sets it off inside `restore()`.
        #[cfg(test)]
        crate::ui::set_tui_active(false);
    }
}
