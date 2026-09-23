//! Which files of the working tree a snapshot holds.
//!
//! When the working directory is the top of a git repository, git decides:
//! `git ls-files --cached --others --exclude-standard` is every file the
//! user tracks or would add, with every ignore rule they have honoured
//! exactly — nested `.gitignore`s, `.git/info/exclude`, their global
//! excludes. Anywhere else the tree is walked with the root `.gitignore`.
//! Either way, on top:
//!
//! - `.ololo/` always ships — it is the platform's channel (done-files,
//!   judge-requested artifacts), whatever the user ignores;
//! - dependency stores and build output never do ([`super::PRUNED_DIRS`]),
//!   nor `.git` or Finder junk;
//! - nothing that looks like a secret does ([`looks_secret`]): `.env`
//!   files, private keys, credential files;
//! - nor anything the root `.ololoignore` (gitignore syntax) names.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use super::{PRUNED_DIRS, path_to_forward_slashes};

/// The user's own excludes for what ololo uploads, beside their
/// `.gitignore` (gitignore syntax, read from the working-tree root).
pub const OLOLO_IGNORE: &str = ".ololoignore";

/// The platform's directory inside the working tree.
const OLOLO_DIR: &str = ".ololo";

/// What [`list`] found.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Listing {
    /// Working-tree-relative paths, sorted.
    pub files: Vec<PathBuf>,
    /// Whether git produced the candidate list.
    pub from_git: bool,
    /// Paths left out because they look like secrets.
    pub secrets: Vec<PathBuf>,
}

/// The files a snapshot of `worktree` holds.
pub fn list(worktree: &Path) -> std::io::Result<Listing> {
    let (mut candidates, from_git) = match git_listing(worktree) {
        Some(paths) => (paths, true),
        None => {
            let gitignore = patterns(worktree, ".gitignore");
            let mut out = Vec::new();
            walk(worktree, worktree, &mut out, &|rel, is_dir| {
                ignored(&gitignore, rel, is_dir)
            })?;
            (out, false)
        }
    };
    let ololo = worktree.join(OLOLO_DIR);
    if ololo.is_dir() {
        let mut seen: HashSet<PathBuf> = candidates.iter().cloned().collect();
        let mut platform = Vec::new();
        walk(worktree, &ololo, &mut platform, &|_, _| false)?;
        candidates.extend(platform.into_iter().filter(|p| seen.insert(p.clone())));
    }

    let own_excludes = patterns(worktree, OLOLO_IGNORE);
    let mut listing = Listing {
        from_git,
        ..Listing::default()
    };
    for rel in candidates {
        if pruned(&rel) {
            continue;
        }
        let platform = rel.starts_with(OLOLO_DIR);
        if !platform && excluded_by(&own_excludes, &rel) {
            continue;
        }
        if looks_secret(&rel) {
            listing.secrets.push(rel);
            continue;
        }
        listing.files.push(rel);
    }
    listing.files.sort();
    listing.files.dedup();
    listing.secrets.sort();
    Ok(listing)
}

/// `git ls-files` for a working tree that is a repository's top level.
/// `None` — not a repository root, no git, or git refused — means walk.
fn git_listing(worktree: &Path) -> Option<Vec<PathBuf>> {
    // Only a repository ROOT: a directory under someone's dotfiles repo
    // (a home directory tracked with `*` ignored) must not inherit its
    // rules and snapshot nothing.
    if !worktree.join(".git").exists() {
        return None;
    }
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(worktree)
        .args([
            "-c",
            "core.quotePath=false",
            "ls-files",
            "-z",
            "--cached",
            "--others",
            "--exclude-standard",
        ])
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    Some(
        out.stdout
            .split(|b| *b == 0)
            .filter(|s| !s.is_empty())
            .map(|s| PathBuf::from(String::from_utf8_lossy(s).as_ref()))
            .collect(),
    )
}

/// Recursive walk below `dir`, pruning `.git`, [`PRUNED_DIRS`] and what
/// `skip(rel, is_dir)` says; files (and symlinks — resolved when read) are
/// pushed working-tree-relative.
fn walk(
    root: &Path,
    dir: &Path,
    out: &mut Vec<PathBuf>,
    skip: &dyn Fn(&Path, bool) -> bool,
) -> std::io::Result<()> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        // An unreadable directory is left out, not a failed snapshot.
        Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => return Ok(()),
        Err(e) => return Err(e),
    };
    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        let name = entry.file_name();
        if name == ".git" {
            continue;
        }
        let rel = path.strip_prefix(root).unwrap_or(&path).to_path_buf();
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            if PRUNED_DIRS.iter().any(|d| name == *d) || skip(&rel, true) {
                continue;
            }
            walk(root, &path, out, skip)?;
        } else if file_type.is_file() || file_type.is_symlink() {
            if name == ".DS_Store" || skip(&rel, false) {
                continue;
            }
            out.push(rel);
        }
    }
    Ok(())
}

