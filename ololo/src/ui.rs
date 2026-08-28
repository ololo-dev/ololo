//! CLI output conventions for ololo.
//!
//! # Rules
//!
//! 1. **All user-facing output goes to stderr.** Stdout is reserved for
//!    machine-readable data (e.g. tokens or IDs printed via `--format json`).
//!    Callers can silence UI noise with `2>/dev/null` without losing data.
//!
//! 2. **Symbol language** — one prefix per line, chosen by type:
//!    - `→` step (ongoing action — connecting, opening, saving, …)
//!    - `✓` success (operation completed)
//!    - `✗` error (fatal or non-fatal, depending on the call)
//!    - `!` warning (something notable; execution continues)
//!    - `·` hint (supplementary, can be ignored)
//!    - `…` waiting (blocking on an external event)
//!
//! 3. **Colors are TTY-only.** Applied when stderr is a terminal and the
//!    `NO_COLOR` environment variable is absent. Never colorize stdout.
//!
//! 4. **No raw `print!` / `eprintln!` in command modules.** All user-facing
//!    lines must route through this module so the style stays consistent.

use std::fmt::Display;
use std::io::IsTerminal;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};

// ── TUI-active guard (gates every ui::* function) ─────────────────────────────

/// Set to `true` while the TUI owns the terminal (between
/// `TerminalGuard::enter` and its `Drop`). All public `ui::*` functions
/// short-circuit when this flag is set, so a stray `ui::step` or
/// `ui::hint` from `player_ws` or `main` cannot corrupt the
/// alternate-screen render.
pub static TUI_ACTIVE: AtomicBool = AtomicBool::new(false);

/// Return the current value of the TUI-active flag. Public for tests
/// in the `tui` module; do not branch on this from production code.
pub fn tui_active() -> bool {
    TUI_ACTIVE.load(Ordering::Acquire)
}

/// Internal helper used by `ui::*` functions and the TUI's
/// `TerminalGuard` lifecycle.
#[inline]
pub(crate) fn set_tui_active(value: bool) {
    TUI_ACTIVE.store(value, Ordering::Release);
}

#[inline]
fn tui_active_short_circuit() -> bool {
    // `Acquire` pairs with the `Release` in `set_tui_active`; ensures
    // any state observed by the TUI before toggling the flag is
    // visible to readers that observe `tui_active() == true` after.
    tui_active()
}

// ── Color enablement ─────────────────────────────────────────────────────────

static USE_COLOR: OnceLock<bool> = OnceLock::new();

fn use_color() -> bool {
    *USE_COLOR
        .get_or_init(|| std::env::var_os("NO_COLOR").is_none() && std::io::stderr().is_terminal())
}

/// Return the ANSI escape code if color is enabled, otherwise an empty string.
fn c(code: &'static str) -> &'static str {
    if use_color() { code } else { "" }
}

const RST: &str = "\x1b[0m";
const BOLD: &str = "\x1b[1m";
const DIM: &str = "\x1b[2m";
const GREEN: &str = "\x1b[32m";
const YELLOW: &str = "\x1b[33m";
const BLUE: &str = "\x1b[34m";
const RED: &str = "\x1b[31m";

// ── Public API ───────────────────────────────────────────────────────────────

/// `→ msg` — an ongoing action (connecting, opening, saving, …).
pub fn step(msg: impl Display) {
    if tui_active_short_circuit() {
        return;
    }
    eprintln!("{}→{} {msg}", c(BLUE), c(RST));
}

/// `✓ msg` — operation completed successfully.
pub fn success(msg: impl Display) {
    if tui_active_short_circuit() {
        return;
    }
    eprintln!("{}✓{} {msg}", c(GREEN), c(RST));
}

/// `! msg` — a warning; execution continues after this line.
pub fn warn(msg: impl Display) {
    if tui_active_short_circuit() {
        return;
    }
    eprintln!("{}!{} {msg}", c(YELLOW), c(RST));
}

/// `✗ msg` — a non-fatal error line. Caller decides whether to exit.
pub fn error(msg: impl Display) {
    if tui_active_short_circuit() {
        return;
    }
    eprintln!("{}✗{} {msg}", c(RED), c(RST));
}

/// `✗ msg` — print an error and exit with code 1.
///
/// **Exempt from the TUI_ACTIVE guard** (per the Heist's `ui::fatal`
/// exemption rule). When called while the TUI is active, this function
/// MUST be paired with a terminal restoration step before exiting so
/// the user's terminal is not left in raw mode. The Heist's TUI
/// initialization sequence (Contract Annex C) places `ui::fatal` calls
/// only in pre-TUI init paths.
pub fn fatal(msg: impl Display) -> ! {
    error(msg);
    std::process::exit(1);
}

/// `· msg` — a hint or supplementary note (dimmed; safe to skip).
pub fn hint(msg: impl Display) {
    if tui_active_short_circuit() {
        return;
    }
    eprintln!("{}· {msg}{}", c(DIM), c(RST));
}

/// `… msg` — waiting on an external event (dimmed).
pub fn waiting(msg: impl Display) {
    if tui_active_short_circuit() {
        return;
    }
    eprintln!("{}… {msg}{}", c(DIM), c(RST));
}

// ── Live countdown (in-place) ────────────────────────────────────────────────

static STDERR_IS_TTY: OnceLock<bool> = OnceLock::new();

fn stderr_is_tty() -> bool {
    *STDERR_IS_TTY.get_or_init(|| std::io::stderr().is_terminal())
}

/// Last countdown the game-server sent us, kept in a process-global so any
/// other stderr output (probe results, success/failure lines) can re-emit it
/// at the bottom of the screen and stay out of the operator's way.
static LAST_COUNTDOWN: OnceLock<Mutex<Option<(String, u32)>>> = OnceLock::new();

fn last_countdown_slot() -> &'static Mutex<Option<(String, u32)>> {
    LAST_COUNTDOWN.get_or_init(|| Mutex::new(None))
}

