//! Parent-agent control channel for autonomous play.
//!
//! An AI agent (e.g. Claude Code) can drive an ololo session that itself
//! hosts an inner agent in a PTY: the parent drops message files into an
//! inbox directory and reads back a plain-text dump of the inner agent's
//! screen. Built for debugging sessions end-to-end without a human at the
//! keyboard.
//!
//! Layout, under `{config_dir}/ololo/control/<join_code>/` (override the
//! base with `OLOLO_CONTROL_DIR`):
//!
//! - `inbox/*.md|*.txt` — the parent writes one message per file; ololo
//!   picks files up in name order (zero-pad to sequence: `001-….md`),
//!   pastes the content into the inner agent's PTY as one bracketed paste
//!   followed by Enter, and moves the file to `sent/`.
//! - `sent/` — processed messages, kept for audit.
//! - `screen.txt` — latest plain-text render of the inner agent's screen,
//!   refreshed about once a second while it changes.
//!
//! The control dir deliberately lives OUTSIDE the session worktree:
//! snapshots commit the worktree (including `.ololo/`), and control
//! traffic must never leak into judged snapshots or golf size counts.
//!
//! Same settle discipline as the done-flag watcher: a file is read only
//! once its size is unchanged across two polls, so half-written messages
//! are never sent.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use tokio::sync::mpsc;

/// Poll cadence for the inbox. One tick of latency buys settle detection.
const POLL_INTERVAL: std::time::Duration = std::time::Duration::from_secs(1);

/// Resolve the control directory for a session and create its skeleton.
pub fn control_dir(join_code: &str) -> Option<PathBuf> {
    let base = match std::env::var_os("OLOLO_CONTROL_DIR") {
        Some(d) => PathBuf::from(d),
        None => dirs::config_dir()?.join("ololo").join("control"),
    };
    let dir = base.join(join_code);
    for sub in ["inbox", "sent"] {
        if let Err(e) = std::fs::create_dir_all(dir.join(sub)) {
            tracing::warn!("control dir setup failed at {}: {e}", dir.display());
            return None;
        }
    }
    Some(dir)
}

/// Everything the run loops need to serve the channel: queued parent
/// messages and the screen-dump sink.
pub struct ControlChannel {
    pub rx: mpsc::UnboundedReceiver<String>,
    pub screen: ScreenSink,
    _watcher: tokio::task::JoinHandle<()>,
}

impl ControlChannel {
    /// Spawn the inbox watcher for `dir` and build the channel around it.
    pub fn spawn(dir: PathBuf) -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        let screen = ScreenSink::new(dir.join("screen.txt"));
        let watcher = tokio::spawn(watch_inbox(dir, tx));
        Self {
            rx,
            screen,
            _watcher: watcher,
        }
    }
}

impl Drop for ControlChannel {
    fn drop(&mut self) {
        self._watcher.abort();
    }
}

/// Per-file memory across polls: size at last sight.
#[derive(Default)]
pub struct InboxState {
    seen: HashMap<PathBuf, u64>,
}

/// One poll over `inbox/`: returns files whose size settled since the last
/// poll, in name order. Settled files are forgotten from `state` — the
/// caller moves them out of the inbox, so they will not reappear.
pub fn settled_messages(state: &mut InboxState, inbox: &Path) -> Vec<PathBuf> {
    let entries = match std::fs::read_dir(inbox) {
        Ok(e) => e,
        Err(_) => return Vec::new(),
    };
    let mut ready = Vec::new();
    let mut present = HashMap::new();
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        if !(name.ends_with(".md") || name.ends_with(".txt")) {
            continue;
        }
        let Ok(meta) = entry.metadata() else { continue };
        if !meta.is_file() {
            continue;
        }
        let path = entry.path();
        let size = meta.len();
        if state.seen.get(&path) == Some(&size) {
            ready.push(path.clone());
        }
        present.insert(path, size);
    }
    for path in &ready {
        present.remove(path);
    }
    state.seen = present;
    ready.sort();
    ready
}

/// Read a settled message and archive it to `sent/`. Empty (whitespace-only)
/// files are archived but produce no message.
fn consume(path: &Path, sent_dir: &Path) -> Option<String> {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) => {
            tracing::warn!(
                "control message unreadable, skipping {}: {e}",
                path.display()
            );
            return None;
        }
    };
    let archived = sent_dir.join(path.file_name().unwrap_or_default());
    if std::fs::rename(path, &archived).is_err() {
        // Cross-device or permission trouble — at least stop re-sending it.
        let _ = std::fs::remove_file(path);
    }
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(trimmed.to_string())
}

async fn watch_inbox(dir: PathBuf, tx: mpsc::UnboundedSender<String>) {
    let inbox = dir.join("inbox");
    let sent = dir.join("sent");
    let mut state = InboxState::default();
    let mut tick = tokio::time::interval(POLL_INTERVAL);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tick.tick().await;
        for path in settled_messages(&mut state, &inbox) {
            if let Some(text) = consume(&path, &sent)
                && tx.send(text).is_err()
            {
                return; // run loop gone — session over
            }
        }
    }
}