/// Never shipped, whoever listed it: `.git` internals, dependency stores
/// and build output at any depth, Finder junk.
fn pruned(rel: &Path) -> bool {
    let mut components = rel.components().peekable();
    while let Some(component) = components.next() {
        let std::path::Component::Normal(name) = component else {
            continue;
        };
        let is_last = components.peek().is_none();
        if name == ".git" || (!is_last && PRUNED_DIRS.iter().any(|d| name == *d)) {
            return true;
        }
        if is_last && name == ".DS_Store" {
            return true;
        }
    }
    false
}

/// Ignore patterns from `worktree/<file>`, if it exists.
fn patterns(worktree: &Path, file: &str) -> gix::ignore::Search {
    let mut search = gix::ignore::Search::default();
    let path = worktree.join(file);
    if let Ok(bytes) = std::fs::read(&path) {
        search.add_patterns_buffer(
            &bytes,
            path,
            Some(worktree),
            gix::ignore::search::Ignore::default(),
        );
    }
    search
}

/// Whether `rel` itself matches: directories are matched as directories,
/// so `secrets/` and `logs/` patterns hold.
fn ignored(search: &gix::ignore::Search, rel: &Path, is_dir: bool) -> bool {
    let rel = path_to_forward_slashes(rel);
    search
        .pattern_matching_relative_path(
            rel.as_bytes().into(),
            Some(is_dir),
            gix::glob::pattern::Case::Sensitive,
        )
        .is_some_and(|m| !m.pattern.is_negative())
}

/// Whether `rel` or any directory above it matches — for a flat candidate
/// list (git's), where no walk pruned the directories.
fn excluded_by(search: &gix::ignore::Search, rel: &Path) -> bool {
    let mut prefix = PathBuf::new();
    let mut components = rel.components().peekable();
    while let Some(component) = components.next() {
        prefix.push(component);
        let is_dir = components.peek().is_some();
        if ignored(search, &prefix, is_dir) {
            return true;
        }
    }
    false
}

/// Files that exist to hold credentials never leave the machine, even
/// when the repository tracks them: environment files (their `.example`
/// and friends excepted), private keys and keystores, tool credential
/// files, and whatever sits in an `.ssh`, `.aws` or `.gnupg` directory.
pub fn looks_secret(rel: &Path) -> bool {
    const DIRS: &[&str] = &[".ssh", ".aws", ".gnupg"];
    const NAMES: &[&str] = &[
        ".env",
        ".envrc",
        ".netrc",
        ".npmrc",
        ".pypirc",
        ".pgpass",
        ".git-credentials",
        "id_rsa",
        "id_dsa",
        "id_ecdsa",
        "id_ed25519",
    ];
    const EXTENSIONS: &[&str] = &[
        "pem", "key", "p12", "pfx", "jks", "keystore", "kdbx", "tfstate",
    ];
    const ENV_TEMPLATES: &[&str] = &["example", "sample", "template", "dist", "defaults"];

    let in_secret_dir = rel.parent().is_some_and(|parent| {
        parent
            .components()
            .any(|c| DIRS.iter().any(|d| c.as_os_str() == *d))
    });
    if in_secret_dir {
        return true;
    }
    let Some(name) = rel.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    let name = name.to_ascii_lowercase();
    if NAMES.contains(&name.as_str()) {
        return true;
    }
    if let Some(variant) = name.strip_prefix(".env.") {
        return !ENV_TEMPLATES.iter().any(|t| variant.ends_with(t));
    }
    if name.ends_with(".tfstate.backup") {
        return true;
    }
    rel.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
}

