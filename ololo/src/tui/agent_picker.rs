#![allow(dead_code)]

//! Agent picker: validates a `--launch <bin>` argument or selects
//! from the `probe::AI_AGENT_NAMES` list filtered by `which::which`.
//!
//! This module is the pre-flight non-interactive validator. The
//! interactive ratatui picker UI is hosted by the TUI's render
//! layer (WP-005) and uses `select_agent` to validate the user's
//! choice before the agent PTY is spawned.

use crate::probe::AI_AGENT_NAMES;
use anyhow::{Result, anyhow, bail};
use std::path::{Path, PathBuf};

use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Widget};

#[derive(Debug, Clone)]
pub struct PickedAgent {
    pub command: String,
    pub argv: Vec<String>,
    pub source: AgentSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentSource {
    Opencode,
    Claude,
    Other,
}

/// How an agent presents itself once launched.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentKind {
    /// Draws in the terminal, so `ololo` hosts it in the PTY pane.
    Terminal,
    /// Opens its own window. Hosting it in a PTY yields an empty pane that
    /// looks like a hang, so it is launched detached and the pane explains
    /// where the agent went.
    Desktop,
}

/// Binaries that open a window instead of drawing in the terminal.
///
/// Deliberately a short allow-list of launchers we are sure about rather than
/// a guess: mistaking a terminal agent for a desktop one costs the
/// participant their agent pane, which is worse than the empty pane this
/// avoids. CLI agents that merely belong to a desktop product (Cursor's
/// `cursor-agent`, Antigravity's `agy`) are terminal agents and stay off it.
const DESKTOP_AGENTS: &[&str] = &["zed", "cursor", "antigravity"];

/// Classify a picked agent binary by how it presents itself.
pub fn agent_kind(binary: &str) -> AgentKind {
    // The picker hands over a bare name, `--launch` may hand over a path.
    let stem = std::path::Path::new(binary)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(binary);
    if DESKTOP_AGENTS.contains(&stem) {
        AgentKind::Desktop
    } else {
        AgentKind::Terminal
    }
}

pub struct PickerArgs<'a> {
    pub launch_override: Option<&'a str>,
    pub detected: &'a [String],
}

fn validate_name(name: &str) -> Result<()> {
    if name.contains('\n') || name.contains('\r') || name.contains('\0') {
        bail!("agent path contains forbidden byte: {name:?}");
    }
    Ok(())
}

fn validate_executable(path: &Path) -> Result<()> {
    if !path.is_file() {
        bail!("agent path '{}' is not a file", path.display());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(path)?.permissions().mode();
        if mode & 0o111 == 0 {
            bail!("agent '{}' is not executable", path.display());
        }
    }
    Ok(())
}

fn lookup(name: &str) -> Result<PathBuf> {
    let path =
        which::which(name).map_err(|_| anyhow!("agent binary '{name}' not found on $PATH"))?;
    let canonical = std::fs::canonicalize(&path)
        .map_err(|e| anyhow!("agent '{name}' canonicalize failed: {e}"))?;
    validate_executable(&canonical)?;
    Ok(canonical)
}

pub fn select_agent(args: PickerArgs<'_>) -> Result<PickedAgent> {
    if let Some(cmd) = args.launch_override {
        validate_name(cmd)?;
        let path = lookup(cmd)?;
        let source = match cmd {
            "opencode" => AgentSource::Opencode,
            "claude" => AgentSource::Claude,
            _ => AgentSource::Other,
        };
        return Ok(PickedAgent {
            command: path.to_string_lossy().to_string(),
            argv: vec![],
            source,
        });
    }
    if args.detected.is_empty() {
        bail!("no AI coding agent detected on $PATH (opencode, claude); pass --launch <bin>");
    }
    // Prefer opencode, then claude, then first detected.
    let chosen = args
        .detected
        .iter()
        .find(|n| *n == "opencode")
        .or_else(|| args.detected.iter().find(|n| *n == "claude"))
        .unwrap_or_else(|| &args.detected[0]);
    let path = lookup(chosen)?;
    let source = match chosen.as_str() {
        "opencode" => AgentSource::Opencode,
        "claude" => AgentSource::Claude,
        _ => AgentSource::Other,
    };
    Ok(PickedAgent {
        command: path.to_string_lossy().to_string(),
        argv: vec![],
        source,
    })
}

