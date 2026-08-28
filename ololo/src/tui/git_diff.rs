use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DiffStats {
    pub added: u32,
    pub modified: u32,
    pub deleted: u32,
}

pub fn compute_diff(
    git_dir: &Path,
    worktree: &Path,
    baseline_tree: gix::ObjectId,
) -> Result<DiffStats, Box<dyn std::error::Error>> {
    use crate::snapshot;
    let repo = gix::ThreadSafeRepository::open_opts(
        git_dir,
        gix::open::Options::default()
            .with(gix::sec::Trust::Full)
            .open_path_as_is(true),
    )?
    .to_thread_local();

    let current_tree = snapshot::stage_all(&repo, worktree)?;

    let baseline = repo.find_tree(baseline_tree)?;
    let current = repo.find_tree(current_tree)?;

    let mut stats = DiffStats::default();
    for change in repo.diff_tree_to_tree(&baseline, &current, None)? {
        match change {
            gix::object::tree::diff::ChangeDetached::Addition { entry_mode, .. }
            | gix::object::tree::diff::ChangeDetached::Deletion { entry_mode, .. } => {
                if entry_mode.is_tree() {
                    continue;
                }
                match change {
                    gix::object::tree::diff::ChangeDetached::Addition { .. } => stats.added += 1,
                    gix::object::tree::diff::ChangeDetached::Deletion { .. } => stats.deleted += 1,
                    _ => {}
                }
            }
            gix::object::tree::diff::ChangeDetached::Modification { entry_mode, .. } => {
                if entry_mode.is_tree() {
                    continue;
                }
                stats.modified += 1;
            }
            gix::object::tree::diff::ChangeDetached::Rewrite { .. } => {}
        }
    }
    Ok(stats)
}

// ponytail: 5s hardcoded interval matches the token watcher; add a flag if tuning is needed.
pub fn spawn_git_diff_task(
    bus: std::sync::Arc<dyn crate::tui::EventSink>,
    git_dir: std::path::PathBuf,
    worktree: std::path::PathBuf,
    baseline_tree: gix::ObjectId,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(5));
        loop {
            interval.tick().await;
            let git_dir = git_dir.clone();
            let worktree = worktree.clone();
            match tokio::task::spawn_blocking(move || {
                compute_diff(&git_dir, &worktree, baseline_tree).map_err(|e| e.to_string())
            })
            .await
            {
                Ok(Ok(stats)) => {
                    let _ = bus.send(
                        crate::tui::event::Origin::Network,
                        crate::tui::event::TuiEvent::GitDiffUpdate(stats),
                    );
                }
                Ok(Err(e)) => tracing::warn!("git diff computation failed: {e}"),
                Err(e) => tracing::warn!("git diff task join error: {e}"),
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snapshot::SnapshotRepo;
    use crate::test_util::test_util::{HOME_LOCK, HomeGuard};

    fn git_dir_for(home: &std::path::Path, profile: &str, code: &str) -> std::path::PathBuf {
        home.join(".config/ololo/repos").join(profile).join(code)
    }

    #[test]
    fn test_compute_diff_add_modify_delete() {
        let _g = HOME_LOCK.lock().unwrap();
        let home = tempfile::tempdir().expect("home tempdir");
        let worktree = tempfile::tempdir().expect("worktree tempdir");
        let _h = HomeGuard::set(home.path().to_str().unwrap());

        let snapshot = SnapshotRepo::new("default", "DIFF1", worktree.path(), None, None)
            .expect("new succeeds");

        std::fs::write(worktree.path().join("keep.txt"), "keep").unwrap();
        std::fs::write(worktree.path().join("modify.txt"), "v1").unwrap();
        std::fs::write(worktree.path().join("delete.txt"), "gone").unwrap();

        snapshot.commit_session_start().expect("baseline commit");
        let baseline_tree = snapshot.head_tree_id().expect("baseline tree id");
        let git_dir = git_dir_for(home.path(), "default", "DIFF1");
        drop(snapshot);

        std::fs::write(worktree.path().join("new.txt"), "new").unwrap();
        std::fs::write(worktree.path().join("modify.txt"), "v2-modified").unwrap();
        std::fs::remove_file(worktree.path().join("delete.txt")).unwrap();

        let stats =
            compute_diff(&git_dir, worktree.path(), baseline_tree).expect("compute_diff succeeds");

        assert_eq!(
            stats,
            DiffStats {
                added: 1,
                modified: 1,
                deleted: 1
            }
        );
    }

    #[test]
    fn test_compute_diff_no_changes() {
        let _g = HOME_LOCK.lock().unwrap();
        let home = tempfile::tempdir().expect("home tempdir");
        let worktree = tempfile::tempdir().expect("worktree tempdir");
        let _h = HomeGuard::set(home.path().to_str().unwrap());

        let snapshot = SnapshotRepo::new("default", "DIFF2", worktree.path(), None, None)
            .expect("new succeeds");
        std::fs::write(worktree.path().join("a.txt"), "alpha").unwrap();
        snapshot.commit_session_start().expect("baseline commit");
        let baseline_tree = snapshot.head_tree_id().expect("baseline tree id");
        let git_dir = git_dir_for(home.path(), "default", "DIFF2");
        drop(snapshot);

        let stats =
            compute_diff(&git_dir, worktree.path(), baseline_tree).expect("compute_diff succeeds");

        assert_eq!(stats, DiffStats::default());
    }
}
