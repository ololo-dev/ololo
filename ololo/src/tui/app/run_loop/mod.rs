//! The ololo TUI render loop: the full `tokio::select!` over the event bus,
//! crossterm `EventStream`, tick, and render timer, plus the free helpers it
//! uses for PTY geometry and mouse translation.

use crate::tui::app::keymap::key_to_pty_bytes;
use crate::tui::app::{InputFocus, RENDER_INTERVAL_MS, TuiApp};
use crate::tui::event::{QuitReason, TuiEvent};
use crate::tui::pty_host::PtyHost;
use crate::tui::render::{TerminalGuard, view};
use futures_util::StreamExt;
use ratatui::Terminal;
use ratatui::layout::Rect;
use std::time::Duration;
use tokio::sync::mpsc;

/// What the render loop ended with — the caller prints the session outcome
/// AFTER the terminal guard has restored the normal screen. Anything shown
/// inside the TUI (the log line, the header badge) vanishes with the
/// alternate screen, which is why players saw sessions "just close" with no
/// explanation.
#[derive(Debug, Clone)]
pub struct TuiRunOutcome {
    /// Value to pass to `std::process::exit`.
    pub exit_code: i32,
    pub quit_reason: Option<QuitReason>,
    /// Header status at exit — distinguishes a cancelled session from a
    /// finished one, which `QuitReason::SessionComplete` alone cannot.
    pub final_status: crate::tui::header::Status,
}

