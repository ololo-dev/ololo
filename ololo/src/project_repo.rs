//! Putting the project's repository in place before a session starts.
//!
//! A project may name a git repository — the code its sessions start from
//! (`repo_url`, and `repo_ref`: a branch, tag or commit). Before `ololo
//! start` or `ololo join` asks to upload the folder or takes its first
//! snapshot, [`prepare`] makes sure the session works in that repository:
//!
//! - inside a clone of it already (any remote, any subfolder): work at the
//!   clone's top, as it is;
//! - in an empty folder: clone it right here;
//! - anywhere else: clone it into `./<name>` — or use the clone already
//!   there — and work in that folder;
//! - inside some other git repository: refuse, rather than nest a clone in
//!   someone's project.
//!
//! An existing clone is never touched — no fetch, no checkout: what is in
//! it is the player's. A clone runs with only the https and ssh transports
//! allowed, without submodules, and with the URL after `--`, on top of the
//! server-side validation the URL already passed.

use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, Result, anyhow, bail};
use arena_core::project_repo::ProjectRepo;

use crate::ui;

/// The transports a clone may use.
const ALLOWED_PROTOCOLS: &str = "https:ssh";

/// Finder and Explorer litter an empty folder is allowed to hold.
const IGNORABLE: [&str; 3] = [".DS_Store", "Thumbs.db", "desktop.ini"];

/// The repository a project lookup names, validated the way the server
/// validated it — `None` when it names none.
pub fn of_project(project: &serde_json::Value) -> Result<Option<ProjectRepo>> {
    let Some(url) = project.get("repo_url").and_then(|v| v.as_str()) else {
        return Ok(None);
    };
    let git_ref = project.get("repo_ref").and_then(|v| v.as_str());
    ProjectRepo::parse(url, git_ref)
        .map(Some)
        .map_err(|e| anyhow!("the project names a repository ololo will not clone ({url}): {e}"))
}

/// Where the session will work, decided from the folder it was started in.
#[derive(Debug, PartialEq, Eq)]
pub enum Plan {
    /// A clone of the repository is already here: work at its top.
    Use(PathBuf),
    /// Clone the repository into this folder and work there.
    Clone(PathBuf),
}

/// Make the session's working folder hold the project's repository, and
/// move into it. Returns the folder the session works in.
pub fn prepare(repo: &ProjectRepo) -> Result<PathBuf> {
    let cwd = std::env::current_dir().context("reading the working directory")?;
    let workspace = match plan(&cwd, repo)? {
        Plan::Use(root) => {
            if root != cwd {
                ui::step(format!(
                    "Working in {}, a clone of {}",
                    root.display(),
                    repo.url
                ));
            }
            root
        }
        Plan::Clone(target) => {
            ui::step(format!("Cloning {} into {}...", repo.url, target.display()));
            clone(
                &repo.url,
                repo.git_ref.as_deref(),
                &target,
                ALLOWED_PROTOCOLS,
            )?;
            ui::success(format!("Cloned {}", repo.url));
            target
        }
    };
    if workspace != cwd {
        std::env::set_current_dir(&workspace)
            .with_context(|| format!("moving into {}", workspace.display()))?;
        ui::step(format!("The session works in {}", workspace.display()));
    }
    Ok(workspace)
}

/// What [`prepare`] will do in `cwd`.
pub fn plan(cwd: &Path, repo: &ProjectRepo) -> Result<Plan> {
    let wanted = normalize(&repo.url)
        .ok_or_else(|| anyhow!("cannot read the repository URL {}", repo.url))?;
    if let Some(top) = toplevel(cwd) {
        if is_clone_of(&top, &wanted) {
            return Ok(Plan::Use(top));
        }
        let remotes = remotes(&top);
        bail!(
            "{} is inside another git repository ({}{}), not a clone of {} — run ololo in a \
             clone of it, or in an empty folder to have it cloned",
            cwd.display(),
            top.display(),
            if remotes.is_empty() {
                String::new()
            } else {
                format!(", remote {}", remotes.join(", "))
            },
            repo.url
        );
    }
    if is_empty_dir(cwd)? {
        return Ok(Plan::Clone(cwd.to_path_buf()));
    }
    let sub = cwd.join(dir_name(&repo.url));
    if !sub.exists() {
        return Ok(Plan::Clone(sub));
    }
    if sub.is_dir() {
        // git reports resolved paths; so must the comparison.
        let resolved = sub.canonicalize().unwrap_or_else(|_| sub.clone());
        if toplevel(&resolved).as_deref() == Some(resolved.as_path())
            && is_clone_of(&resolved, &wanted)
        {
            return Ok(Plan::Use(resolved));
        }
    }
    bail!(
        "{} is already here and is not a clone of {} — run ololo in an empty folder, or in a \
         clone of the repository",
        sub.display(),
        repo.url
    )
}

