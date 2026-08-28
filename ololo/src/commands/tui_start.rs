//! `ololo start --tui` / `ololo join --tui` TUI-mode entry point.

use anyhow::{Result, anyhow};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::player_ws;
use crate::snapshot;
use crate::tui::app::{InputFocus, PTY_SCROLLBACK_LINES, TuiApp};
use crate::tui::event::{Origin, TuiEvent};
use crate::tui::header::HeaderState;
use crate::tui::pty_host::{PtyHost, SpawnArgs};
use crate::tui::{self, log_redir};
use crate::util;

/// Open the snapshot repo for a session, write the session-start commit,
/// and push it. Best-effort: any failure logs and returns `None` — probes
/// and scoring never depend on snapshotting. The returned baseline tree id
/// feeds the TUI's diff task; headless mode ignores it.
fn init_snapshot_repo(
    profile: &str,
    join_code: &str,
    worktree: &std::path::Path,
    git_remote_url: Option<String>,
    pat: String,
) -> Option<(
    Arc<std::sync::Mutex<snapshot::SnapshotRepo>>,
    Option<gix::ObjectId>,
)> {
    match snapshot::SnapshotRepo::new(profile, join_code, worktree, git_remote_url, Some(pat)) {
        Ok(snap) => {
            if let Err(e) = snap.commit_session_start() {
                tracing::warn!("session-start snapshot commit failed: {e}");
                return None;
            }
            let _ = snap.push_to_remote();
            let baseline = match snap.head_tree_id() {
                Ok(t) => Some(t),
                Err(e) => {
                    tracing::warn!("snapshot head_tree_id failed, skipping diff task: {e}");
                    None
                }
            };
            Some((Arc::new(std::sync::Mutex::new(snap)), baseline))
        }
        Err(e) => {
            tracing::warn!("snapshot repo init failed: {e}");
            None
        }
    }
}

/// Memory sources (AGENTS.md / README.md) are published as the player edits
/// them, and completion flag files (`.ololo/*-done.md`) the moment they
/// land. Both need the snapshot repo; both report their pushes over the
/// same frame channel.
fn spawn_sync_tasks(
    snap: &Arc<std::sync::Mutex<snapshot::SnapshotRepo>>,
    sink: Arc<dyn crate::tui::EventSink>,
) -> (
    player_ws::MemoryChannel,
    tokio::task::JoinHandle<()>,
    tokio::task::JoinHandle<()>,
) {
    let (frame_tx, frame_rx) = tokio::sync::mpsc::unbounded_channel();
    let (handle, mem_task) = crate::memory_sync::spawn(Arc::clone(snap), frame_tx.clone());
    let flag_task = crate::done_flag::spawn(Arc::clone(snap), frame_tx, Some(sink));
    (
        player_ws::MemoryChannel {
            handle,
            frames: frame_rx,
        },
        mem_task,
        flag_task,
    )
}

/// PTY geometry for a headless-hosted agent: no terminal dictates a size,
/// so pick one wide enough that agent TUIs (claude, codex, …) render
/// readable `screen.txt` dumps.
const HEADLESS_PTY_COLS: u16 = 160;
const HEADLESS_PTY_ROWS: u16 = 48;