/// Headless loop: no terminal — the bus events drive the same
/// `TuiApp::on_event` state machine (snapshot commits on task completion,
/// artifact sync on tick, stats reporting), and a few milestones are
/// narrated as plain lines. Exits on its own when the session completes or
/// the socket closes — there is no user to press F10.
///
/// This is what `--no-tui` runs: the human (or the coding agent driving
/// the terminal, e.g. Claude Code validating a project) plays by editing
/// the working directory while this loop keeps the session honest.
///
/// With `--agent` the loop additionally hosts the agent in a PTY it never
/// renders: a parent agent drives it through the control channel (message
/// inbox in, `screen.txt` out) — autonomous play, built for debugging.
pub async fn run_headless(
    mut app: TuiApp,
    mut rx: mpsc::Receiver<TuiEvent>,
    mut pty: Option<PtyHost>,
    mut pty_rx: Option<mpsc::Receiver<Vec<u8>>>,
    mut control: Option<crate::control::ControlChannel>,
) -> TuiRunOutcome {
    use crate::tui::header::Status;
    let mut tick = tokio::time::interval(Duration::from_secs(1));
    // Stall watchdog. `on_event` does real synchronous work (git commits,
    // artifact sync), and when one of those wedged — a push whose network
    // stalled — this loop went silent for the rest of the session while
    // probes kept running elsewhere, indistinguishable from a dead process
    // (session 3XDEWR). The watchdog runs on another worker, so it can
    // still speak while this task is stuck, naming the stall instead of
    // leaving a mute log.
    let alive = std::sync::Arc::new(std::sync::atomic::AtomicI64::new(0));
    let watchdog = {
        let alive = alive.clone();
        let started = std::time::Instant::now();
        tokio::spawn(async move {
            let mut stalled = false;
            loop {
                tokio::time::sleep(Duration::from_secs(15)).await;
                let last = alive.load(std::sync::atomic::Ordering::Relaxed);
                let idle = started.elapsed().as_secs() as i64 - last;
                if idle >= 30 && !stalled {
                    stalled = true;
                    crate::ui::warn(format!(
                        "session loop stalled for {idle}s (likely a hung git push or \
                         artifact sync) — probes keep running in the background"
                    ));
                } else if idle < 30 && stalled {
                    stalled = false;
                    crate::ui::hint("session loop recovered");
                }
            }
        })
    };
    let loop_started = std::time::Instant::now();
    let outcome: (i32, Option<QuitReason>) = loop {
        alive.store(
            loop_started.elapsed().as_secs() as i64,
            std::sync::atomic::Ordering::Relaxed,
        );
        tokio::select! {
            _ = tick.tick() => {
                app.on_event(TuiEvent::Tick);
                if let Some(pty) = pty.as_mut()
                    && let Ok(Some(status)) = pty.child.try_wait() {
                        let code = status.exit_code();
                        app.should_quit = Some(QuitReason::AgentExited(code as i32));
                    }
                if let Some(c) = control.as_mut() {
                    c.screen.maybe_write(app.pty_parser.screen());
                }
            }
            Some(data) = async {
                match pty_rx.as_mut() {
                    Some(rx) => rx.recv().await,
                    None => std::future::pending().await,
                }
            } => {
                app.pty_parser.process(&data);
            }
            Some(text) = async {
                match control.as_mut() {
                    Some(c) => c.rx.recv().await,
                    None => std::future::pending().await,
                }
            } => {
                match pty.as_mut() {
                    Some(pty) => {
                        crate::ui::step("control: message → agent");
                        deliver_control_message(&mut app, pty, &text).await;
                    }
                    None => crate::ui::warn(
                        "control message dropped: no agent hosted (pass --agent)",
                    ),
                }
            }
            ev = rx.recv() => {
                match ev {
                    // Headless has no popup renderer — ask on the terminal
                    // instead. Spawned so the tick keeps running while the
                    // question is open (the WS loop is already blocked
                    // awaiting the answer). A non-TTY stdin declines.
                    Some(TuiEvent::PermissionRequest(prompt)) => {
                        tokio::spawn(crate::permissions::respond_from_stdin(prompt));
                    }
                    Some(e) => {
                        narrate_headless(&e);
                        app.on_event(e);
                    }
                    None => {
                        app.should_quit = Some(QuitReason::WsClosed);
                    }
                }
            }
        }
        // The session ended: `on_event` above already ran the final task
        // commits for the Complete transition; nothing left to wait for.
        if app.should_quit.is_none()
            && matches!(app.header.status, Status::Complete | Status::Cancelled)
        {
            app.should_quit = Some(QuitReason::SessionComplete);
        }
        if let Some(reason) = app.should_quit.take() {
            let code = match &reason {
                QuitReason::SessionComplete => 0,
                QuitReason::AgentExited(c) => *c,
                QuitReason::UserRequested => 0,
                _ => 1,
            };
            break (code, Some(reason));
        }
    };
    watchdog.abort();
    if let Some(mut pty) = pty {
        let _ = pty.shutdown().await;
    }
    TuiRunOutcome {
        exit_code: outcome.0,
        quit_reason: outcome.1,
        final_status: app.header.status,
    }
}

/// Deliver a parent-agent message into the hosted agent's PTY and submit
/// it with Enter. Multi-line messages go as one bracketed paste so prompts
/// arrive as a block; single-line messages go as raw typed bytes, because
/// agent menu dialogs (e.g. claude's permission screens) ignore pasted
/// text but respond to typed digits — and slash commands only trigger
/// their palette when typed.
async fn deliver_control_message(app: &mut TuiApp, pty: &mut PtyHost, text: &str) {
    app.pty_parser.screen_mut().set_scrollback(0);
    if text.contains('\n') {
        let bracketed = app.pty_parser.screen().bracketed_paste();
        let _ = pty.write_input(&encode_paste(text, bracketed));
    } else {
        let _ = pty.write_input(text.as_bytes());
    }
    // Give the agent a beat to ingest the input before Enter — submitted
    // immediately, agents that process pastes asynchronously (claude) drop
    // the Enter and leave the message sitting in the input box.
    tokio::time::sleep(Duration::from_millis(300)).await;
    let _ = pty.write_input(b"\r");
}

