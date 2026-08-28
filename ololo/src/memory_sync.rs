//! Publish memory source files (`AGENTS.md`, `README.md`) as they change.
//!
//! The server extracts session memory from these two files at the HEAD of the
//! player's pushed repo, but memory was only re-extracted after a
//! *task-completion* commit. A player who rewrote AGENTS.md mid-task kept
//! being probed with stale values until the next task ended.
//!
//! So: on every probe, check whether those files changed, and if they did,
//! commit just them, push, and tell the server. Everything here is
//! best-effort — a failure logs and gameplay continues.
//!
//! It runs as its own task rather than inline in the probe handler because
//! committing and pushing are blocking and would otherwise stall the UI on
//! every probe. Signals coalesce: a sync already in flight absorbs the ones
//! that arrive while it runs, so a fast probe cadence cannot queue up pushes.

use std::sync::{Arc, Mutex};

use arena_core::protocol::PlayerAgentClientFrame;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};

use crate::snapshot::SnapshotRepo;

/// Handle used by the probe loop to ask for a sync.
#[derive(Clone)]
pub struct MemorySyncHandle {
    tx: UnboundedSender<()>,
}

impl MemorySyncHandle {
    /// Ask for a check. Cheap and non-blocking; safe to call per probe.
    pub fn request(&self) {
        // A full channel or dead receiver just means no sync happens; the
        // next probe asks again.
        let _ = self.tx.send(());
    }
}

/// Spawn the sync task.
///
/// `frame_tx` carries the "I pushed" notification back to the websocket
/// writer; the sync task cannot send on the socket itself.
pub fn spawn(
    snapshot: Arc<Mutex<SnapshotRepo>>,
    frame_tx: UnboundedSender<PlayerAgentClientFrame>,
) -> (MemorySyncHandle, tokio::task::JoinHandle<()>) {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<()>();
    let handle = tokio::spawn(async move {
        while rx.recv().await.is_some() {
            drain(&mut rx);
            let snap = Arc::clone(&snapshot);
            // gix + `git push` are blocking; keep them off the runtime.
            let pushed = tokio::task::spawn_blocking(move || sync_once(&snap))
                .await
                .unwrap_or(false);
            if pushed
                && frame_tx
                    .send(PlayerAgentClientFrame::MemorySourcesPushed)
                    .is_err()
            {
                // Socket is gone; nothing left to notify.
                break;
            }
        }
    });
    (MemorySyncHandle { tx }, handle)
}

/// Collapse the signals that piled up while a sync was running — they all
/// mean the same thing, and one check sees the latest file contents anyway.
fn drain(rx: &mut UnboundedReceiver<()>) {
    while rx.try_recv().is_ok() {}
}

/// Commit + push the memory files. Returns true when something was pushed.
fn sync_once(snapshot: &Mutex<SnapshotRepo>) -> bool {
    let guard = match snapshot.lock() {
        Ok(g) => g,
        // Another thread panicked mid-commit; the repo may be mid-write, so
        // skip this round rather than risk compounding it.
        Err(e) => {
            tracing::warn!("memory sync: snapshot lock poisoned: {e}");
            return false;
        }
    };
    match guard.commit_memory_sources() {
        Ok(false) => false,
        Ok(true) => {
            if let Err(e) = guard.push_to_remote() {
                tracing::warn!("memory sync: push failed: {e}");
                // The commit is local-only for now; a later sync or the next
                // task commit pushes it. Do not claim it reached the server.
                return false;
            }
            tracing::debug!("memory sync: pushed updated memory sources");
            true
        }
        Err(e) => {
            tracing::warn!("memory sync: commit failed: {e}");
            false
        }
    }
}
