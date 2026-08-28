use std::fs;
use std::path::Path;

use gix::open::Options as OpenOptions;
use gix::sec::Trust;
use gix::status::UntrackedFiles;

/// Probe whether gix can walk a worktree whose git-dir lives OUTSIDE it
/// (no `.git` written into the worktree).
///
/// Strategies, tried in order until one discovers the written files:
///   1. bare repo + `core.worktree` config override
///   2. (fallback b) bare repo re-opened with `core.bare=false` + `core.worktree`
///
/// gix has no `Kind::Worktree` in `create::Kind` (only `WithWorktree` and
/// `Bare`), so linked-worktree init (fallback a) is not available via
/// `init_opts`.
#[test]
fn isolated_git_dir_status_walk() {
    let worktree = tempfile::tempdir().expect("worktree tempdir");
    let worktree_path = worktree.path();

    fs::write(worktree_path.join("hello.txt"), "hello\n").expect("write hello.txt");
    fs::create_dir_all(worktree_path.join("src")).expect("mkdir src");
    fs::write(worktree_path.join("src").join("main.rs"), "fn main() {}\n").expect("write main.rs");

    let git_dir = tempfile::tempdir().expect("git dir tempdir");
    let git_dir_path = git_dir.path();

    let worktree_override = format!("core.worktree={}", worktree_path.display());

    // --- Strategy 1: bare + core.worktree ---
    let thread_safe = gix::ThreadSafeRepository::init_opts(
        git_dir_path,
        gix::create::Kind::Bare,
        gix::create::Options::default(),
        OpenOptions::default()
            .with(Trust::Full)
            .config_overrides([worktree_override.as_str()]),
    )
    .expect("init bare repo with core.worktree override");

    let repo: gix::Repository = thread_safe.to_thread_local();
    eprintln!(
        "strategy 1 (bare+worktree): workdir()={:?}, is_bare={}",
        repo.workdir(),
        repo.is_bare()
    );

    let discovered = match collect_untracked(&repo) {
        Ok(paths) => paths,
        Err(e) => {
            eprintln!("strategy 1 failed: {e}");
            Vec::new()
        }
    };

    let discovered = if files_found(&discovered) {
        eprintln!("STRATEGY 1 SUCCEEDED (bare+core.worktree override)");
        discovered
    } else {
        // --- Strategy 2 (fallback b): write core.bare=false + core.worktree to the bare repo's
        // on-disk config, then open normally. This is the canonical git setup for an
        // isolated git-dir with external worktree. ---
        let config_path = git_dir_path.join("config");
        let mut config_content = fs::read_to_string(&config_path).unwrap_or_default();
        config_content.push_str(&format!(
            "\n[core]\n\tbare = false\n\tworktree = {}\n",
            worktree_path.display()
        ));
        fs::write(&config_path, config_content).expect("write config");

        let repo2: gix::Repository = gix::ThreadSafeRepository::open_opts(
            git_dir_path,
            OpenOptions::default()
                .with(Trust::Full)
                .open_path_as_is(true),
        )
        .expect("reopen with disk config")
        .to_thread_local();

        eprintln!(
            "strategy 2 (disk config bare=false+worktree): workdir()={:?}, is_bare={}",
            repo2.workdir(),
            repo2.is_bare()
        );

        match collect_untracked(&repo2) {
            Ok(paths) if files_found(&paths) => {
                eprintln!("STRATEGY 2 SUCCEEDED (disk config: core.bare=false + core.worktree)");
                paths
            }
            Ok(paths) => {
                panic!(
                    "strategy 2 ran but did not discover files; got: {:?}",
                    paths
                );
            }
            Err(e) => {
                panic!("all strategies failed; strategy 2 error: {e}");
            }
        }
    };

    assert!(
        discovered.iter().any(|p| p.ends_with("hello.txt")),
        "should discover hello.txt; got: {:?}",
        discovered
    );
    assert!(
        discovered.iter().any(|p| p.ends_with("main.rs")),
        "should discover src/main.rs; got: {:?}",
        discovered
    );
}

fn files_found(paths: &[String]) -> bool {
    paths
        .iter()
        .any(|p| p.ends_with("hello.txt") || p.ends_with("main.rs"))
}

fn collect_untracked(repo: &gix::Repository) -> Result<Vec<String>, Box<dyn std::error::Error>> {
    let mut out = Vec::new();

    let platform = repo
        .status(gix::progress::Discard)?
        .untracked_files(UntrackedFiles::Files);

    let iter = platform.into_index_worktree_iter(Vec::new())?;

    for item in iter {
        let item = item?;
        match item {
            gix::status::index_worktree::Item::DirectoryContents { entry, .. } => {
                out.push(entry.rela_path.to_string());
            }
            gix::status::index_worktree::Item::Modification { rela_path, .. } => {
                out.push(rela_path.to_string());
            }
            _ => {}
        }
    }

    Ok(out)
}

// ponytail: walk_dir fallback retained for diagnosis if both strategies return
// empty (confirms whether repo knows its worktree path at all). Not on the
// production path.
#[allow(dead_code)]
fn walk_dir(root: &Path, dir: &Path, out: &mut Vec<String>) {
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let p = entry.path();
        if p.is_dir() {
            if p.file_name().map(|n| n == ".git").unwrap_or(false) {
                continue;
            }
            walk_dir(root, &p, out);
        } else if let Ok(rel) = p.strip_prefix(root) {
            out.push(rel.to_string_lossy().into_owned());
        }
    }
}