/// Print the bus events `player_ws`'s own text-mode output does not already
/// cover: snapshot requests, grades, and joins. Probe pushes, countdowns,
/// and session transitions are narrated by `player_ws` via `crate::ui`.
fn narrate_headless(ev: &TuiEvent) {
    match ev {
        TuiEvent::SnapshotRequested {
            task_id, reason, ..
        } => {
            crate::ui::step(format!("snapshot requested ({reason}) for task {task_id}"));
        }
        TuiEvent::ProbeGraded {
            probe_id,
            outcome,
            point_delta,
            ..
        } => {
            crate::ui::step(format!("graded {probe_id}: {outcome:?} ({point_delta:+})"));
        }
        TuiEvent::MemberJoined { name } => {
            crate::ui::step(format!("member joined: {name}"));
        }
        _ => {}
    }
}

/// Minimal render loop: enter raw mode, install the terminal guard,
/// then drive a `tokio::select!` over the bus, the crossterm
/// `EventStream`, a 1Hz tick, and a 33ms render timer.
pub async fn run(
    mut app: TuiApp,
    mut rx: mpsc::Receiver<TuiEvent>,
    session_label: &str,
    project_label: &str,
    mut pty: Option<&mut PtyHost>,
    pty_rx: &mut Option<mpsc::Receiver<Vec<u8>>>,
    mut control: Option<crate::control::ControlChannel>,
) -> TuiRunOutcome {
    use crossterm::event::{KeyCode, KeyEventKind};
    use ratatui::backend::CrosstermBackend;
    let backend = CrosstermBackend::new(std::io::stdout());
    let mut terminal = match Terminal::new(backend) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("TUI: failed to create terminal: {e}");
            return TuiRunOutcome {
                exit_code: 1,
                quit_reason: None,
                final_status: app.header.status,
            };
        }
    };
    let _guard = match TerminalGuard::enter() {
        Some(g) => g,
        None => {
            eprintln!("TUI: cannot enter raw mode (no TTY?)");
            return TuiRunOutcome {
                exit_code: 1,
                quit_reason: None,
                final_status: app.header.status,
            };
        }
    };
    let mut tick = tokio::time::interval(Duration::from_secs(1));
    let mut render_timer = tokio::time::interval(Duration::from_millis(RENDER_INTERVAL_MS));
    let mut events = crossterm::event::EventStream::new();

    // Fit the PTY to the real terminal and the real initial layout before
    // the first frame — the caller's spawn-time guess can only be a guess,
    // and an agent that boots against the wrong width paints clipped lines
    // until something else happens to trigger a relayout.
    {
        let (term_cols, term_rows) = crossterm::terminal::size().unwrap_or((80, 24));
        apply_pty_layout(&mut app, pty.as_deref_mut(), term_cols, term_rows);
    }

    let outcome: (i32, Option<QuitReason>) = loop {
        tokio::select! {
            biased;
            _ = render_timer.tick() => {
                if let Err(e) = terminal.draw(|f| view(f, &app)) {
                    eprintln!("TUI: draw error: {e}");
                    break (1, None);
                }
            }
            _ = tick.tick() => {
                app.on_event(TuiEvent::Tick);
                if let Some(pty) = pty.as_mut()
                    && let Ok(Some(status)) = pty.child.try_wait() {
                        let code = status.exit_code();
                        app.should_quit = Some(QuitReason::AgentExited(code as i32));
                    }
                if let Some(c) = control.as_mut() {
                    c.screen.maybe_write(app.pty_parser.screen());
                }
            }
            Some(data) = async {
                match pty_rx.as_mut() {
                    Some(rx) => rx.recv().await,
                    None => std::future::pending().await,
                }
            } => {
                app.pty_parser.process(&data);
                app.on_event(TuiEvent::Tick);
            }
            Some(text) = async {
                match control.as_mut() {
                    Some(c) => c.rx.recv().await,
                    None => std::future::pending().await,
                }
            } => {
                if let Some(pty) = pty.as_mut() {
                    deliver_control_message(&mut app, pty, &text).await;
                } else {
                    tracing::warn!("control message dropped: no agent PTY");
                }
            }
            ev = events.next() => {
                match ev {
                    Some(Ok(crossterm::event::Event::Key(k))) => {
                        if k.kind == KeyEventKind::Press {
                            if k.code == KeyCode::F(10) {
                                app.should_quit = Some(QuitReason::UserRequested);
                            } else if k.code == KeyCode::F(9) && pty.is_some() {
                                // Explicit focus choice cancels any pending
                                // modal restore.
                                app.focus_return = None;
                                app.set_input_focus(match app.input_focus {
                                    InputFocus::Tui => InputFocus::Pty,
                                    InputFocus::Pty => InputFocus::Tui,
                                });
                            } else if matches!(
                                k.code,
                                KeyCode::F(1)
                                    | KeyCode::F(2)
                                    | KeyCode::F(3)
                                    | KeyCode::F(4)
                                    | KeyCode::F(5)
                            ) {
                                // Global ololo hotkeys (help / last-failed /
                                // sidebar / chat view) — never forwarded to
                                // the agent.
                                app.on_key(k.code, k.modifiers);
                            } else if app.input_focus == InputFocus::Pty && pty.is_some() {
                                // Any keystroke returns the view to live
                                // output -- same convention as a pager/tmux
                                // copy-mode: typing means you're done reading
                                // history. No-op when already at 0.
                                app.pty_parser.screen_mut().set_scrollback(0);
                                if let Some(pty) = pty.as_mut() {
                                    if let Some(bytes) = key_to_pty_bytes(k.code, k.modifiers) {
                                        let _ = pty.write_input(&bytes);
                                    } else if let KeyCode::Char(c) = k.code {
                                        let mut buf = [0u8; 4];
                                        let s = c.encode_utf8(&mut buf);
                                        let _ = pty.write_input(s.as_bytes());
                                    }
                                }
                            } else {
                                app.on_key(k.code, k.modifiers);
                            }
                        }
                    }
                    Some(Ok(crossterm::event::Event::Resize(c, r))) => {
                        app.on_event(TuiEvent::Resized { cols: c, rows: r });
                        apply_pty_layout(&mut app, pty.as_deref_mut(), c, r);
                    }
                    Some(Ok(crossterm::event::Event::Mouse(m))) => {
                        // Header link click: open the session dashboard in the
                        // browser. Only the link's own column range is hit-tested;
                        // clicks elsewhere in the header fall through to the pty
                        // handler below (which ignores them, since the header is
                        // outside the pty block).
                        if m.row == 0
                            && matches!(
                                m.kind,
                                crossterm::event::MouseEventKind::Down(
                                    crossterm::event::MouseButton::Left
                                )
                            )
                        {
                            let (session_range, project_range) =
                                crate::tui::render::header::header_link_ranges(
                                    &app.header.session_url,
                                    &app.header.project_url,
                                );
                            let clicked = if let Some((s, e)) = session_range
                                && m.column >= s
                                && m.column < e
                            {
                                Some(app.header.session_url.clone())
                            } else if let Some((s, e)) = project_range
                                && m.column >= s
                                && m.column < e
                            {
                                Some(app.header.project_url.clone())
                            } else {
                                None
                            };
                            if let Some(url) = clicked {
                                tokio::spawn(async move {
                                    let _ = open::that(url);
                                });
                            }
                        }
                        // The chat sidebar is a mouse surface of its own —
                        // from either focus, because reading the chat must
                        // not require an F9 round-trip: wheel scrolls the
                        // transcript, a click on a bubble selects it (again:
                        // sends it to the agent), a click on the compose bar
                        // opens the compose line.
                        {
                            let (term_cols, term_rows) =
                                crossterm::terminal::size().unwrap_or((80, 24));
                            if let Some(area) = crate::tui::render::chat::chat_area(
                                &app, term_cols, term_rows,
                            ) && m.column >= area.x
                                && m.row >= area.y
                            {
                                match m.kind {
                                    crossterm::event::MouseEventKind::ScrollUp => {
                                        app.chat_cursor = None;
                                        app.chat_scroll_by(3);
                                    }
                                    crossterm::event::MouseEventKind::ScrollDown => {
                                        app.chat_cursor = None;
                                        app.chat_scroll_by(-3);
                                    }
                                    crossterm::event::MouseEventKind::Down(
                                        crossterm::event::MouseButton::Left,
                                    ) => {
                                        if crate::tui::render::chat::compose_bar_row(
                                            &app, term_cols, term_rows,
                                        ) == Some(m.row)
                                        {
                                            app.open_chat_compose();
                                        } else if let Some(idx) =
                                            crate::tui::render::chat::bubble_at(
                                                &app, term_cols, term_rows, m.column,
                                                m.row,
                                            )
                                        {
                                            app.chat_click_bubble(idx);
                                        }
                                    }
                                    _ => {}
                                }
                                continue;
                            }
                        }
                        // Hit-test against the *current* pty inner rect first --
                        // scrolling/clicks only apply while hovering the block.
                        if let Some(pty) = pty.as_mut() {
                            let (term_cols, term_rows) =
                                crossterm::terminal::size().unwrap_or((80, 24));
                            let has_tokens =
                                app.tokens.as_ref().is_some_and(|v| !v.is_empty());
                            let chat_view =
                                app.sidebar_view == crate::tui::app::SidebarView::Chat;
                            let rect = pty_inner_rect(
                                term_cols, term_rows, app.show_sidebar, has_tokens, chat_view,
                            );
                            if let Some((local_col, local_row)) =
                                mouse_to_pty_local(m.column, m.row, rect)
                            {
                                let screen = app.pty_parser.screen();
                                let mode = screen.mouse_protocol_mode();
                                if mode != vt100::MouseProtocolMode::None {
                                    // Agent has its own scroll/mouse handling
                                    // (e.g. opencode) -- forward the raw
                                    // protocol bytes and let it drive itself.
                                    let encoding = screen.mouse_protocol_encoding();
                                    if let Some(bytes) = mouse_event_to_pty_bytes(
                                        m.kind, m.modifiers, local_col, local_row, mode, encoding,
                                    ) {
                                        let _ = pty.write_input(&bytes);
                                    }
                                } else if screen.alternate_screen() {
                                    // Alt-screen app with no mouse support --
                                    // real terminals translate wheel to arrow
                                    // keys here (xterm's alternateScroll); do
                                    // the same since there's no scrollback to
                                    // show (alt-screen never has any, in
                                    // vt100 or real terminals).
                                    if let Some(bytes) = mouse_scroll_fallback_bytes(m.kind) {
                                        let _ = pty.write_input(&bytes);
                                    }
                                } else if let Some(delta) = scrollback_delta(m.kind) {
                                    // Primary screen, agent just prints
                                    // sequential output and expects the
                                    // *terminal* to keep scrollback (e.g.
                                    // claude code, pi) -- scroll ololo's own
                                    // view like a real terminal would,
                                    // without sending the agent anything.
                                    let current = screen.scrollback();
                                    let new_offset = current.saturating_add_signed(delta);
                                    app.pty_parser.screen_mut().set_scrollback(new_offset);
                                }
                            }
                        }
                    }
                    Some(Ok(crossterm::event::Event::Paste(text))) => {
                        if app.input_focus == InputFocus::Pty
                            && let Some(pty) = pty.as_mut()
                        {
                            app.pty_parser.screen_mut().set_scrollback(0);
                            let bracketed = app.pty_parser.screen().bracketed_paste();
                            let bytes = encode_paste(&text, bracketed);
                            let _ = pty.write_input(&bytes);
                        }
                    }
                    Some(Ok(_)) => {}
                    Some(Err(e)) => {
                        eprintln!("TUI: event error: {e}");
                        break (1, None);
                    }
                    None => {
                        app.should_quit = Some(QuitReason::TtyLost);
                    }
                }
            }
            ev = rx.recv() => {
                match ev {
                    Some(e) => app.on_event(e),
                    None => {
                        app.should_quit = Some(QuitReason::WsClosed);
                    }
                }
            }
        }
        // Probe-popup "paste to agent": app queues the text (it has no PTY
        // writer); deliver it here as a paste so the agent sees one atomic
        // block instead of keystrokes.
        if let Some(text) = app.pty_paste_pending.take()
            && let Some(pty) = pty.as_mut()
        {
            app.pty_parser.screen_mut().set_scrollback(0);
            let bracketed = app.pty_parser.screen().bracketed_paste();
            let _ = pty.write_input(&encode_paste(&text, bracketed));
        }
        if app.pty_resize_pending {
            app.pty_resize_pending = false;
            let (term_cols, term_rows) = crossterm::terminal::size().unwrap_or((80, 24));
            apply_pty_layout(&mut app, pty.as_deref_mut(), term_cols, term_rows);
        }
        if let Some(reason) = app.should_quit.take() {
            let _ = terminal.draw(|f| view(f, &app));
            let code = match &reason {
                QuitReason::SessionComplete => 0,
                QuitReason::AgentExited(c) => *c,
                QuitReason::UserRequested => 0,
                _ => 1,
            };
            break (code, Some(reason));
        }
    };

    let _ = (session_label, project_label);
    TuiRunOutcome {
        exit_code: outcome.0,
        quit_reason: outcome.1,
        final_status: app.header.status,
    }
}