/// `git clone` `url` (at `git_ref`) into `target`, allowing only the
/// transports `allowed` names (colon-separated, as `GIT_ALLOW_PROTOCOL`).
pub fn clone(url: &str, git_ref: Option<&str>, target: &Path, allowed: &str) -> Result<()> {
    // `--branch` takes branches and tags; a commit is checked out after.
    let commit = git_ref.filter(|r| looks_like_commit(r));
    let mut cmd = git();
    cmd.env("GIT_ALLOW_PROTOCOL", allowed)
        .args(["clone", "--no-recurse-submodules"]);
    if let Some(branch) = git_ref.filter(|_| commit.is_none()) {
        cmd.args(["--branch", branch]);
    }
    cmd.arg("--").arg(url).arg(target);
    // Nobody to answer a credential prompt: fail instead of hanging.
    if !std::io::stdin().is_terminal() {
        cmd.env("GIT_TERMINAL_PROMPT", "0");
    }
    let status = cmd
        .status()
        .map_err(|e| anyhow!("ololo needs git to clone the project's repository: {e}"))?;
    if !status.success() {
        bail!(
            "cloning {url} failed ({status}) — check that git can reach it from here (credentials, \
             ssh keys)"
        );
    }
    if let Some(commit) = commit {
        let status = git()
            .arg("-C")
            .arg(target)
            .args(["checkout", "--quiet", "--detach", commit])
            .status()
            .context("running git checkout")?;
        if !status.success() {
            bail!("the clone has no commit {commit}");
        }
    }
    Ok(())
}

/// A git command that ignores whatever repository the environment points at.
fn git() -> Command {
    let mut cmd = Command::new("git");
    cmd.env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE");
    cmd
}

/// The top of the git working tree `dir` is in, if any.
fn toplevel(dir: &Path) -> Option<PathBuf> {
    let out = git()
        .arg("-C")
        .arg(dir)
        .args(["rev-parse", "--show-toplevel"])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let top = String::from_utf8(out.stdout).ok()?;
    let top = PathBuf::from(top.trim_end_matches(['\n', '\r']));
    // Compare like with like: git reports the resolved path.
    Some(top.canonicalize().unwrap_or(top))
}

/// Every remote URL the repository at `dir` knows.
fn remotes(dir: &Path) -> Vec<String> {
    let Ok(out) = git()
        .arg("-C")
        .arg(dir)
        .args(["config", "--get-regexp", r"^remote\..*\.url$"])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
    else {
        return Vec::new();
    };
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|line| line.split_once(' ').map(|(_, url)| url.trim().to_string()))
        .collect()
}

fn is_clone_of(dir: &Path, wanted: &str) -> bool {
    remotes(dir)
        .iter()
        .any(|url| normalize(url).as_deref() == Some(wanted))
}

fn is_empty_dir(dir: &Path) -> Result<bool> {
    let entries = std::fs::read_dir(dir).with_context(|| format!("reading {}", dir.display()))?;
    for entry in entries {
        let entry = entry?;
        if !IGNORABLE.contains(&entry.file_name().to_string_lossy().as_ref()) {
            return Ok(false);
        }
    }
    Ok(true)
}

fn looks_like_commit(git_ref: &str) -> bool {
    (7..=40).contains(&git_ref.len()) && git_ref.chars().all(|c| c.is_ascii_hexdigit())
}