pub fn detected_agents() -> Vec<String> {
    AI_AGENT_NAMES
        .iter()
        .filter(|name| which::which(name).is_ok())
        .map(|s| s.to_string())
        .collect()
}

pub fn revalidate(picked: &PickedAgent) -> Result<()> {
    let path = std::path::Path::new(&picked.command);
    if !path.is_file() {
        bail!("agent '{}' missing at session start", picked.command);
    }
    Ok(())
}

/// Interactive ratatui selector for the detected AI coding agents, plus
/// a "custom command…" row for launching anything on $PATH with args.
/// Returns the chosen agent name or command line, or `None` if the user
/// cancelled (Esc/q) or the terminal couldn't be taken over. Prefers
/// `opencode`, then `claude`, then the first detected as the initial
/// highlight. Blocking: call inside a tokio runtime (uses
/// `block_in_place` so the rest of the async app keeps running).
pub fn pick_agent_tui(detected: &[String]) -> Option<String> {
    tokio::task::block_in_place(|| run_picker(detected))
}

/// Label of the synthetic last picker row that opens the free-form
/// command input ("claude --model glm-5.2:cloud", any binary on $PATH).
const CUSTOM_ENTRY: &str = "custom command…";

/// Validate a typed custom command: parseable, and its program resolves
/// on $PATH. Returns a user-facing error message on failure.
fn validate_custom_command(input: &str) -> Result<(), String> {
    let cmd = crate::agent_cmd::AgentCommand::parse(input).map_err(|e| e.to_string())?;
    which::which(&cmd.program)
        .map(|_| ())
        .map_err(|_| format!("'{}' not found on $PATH", cmd.program))
}