// ── Tests: pty_inner_rect ─────────────────────────────────────────────────────

/// Re-fit the agent to the current layout (sidebar toggled, tokens panel
/// appeared, terminal resized): resize the OS PTY — SIGWINCH makes the
/// agent repaint for the new width — AND the local `vt100` parser that
/// ololo renders the main pane from. The two must change in lockstep:
/// resizing only the OS side leaves the parser wrapping the agent's
/// repaint against the old grid, which shows up as a stale/mangled pane.
fn apply_pty_layout(app: &mut TuiApp, pty: Option<&mut PtyHost>, term_cols: u16, term_rows: u16) {
    let has_tokens = app.tokens.as_ref().is_some_and(|v| !v.is_empty());
    let chat_view = app.sidebar_view == crate::tui::app::SidebarView::Chat;
    let rect = pty_inner_rect(
        term_cols,
        term_rows,
        app.show_sidebar,
        has_tokens,
        chat_view,
    );
    let (inner_cols, inner_rows) = (rect.width, rect.height);
    if let Some(pty) = pty {
        let _ = pty.resize(inner_rows, inner_cols);
    }
    app.pty_parser.screen_mut().set_size(inner_rows, inner_cols);
    app.pty_cols = inner_cols;
    app.pty_rows = inner_rows;
}