/// A remote URL reduced to `host/path`, so the https, ssh and `git@host:`
/// spellings of one repository compare equal: scheme, user, port, a
/// trailing `.git` or `/` and letter case all dropped. `None` for what is
/// not a network remote (a local path, a `file://` URL).
pub fn normalize(url: &str) -> Option<String> {
    let url = url.trim();
    let (host, path) = if let Some((scheme, rest)) = url.split_once("://") {
        if !matches!(
            scheme,
            "https" | "http" | "ssh" | "git" | "git+ssh" | "ssh+git"
        ) {
            return None;
        }
        let (authority, path) = rest.split_once('/')?;
        let host_port = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
        let host = host_port.split_once(':').map_or(host_port, |(h, _)| h);
        (host, path)
    } else {
        let (user_host, path) = url.split_once(':')?;
        let host = user_host.rsplit_once('@').map_or(user_host, |(_, h)| h);
        if host.is_empty() || host.contains('/') {
            return None;
        }
        (host, path)
    };
    let path = path.trim_matches('/');
    let path = path
        .strip_suffix(".git")
        .unwrap_or(path)
        .trim_end_matches('/');
    if host.is_empty() || path.is_empty() {
        return None;
    }
    Some(format!("{host}/{path}").to_ascii_lowercase())
}

