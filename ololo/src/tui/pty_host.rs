#![allow(dead_code)]

//! PTY hosting for the launched AI agent.
//!
//! WP-006: real `portable-pty` wiring + SIGHUP/SIGKILL shutdown + the
//! `grid_to_ratatui` adapter. The render layer's
//! `pty_host::grid_to_ratatui` is the bridge between the agent's
//! `vt100::Parser` and ratatui's `Buffer`.

use anyhow::Result;
use portable_pty::{CommandBuilder, MasterPty, PtySize, native_pty_system};
use ratatui::layout::Rect;
use std::io::Read;
use std::io::Write;
use std::time::Duration;
use tokio::sync::mpsc;
use vt100::{Cell as VtCell, Color as VtColor, Parser, Screen};

pub const SHUTDOWN_GRACE_MS: u64 = 500;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Color {
    Default,
    Rgb(u8, u8, u8),
    Indexed(u8),
}

impl Color {
    pub fn from_vt(c: VtColor) -> Self {
        match c {
            VtColor::Default => Color::Default,
            VtColor::Idx(i) => Color::Indexed(i),
            VtColor::Rgb(r, g, b) => Color::Rgb(r, g, b),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Cell {
    pub ch: char,
    pub fg: Color,
    pub bg: Color,
    pub bold: bool,
    pub underline: bool,
    pub reverse: bool,
}

impl Default for Cell {
    fn default() -> Self {
        Self {
            ch: ' ',
            fg: Color::Default,
            bg: Color::Default,
            bold: false,
            underline: false,
            reverse: false,
        }
    }
}

fn from_vt_cell(c: &VtCell) -> Cell {
    let ch = c.contents().chars().next().unwrap_or(' ');
    Cell {
        ch,
        fg: Color::from_vt(c.fgcolor()),
        bg: Color::from_vt(c.bgcolor()),
        bold: c.bold(),
        underline: c.underline(),
        reverse: c.inverse(),
    }
}

/// vt100 is 0-indexed; ratatui is 0-indexed — explicit adapter with
/// no hidden offset. Pads/truncates to `area` exactly.
///
/// If the PTY cursor is visible (`screen.hide_cursor()` is false), the
/// cell at the cursor position is flipped to reverse video so the cursor
/// shows up as a solid block in the ratatui render — the agent's own
/// cursor escape sequences are consumed by the vt100 parser and never
/// reach the real terminal, so without this the cursor is invisible.
pub fn grid_to_ratatui(parser: &Parser, area: Rect) -> Vec<Vec<Cell>> {
    let rows = area.height as usize;
    let cols = area.width as usize;
    let screen: &Screen = parser.screen();
    let (cur_row, cur_col) = screen.cursor_position();
    let cursor_visible = !screen.hide_cursor();
    let mut out = Vec::with_capacity(rows);
    for row in 0..rows {
        let mut line = Vec::with_capacity(cols);
        for col in 0..cols {
            let mut cell = screen
                .cell(row as u16, col as u16)
                .map(from_vt_cell)
                .unwrap_or_default();
            if cursor_visible && row as u16 == cur_row && col as u16 == cur_col {
                cell.reverse = !cell.reverse;
            }
            line.push(cell);
        }
        out.push(line);
    }
    out
}

pub struct PtyHost {
    _master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    reader: Option<Box<dyn Read + Send>>,
    pub child: Box<dyn portable_pty::Child + Send + Sync>,
}

#[derive(Debug, thiserror::Error)]
pub enum PtyError {
    #[error("portable-pty: {0}")]
    Pty(String),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

pub struct SpawnArgs<'a> {
    pub program: &'a str,
    pub args: &'a [String],
    pub rows: u16,
    pub cols: u16,
    pub env: &'a [(String, String)],
    pub cwd: Option<&'a std::path::Path>,
}

impl PtyHost {
    pub fn spawn(args: SpawnArgs<'_>) -> Result<Self> {
        let pty_system = native_pty_system();
        let pair = pty_system
            .openpty(PtySize {
                rows: args.rows,
                cols: args.cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| PtyError::Pty(e.to_string()))?;
        let mut cmd = CommandBuilder::new(args.program);
        for a in args.args {
            cmd.arg(a);
        }
        for (k, v) in args.env {
            cmd.env(k, v);
        }
        if let Some(cwd) = args.cwd {
            cmd.cwd(cwd);
        }
        let child = pair
            .slave
            .spawn_command(cmd)
            .map_err(|e| PtyError::Pty(e.to_string()))?;
        let writer = pair
            .master
            .take_writer()
            .map_err(|e| PtyError::Pty(e.to_string()))?;
        let reader = pair
            .master
            .try_clone_reader()
            .map_err(|e| PtyError::Pty(e.to_string()))?;
        Ok(Self {
            _master: pair.master,
            writer,
            reader: Some(reader),
            child,
        })
    }

    pub fn process_id(&self) -> Option<u32> {
        self.child.process_id()
    }

    pub fn resize(&mut self, rows: u16, cols: u16) -> Result<(), PtyError> {
        self._master
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| PtyError::Pty(e.to_string()))
    }

    pub fn write_input(&mut self, data: &[u8]) -> Result<()> {
        self.writer.write_all(data)?;
        self.writer.flush()?;
        Ok(())
    }

    pub fn take_reader(&mut self) -> Option<Box<dyn Read + Send>> {
        self.reader.take()
    }

    pub fn spawn_reader(&mut self) -> Option<mpsc::Receiver<Vec<u8>>> {
        let reader = self.take_reader()?;
        let (tx, rx) = mpsc::channel::<Vec<u8>>(64);
        std::thread::spawn(move || {
            let mut reader = reader;
            let mut buf = [0u8; 4096];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        if tx.blocking_send(buf[..n].to_vec()).is_err() {
                            break;
                        }
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(5));
                        continue;
                    }
                    Err(_) => break,
                }
            }
        });
        Some(rx)
    }

    /// SIGHUP → 500ms → SIGKILL. Unix uses nix; Windows uses Child::kill.
    pub async fn shutdown(&mut self) -> Result<()> {
        #[cfg(unix)]
        {
            use nix::sys::signal::{Signal, kill};
            use nix::unistd::Pid;
            if let Some(pid) = self.child.process_id() {
                let pid = Pid::from_raw(pid as i32);
                let _ = kill(pid, Signal::SIGHUP);
                tokio::time::sleep(Duration::from_millis(SHUTDOWN_GRACE_MS)).await;
                if self.child.process_id().is_some() {
                    let _ = kill(pid, Signal::SIGKILL);
                }
            }
        }
        #[cfg(windows)]
        {
            let _ = self.child.kill();
        }
        let _ = self.child.wait();
        Ok(())
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grid_to_ratatui_returns_correct_shape() {
        let parser = Parser::new(24, 80, 0);
        let cells = grid_to_ratatui(&parser, Rect::new(0, 0, 80, 24));
        assert_eq!(cells.len(), 24);
        assert_eq!(cells[0].len(), 80);
    }

    #[test]
    fn grid_to_ratatui_pads_when_area_exceeds_parser() {
        // Parser is 10x10; area is 5x5 — should still return 5x5.
        let parser = Parser::new(10, 10, 0);
        let cells = grid_to_ratatui(&parser, Rect::new(0, 0, 5, 5));
        assert_eq!(cells.len(), 5);
        assert_eq!(cells[0].len(), 5);
    }

    #[test]
    fn grid_to_ratatui_shows_cursor_as_reverse_video_when_visible() {
        // Write "hi" then move cursor to row 0 col 1; default cursor is visible.
        let mut parser = Parser::new(24, 80, 0);
        parser.process(b"hi");
        let cells = grid_to_ratatui(&parser, Rect::new(0, 0, 80, 24));
        // cursor_position after "hi" is (0, 2) — the cell after the text.
        let (cur_row, cur_col) = parser.screen().cursor_position();
        assert_eq!((cur_row, cur_col), (0, 2));
        assert!(
            cells[cur_row as usize][cur_col as usize].reverse,
            "cell at cursor position must be reverse-video when cursor is visible"
        );
    }

    #[test]
    fn grid_to_ratatui_hides_cursor_when_agent_hides_it() {
        let mut parser = Parser::new(24, 80, 0);
        parser.process(b"hi");
        parser.process(b"\x1b[?25l"); // DECTCEM off — hide cursor
        let cells = grid_to_ratatui(&parser, Rect::new(0, 0, 80, 24));
        let (cur_row, cur_col) = parser.screen().cursor_position();
        assert!(
            !cells[cur_row as usize][cur_col as usize].reverse,
            "cell at cursor position must NOT be reverse-video when cursor is hidden"
        );
    }

    #[test]
    fn cell_default_is_space() {
        let c = Cell::default();
        assert_eq!(c.ch, ' ');
        assert_eq!(c.fg, Color::Default);
    }

    #[test]
    fn color_from_vt_maps_bright_to_indexed() {
        assert_eq!(Color::from_vt(VtColor::Idx(9)), Color::Indexed(9));
        assert_eq!(Color::from_vt(VtColor::Default), Color::Default);
        assert_eq!(Color::from_vt(VtColor::Idx(42)), Color::Indexed(42));
        assert_eq!(Color::from_vt(VtColor::Rgb(1, 2, 3)), Color::Rgb(1, 2, 3));
    }

    #[cfg(unix)]
    #[test]
    fn shutdown_kills_long_running_child() {
        let args = SpawnArgs {
            program: "sh",
            args: &["-c".to_string(), "trap '' SIGHUP; sleep 60".to_string()],
            rows: 24,
            cols: 80,
            env: &[],
            cwd: None,
        };
        let mut host = PtyHost::spawn(args).expect("spawn");
        assert!(host.process_id().is_some());
        let pid = host.process_id().unwrap();
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("rt");
        let start = std::time::Instant::now();
        let result = rt.block_on(async {
            tokio::time::timeout(Duration::from_millis(1500), host.shutdown()).await
        });
        let elapsed = start.elapsed();
        assert!(result.is_ok(), "shutdown did not complete in 1.5s");
        assert!(
            elapsed < Duration::from_millis(1500),
            "shutdown took too long: {elapsed:?}"
        );
        let _ = pid;
    }
}