/// Compute the PTY inner content `Rect` (post-border, post-margin) matching
/// the render layout in `render.rs::view`. Single source of truth for pty
/// resize sizing *and* mouse hit-testing/coordinate translation — both need
/// the same rectangle, so both call this instead of re-deriving it.
/// `chat_view` mirrors `render.rs`: the chat sidebar takes 45% of the
/// terminal, so the agent keeps only 55% — sizing the PTY at 80% there
/// made the agent wrap its output past the pane's real edge.
pub(crate) fn pty_inner_rect(
    term_cols: u16,
    term_rows: u16,
    show_sidebar: bool,
    has_tokens: bool,
    chat_view: bool,
) -> Rect {
    if show_sidebar {
        let main_pct = if chat_view {
            0.55f32
        } else if has_tokens {
            0.65f32
        } else {
            0.8f32
        };
        let main_cols = ((term_cols as f32 * main_pct) as u16).max(20);
        Rect::new(
            1,
            2,
            main_cols.saturating_sub(2).max(20),
            term_rows.saturating_sub(3).max(5),
        )
    } else {
        Rect::new(
            2,
            3,
            term_cols.saturating_sub(4).max(20),
            term_rows.saturating_sub(4).max(5),
        )
    }
}

/// Encode a crossterm `MouseEvent` (already hit-test-translated to local pty
/// coordinates) into the byte sequence the embedded agent's own terminal
/// parser expects — gated on the mouse mode/encoding *the agent itself*
/// requested via DECSET (tracked by `vt100::Screen::mouse_protocol_mode` /
/// `mouse_protocol_encoding`, set from the agent's own escape sequences).
/// Mirrors real xterm behavior: no request, no forwarding; `Press` (X10)
/// forwards clicks+wheel only, `PressRelease` adds release, `ButtonMotion`
/// adds drag, `AnyMotion` adds plain hover-move too.
fn mouse_event_to_pty_bytes(
    kind: crossterm::event::MouseEventKind,
    modifiers: crossterm::event::KeyModifiers,
    local_col: u16,
    local_row: u16,
    mode: vt100::MouseProtocolMode,
    encoding: vt100::MouseProtocolEncoding,
) -> Option<Vec<u8>> {
    use crossterm::event::{KeyModifiers, MouseEventKind};
    use vt100::{MouseProtocolEncoding, MouseProtocolMode};

    if mode == MouseProtocolMode::None {
        return None;
    }

    fn button_code(b: crossterm::event::MouseButton) -> u8 {
        match b {
            crossterm::event::MouseButton::Left => 0,
            crossterm::event::MouseButton::Middle => 1,
            crossterm::event::MouseButton::Right => 2,
        }
    }

    // (button code before modifiers, is this a release?)
    let (cb, release) = match kind {
        MouseEventKind::Down(b) => (button_code(b), false),
        MouseEventKind::Up(b) => {
            if mode == MouseProtocolMode::Press {
                // X10 mode reports press only; no release events exist.
                return None;
            }
            (button_code(b), true)
        }
        MouseEventKind::Drag(b) => {
            if !matches!(
                mode,
                MouseProtocolMode::ButtonMotion | MouseProtocolMode::AnyMotion
            ) {
                return None;
            }
            (button_code(b) + 32, false)
        }
        MouseEventKind::Moved => {
            if mode != MouseProtocolMode::AnyMotion {
                return None;
            }
            (3 + 32, false)
        }
        MouseEventKind::ScrollUp => (64, false),
        MouseEventKind::ScrollDown => (65, false),
        MouseEventKind::ScrollLeft => (66, false),
        MouseEventKind::ScrollRight => (67, false),
    };

    let mut cb = cb;
    if modifiers.contains(KeyModifiers::SHIFT) {
        cb += 4;
    }
    if modifiers.contains(KeyModifiers::ALT) {
        cb += 8;
    }
    if modifiers.contains(KeyModifiers::CONTROL) {
        cb += 16;
    }

    let cx = local_col + 1;
    let cy = local_row + 1;

    match encoding {
        MouseProtocolEncoding::Sgr => {
            let term = if release { 'm' } else { 'M' };
            Some(format!("\x1b[<{cb};{cx};{cy}{term}").into_bytes())
        }
        MouseProtocolEncoding::Default | MouseProtocolEncoding::Utf8 => {
            // ponytail: legacy 1-byte-per-field X10 encoding, clamped at 223
            // (byte 255) same as real xterm; release always reports button 3
            // (X10 can't tell which button let go). Full UTF-8 coordinate
            // encoding (>223) isn't implemented — SGR above is the correct,
            // uncapped path and is what every modern TUI requests.
            let cb_x10 = if release { 3 } else { cb };
            let clamp = |v: u16| -> u8 { v.min(223) as u8 + 32 };
            Some(vec![0x1b, b'[', b'M', cb_x10 + 32, clamp(cx), clamp(cy)])
        }
    }
}