/// The folder `git clone` would make: the URL's last path segment, less
/// `.git`.
pub fn dir_name(url: &str) -> String {
    let path = url.trim().trim_end_matches('/');
    let last = path.rsplit(['/', ':']).next().unwrap_or(path);
    let name = last.strip_suffix(".git").unwrap_or(last);
    if name.is_empty() || name == "." || name == ".." {
        "repository".to_string()
    } else {
        name.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repo(url: &str) -> ProjectRepo {
        ProjectRepo::parse(url, None).unwrap()
    }

    fn run(dir: &Path, args: &[&str]) {
        let status = git()
            .arg("-C")
            .arg(dir)
            .args(args)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .unwrap();
        assert!(status.success(), "git {args:?}");
    }

    /// A repository at `dir` whose `origin` is `url`.
    fn clone_of(dir: &Path, url: &str) {
        std::fs::create_dir_all(dir).unwrap();
        run(dir, &["init", "--quiet"]);
        run(dir, &["remote", "add", "origin", url]);
    }

    fn canonical(p: &Path) -> PathBuf {
        p.canonicalize().unwrap()
    }

    #[test]
    fn one_repository_spelled_three_ways_is_one_repository() {
        let https = normalize("https://github.com/Ololo-Dev/ololo.git").unwrap();
        assert_eq!(https, "github.com/ololo-dev/ololo");
        for same in [
            "https://github.com/ololo-dev/ololo",
            "https://github.com/ololo-dev/ololo/",
            "https://token@github.com/ololo-dev/ololo.git",
            "ssh://git@github.com/ololo-dev/ololo.git",
            "ssh://git@github.com:22/ololo-dev/ololo.git",
            "git@github.com:ololo-dev/ololo.git",
            "git@github.com:/ololo-dev/ololo",
        ] {
            assert_eq!(normalize(same).as_deref(), Some(https.as_str()), "{same}");
        }
        assert_ne!(
            normalize("https://github.com/ololo-dev/app").unwrap(),
            https
        );
        assert_eq!(normalize("/home/me/ololo"), None);
        assert_eq!(normalize("file:///home/me/ololo"), None);
        assert_eq!(normalize("../ololo"), None);
    }

    #[test]
    fn the_folder_is_the_one_git_clone_would_make() {
        assert_eq!(dir_name("https://github.com/ololo-dev/ololo.git"), "ololo");
        assert_eq!(dir_name("https://github.com/ololo-dev/ololo/"), "ololo");
        assert_eq!(dir_name("git@github.com:ololo-dev/app.git"), "app");
        assert_eq!(dir_name("git@host:repo"), "repo");
    }

    #[test]
    fn an_empty_folder_takes_the_clone_itself() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(".DS_Store"), "").unwrap();
        let plan = plan(dir.path(), &repo("https://github.com/a/b.git")).unwrap();
        assert_eq!(plan, Plan::Clone(dir.path().to_path_buf()));
    }

    #[test]
    fn a_busy_folder_gets_a_subfolder_named_after_the_repository() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("notes.txt"), "mine").unwrap();
        let plan = plan(dir.path(), &repo("https://github.com/a/b.git")).unwrap();
        assert_eq!(plan, Plan::Clone(dir.path().join("b")));
    }

    #[test]
    fn a_clone_is_used_as_it_is_from_anywhere_inside_it() {
        let dir = tempfile::tempdir().unwrap();
        clone_of(dir.path(), "git@github.com:a/b.git");
        std::fs::create_dir_all(dir.path().join("src/deep")).unwrap();
        let wanted = repo("https://github.com/a/b");
        assert_eq!(
            plan(dir.path(), &wanted).unwrap(),
            Plan::Use(canonical(dir.path()))
        );
        assert_eq!(
            plan(&dir.path().join("src/deep"), &wanted).unwrap(),
            Plan::Use(canonical(dir.path()))
        );
    }

    #[test]
    fn a_clone_in_the_subfolder_is_used_again() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("notes.txt"), "mine").unwrap();
        clone_of(&dir.path().join("b"), "https://github.com/a/b.git");
        assert_eq!(
            plan(dir.path(), &repo("https://github.com/a/b.git")).unwrap(),
            Plan::Use(canonical(&dir.path().join("b")))
        );
    }

    #[test]
    fn someone_elses_repository_or_folder_is_left_alone() {
        let dir = tempfile::tempdir().unwrap();
        clone_of(dir.path(), "https://github.com/someone/else.git");
        let err = plan(dir.path(), &repo("https://github.com/a/b.git")).unwrap_err();
        assert!(
            err.to_string().contains("inside another git repository"),
            "{err}"
        );

        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("notes.txt"), "mine").unwrap();
        std::fs::create_dir_all(dir.path().join("b")).unwrap();
        std::fs::write(dir.path().join("b/other.txt"), "not a clone").unwrap();
        let err = plan(dir.path(), &repo("https://github.com/a/b.git")).unwrap_err();
        assert!(err.to_string().contains("is not a clone of"), "{err}");
    }

    /// A bare repository with one commit on `main`, tagged `v1`, and a
    /// second commit on `main` after the tag. Returns its `file://` URL and
    /// the first commit.
    fn upstream(root: &Path) -> (String, String) {
        let work = root.join("work");
        std::fs::create_dir_all(&work).unwrap();
        run(&work, &["init", "--quiet", "--initial-branch=main"]);
        run(&work, &["config", "user.email", "t@example.com"]);
        run(&work, &["config", "user.name", "t"]);
        std::fs::write(work.join("a.txt"), "one").unwrap();
        run(&work, &["add", "a.txt"]);
        run(&work, &["commit", "--quiet", "-m", "one"]);
        run(&work, &["tag", "v1"]);
        let first = String::from_utf8(
            git()
                .arg("-C")
                .arg(&work)
                .args(["rev-parse", "HEAD"])
                .output()
                .unwrap()
                .stdout,
        )
        .unwrap()
        .trim()
        .to_string();
        std::fs::write(work.join("a.txt"), "two").unwrap();
        run(&work, &["commit", "--quiet", "-am", "two"]);
        let bare = root.join("upstream.git");
        run(
            root,
            &[
                "clone",
                "--quiet",
                "--bare",
                work.to_str().unwrap(),
                bare.to_str().unwrap(),
            ],
        );
        (format!("file://{}", bare.display()), first)
    }

    #[test]
    fn a_clone_lands_on_the_ref_the_project_names() {
        let root = tempfile::tempdir().unwrap();
        let (url, first) = upstream(root.path());
        let read = |dir: &Path| std::fs::read_to_string(dir.join("a.txt")).unwrap();

        let head = root.path().join("head");
        clone(&url, None, &head, "file").unwrap();
        assert_eq!(read(&head), "two");

        let tag = root.path().join("tag");
        clone(&url, Some("v1"), &tag, "file").unwrap();
        assert_eq!(read(&tag), "one");

        let commit = root.path().join("commit");
        clone(&url, Some(&first[..12]), &commit, "file").unwrap();
        assert_eq!(read(&commit), "one");
    }

    #[test]
    fn a_transport_the_project_may_not_use_is_refused() {
        let root = tempfile::tempdir().unwrap();
        let (url, _) = upstream(root.path());
        assert!(clone(&url, None, &root.path().join("x"), ALLOWED_PROTOCOLS).is_err());
        assert!(!root.path().join("x").exists());
    }
}