/// Keep `.ololo/` out of the user's own commits: the agent working in
/// their repository would otherwise `git add` the platform's done-files and
/// artifacts along with its change. Appends to `.git/info/exclude` — local
/// to this clone, never committed — once. Returns whether it wrote.
pub fn exclude_platform_dir_locally(worktree: &Path) -> std::io::Result<bool> {
    let git_dir = worktree.join(".git");
    // A `.git` file (worktrees, submodules) points elsewhere; leave those.
    if !git_dir.is_dir() {
        return Ok(false);
    }
    let info = git_dir.join("info");
    let exclude = info.join("exclude");
    let current = std::fs::read_to_string(&exclude).unwrap_or_default();
    let already = current.lines().any(|line| {
        matches!(
            line.trim(),
            ".ololo" | ".ololo/" | "/.ololo" | "/.ololo/" | ".ololo/*" | "/.ololo/*"
        )
    });
    if already {
        return Ok(false);
    }
    std::fs::create_dir_all(&info)?;
    let mut text = current;
    if !text.is_empty() && !text.ends_with('\n') {
        text.push('\n');
    }
    text.push_str("# ololo session files (done-files, artifacts)\n/.ololo/\n");
    std::fs::write(&exclude, text)?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(root: &Path, rel: &str, body: &str) {
        let path = root.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, body).unwrap();
    }

    fn git(dir: &Path, args: &[&str]) {
        let out = std::process::Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@t")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@t")
            .output()
            .expect("git runs");
        assert!(
            out.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    fn names(listing: &Listing) -> Vec<String> {
        listing
            .files
            .iter()
            .map(|p| path_to_forward_slashes(p))
            .collect()
    }

    #[test]
    fn a_repository_is_listed_the_way_git_sees_it() {
        let dir = tempfile::tempdir().unwrap();
        let w = dir.path();
        git(w, &["init", "-q"]);
        write(w, ".gitignore", "/generated/\n*.log\n");
        write(w, "src/app.js", "app");
        write(w, "src/.gitignore", "local-only.txt\n");
        write(w, "src/local-only.txt", "nested ignore");
        write(w, "generated/out.js", "generated");
        write(w, "run.log", "log");
        write(w, "notes/todo.md", "untracked but not ignored");
        write(w, ".ololo/task-1-done.md", "done");
        // The user ignores the platform dir; it ships anyway.
        write(w, ".git/info/exclude", ".ololo/\n");
        git(w, &["add", "src/app.js", ".gitignore"]);
        git(w, &["commit", "-q", "-m", "init"]);

        let listing = list(w).unwrap();
        assert!(listing.from_git);
        assert_eq!(
            names(&listing),
            [
                ".gitignore",
                ".ololo/task-1-done.md",
                "notes/todo.md",
                "src/.gitignore",
                "src/app.js",
            ]
        );
    }

    #[test]
    fn a_plain_directory_is_walked_with_directory_patterns_honoured() {
        let dir = tempfile::tempdir().unwrap();
        let w = dir.path();
        write(w, ".gitignore", "secrets/\nlogs/\n*.tmp\n");
        write(w, "main.py", "print(1)");
        write(w, "secrets/token.txt", "shh");
        write(w, "logs/today.txt", "log");
        write(w, "scratch.tmp", "tmp");
        write(w, "node_modules/x/index.js", "dep");
        write(w, "src/.DS_Store", "finder");

        let listing = list(w).unwrap();
        assert!(!listing.from_git);
        assert_eq!(names(&listing), [".gitignore", "main.py"]);
    }

    #[test]
    fn secrets_and_ololoignore_stay_home() {
        let dir = tempfile::tempdir().unwrap();
        let w = dir.path();
        for rel in [
            ".env",
            ".env.local",
            "config/.env.production",
            "certs/server.pem",
            "deploy/id_ed25519",
            "infra/terraform.tfstate",
            ".aws/credentials",
        ] {
            write(w, rel, "secret");
        }
        write(w, ".env.example", "PORT=3000");
        write(w, ".ololoignore", "fixtures/\n*.mp4\n");
        write(w, "fixtures/big.json", "{}");
        write(w, "demo.mp4", "video");
        write(w, "app.ts", "code");

        let listing = list(w).unwrap();
        assert_eq!(names(&listing), [".env.example", ".ololoignore", "app.ts"]);
        assert_eq!(listing.secrets.len(), 7, "{:?}", listing.secrets);
    }

    #[test]
    fn what_counts_as_a_secret() {
        for secret in [
            ".env",
            "a/.ENV",
            ".env.staging",
            "k.key",
            ".envrc",
            "store.JKS",
            "id_rsa",
            ".npmrc",
            "x/.ssh/config",
            "state.tfstate.backup",
        ] {
            assert!(looks_secret(Path::new(secret)), "{secret}");
        }
        for ok in [
            ".env.example",
            ".env.local.sample",
            "id_rsa.pub",
            "keys.ts",
            "environment.ts",
            "src/ssh/client.rs",
        ] {
            assert!(!looks_secret(Path::new(ok)), "{ok}");
        }
    }

    #[test]
    fn a_home_directory_repository_does_not_capture_a_folder_below_it() {
        // `~` tracked as a dotfiles repo with everything ignored: a session
        // folder inside it is not a repository root, so it is walked.
        let home = tempfile::tempdir().unwrap();
        git(home.path(), &["init", "-q"]);
        write(home.path(), ".gitignore", "*\n");
        let w = home.path().join("play");
        write(&w, "index.html", "<p>");

        let listing = list(&w).unwrap();
        assert!(!listing.from_git);
        assert_eq!(names(&listing), ["index.html"]);
    }

    #[test]
    fn the_platform_dir_is_excluded_from_the_users_commits_once() {
        let dir = tempfile::tempdir().unwrap();
        let w = dir.path();
        git(w, &["init", "-q"]);
        assert!(exclude_platform_dir_locally(w).unwrap());
        assert!(!exclude_platform_dir_locally(w).unwrap(), "idempotent");
        let exclude = std::fs::read_to_string(w.join(".git/info/exclude")).unwrap();
        assert_eq!(exclude.matches("/.ololo/").count(), 1, "{exclude}");

        let plain = tempfile::tempdir().unwrap();
        assert!(!exclude_platform_dir_locally(plain.path()).unwrap());
    }
}
