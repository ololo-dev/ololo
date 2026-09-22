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
            // gix is blocking; keep it off the runtime.
            let committed = tokio::task::spawn_blocking(move || commit_once(&snap))
                .await
                .unwrap_or(false);
            let pushed = committed && push(&snapshot).await;
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

/// Commit the memory files. Returns true when something was committed.
fn commit_once(snapshot: &Mutex<SnapshotRepo>) -> bool {
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
        Ok(committed) => committed,
        Err(e) => {
            tracing::warn!("memory sync: commit failed: {e}");
            false
        }
    }
}

/// Push through the background pusher (inline when none is attached) and
/// say whether the commit reached the server. A push that did not land
/// leaves the commit local-only for now; a later push catches up, and the
/// server is not told anything it cannot yet read.
async fn push(snapshot: &Arc<Mutex<SnapshotRepo>>) -> bool {
    let pusher = snapshot.lock().ok().and_then(|g| g.pusher());
    let outcome = match pusher {
        Some(pusher) => pusher.push_and_wait().await,
        None => {
            let snap = Arc::clone(snapshot);
            tokio::task::spawn_blocking(move || {
                snap.lock()
                    .ok()
                    .and_then(|g| g.push_to_remote().ok())
                    .unwrap_or(crate::snapshot::PushOutcome::Failed {
                        head: None,
                        error: "snapshot unavailable".into(),
                    })
            })
            .await
            .unwrap_or(crate::snapshot::PushOutcome::Failed {
                head: None,
                error: "push task panicked".into(),
            })
        }
    };
    if outcome.is_pushed() {
        tracing::debug!("memory sync: pushed updated memory sources");
        true
    } else {
        tracing::warn!("memory sync: push did not land: {outcome:?}");
        false
    }
}