/// Headless mode for `start`/`join` (`--no-tui`): no terminal UI — probes
/// execute in the current directory, snapshots, memory sync, and
/// completion-flag watching all work exactly as in TUI mode, and progress
/// is narrated as plain lines.
///
/// Without `--agent` the "agent" is whoever edits the working directory —
/// a human in another terminal, or a coding agent like Claude Code
/// validating a project end-to-end. With `--agent` the given agent is
/// hosted in an invisible PTY and driven through the control channel
/// (autonomous play — a parent agent writes message files, reads
/// `screen.txt`).
pub async fn run_headless_start(
    profile: &str,
    cfg: &crate::config::Config,
    join_code: String,
    project_slug: String,
    git_remote_url: Option<String>,
    pat: String,
    agent: Option<String>,
) -> Result<()> {
    use crate::tui::event::BusSink;

    let (bus, rx, dropped) = BusSink::new();
    let bus_dyn: Arc<dyn crate::tui::EventSink> = bus.clone();

    let header = HeaderState::new(join_code.clone(), project_slug.clone());
    let http_base = util::http_base_url(&cfg.server_url);
    // Rendered never — but with a hosted agent the parser feeds the
    // control channel's screen.txt, so size it to the agent PTY.
    let (pty_cols, pty_rows) = if agent.is_some() {
        (HEADLESS_PTY_COLS, HEADLESS_PTY_ROWS)
    } else {
        (80, 24)
    };
    let parser = vt100::Parser::new(pty_rows, pty_cols, 0);
    let mut app = TuiApp::new_for_test(header, parser, dropped, pty_cols, pty_rows);
    app.header.session_url = format!("{http_base}/s/{join_code}");
    app.header.project_url = format!("{http_base}/projects/{project_slug}");

    // Host the agent (if any) before any network work, like TUI mode does.
    let (maybe_pty, pty_rx_opt) = match agent.as_deref() {
        Some(spec) => {
            let agent_cmd = crate::agent_cmd::AgentCommand::parse(spec)?;
            let agent_name = agent_cmd.program_name().to_string();
            if crate::tui::agent_picker::agent_kind(&agent_name)
                == crate::tui::agent_picker::AgentKind::Desktop
            {
                anyhow::bail!(
                    "'{agent_name}' is a desktop agent — headless hosting needs a terminal agent"
                );
            }
            let agent_bin = which::which(&agent_cmd.program)
                .map_err(|e| anyhow!("agent '{}' not found on $PATH: {e}", agent_cmd.program))?;
            let mut pty = PtyHost::spawn(SpawnArgs {
                program: &agent_bin.to_string_lossy(),
                args: &agent_cmd.args,
                rows: pty_rows,
                cols: pty_cols,
                env: &[("TERM".to_string(), "xterm-256color".to_string())],
                cwd: std::env::current_dir().ok().as_deref(),
            })
            .map_err(|e| anyhow!("failed to spawn agent '{agent_name}' in PTY: {e}"))?;
            let rx = pty.spawn_reader();
            app.has_pty = true;
            app.agent_label = agent_cmd.display();
            (Some(pty), rx)
        }
        None => (None, None),
    };

    let control = if maybe_pty.is_some() {
        crate::control::control_dir(&join_code).map(|dir| {
            crate::ui::field("control", dir.display().to_string());
            crate::ui::hint(
                "Drop *.md message files into control/inbox to talk to the hosted agent; \
                 read control/screen.txt for its screen.",
            );
            crate::control::ControlChannel::spawn(dir)
        })
    } else {
        None
    };

    let worktree_cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    let maybe_snapshot = init_snapshot_repo(
        profile,
        &join_code,
        &worktree_cwd,
        git_remote_url,
        pat.clone(),
    )
    .map(|(snap, _baseline)| snap);

    let (memory_channel, _memory_task, _flag_task) = match maybe_snapshot.as_ref() {
        Some(snap) => {
            app.snapshot = Some(Arc::clone(snap));
            let (channel, mem_task, flag_task) = spawn_sync_tasks(snap, bus_dyn.clone());
            (Some(channel), Some(mem_task), Some(flag_task))
        }
        None => (None, None, None),
    };

    let ws_base = util::ws_base_url(&cfg.server_url);
    let ws_code = join_code.clone();
    let ws_pat = cfg.token.clone();
    let _probe_task = tokio::spawn(async move {
        if let Err(e) =
            player_ws::run_with_sink(&ws_base, &ws_code, &ws_pat, Some(bus_dyn), memory_channel)
                .await
        {
            tracing::warn!("player_ws error: {e}");
        }
    });
    // The WS task now holds the only sink: when it ends, the bus closes and
    // the headless loop sees it. Holding `bus` here would keep the channel
    // open forever and hang the loop on a dead socket.
    drop(bus);

    let (stats_tx, stats_handle) =
        crate::task_stats::spawn_reporter(cfg.server_url.clone(), pat, join_code.clone());
    app.stats_tx = Some(stats_tx);

    crate::ui::hint(
        "Headless play: probes run in this directory. Build here; \
         declare completion the way the task says (e.g. its .ololo flag file).",
    );

    let outcome = crate::tui::run_headless(app, rx, maybe_pty, pty_rx_opt, control).await;

    {
        use crate::tui::QuitReason;
        use crate::tui::Status;
        match (&outcome.quit_reason, outcome.final_status) {
            (Some(QuitReason::SessionComplete), Status::Cancelled) => {
                crate::ui::warn("Session cancelled — no more probes will run.");
                crate::ui::field("session", &join_code);
            }
            (Some(QuitReason::SessionComplete), _) => {
                crate::ui::success("Session finished — results are on the session page.");
                crate::ui::field("session", &join_code);
            }
            (Some(QuitReason::WsClosed), _) => {
                crate::ui::warn("Connection to the game server was lost.");
                crate::ui::hint(format!(
                    "If the session is still running, reconnect with: ololo join {join_code}"
                ));
            }
            _ => {}
        }
    }

    // Final snapshot commit (best-effort, 5s timeout).
    if let Some(snap_arc) = maybe_snapshot {
        let commit = tokio::task::spawn_blocking(move || {
            let snap = snap_arc.lock().map_err(|e| anyhow!("{e}"))?;
            snap.commit_final().map_err(|e| anyhow!("{e}"))?;
            snap.push_to_remote().map_err(|e| anyhow!("{e}"))
        });
        match tokio::time::timeout(Duration::from_secs(5), commit).await {
            Ok(Ok(Ok(()))) => {}
            Ok(Ok(Err(e))) => tracing::warn!("final snapshot commit failed: {e}"),
            Ok(Err(e)) => tracing::warn!("final snapshot commit join error: {e}"),
            Err(_) => tracing::warn!("final snapshot lost: commit timed out after 5s"),
        }
    }

    // The drain must survive one full agent-log scan (tens of seconds on a
    // machine with a season of logs) — a 10s cap dropped the stats of every
    // session that finished with reports still queued.
    if !stats_handle.is_finished() {
        crate::ui::step("Uploading task stats (scanning local agent logs)…");
    }
    match tokio::time::timeout(Duration::from_secs(90), stats_handle).await {
        Ok(_) => {}
        Err(_) => tracing::warn!("task stats reporter timed out; pending reports dropped"),
    }

    if outcome.exit_code != 0 {
        std::process::exit(outcome.exit_code);
    }
    Ok(())
}