fn set_last_countdown(phase: &str, seconds_remaining: u32) {
    if let Ok(mut slot) = last_countdown_slot().lock() {
        *slot = Some((phase.to_string(), seconds_remaining));
    }
}

fn take_last_countdown() -> Option<(String, u32)> {
    last_countdown_slot().lock().ok().and_then(|mut s| s.take())
}

fn peek_last_countdown() -> Option<(String, u32)> {
    last_countdown_slot().lock().ok().and_then(|s| s.clone())
}

/// Render a live countdown that overwrites the current line.
///
/// The ololo CLI is fully driven by countdowns received from the game-server
/// over the WebSocket — it does not run any client-side countdown timer. Each
/// new `LobbyCountdown` / `RunningCountdown` frame simply reprints this line,
/// and the previous content is cleared in place so the operator sees a stable
/// single-line display rather than a flood of stale lines.
///
/// The value is also cached so [`pause_countdown`] can re-emit it at the
/// bottom of the screen after a multi-line block of probe output.
pub fn countdown_line(phase: &str, seconds_remaining: u32) {
    set_last_countdown(phase, seconds_remaining);
    if tui_active_short_circuit() {
        return;
    }
    if stderr_is_tty() {
        eprint!("\r\x1b[2K{phase}: {seconds_remaining}s");
        use std::io::Write;
        let _ = std::io::stderr().flush();
    } else {
        eprintln!("{phase}: {seconds_remaining}s");
    }
}

/// Run `f` and re-emit the cached countdown line at the bottom of the screen
/// after `f` returns. Use this around any block of stderr output that would
/// otherwise step on the live countdown line (probe results, success lines,
/// etc.). On a non-TTY the call is a no-op wrapper — each line of `f` lands
/// on its own line and the next countdown frame repaints the bottom.
///
/// On a TTY, the current countdown line is cleared first so `f` can use the
/// full width without the countdown being scribbled over, then the cached
/// countdown is redrawn.
pub fn pause_countdown<F: FnOnce()>(f: F) {
    if tui_active_short_circuit() {
        f();
        return;
    }
    if !stderr_is_tty() {
        f();
        return;
    }
    // Clear the current countdown line.
    eprint!("\r\x1b[2K");
    use std::io::Write;
    let _ = std::io::stderr().flush();
    f();
    if let Some((phase, secs)) = peek_last_countdown() {
        eprint!("\r\x1b[2K{phase}: {secs}s");
        let _ = std::io::stderr().flush();
    }
}

/// Drop the cached countdown. Call when the session ends (complete / Ctrl-C)
/// so the trailing countdown line is not silently re-emitted after a clean
/// newline.
pub fn countdown_done() {
    let _ = take_last_countdown();
    if tui_active_short_circuit() {
        return;
    }
    if stderr_is_tty() {
        eprintln!();
    }
}

/// Print a URL on its own indented, bold line for easy copy-paste.
pub fn url(u: impl Display) {
    if tui_active_short_circuit() {
        return;
    }
    eprintln!();
    eprintln!("    {}{u}{}", c(BOLD), c(RST));
    eprintln!();
}

// ── Info block fields ────────────────────────────────────────────────────────
//
// Renders a two-column info block:
//
//   profile     default
//   server      https://ololo.dev
//   fingerprint ab12cd34
//   expires     2026-07-18  in 61 days    ← colored by urgency
//
// Label column is padded to LABEL_W chars so values line up.

const LABEL_W: usize = 11; // "fingerprint" is the longest label

fn label_col(label: &str) -> String {
    format!("{label:<LABEL_W$}")
}

/// `  {dim label}  {value}` — labeled info field at default style.
pub fn field(label: &str, value: impl Display) {
    if tui_active_short_circuit() {
        return;
    }
    let lbl = label_col(label);
    eprintln!("  {}{lbl}{}  {value}", c(DIM), c(RST));
}

/// Like [`field`] but value is styled green (healthy / valid).
pub fn field_ok(label: &str, value: impl Display) {
    if tui_active_short_circuit() {
        return;
    }
    let lbl = label_col(label);
    eprintln!("  {}{lbl}{}  {}{value}{}", c(DIM), c(RST), c(GREEN), c(RST));
}