fn run_picker(detected: &[String]) -> Option<String> {
    use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
    use crossterm::execute;
    use ratatui::backend::CrosstermBackend;
    use std::io::stdout;

    // Detected agents plus the free-form entry as the last row.
    let mut rows: Vec<String> = detected.to_vec();
    rows.push(CUSTOM_ENTRY.to_string());

    let preferred = detected
        .iter()
        .position(|n| n == "opencode")
        .or_else(|| detected.iter().position(|n| n == "claude"))
        .unwrap_or(0);
    let mut state = ListState::default();
    state.select(Some(preferred));

    if crossterm::terminal::enable_raw_mode().is_err() {
        return None;
    }
    let _ = execute!(
        stdout(),
        crossterm::terminal::EnterAlternateScreen,
        crossterm::event::EnableMouseCapture,
    );

    let backend = CrosstermBackend::new(stdout());
    let mut terminal = match ratatui::Terminal::new(backend) {
        Ok(t) => t,
        Err(_) => {
            cleanup_picker();
            return None;
        }
    };

    // None → list mode; Some((buffer, error)) → typing a custom command.
    let mut input: Option<(String, Option<String>)> = None;

    let result = loop {
        match &input {
            None => draw_picker(&mut terminal, &rows, &mut state),
            Some((buf, err)) => draw_custom_input(&mut terminal, buf, err.as_deref()),
        }
        let Ok(ev) = event::read() else { break None };
        let Event::Key(k) = ev else {
            if matches!(ev, Event::Resize(..)) {
                continue;
            }
            break None;
        };
        if k.kind != KeyEventKind::Press {
            continue;
        }
        if let Some((buf, err)) = &mut input {
            match k.code {
                KeyCode::Enter => {
                    let candidate = buf.trim().to_string();
                    if candidate.is_empty() {
                        continue;
                    }
                    match validate_custom_command(&candidate) {
                        Ok(()) => break Some(candidate),
                        Err(msg) => *err = Some(msg),
                    }
                }
                KeyCode::Esc => input = None,
                KeyCode::Backspace => {
                    buf.pop();
                    *err = None;
                }
                KeyCode::Char('u') if k.modifiers.contains(KeyModifiers::CONTROL) => {
                    buf.clear();
                    *err = None;
                }
                KeyCode::Char(c) if !k.modifiers.contains(KeyModifiers::CONTROL) => {
                    buf.push(c);
                    *err = None;
                }
                _ => {}
            }
            continue;
        }
        match k.code {
            KeyCode::Up | KeyCode::Char('k') => {
                let i = state.selected().unwrap_or(0);
                if i > 0 {
                    state.select(Some(i - 1));
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                let i = state.selected().unwrap_or(0);
                if i + 1 < rows.len() {
                    state.select(Some(i + 1));
                }
            }
            KeyCode::Enter => {
                let i = state.selected().unwrap_or(preferred);
                if i == rows.len() - 1 {
                    input = Some((String::new(), None));
                } else {
                    break rows.get(i).cloned();
                }
            }
            KeyCode::Esc | KeyCode::Char('q') => break None,
            KeyCode::Char('g') if k.modifiers.contains(KeyModifiers::CONTROL) => {
                state.select(Some(0));
            }
            KeyCode::Char('G') if k.modifiers.contains(KeyModifiers::SHIFT) => {
                state.select(Some(rows.len() - 1));
            }
            _ => {}
        }
    };

    cleanup_picker();
    result
}

fn draw_custom_input(
    terminal: &mut ratatui::Terminal<ratatui::backend::CrosstermBackend<std::io::Stdout>>,
    buffer: &str,
    error: Option<&str>,
) {
    let _ = terminal.draw(|f| {
        let area = centered_box(f.area(), 70, 4);
        Clear.render(area, f.buffer_mut());
        let block = Block::default()
            .title(" Custom agent command ")
            .borders(Borders::ALL);
        let inner = block.inner(area);
        f.render_widget(block, area);

        let splits = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),
                Constraint::Length(1),
                Constraint::Length(1),
                Constraint::Min(0),
            ])
            .split(inner);
        f.render_widget(
            Paragraph::new(Line::from(vec![
                Span::raw("  > "),
                Span::styled(buffer, Style::default().add_modifier(Modifier::BOLD)),
                Span::styled("▏", Style::default().fg(Color::Cyan)),
            ])),
            splits[0],
        );
        let hint: Line = match error {
            Some(msg) => Line::from(Span::styled(
                format!("  ✗ {msg}"),
                Style::default().fg(Color::Red),
            )),
            None => Line::from(Span::styled(
                "  e.g. claude --model glm-5.2:cloud",
                Style::default().fg(Color::DarkGray),
            )),
        };
        f.render_widget(Paragraph::new(hint), splits[1]);
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "Enter launch · Esc back",
                Style::default().fg(Color::DarkGray),
            )))
            .alignment(Alignment::Center),
            splits[2],
        );
    });
}

fn cleanup_picker() {
    use crossterm::execute;
    use std::io::stdout;
    let _ = crossterm::terminal::disable_raw_mode();
    let _ = execute!(
        stdout(),
        crossterm::terminal::LeaveAlternateScreen,
        crossterm::event::DisableMouseCapture,
    );
}

fn draw_picker(
    terminal: &mut ratatui::Terminal<ratatui::backend::CrosstermBackend<std::io::Stdout>>,
    detected: &[String],
    state: &mut ListState,
) {
    let _ = terminal.draw(|f| {
        let area = centered_box(f.area(), 50, detected.len() as u16 + 4);
        Clear.render(area, f.buffer_mut());
        let block = Block::default()
            .title(" Select AI coding agent ")
            .borders(Borders::ALL);
        let inner = block.inner(area);
        f.render_widget(block, area);

        let items: Vec<ListItem> = detected
            .iter()
            .map(|name| ListItem::new(Line::from(Span::raw(format!("  {name}")))))
            .collect();
        let list = List::new(items)
            .highlight_style(
                Style::default()
                    .bg(Color::Cyan)
                    .fg(Color::Black)
                    .add_modifier(Modifier::BOLD),
            )
            .highlight_symbol("▶ ");

        let splits = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(0), Constraint::Length(1)])
            .split(inner);
        f.render_stateful_widget(list, splits[0], state);
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "↑/↓ move · Enter select · Esc cancel",
                Style::default().fg(Color::DarkGray),
            )))
            .alignment(Alignment::Center),
            splits[1],
        );
    });
}

