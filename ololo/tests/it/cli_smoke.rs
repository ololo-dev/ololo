//! Smoke tests for the ololo binary CLI surface.

use std::process::Command;
use std::time::Duration;

fn ololo_bin() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_ololo"))
}

#[test]
fn ololo_help_prints_usage() {
    let out = Command::new(ololo_bin())
        .arg("--help")
        .output()
        .expect("spawn ololo --help");
    assert!(out.status.success(), "ololo --help failed: {out:?}");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("login"),
        "expected 'login' in help: {stdout}"
    );
    assert!(
        stdout.contains("start"),
        "expected 'start' in help: {stdout}"
    );
    assert!(stdout.contains("join"), "expected 'join' in help: {stdout}");
}

#[test]
fn ololo_no_subcommand_errors() {
    let out = Command::new(ololo_bin())
        .output()
        .expect("spawn ololo (no args)");
    assert!(!out.status.success(), "expected nonzero exit; got {out:?}");
}

#[test]
fn ololo_start_help_shows_slug() {
    let out = Command::new(ololo_bin())
        .args(["start", "--help"])
        .output()
        .expect("spawn ololo start --help");
    assert!(out.status.success(), "ololo start --help failed: {out:?}");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("slug"),
        "expected 'slug' in start help: {stdout}"
    );
}

#[test]
fn ololo_start_missing_slug_errors() {
    let out = Command::new(ololo_bin())
        .arg("start")
        .output()
        .expect("spawn ololo start (no slug)");
    assert!(
        !out.status.success(),
        "expected nonzero exit when slug missing; got {out:?}"
    );
}

#[test]
fn ololo_join_help_shows_code() {
    let out = Command::new(ololo_bin())
        .args(["join", "--help"])
        .output()
        .expect("spawn ololo join --help");
    assert!(out.status.success(), "ololo join --help failed: {out:?}");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("code"),
        "expected 'code' in join help: {stdout}"
    );
}

#[test]
fn ololo_start_tui_flag_appears_in_help() {
    let out = Command::new(ololo_bin())
        .args(["start", "--help"])
        .output()
        .expect("spawn ololo start --help");
    assert!(out.status.success(), "ololo start --help failed: {out:?}");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("--tui"),
        "expected '--tui' in start help: {stdout}"
    );
}

#[test]
fn ololo_join_tui_flag_appears_in_help() {
    let out = Command::new(ololo_bin())
        .args(["join", "--help"])
        .output()
        .expect("spawn ololo join --help");
    assert!(out.status.success(), "ololo join --help failed: {out:?}");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("--tui"),
        "expected '--tui' in join help: {stdout}"
    );
}

#[test]
fn ololo_start_tui_no_tty_errors_cleanly() {
    use std::process::Stdio;
    use std::time::{Duration, Instant};
    let mut child = Command::new(ololo_bin())
        .args(["start", "fake-slug", "--tui"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn ololo start --tui");
    let start = Instant::now();
    // Generous budget: the no-TTY guard exits in ~35ms, but the first exec
    // of a freshly linked debug binary can stall for seconds on macOS
    // (code-signature validation). The assertion is "exits, doesn't hang".
    let result = child.wait_timeout(Duration::from_secs(10));
    let _ = start;
    match result {
        Some(status) => {
            assert!(
                !status.success(),
                "expected nonzero exit without TTY; got {status:?}"
            );
        }
        None => {
            // Process is still running; kill it and report failure.
            let _ = child.kill();
            let _ = child.wait();
            panic!("ololo start --tui did not exit within 2s without a TTY");
        }
    }
}

trait CommandExt {
    fn wait_timeout(&mut self, timeout: Duration) -> Option<std::process::ExitStatus>;
}

impl CommandExt for std::process::Child {
    fn wait_timeout(&mut self, timeout: Duration) -> Option<std::process::ExitStatus> {
        let start = std::time::Instant::now();
        loop {
            match self.try_wait() {
                Ok(Some(status)) => return Some(status),
                Ok(None) => {
                    if start.elapsed() > timeout {
                        return None;
                    }
                    std::thread::sleep(Duration::from_millis(50));
                }
                Err(_) => return None,
            }
        }
    }
}