/// Writes the inner agent's screen as plain text, only when it changed.
pub struct ScreenSink {
    path: PathBuf,
    last_hash: u64,
}

impl ScreenSink {
    pub fn new(path: PathBuf) -> Self {
        Self { path, last_hash: 0 }
    }

    /// Dump `screen` (vt100 contents) if it differs from the last dump.
    pub fn maybe_write(&mut self, screen: &vt100::Screen) {
        use std::hash::{Hash, Hasher};
        let text = screen.contents();
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        text.hash(&mut hasher);
        let hash = hasher.finish();
        if hash == self.last_hash {
            return;
        }
        self.last_hash = hash;
        if let Err(e) = std::fs::write(&self.path, &text) {
            tracing::warn!("screen dump failed at {}: {e}", self.path.display());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(dir: &Path, name: &str, content: &str) -> PathBuf {
        let p = dir.join(name);
        std::fs::write(&p, content).unwrap();
        p
    }

    #[test]
    fn message_settles_after_two_polls_and_fires_once() {
        let tmp = tempfile::tempdir().unwrap();
        let inbox = tmp.path().join("inbox");
        std::fs::create_dir_all(&inbox).unwrap();
        let mut state = InboxState::default();

        write(&inbox, "001-hello.md", "run the tests");
        // First sight: recorded, not ready.
        assert!(settled_messages(&mut state, &inbox).is_empty());
        // Second sight, same size: ready.
        let ready = settled_messages(&mut state, &inbox);
        assert_eq!(ready.len(), 1);
        // Settled files are forgotten; if the caller leaves the file in
        // place it re-enters the settle cycle rather than double-firing.
        assert!(settled_messages(&mut state, &inbox).is_empty());
    }

    #[test]
    fn growing_file_is_not_ready_and_ordering_is_by_name() {
        let tmp = tempfile::tempdir().unwrap();
        let inbox = tmp.path().join("inbox");
        std::fs::create_dir_all(&inbox).unwrap();
        let mut state = InboxState::default();

        write(&inbox, "002-b.md", "second");
        write(&inbox, "001-a.md", "first");
        assert!(settled_messages(&mut state, &inbox).is_empty());
        // 002 grows between polls — only after it settles again is it sent.
        write(&inbox, "002-b.md", "second, but longer now");
        let ready = settled_messages(&mut state, &inbox);
        assert_eq!(ready.len(), 1);
        assert!(ready[0].ends_with("001-a.md"));
        let ready = settled_messages(&mut state, &inbox);
        assert_eq!(ready.len(), 1);
        assert!(ready[0].ends_with("002-b.md"));
    }

    #[test]
    fn non_message_files_are_ignored() {
        let tmp = tempfile::tempdir().unwrap();
        let inbox = tmp.path().join("inbox");
        std::fs::create_dir_all(&inbox).unwrap();
        let mut state = InboxState::default();
        write(&inbox, ".DS_Store", "junk");
        write(&inbox, "notes.json", "{}");
        assert!(settled_messages(&mut state, &inbox).is_empty());
        assert!(settled_messages(&mut state, &inbox).is_empty());
    }

    #[test]
    fn consume_archives_and_trims() {
        let tmp = tempfile::tempdir().unwrap();
        let inbox = tmp.path().join("inbox");
        let sent = tmp.path().join("sent");
        std::fs::create_dir_all(&inbox).unwrap();
        std::fs::create_dir_all(&sent).unwrap();
        let p = write(&inbox, "001.md", "  do the thing\n");
        assert_eq!(consume(&p, &sent), Some("do the thing".to_string()));
        assert!(!p.exists());
        assert!(sent.join("001.md").exists());
        // Whitespace-only → archived, no message.
        let p = write(&inbox, "002.md", "\n  \n");
        assert_eq!(consume(&p, &sent), None);
        assert!(sent.join("002.md").exists());
    }

    #[test]
    fn screen_sink_writes_only_on_change() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("screen.txt");
        let mut parser = vt100::Parser::new(5, 20, 0);
        parser.process(b"hello");
        let mut sink = ScreenSink::new(path.clone());
        sink.maybe_write(parser.screen());
        let first = std::fs::metadata(&path).unwrap().modified().unwrap();
        assert!(std::fs::read_to_string(&path).unwrap().contains("hello"));
        // Unchanged screen → no rewrite.
        std::fs::write(&path, "tampered").unwrap();
        sink.maybe_write(parser.screen());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "tampered");
        let _ = first;
        // Changed screen → rewritten.
        parser.process(b" world");
        sink.maybe_write(parser.screen());
        assert!(
            std::fs::read_to_string(&path)
                .unwrap()
                .contains("hello world")
        );
    }
}
