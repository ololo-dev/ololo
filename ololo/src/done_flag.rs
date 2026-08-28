//! Watch for completion flag files and publish them the moment they land.
//!
//! Open-ended tasks complete via a flag file (`.ololo/<name>-done.md`) that
//! the player's agent writes when the build is ready. Left alone, the server
//! only notices at the completion probe's next interval tick — up to a
//! couple of minutes of dead air between "done" and the judges. So: poll the
//! flag directory, and when a new flag file appears and its size settles
//! across two polls, commit the whole working tree, push it, and tell the
//! server so it can dispatch the completion probe right away.
//!
//! Everything is best-effort — a failure logs and the interval machinery
//! still completes the task on its own cadence.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use arena_core::protocol::PlayerAgentClientFrame;
use tokio::sync::mpsc::UnboundedSender;

use crate::snapshot::SnapshotRepo;

/// Directory (relative to the worktree) where flag files live.
const FLAG_DIR: &str = ".ololo";
/// Flag files are named `<name>-done.md`.
const FLAG_SUFFIX: &str = "-done.md";
/// Poll cadence. One tick of extra latency buys write-settling detection.
const POLL_INTERVAL: std::time::Duration = std::time::Duration::from_secs(2);

/// Per-file memory across polls: the size/mtime at last sight and whether
/// the file was already published.
#[derive(Default)]
pub struct FlagWatchState {
    seen: HashMap<PathBuf, SeenFlag>,
}

struct SeenFlag {
    size: u64,
    mtime: Option<std::time::SystemTime>,
    published: bool,
    /// The file already existed when the watcher started — a leftover from
    /// an earlier session, not this session's completion. It stays quiet
    /// until the agent rewrites it, which re-arms it as a fresh flag.
    baseline: bool,
}

impl FlagWatchState {
    /// Snapshot the flags already on disk. A session started in a repo
    /// that played before finds old `*-done.md` files — they are history,
    /// not something the server asked for, and must not publish (or show
    /// in the chat) at boot.
    pub fn baseline(worktree: &Path) -> Self {
        let mut state = Self::default();
        let flag_dir = worktree.join(FLAG_DIR);
        let Ok(entries) = std::fs::read_dir(&flag_dir) else {
            return state;
        };
        for entry in entries.flatten() {
            let name = entry.file_name();
            let Some(name) = name.to_str() else { continue };
            if !name.ends_with(FLAG_SUFFIX) {
                continue;
            }
            let Ok(meta) = entry.metadata() else { continue };
            if !meta.is_file() {
                continue;
            }
            state.seen.insert(
                entry.path(),
                SeenFlag {
                    size: meta.len(),
                    mtime: meta.modified().ok(),
                    published: true,
                    baseline: true,
                },
            );
        }
        state
    }
}

/// One poll over `{worktree}/.ololo`: flag files not yet published whose
/// size is unchanged since the previous poll (i.e. the write settled).
/// Returned files are marked published — each fires exactly once.
/// Baseline files (present before the watcher started) fire only after
/// the agent rewrites them this session.
pub fn settled_new_flags(state: &mut FlagWatchState, worktree: &Path) -> Vec<PathBuf> {
    let flag_dir = worktree.join(FLAG_DIR);
    let entries = match std::fs::read_dir(&flag_dir) {
        Ok(e) => e,
        // No `.ololo` yet — nothing declared done.
        Err(_) => return Vec::new(),
    };
    let mut ready = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        if !name.ends_with(FLAG_SUFFIX) {
            continue;
        }
        let Ok(meta) = entry.metadata() else { continue };
        if !meta.is_file() {
            continue;
        }
        let mtime = meta.modified().ok();
        let path = entry.path();
        match state.seen.get_mut(&path) {
            None => {
                state.seen.insert(
                    path,
                    SeenFlag {
                        size: meta.len(),
                        mtime,
                        published: false,
                        baseline: false,
                    },
                );
            }
            // A leftover flag rewritten this session is a fresh completion:
            // re-arm it and let the settle logic below fire it.
            Some(flag) if flag.baseline && flag.published => {
                if flag.size != meta.len() || flag.mtime != mtime {
                    flag.size = meta.len();
                    flag.mtime = mtime;
                    flag.published = false;
                }
            }
            Some(flag) if flag.published => {}
            Some(flag) if flag.size == meta.len() && flag.mtime == mtime => {
                flag.published = true;
                ready.push(path);
            }
            Some(flag) => {
                flag.size = meta.len();
                flag.mtime = mtime;
            }
        }
    }
    ready
}