/// TUI mode for `start`: sets up the file-writing tracing subscriber,
/// builds the bus, spawns the player-agent WebSocket with the bus
/// as its `EventSink`, and hands control to the ratatui render
/// loop. Returns the process exit code.
pub async fn run_tui_start(
    profile: &str,
    cfg: &crate::config::Config,
    join_code: String,
    project_slug: String,
    debug: bool,
    agent: String,
    git_remote_url: Option<String>,
    pat: String,
) -> Result<()> {
    use crate::tui::event::BusSink;

    let _handles =
        log_redir::setup_tracing(profile).map_err(|e| anyhow!("tracing setup failed: {e}"))?;

    // Build the bus.
    let (bus, rx, dropped) = BusSink::new();
    let bus_dyn: Arc<dyn crate::tui::EventSink> = bus.clone();

    // Build the TuiApp with the joining session metadata.
    let mut header = HeaderState::new(join_code.clone(), project_slug.clone());
    // Cache-only read (no network): the passive check in main() refreshes it.
    header.update_available = crate::commands::update::cached_newer_version();

    // `agent` is already resolved by the caller (run_start/run_join), which
    // runs detection + selection before handing it here. Don't re-prompt.
    // It may be a full command line ("claude --model glm-5.2:cloud").
    let agent_cmd = crate::agent_cmd::AgentCommand::parse(&agent)?;
    let agent_name = agent_cmd.program_name().to_string();
    let (term_cols, term_rows) = crossterm::terminal::size().unwrap_or((80, 24));
    // The agent PTY must be spawned at the size its pane will actually
    // render at — the default chat sidebar takes 45%, so the agent gets
    // 55% (sidebar on, no tokens yet). A wider guess here made agents
    // wrap their output past the visible edge until the first relayout.
    let initial_rect = crate::tui::app::pty_inner_rect(term_cols, term_rows, true, false, true);
    let (pty_cols, pty_rows) = (initial_rect.width, initial_rect.height);
    let parser = vt100::Parser::new(pty_rows, pty_cols, PTY_SCROLLBACK_LINES);
    let mut app = TuiApp::new_for_test(header, parser, dropped, pty_cols, pty_rows);

    // Session page link for the header (clickable in OSC 8 terminals).
    let http_base = util::http_base_url(&cfg.server_url);
    app.header.session_url = format!("{http_base}/s/{}", join_code);
    app.header.project_url = format!("{http_base}/projects/{}", project_slug);

    let mut maybe_snapshot: Option<Arc<std::sync::Mutex<snapshot::SnapshotRepo>>> = None;
    let mut _git_diff_task: Option<tokio::task::JoinHandle<()>> = None;

    let agent_bin = which::which(&agent_cmd.program)
        .map_err(|e| anyhow!("agent '{}' not found on $PATH: {e}", agent_cmd.program))?;
    app.agent_label = agent_cmd.display();
    app.show_sidebar = true;

    // A desktop agent draws in its own window. Hosting it in a PTY leaves an
    // empty pane that reads as a hang, and the launcher usually exits at once
    // anyway — so start it detached in the working directory and let the
    // agent pane explain where it went.
    let (mut maybe_pty, mut pty_rx_opt) = match crate::tui::agent_picker::agent_kind(&agent_name) {
        crate::tui::agent_picker::AgentKind::Desktop => {
            app.agent_is_desktop = true;
            app.input_focus = InputFocus::Tui;
            let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
            // Best-effort: a launcher that fails to start is reported and the
            // session continues — probes and scoring do not depend on it.
            match std::process::Command::new(&agent_bin)
                .args(&agent_cmd.args)
                .arg(&cwd)
                .spawn()
            {
                Ok(_) => tracing::info!(agent = %agent_name, "launched desktop agent"),
                Err(e) => {
                    tracing::warn!(agent = %agent_name, "desktop agent failed to launch: {e}")
                }
            }
            (None, None)
        }
        crate::tui::agent_picker::AgentKind::Terminal => {
            let mut pty = PtyHost::spawn(SpawnArgs {
                program: &agent_bin.to_string_lossy(),
                args: &agent_cmd.args,
                rows: pty_rows,
                cols: pty_cols,
                env: &[("TERM".to_string(), "xterm-256color".to_string())],
                cwd: std::env::current_dir().ok().as_deref(),
            })
            .map_err(|e| anyhow!("failed to spawn agent '{agent_name}' in PTY: {e}"))?;
            let rx = pty.spawn_reader();
            app.has_pty = true;
            app.input_focus = InputFocus::Pty;
            (Some(pty), rx)
        }
    };

    // ponytail: snapshot init is best-effort — failures are logged and swallowed.
    let worktree_cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    if let Some((snap_arc, baseline)) = init_snapshot_repo(
        profile,
        &join_code,
        &worktree_cwd,
        git_remote_url.clone(),
        pat.clone(),
    ) {
        if let Some(baseline_tree) = baseline {
            let git_dir = snap_arc
                .lock()
                .map(|g| g.git_dir().to_path_buf())
                .unwrap_or_default();
            _git_diff_task = Some(crate::tui::git_diff::spawn_git_diff_task(
                bus_dyn.clone(),
                git_dir,
                worktree_cwd,
                baseline_tree,
            ));
        }
        app.snapshot = Some(snap_arc.clone());
        maybe_snapshot = Some(snap_arc);
    }

    let (memory_channel, _memory_task, _flag_task) = match maybe_snapshot.as_ref() {
        Some(snap) => {
            let (channel, mem_task, flag_task) = spawn_sync_tasks(snap, bus_dyn.clone());
            (Some(channel), Some(mem_task), Some(flag_task))
        }
        None => (None, None, None),
    };

    // Spawn the player-agent WebSocket loop with the bus as the sink.
    let ws_base = util::ws_base_url(&cfg.server_url);
    let ws_code = join_code.clone();
    let ws_pat = cfg.token.clone();
    let sink_for_ws = bus_dyn.clone();
    let _probe_task = tokio::spawn(async move {
        if let Err(e) = player_ws::run_with_sink(
            &ws_base,
            &ws_code,
            &ws_pat,
            Some(sink_for_ws),
            memory_channel,
        )
        .await
        {
            tracing::warn!("player_ws error: {e}");
        }
    });

    // Spawn the agent-tokens watch task — feeds token usage data into the TUI bus.
    // ponytail: 5s hardcoded interval; add --tokens-interval flag if tuning is needed.
    // Only sessions started at/after TUI launch are forwarded — past sessions skipped.
    let session_start_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    let tokens_sink = bus_dyn.clone();
    let _tokens_task = tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(5)).await;
            // agent-tokens scans local files/DBs — keep it off the runtime.
            let Ok((counts, stats)) = tokio::task::spawn_blocking(move || {
                (
                    agent_tokens::snapshot(Some(session_start_ms)),
                    agent_tokens::stats_snapshot(Some(session_start_ms)),
                )
            })
            .await
            else {
                continue;
            };
            let counts: Vec<_> = counts
                .into_iter()
                .filter(|s| s.last_seen_at_ms.is_some_and(|t| t >= session_start_ms))
                .collect();
            let stats: Vec<_> = stats
                .into_iter()
                .filter(|s| s.last_seen_at_ms.is_some_and(|t| t >= session_start_ms))
                .collect();
            if counts.is_empty() {
                continue;
            }
            if tokens_sink
                .send(Origin::Network, TuiEvent::TokensUpdate { counts, stats })
                .is_err()
            {
                break;
            }
        }
    });

    // The Antigravity IDE keeps no usage log on disk — usage lives behind
    // its language server's local RPC. When that agent is picked, a
    // background sync mirrors the RPC data into the cache the agent-tokens
    // extractor reads, so the polls above see it like any other agent log.
    let _antigravity_sync_task = crate::antigravity_sync::spawn_if_selected(&agent_name);

    // Spawn the task-stats reporter: when a task completes, agent activity
    // for its window is collected from local agent logs and POSTed to the
    // server (shown on the player's session page). Best-effort.
    let (stats_tx, stats_handle) =
        crate::task_stats::spawn_reporter(cfg.server_url.clone(), pat.clone(), join_code.clone());
    app.stats_tx = Some(stats_tx);

    let _ = debug; // reserved for future use

    // Control channel: lets a parent agent (or a debugging human in another
    // terminal) inject messages into the hosted agent and read its screen
    // while the TUI runs — e.g. an ololo TUI inside tmux driven by Claude.
    let control = if maybe_pty.is_some() {
        crate::control::control_dir(&join_code).map(|dir| {
            tracing::info!(dir = %dir.display(), "control channel active");
            crate::control::ControlChannel::spawn(dir)
        })
    } else {
        None
    };

    // Run the render loop until the user quits or the session ends.
    let outcome = tui::run(
        app,
        rx,
        &join_code,
        &project_slug,
        maybe_pty.as_mut(),
        &mut pty_rx_opt,
        control,
    )
    .await;
    let exit_code = outcome.exit_code;

    // The alternate screen just vanished with everything on it. Say why the
    // TUI closed, in the terminal the player is now looking at.
    {
        use crate::tui::QuitReason;
        use crate::tui::Status;
        match (&outcome.quit_reason, outcome.final_status) {
            (Some(QuitReason::SessionComplete), Status::Cancelled) => {
                crate::ui::warn("Session cancelled — no more probes will run.");
                crate::ui::field("session", &join_code);
            }
            (Some(QuitReason::SessionComplete), _) => {
                crate::ui::success("Session finished — results are on the session page.");
                crate::ui::field("session", &join_code);
            }
            (Some(QuitReason::WsClosed), _) => {
                crate::ui::warn("Connection to the game server was lost.");
                crate::ui::hint(format!(
                    "If the session is still running, reconnect with: ololo join {join_code}"
                ));
            }
            _ => {}
        }
    }

    // Abort the git diff task before commit_final to prevent concurrent stage_all.
    if let Some(h) = _git_diff_task {
        h.abort();
    }

    // Final snapshot commit (best-effort, 5s timeout) before PTY teardown.
    if let Some(snap_arc) = maybe_snapshot {
        let commit = tokio::task::spawn_blocking(move || {
            let snap = snap_arc.lock().map_err(|e| anyhow!("{e}"))?;
            snap.commit_final().map_err(|e| anyhow!("{e}"))?;
            snap.push_to_remote().map_err(|e| anyhow!("{e}"))
        });
        match tokio::time::timeout(Duration::from_secs(5), commit).await {
            Ok(Ok(Ok(()))) => {}
            Ok(Ok(Err(e))) => tracing::warn!("final snapshot commit failed: {e}"),
            Ok(Err(e)) => tracing::warn!("final snapshot commit join error: {e}"),
            Err(_) => tracing::warn!("final snapshot lost: commit timed out after 5s"),
        }
    }

    // Drain pending task-stats reports (the app — and with it the channel
    // sender — was dropped when the render loop returned, so the reporter
    // exits once its queue is empty).
    // The drain must survive one full agent-log scan (tens of seconds on a
    // machine with a season of logs) — a 10s cap dropped the stats of every
    // session that finished with reports still queued.
    if !stats_handle.is_finished() {
        crate::ui::step("Uploading task stats (scanning local agent logs)…");
    }
    match tokio::time::timeout(Duration::from_secs(90), stats_handle).await {
        Ok(_) => {}
        Err(_) => tracing::warn!("task stats reporter timed out; pending reports dropped"),
    }

    if let Some(mut pty) = maybe_pty {
        let _ = pty.shutdown().await;
    }
    if exit_code != 0 {
        std::process::exit(exit_code);
    }
    Ok(())
}