/// xterm's "alternateScroll" behavior (DECSET 1007, on by default in real
/// terminals): when the foreground app is on the alternate screen and has
/// NOT enabled its own mouse reporting, the terminal translates wheel
/// scroll into arrow-key presses instead of doing nothing. Nearly every
/// interactive CLI already handles Up/Down (history, line navigation), so
/// this is what makes scrolling work no matter which agent is running,
/// not just ones that implement xterm's mouse protocol themselves (e.g.
/// opencode does; plenty of others, `pi` included, don't).
///
/// Only called as a fallback when `mouse_event_to_pty_bytes` returned
/// `None` — i.e. the agent isn't already handling this event itself.
fn mouse_scroll_fallback_bytes(kind: crossterm::event::MouseEventKind) -> Option<Vec<u8>> {
    use crossterm::event::MouseEventKind;
    // ponytail: fixed 3 lines/notch (common terminal default); a
    // configurable speed is the upgrade path if this ever feels off.
    const WHEEL_LINES: usize = 3;
    let letter: u8 = match kind {
        MouseEventKind::ScrollUp => b'A',
        MouseEventKind::ScrollDown => b'B',
        _ => return None,
    };
    let mut bytes = Vec::with_capacity(3 * WHEEL_LINES);
    for _ in 0..WHEEL_LINES {
        bytes.extend_from_slice(&[0x1b, b'[', letter]);
    }
    Some(bytes)
}