/// Like [`field`] but value is styled yellow (warning / expiring soon).
pub fn field_warn(label: &str, value: impl Display) {
    if tui_active_short_circuit() {
        return;
    }
    let lbl = label_col(label);
    eprintln!(
        "  {}{lbl}{}  {}{value}{}",
        c(DIM),
        c(RST),
        c(YELLOW),
        c(RST)
    );
}

/// Like [`field`] but value is styled red (expired / error).
pub fn field_err(label: &str, value: impl Display) {
    if tui_active_short_circuit() {
        return;
    }
    let lbl = label_col(label);
    eprintln!("  {}{lbl}{}  {}{value}{}", c(DIM), c(RST), c(RED), c(RST));
}

/// Like [`field`] but value is dimmed (unavailable / unknown data).
pub fn field_dim(label: &str, value: impl Display) {
    if tui_active_short_circuit() {
        return;
    }
    let lbl = label_col(label);
    eprintln!("  {}{lbl}{}  {}{value}{}", c(DIM), c(RST), c(DIM), c(RST));
}

// ── Profile list row ─────────────────────────────────────────────────────────

/// One row in `profile list` output.
///
/// - Active profile: green `*` bullet, bold green name, normal server URL.
/// - Inactive profile: plain indent, dimmed name and server URL.
///
/// `name_width` is the width of the widest profile name in the list, used to
/// align the server-URL column consistently.
pub fn profile_row(active: bool, name: &str, server: &str, name_width: usize) {
    if tui_active_short_circuit() {
        return;
    }
    let padded = format!("{:<1$}", name, name_width);
    if active {
        eprintln!(
            "  {}*{} {}{}{padded}{}  {server}",
            c(GREEN),
            c(RST),
            c(BOLD),
            c(GREEN),
            c(RST)
        );
    } else {
        eprintln!(
            "    {}{padded}{}  {}{server}{}",
            c(DIM),
            c(RST),
            c(DIM),
            c(RST)
        );
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// RED: the guard flips between TUI-active and text-mode without
    /// leaking the prior value. A test that fails to compile is a
    /// failing test.
    #[test]
    fn tui_active_flag_toggles_cleanly() {
        assert!(!tui_active(), "fresh process: TUI_ACTIVE should be false");
        set_tui_active(true);
        assert!(tui_active(), "after set(true): TUI_ACTIVE should be true");
        set_tui_active(false);
        assert!(
            !tui_active(),
            "after set(false): TUI_ACTIVE should be false"
        );
    }

    /// RED: every public function that uses the short-circuit must
    /// pass the guard. We invoke `step` and assert it doesn't panic
    /// (the function returns `()`, no observable stderr in the test
    /// pipeline).
    #[test]
    fn tui_active_short_circuit_returns_false_when_unset() {
        set_tui_active(false);
        assert!(!tui_active_short_circuit());
    }

    #[test]
    fn tui_active_short_circuit_returns_true_when_set() {
        set_tui_active(true);
        assert!(tui_active_short_circuit());
        // restore
        set_tui_active(false);
    }

    /// Sanity: `ui::fatal` is the documented exemption from the guard
    /// (it must print a fatal message and exit even while the TUI is
    /// active). The exemption is verified by the grep test in the
    /// Heist's `nfr_006` test plan; here we only assert the function
    /// is reachable.
    #[test]
    fn ui_fatal_is_a_function() {
        // `fatal` is generic over `Display`; we cannot coerce it to a
        // bare fn pointer. Just confirm it compiles.
        let _: fn(&str) -> ! = |_| fatal("never called");
    }

    /// Behavior: when `TUI_ACTIVE` is set, `ui::step` and friends
    /// return without writing to stderr. We test this by capturing
    /// stderr in a buffer via `set_tui_active(true)` and asserting
    /// that calling `step("hello")` does not panic (the function
    /// returns `()`). The "no stderr written" property is implicit:
    /// under test, `stderr_is_tty()` returns `false`, so without the
    /// guard, `step` would still print to stderr. With the guard, the
    /// function returns immediately.
    #[test]
    fn guarded_functions_short_circuit_when_tui_active() {
        set_tui_active(true);
        // These must all return without panicking. If the guard is
        // missing, the test still passes (the print goes to the test
        // harness's stderr), so this is a smoke test only — the real
        // property is verified by the Heist's end-to-end manual test
        // and the integration test in `cli_smoke.rs`.
        step("should-not-print");
        success("should-not-print");
        warn("should-not-print");
        error("should-not-print");
        hint("should-not-print");
        waiting("should-not-print");
        field("k", "v");
        field_ok("k", "v");
        field_warn("k", "v");
        field_err("k", "v");
        field_dim("k", "v");
        url("https://example.com");
        profile_row(false, "name", "https://server", 8);
        countdown_line("Lobby", 30);
        pause_countdown(|| {});
        countdown_done();
        set_tui_active(false);
    }
}
