//! The background pusher: one task that pushes `main` whenever asked,
//! never blocking the caller, with backoff on failure and recovery on a
//! rejected non-fast-forward push.
//!
//! Requests coalesce — a push of `main` carries every commit made so far,
//! so the signals that pile up while one push runs collapse into the next.
//! Callers that need to know when their commit has landed (the memory
//! sync and the completion flag tell the server "I pushed") wait on the
//! outcome of the push that follows their request.

use std::sync::{Arc, Mutex};

use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use tokio::sync::oneshot;

use super::{PushOutcome, SnapshotRepo};

/// Delays between retries of a failed push, seconds.
const BACKOFF_SECS: [u64; 5] = [2, 4, 8, 16, 30];

struct Request {
    reply: Option<oneshot::Sender<PushOutcome>>,
}

#[derive(Clone)]
pub struct PusherHandle {
    tx: UnboundedSender<Request>,
}

impl PusherHandle {
    /// Ask for a push. Cheap and non-blocking; safe to call per commit.
    pub fn request(&self) {
        let _ = self.tx.send(Request { reply: None });
    }

    /// Ask for a push and wait for the outcome of the push that follows.
    pub async fn push_and_wait(&self) -> PushOutcome {
        let (tx, rx) = oneshot::channel();
        if self.tx.send(Request { reply: Some(tx) }).is_err() {
            return PushOutcome::Failed {
                head: None,
                error: "pusher is gone".into(),
            };
        }
        rx.await.unwrap_or(PushOutcome::Failed {
            head: None,
            error: "pusher dropped the request".into(),
        })
    }
}

/// Spawn the pusher for `snapshot`. The handle is also attached to the
/// repo, so `SnapshotRepo::request_push` goes through it.
pub fn spawn(snapshot: Arc<Mutex<SnapshotRepo>>) -> (PusherHandle, tokio::task::JoinHandle<()>) {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Request>();
    let handle = PusherHandle { tx };
    if let Ok(guard) = snapshot.lock() {
        guard.attach_pusher(handle.clone());
    }
    let task = tokio::spawn(async move {
        while let Some(first) = rx.recv().await {
            let mut waiters: Vec<oneshot::Sender<PushOutcome>> = first.reply.into_iter().collect();
            drain(&mut rx, &mut waiters);
            let outcome = push_with_retry(&snapshot, &mut rx, &mut waiters).await;
            for reply in waiters {
                let _ = reply.send(outcome.clone());
            }
        }
    });
    (handle, task)
}

/// Collapse the requests that piled up, keeping their reply channels.
fn drain(rx: &mut UnboundedReceiver<Request>, waiters: &mut Vec<oneshot::Sender<PushOutcome>>) {
    while let Ok(req) = rx.try_recv() {
        waiters.extend(req.reply);
    }
}

/// Push until it lands, the retries run out, or the remote is off. A
/// rejected push triggers one recovery (resync to the server's line, the
/// working tree re-committed on top) followed by a fresh push. Requests
/// arriving during a backoff wait cut it short — they mean new commits.
async fn push_with_retry(
    snapshot: &Arc<Mutex<SnapshotRepo>>,
    rx: &mut UnboundedReceiver<Request>,
    waiters: &mut Vec<oneshot::Sender<PushOutcome>>,
) -> PushOutcome {
    let mut attempt = 0usize;
    let mut recovered = false;
    loop {
        let snap = Arc::clone(snapshot);
        let outcome = tokio::task::spawn_blocking(move || {
            let target = snap.lock().ok().and_then(|g| g.push_target());
            let Some(target) = target else {
                return PushOutcome::Disabled;
            };
            // The push itself runs without the repo lock: git reads the ref
            // once, and a commit made meanwhile rides the next push.
            let outcome = target.push();
            if let Ok(g) = snap.lock() {
                g.record_push_outcome(&outcome);
            }
            outcome
        })
        .await
        .unwrap_or(PushOutcome::Failed {
            head: None,
            error: "push task panicked".into(),
        });

        match &outcome {
            PushOutcome::Pushed { .. } | PushOutcome::Disabled => return outcome,
            PushOutcome::Rejected { .. } => {
                if recovered {
                    tracing::warn!("snapshot push rejected again after a resync; giving up");
                    return outcome;
                }
                recovered = true;
                let snap = Arc::clone(snapshot);
                let resync = tokio::task::spawn_blocking(move || {
                    snap.lock()
                        .map_err(|e| format!("snapshot lock poisoned: {e}"))
                        .and_then(|g| g.recover_from_remote().map_err(|e| e.to_string()))
                })
                .await
                .unwrap_or_else(|e| Err(format!("resync task panicked: {e}")));
                if let Err(e) = resync {
                    tracing::warn!("snapshot resync after a rejected push failed: {e}");
                    return outcome;
                }
                // Push the resynced line right away.
                continue;
            }
            PushOutcome::Failed { .. } | PushOutcome::Timeout { .. } => {
                if attempt >= BACKOFF_SECS.len() {
                    tracing::warn!(
                        "snapshot push still failing after {attempt} retries; waiting for the next commit"
                    );
                    return outcome;
                }
                let delay = std::time::Duration::from_secs(BACKOFF_SECS[attempt]);
                attempt += 1;
                tokio::select! {
                    _ = tokio::time::sleep(delay) => {}
                    req = rx.recv() => match req {
                        Some(req) => {
                            waiters.extend(req.reply);
                            drain(rx, waiters);
                        }
                        // Every handle dropped: finish this attempt's work
                        // and let the loop end.
                        None => return outcome,
                    }
                }
            }
        }
    }
}