/// Signed delta (in `vt100::Screen::scrollback()` offset units) for a wheel
/// event: positive moves further back in history, matching xterm's
/// `alternateScroll` line-count convention. `None` for click/drag/plain-move
/// and horizontal wheel -- there's no universal "further back" meaning for
/// those. Same `WHEEL_LINES` speed as `mouse_scroll_fallback_bytes` so the
/// two feel consistent regardless of which one ends up handling a given
/// scroll (agent's own scrollback, alt-screen arrow-key translation, or
/// ololo's native primary-screen scrollback).
fn scrollback_delta(kind: crossterm::event::MouseEventKind) -> Option<isize> {
    use crossterm::event::MouseEventKind;
    const WHEEL_LINES: isize = 3;
    match kind {
        MouseEventKind::ScrollUp => Some(WHEEL_LINES),
        MouseEventKind::ScrollDown => Some(-WHEEL_LINES),
        _ => None,
    }
}

// ── Tests: scrollback_delta ────────────────────────────────────────────────────

// ── Tests: mouse_scroll_fallback_bytes ────────────────────────────────────────

/// Bracketed paste (`vt100::Screen::bracketed_paste`), wraps it in
/// `ESC[200~ ... ESC[201~` so the agent treats it as one atomic paste
/// instead of a burst of individual keystrokes (which is what crossterm
/// would otherwise deliver one `Event::Key` at a time, indistinguishable
/// from real typing — e.g. a pasted Tab would toggle ololo's own focus).
fn encode_paste(text: &str, bracketed: bool) -> Vec<u8> {
    if bracketed {
        format!("\x1b[200~{text}\x1b[201~").into_bytes()
    } else {
        text.as_bytes().to_vec()
    }
}

// ── Tests: encode_paste ───────────────────────────────────────────────────────

/// Hit-test an absolute terminal `(col, row)` mouse position against the
/// pty inner `Rect` (from `pty_inner_rect`), returning 0-based local
/// coordinates inside the pty's own `pty_cols` x `pty_rows` grid, or `None`
/// if the point falls outside the block (header, sidebar, borders, margins).
fn mouse_to_pty_local(col: u16, row: u16, rect: Rect) -> Option<(u16, u16)> {
    if rect.width == 0 || rect.height == 0 {
        return None;
    }
    if col < rect.x || row < rect.y {
        return None;
    }
    let local_col = col - rect.x;
    let local_row = row - rect.y;
    if local_col >= rect.width || local_row >= rect.height {
        return None;
    }
    Some((local_col, local_row))
}

// ── Tests: mouse_event_to_pty_bytes ───────────────────────────────────────────

// ── Tests: mouse_to_pty_local ─────────────────────────────────────────────────

#[cfg(test)]
mod tests;