/// Spawn the watcher task. `frame_tx` carries the "flag pushed" notification
/// to the websocket writer; the watcher cannot send on the socket itself.
/// `sink` (when present) receives the flag's contents so the chat view can
/// show the player's done-note the way the web chat does.
pub fn spawn(
    snapshot: Arc<Mutex<SnapshotRepo>>,
    frame_tx: UnboundedSender<PlayerAgentClientFrame>,
    sink: Option<Arc<dyn crate::tui::EventSink>>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let worktree = match snapshot.lock() {
            Ok(g) => g.worktree().to_path_buf(),
            Err(e) => {
                tracing::warn!("flag watch: snapshot lock poisoned at startup: {e}");
                return;
            }
        };
        // Flags already on disk are a previous session's history — quiet
        // until the agent rewrites them.
        let mut state = FlagWatchState::baseline(&worktree);
        let mut tick = tokio::time::interval(POLL_INTERVAL);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tick.tick().await;
            if frame_tx.is_closed() {
                return; // socket writer gone; the session is over
            }
            for path in settled_new_flags(&mut state, &worktree) {
                let Some(file_name) = path.file_name().map(|n| n.to_string_lossy().to_string())
                else {
                    continue;
                };
                let rel = format!("{FLAG_DIR}/{file_name}");
                // The player's own words about the build — hand them to the
                // chat view before the blocking commit work.
                if let Some(sink) = sink.as_ref() {
                    let text = std::fs::read_to_string(&path).unwrap_or_default();
                    let _ = sink.send(
                        crate::tui::event::Origin::Network,
                        crate::tui::event::TuiEvent::CompletionFlagPublished {
                            path: rel.clone(),
                            text,
                        },
                    );
                }
                let snap = Arc::clone(&snapshot);
                let commit_name = file_name.clone();
                // gix + `git push` are blocking; keep them off the runtime.
                let _ =
                    tokio::task::spawn_blocking(move || publish_once(&snap, &commit_name)).await;
                // The nudge is worth sending even when the push glitched: the
                // completion probe runs locally, and the final task commit
                // catches the tree up.
                if frame_tx
                    .send(PlayerAgentClientFrame::CompletionFlagPushed { path: rel.clone() })
                    .is_err()
                {
                    return;
                }
                tracing::info!("flag watch: completion flag published: {rel}");
            }
        }
    })
}

/// Commit + push the working tree for a freshly settled flag file.
fn publish_once(snapshot: &Mutex<SnapshotRepo>, file_name: &str) {
    let guard = match snapshot.lock() {
        Ok(g) => g,
        // Another thread panicked mid-commit; skip rather than compound it.
        Err(e) => {
            tracing::warn!("flag watch: snapshot lock poisoned: {e}");
            return;
        }
    };
    if let Err(e) = guard.commit_completion_flag(file_name) {
        tracing::warn!("flag watch: commit failed: {e}");
        return;
    }
    if let Err(e) = guard.push_to_remote() {
        tracing::warn!("flag watch: push failed: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_flag(dir: &Path, name: &str, contents: &str) {
        std::fs::create_dir_all(dir.join(FLAG_DIR)).unwrap();
        std::fs::write(dir.join(FLAG_DIR).join(name), contents).unwrap();
    }

    #[test]
    fn flag_fires_once_after_size_settles() {
        let dir = tempfile::tempdir().unwrap();
        let mut state = FlagWatchState::default();

        // No .ololo dir yet.
        assert!(settled_new_flags(&mut state, dir.path()).is_empty());

        write_flag(dir.path(), "weather-widget-done.md", "built the widget");
        // First sighting records the size, does not fire.
        assert!(settled_new_flags(&mut state, dir.path()).is_empty());
        // Second sighting with an unchanged size fires exactly once.
        let fired = settled_new_flags(&mut state, dir.path());
        assert_eq!(fired.len(), 1);
        assert!(fired[0].ends_with(".ololo/weather-widget-done.md"));
        // Never again — even if the file keeps changing.
        write_flag(
            dir.path(),
            "weather-widget-done.md",
            "built the widget, more",
        );
        assert!(settled_new_flags(&mut state, dir.path()).is_empty());
        assert!(settled_new_flags(&mut state, dir.path()).is_empty());
    }

    #[test]
    fn growing_flag_waits_for_the_write_to_settle() {
        let dir = tempfile::tempdir().unwrap();
        let mut state = FlagWatchState::default();

        write_flag(dir.path(), "app-done.md", "a");
        assert!(settled_new_flags(&mut state, dir.path()).is_empty());
        // Still being written: size changed between polls.
        write_flag(dir.path(), "app-done.md", "a longer note about the build");
        assert!(settled_new_flags(&mut state, dir.path()).is_empty());
        // Settled now.
        assert_eq!(settled_new_flags(&mut state, dir.path()).len(), 1);
    }

    #[test]
    fn stale_flags_from_an_earlier_session_stay_quiet_at_boot() {
        let dir = tempfile::tempdir().unwrap();
        // The repo already carries a done-file from a previous session.
        write_flag(dir.path(), "weather-widget-done.md", "old session's note");

        let mut state = FlagWatchState::baseline(dir.path());
        // No matter how long the lobby lasts, the leftover never fires.
        for _ in 0..5 {
            assert!(
                settled_new_flags(&mut state, dir.path()).is_empty(),
                "a pre-existing flag must not publish at boot"
            );
        }
    }

    #[test]
    fn a_rewritten_stale_flag_re_arms_and_fires() {
        let dir = tempfile::tempdir().unwrap();
        write_flag(dir.path(), "weather-widget-done.md", "old session's note");
        let mut state = FlagWatchState::baseline(dir.path());
        assert!(settled_new_flags(&mut state, dir.path()).is_empty());

        // The agent completes the task again this session — same filename.
        write_flag(
            dir.path(),
            "weather-widget-done.md",
            "rebuilt the widget, better this time",
        );
        // Re-arm poll, then the settle poll fires it.
        assert!(settled_new_flags(&mut state, dir.path()).is_empty());
        let fired = settled_new_flags(&mut state, dir.path());
        assert_eq!(fired.len(), 1, "a rewrite this session is a real flag");
        // And only once.
        assert!(settled_new_flags(&mut state, dir.path()).is_empty());
    }

    #[test]
    fn non_flag_files_are_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let mut state = FlagWatchState::default();

        write_flag(dir.path(), "notes.md", "not a flag");
        write_flag(dir.path(), "done.md", "no name prefix, not a flag");
        assert!(settled_new_flags(&mut state, dir.path()).is_empty());
        assert!(settled_new_flags(&mut state, dir.path()).is_empty());
    }
}