/// A centered popup `Rect` sized as a percentage of the terminal,
/// clamped to `content_rows` + border/hint overhead.
fn centered_box(area: Rect, pct: u16, content_rows: u16) -> Rect {
    let w = (((area.width as u32 * pct as u32) / 100) as u16)
        .max(20)
        .min(area.width);
    let h = (content_rows + 2).min(area.height).max(3);
    let x = area.width.saturating_sub(w) / 2;
    let y = area.height.saturating_sub(h) / 2;
    Rect::new(x, y, w, h)
}

/// Detect AI coding agents on `$PATH` with a live TUI progress screen,
/// returning the names of those found. Runs *before* the agent picker so
/// the picker's options are exactly what detection discovered. Blocking
/// (uses `block_in_place`); falls back to a silent `which` scan if the
/// terminal can't be taken over.
pub fn detect_agents_tui() -> Vec<String> {
    let agents: Vec<String> = crate::probe::AI_AGENT_NAMES
        .iter()
        .map(|s| s.to_string())
        .collect();
    if agents.is_empty() {
        return vec![];
    }
    tokio::task::block_in_place(|| run_detect(&agents))
}

/// Detection + selection in one step: show the progress screen, then
/// (if anything was found) the picker. Returns the chosen agent name,
/// or `None` if nothing was detected or the user cancelled.
pub fn detect_and_select_agent_tui() -> Option<String> {
    let detected = detect_agents_tui();
    // Even with nothing detected the picker still offers the custom
    // command row, so the player can type any binary on $PATH.
    let picked = pick_agent_tui(&detected);
    if picked.is_none() && detected.is_empty() {
        eprintln!("No AI coding agent found on $PATH.");
        eprintln!(
            "Install one of: {}",
            crate::probe::AI_AGENT_NAMES.join(", ")
        );
    }
    picked
}

const SPINNERS: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

fn run_detect(agents: &[String]) -> Vec<String> {
    use crossterm::execute;
    use ratatui::backend::CrosstermBackend;
    use std::io::stdout;

    // 0 pending · 1 scanning · 2 found · 3 missing
    let mut status: Vec<u8> = vec![0; agents.len()];
    let mut versions: Vec<Option<String>> = vec![None; agents.len()];
    let mut cursor = 0usize;
    let mut found: Vec<String> = Vec::new();

    if crossterm::terminal::enable_raw_mode().is_err() {
        return fallback_detect(agents);
    }
    let _ = execute!(
        stdout(),
        crossterm::terminal::EnterAlternateScreen,
        crossterm::event::EnableMouseCapture,
    );
    let backend = CrosstermBackend::new(stdout());
    let mut terminal = match ratatui::Terminal::new(backend) {
        Ok(t) => t,
        Err(_) => {
            cleanup_picker();
            return fallback_detect(agents);
        }
    };

    while cursor < agents.len() {
        status[cursor] = 1;
        draw_detect(&mut terminal, agents, &status, &versions);
        let name = agents[cursor].clone();
        if let Some(entry) = crate::probe::probe_tool(&name, true) {
            status[cursor] = 2;
            versions[cursor] = entry.version.clone();
            found.push(entry.name);
        } else {
            status[cursor] = 3;
        }
        cursor += 1;
        draw_detect(&mut terminal, agents, &status, &versions);
    }
    // Let the final ✓/✗ results register before the picker replaces them.
    std::thread::sleep(std::time::Duration::from_millis(400));

    cleanup_picker();
    found
}

/// `which`-only scan used when the TUI can't take over the terminal.
fn fallback_detect(agents: &[String]) -> Vec<String> {
    agents
        .iter()
        .filter(|name| which::which(name).is_ok())
        .cloned()
        .collect()
}

fn draw_detect(
    terminal: &mut ratatui::Terminal<ratatui::backend::CrosstermBackend<std::io::Stdout>>,
    agents: &[String],
    status: &[u8],
    versions: &[Option<String>],
) {
    let spinner = SPINNERS[(now_ms() / 80) as usize % SPINNERS.len()];
    let done = status.iter().all(|&s| s == 2 || s == 3);
    let _ = terminal.draw(|f| {
        let area = centered_box(f.area(), 50, agents.len() as u16 + 4);
        Clear.render(area, f.buffer_mut());
        let block = Block::default()
            .title(" Detecting AI coding agents ")
            .borders(Borders::ALL);
        let inner = block.inner(area);
        f.render_widget(block, area);

        let items: Vec<ListItem> = agents
            .iter()
            .enumerate()
            .map(|(i, name)| {
                let (sym, color, label) = match status[i] {
                    1 => (spinner, Color::Yellow, name.clone()),
                    2 => (
                        "✓",
                        Color::Green,
                        match &versions[i] {
                            Some(v) => format!("{name} {v}"),
                            None => name.clone(),
                        },
                    ),
                    3 => ("✗", Color::DarkGray, name.clone()),
                    _ => ("·", Color::DarkGray, name.clone()),
                };
                ListItem::new(Line::from(Span::styled(
                    format!("  {sym} {label}"),
                    Style::default().fg(color),
                )))
            })
            .collect();
        let list = List::new(items);
        let splits = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(0), Constraint::Length(1)])
            .split(inner);
        f.render_widget(list, splits[0]);
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(
                if done { "scan complete" } else { "scanning…" },
                Style::default().fg(Color::DarkGray),
            )))
            .alignment(Alignment::Center),
            splits[1],
        );
    });
}

fn now_ms() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_newline_in_path() {
        let result = validate_name("good\nbad");
        assert!(result.is_err());
    }

    #[test]
    fn rejects_null_in_path() {
        let result = validate_name("good\0bad");
        assert!(result.is_err());
    }

    #[test]
    fn rejects_carriage_return_in_path() {
        let result = validate_name("good\rbad");
        assert!(result.is_err());
    }

    #[test]
    fn select_agent_with_launch_override_succeeds_for_missing_binary() {
        let args = PickerArgs {
            launch_override: Some("definitely-not-a-real-binary-12345"),
            detected: &[],
        };
        let result = select_agent(args);
        assert!(result.is_err(), "missing binary must fail");
    }

    #[test]
    fn select_agent_with_no_launch_and_no_detected_errors() {
        let args = PickerArgs {
            launch_override: None,
            detected: &[],
        };
        let result = select_agent(args);
        assert!(result.is_err());
    }

    #[test]
    fn desktop_agents_are_classified_as_such() {
        for name in ["zed", "cursor", "antigravity"] {
            assert_eq!(agent_kind(name), AgentKind::Desktop, "{name}");
        }
    }

    #[test]
    fn cli_agents_of_desktop_products_stay_terminal() {
        // `cursor-agent` and `agy` belong to desktop products but are terminal
        // agents themselves — classifying them as desktop would rob the
        // participant of their agent pane.
        for name in [
            "cursor-agent",
            "cursor-cli",
            "agy",
            "claude",
            "opencode",
            "droid",
        ] {
            assert_eq!(agent_kind(name), AgentKind::Terminal, "{name}");
        }
    }

    #[test]
    fn agent_kind_accepts_a_path_not_just_a_bare_name() {
        // `--launch` may hand over a full path.
        assert_eq!(
            agent_kind("/Applications/Zed.app/bin/zed"),
            AgentKind::Desktop
        );
        assert_eq!(agent_kind("/usr/local/bin/opencode"), AgentKind::Terminal);
    }

    #[test]
    fn ai_agent_names_is_nonempty_and_unique() {
        assert!(!AI_AGENT_NAMES.is_empty());
        let mut sorted: Vec<&str> = AI_AGENT_NAMES.to_vec();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), AI_AGENT_NAMES.len(), "duplicates present");
    }

    #[test]
    fn revalidate_passes_for_existing_path() {
        // /bin/sh is on every unix system
        #[cfg(unix)]
        {
            let picked = PickedAgent {
                command: "/bin/sh".to_string(),
                argv: vec![],
                source: AgentSource::Other,
            };
            assert!(revalidate(&picked).is_ok());
        }
    }

    #[test]
    fn revalidate_fails_for_missing_path() {
        let picked = PickedAgent {
            command: "/no/such/path".to_string(),
            argv: vec![],
            source: AgentSource::Other,
        };
        assert!(revalidate(&picked).is_err());
    }
}
