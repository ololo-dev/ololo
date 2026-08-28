use std::path::{Path, PathBuf};

use thiserror::Error;

#[derive(Debug, Error)]
pub enum SnapshotError {
    #[error("invalid path segment {segment:?}: only [A-Za-z0-9_-] allowed")]
    InvalidSegment { segment: String },
    #[error("home directory not found (HOME/USERPROFILE unset)")]
    HomeNotFound,
    #[error("gix error: {0}")]
    Gix(#[from] Box<gix::Error>),
    #[error("gix init failed: {0}")]
    Init(#[from] Box<gix::init::Error>),
    #[error("gix open failed: {0}")]
    Open(#[from] Box<gix::open::Error>),
    #[error("config write failed: {0}")]
    ConfigWrite(String),
    #[error("gix object write failed: {0}")]
    ObjectWrite(#[from] gix::object::write::Error),
    #[error("gix tree edit failed: {0}")]
    TreeEdit(#[from] gix::object::tree::editor::init::Error),
    #[error("gix tree upsert failed: {0}")]
    TreeUpsert(#[from] gix::objs::tree::editor::Error),
    #[error("gix tree write failed: {0}")]
    TreeWrite(#[from] gix::object::tree::editor::write::Error),
    #[error("gix commit read failed: {0}")]
    CommitRead(#[from] gix::object::commit::Error),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

pub struct SnapshotRepo {
    repo: gix::Repository,
    worktree: PathBuf,
    /// Optional remote URL + PAT for pushing task commits to the server-side
    /// per-player bare repo store. `None` disables push (feature off).
    git_remote_url: Option<String>,
    pat: Option<String>,
    /// The task the player is currently working on, when the probe stream has
    /// told us. Auxiliary commits (artifacts, flags, memory) carry it in
    /// their message so the frontend can attribute their diffs to the task.
    /// Interior mutability: writers hold the repo behind a `Mutex` already.
    current_task: std::sync::Mutex<Option<uuid::Uuid>>,
}

impl SnapshotRepo {
    pub fn new(
        profile: &str,
        join_code: &str,
        worktree: &Path,
        git_remote_url: Option<String>,
        pat: Option<String>,
    ) -> Result<Self, SnapshotError> {
        let clean_profile = sanitize_segment(profile)?;
        let clean_join_code = sanitize_segment(join_code)?;
        let git_dir = git_dir_for(&clean_profile, &clean_join_code)?;

        if let Some(parent) = git_dir.parent() {
            std::fs::create_dir_all(parent)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700));
            }
        }

        let worktree_abs = if worktree.is_absolute() {
            worktree.to_path_buf()
        } else {
            std::env::current_dir().unwrap_or_default().join(worktree)
        };

        if !git_dir.exists() {
            gix::ThreadSafeRepository::init_opts(
                &git_dir,
                gix::create::Kind::Bare,
                gix::create::Options::default(),
                gix::open::Options::default().with(gix::sec::Trust::Full),
            )
            .map_err(Box::new)?;
        }

        let config_path = git_dir.join("config");
        let mut config_content = std::fs::read_to_string(&config_path).unwrap_or_default();
        // Drop any prior [core] bare/worktree lines we may have appended before so
        // recurrence stays idempotent. gix's bare init writes `[core] bare = true`.
        // We append a fresh [core] block with our overrides; gix reads later sections
        // first, so the last [core] wins.
        config_content.push_str(&format!(
            "\n[core]\n\tbare = false\n\tworktree = {}\n",
            worktree_abs.display()
        ));
        std::fs::write(&config_path, config_content)
            .map_err(|e| SnapshotError::ConfigWrite(e.to_string()))?;

        let repo: gix::Repository = gix::ThreadSafeRepository::open_opts(
            &git_dir,
            gix::open::Options::default()
                .with(gix::sec::Trust::Full)
                .open_path_as_is(true),
        )
        .map_err(Box::new)?
        .to_thread_local();

        Ok(Self {
            repo,
            worktree: worktree_abs,
            git_remote_url,
            pat,
            current_task: std::sync::Mutex::new(None),
        })
    }
}

fn sanitize_segment(segment: &str) -> Result<String, SnapshotError> {
    if segment.is_empty() {
        return Err(SnapshotError::InvalidSegment {
            segment: segment.to_string(),
        });
    }
    if segment
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        Ok(segment.to_string())
    } else {
        Err(SnapshotError::InvalidSegment {
            segment: segment.to_string(),
        })
    }
}

fn git_dir_for(profile: &str, join_code: &str) -> Result<PathBuf, SnapshotError> {
    let clean_profile = sanitize_segment(profile)?;
    let clean_join_code = sanitize_segment(join_code)?;
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .map_err(|_| SnapshotError::HomeNotFound)?;
    Ok(PathBuf::from(home)
        .join(".config")
        .join("ololo")
        .join("repos")
        .join(clean_profile)
        .join(clean_join_code))
}

impl SnapshotRepo {
    pub fn git_dir(&self) -> &Path {
        self.repo.git_dir()
    }

    /// The working tree this snapshot repo mirrors.
    pub fn worktree(&self) -> &Path {
        &self.worktree
    }

    pub fn stage_all(&self) -> Result<gix::ObjectId, SnapshotError> {
        stage_all(&self.repo, &self.worktree)
    }

    pub fn head_tree_id(&self) -> Result<gix::ObjectId, SnapshotError> {
        let id = self
            .repo
            .head_tree_id()
            .map_err(|e| SnapshotError::Gix(Box::new(gix::Error::from_error(e))))?;
        Ok(id.detach())
    }

    pub fn commit_session_start(&self) -> Result<(), SnapshotError> {
        self.commit_with_message("session start")
    }

    pub fn commit_final(&self) -> Result<(), SnapshotError> {
        self.commit_with_message("final")
    }

    /// Push the local `refs/heads/main` to the configured remote URL using the
    /// system `git` binary over HTTP smart protocol. Auth is HTTP Basic with
    /// the PAT as both username and password (git-http-backend accepts either
    /// field — convention is username "x", password = PAT).
    ///
    /// Best-effort: failures are logged via `tracing::warn` and return `Ok(())`
    /// so a push glitch never blocks the session flow. The local snapshot
    /// commit is already persisted; the next successful push catches up.
    pub fn push_to_remote(&self) -> Result<(), SnapshotError> {
        let (Some(url), Some(pat)) = (&self.git_remote_url, &self.pat) else {
            return Ok(()); // push disabled
        };
        let git_bin = which::which("git").map_err(|e| SnapshotError::ConfigWrite(e.to_string()))?;
        // ponytail: shelling out to git push is the lazy path; gix 0.85 has no
        // push support. Revisit when gix gains push.
        let authed_url = build_authed_url(url, pat);
        let mut cmd = std::process::Command::new(git_bin);
        cmd.arg("-C")
            .arg(self.git_dir())
            // Keep the whole pack in one buffered request with a
            // Content-Length. Past git's default 1MiB postBuffer the client
            // switches to a streamed chunked body, which the server chain
            // (Cloudflare → proxy → CGI bridge) corrupts — every push of
            // session KN5JHB died with "unexpected disconnect while reading
            // sideband packet" while buffered pushes sail through.
            .arg("-c")
            .arg("http.postBuffer=536870912")
            .arg("push")
            .arg("--force")
            .arg(&authed_url)
            .arg("refs/heads/main");
        // The push runs synchronously inside the event loop (commit_tasks →
        // push on task completion), so a git that never returns — a network
        // stall mid-request, a proxy eating the response — used to freeze
        // the whole headless session narration while probes kept running in
        // the background (session 3XDEWR sat mute for 15 minutes). A push
        // this size finishes in seconds; anything past the cap is killed
        // and logged, and the next successful push catches up.
        match run_capped(cmd, PUSH_TIMEOUT) {
            Ok(Some(out)) if out.status.success() => {
                tracing::info!("snapshot pushed to remote store");
            }
            Ok(Some(out)) => {
                tracing::warn!(
                    "git push to remote failed (exit {}): {}",
                    out.status.code().unwrap_or(-1),
                    String::from_utf8_lossy(&out.stderr).trim_end()
                );
            }
            Ok(None) => {
                tracing::warn!(
                    "git push killed after {}s (hung network?); will retry on the next snapshot",
                    PUSH_TIMEOUT.as_secs()
                );
            }
            Err(e) => return Err(SnapshotError::Io(e)),
        }
        Ok(())
    }

    /// Test-only: message of the current head commit on `refs/heads/main`,
    /// or `None` when no commit exists yet.
    #[cfg(test)]
    pub(crate) fn head_commit_message(&self) -> Option<String> {
        let commit = self.repo.head_commit().ok()?;
        Some(commit.message_raw().ok()?.to_string())
    }

    /// Commit the current worktree state with a per-task message:
    /// `feat({task_id}): {title}`. Called when a task's probes are done.
    pub fn commit_task(&self, task_id: uuid::Uuid, title: &str) -> Result<(), SnapshotError> {
        let message = format!("feat({}): {}", task_id, title);
        self.commit_raw(&message)
    }

    /// Remember the task the player is currently working on, so auxiliary
    /// commits (artifacts, flags, memory) can address it in their message.
    pub fn set_current_task(&self, task_id: Option<uuid::Uuid>) {
        if let Ok(mut cur) = self.current_task.lock() {
            *cur = task_id;
        }
    }

    fn current_task(&self) -> Option<uuid::Uuid> {
        self.current_task.lock().ok().and_then(|c| *c)
    }

    /// `"{kind}({task_id}): {subject}"` when the current task is known,
    /// plain `"{kind}: {subject}"` otherwise. Deliberately NOT the `feat(`
    /// prefix — `resolve_task_commit` matches `feat(<task_id>)` to find the
    /// task's *final* snapshot, and these auxiliary commits must never be
    /// mistaken for it. The frontend attributes any `kind(task_id):` message
    /// to the task's Changes view.
    fn addressed_message(&self, kind: &str, subject: &str) -> String {
        match self.current_task() {
            Some(id) => format!("{kind}({id}): {subject}"),
            None => format!("{kind}: {subject}"),
        }
    }

    /// Commit the whole working tree when a completion flag file appears:
    /// `flag({task_id}): {file_name}`. This commit is what makes the flagged
    /// tree visible to the judges the moment the player declares done.
    pub fn commit_completion_flag(&self, file_name: &str) -> Result<(), SnapshotError> {
        self.commit_raw(&self.addressed_message("flag", file_name))
    }

    /// Commit whatever sits under `.ololo/artifacts/**`:
    /// `artifact({task_id}): sync`. The server reads the pushed tree by
    /// folder — the task id in the message is for frontend attribution only.
    pub fn commit_artifacts_sync(&self) -> Result<(), SnapshotError> {
        self.commit_raw(&self.addressed_message("artifact", "sync"))
    }

    /// Commit a work-in-progress checkpoint for an open-ended task:
    /// `wip({task_id}): checkpoint`. Deliberately NOT the `feat(` prefix —
    /// `resolve_task_commit` greps for that to find the task's *final*
    /// snapshot, and a checkpoint must never be mistaken for it. Server-side
    /// probes read HEAD, so this is what keeps their measurements fresh.
    pub fn commit_wip(&self, task_id: uuid::Uuid) -> Result<(), SnapshotError> {
        let message = format!("wip({}): checkpoint", task_id);
        self.commit_raw(&message)
    }

    /// Commit **only** the memory source files (`AGENTS.md`, `README.md`) on
    /// top of the current HEAD, leaving every other path exactly as the last
    /// commit left it. Returns `false` when they are byte-identical to HEAD,
    /// in which case nothing is committed.
    ///
    /// Deliberately not `stage_all`: the point is to publish what the server
    /// extracts memory from as soon as the player edits it, without dragging
    /// their half-finished task code into a commit they did not ask for.
    /// Their work still lands whole at task completion.
    pub fn commit_memory_sources(&self) -> Result<bool, SnapshotError> {
        let head = self.repo.head_commit().ok();
        let base_tree = match &head {
            Some(c) => c.tree()?,
            None => self.repo.empty_tree(),
        };
        let base_tree_id = base_tree.id;
        let mut editor = base_tree.edit()?;

        for name in arena_core::memory::MEMORY_SOURCE_FILES {
            let abs = self.worktree.join(name);
            match std::fs::read(&abs) {
                Ok(bytes) => {
                    let blob_id = self.repo.write_blob(&bytes)?.detach();
                    editor.upsert(name, gix::object::tree::EntryKind::Blob, blob_id)?;
                }
                // Gone from the worktree: mirror the removal, so deleting
                // AGENTS.md actually retracts it from what the server reads.
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    editor.remove(name)?;
                }
                Err(e) => return Err(SnapshotError::Io(e)),
            }
        }

        let tree_id = editor.write()?.detach();
        // An identical tree means the files did not change; committing would
        // add an empty commit on every probe.
        if head.is_some() && tree_id == base_tree_id {
            return Ok(false);
        }

        let time = gix::date::Time::now_local_or_utc();
        let iso = time
            .format(gix::date::time::format::ISO8601_STRICT)
            .unwrap_or_else(|_| time.seconds.to_string());
        self.commit_tree(
            &self.addressed_message("memory", &format!("sources @ {iso}")),
            tree_id,
        )?;
        Ok(true)
    }

    fn commit_with_message(&self, label: &str) -> Result<(), SnapshotError> {
        let time = gix::date::Time::now_local_or_utc();
        let iso = time
            .format(gix::date::time::format::ISO8601_STRICT)
            .unwrap_or_else(|_| time.seconds.to_string());
        let message = format!("ololo snapshot: {label} @ {iso}");
        self.commit_raw(&message)
    }

    fn commit_raw(&self, message: &str) -> Result<(), SnapshotError> {
        let tree_id = self.stage_all()?;
        self.commit_tree(message, tree_id)
    }

    /// Commit an already-built tree onto `refs/heads/main`.
    fn commit_tree(&self, message: &str, tree_id: gix::ObjectId) -> Result<(), SnapshotError> {
        let time = gix::date::Time::now_local_or_utc();
        let sig = gix::actor::Signature {
            name: "ololo-snapshot".into(),
            email: "ololo@local".into(),
            time,
        };
        let mut time_buf = gix::date::parse::TimeBuf::default();
        let sig_ref = sig.to_ref(&mut time_buf);

        let parents: Vec<gix::ObjectId> = match self.repo.head_commit() {
            Ok(c) => vec![c.id],
            Err(_) => Vec::new(),
        };

        self.repo
            .commit_as(
                sig_ref,
                sig_ref,
                "refs/heads/main",
                message,
                tree_id,
                parents,
            )
            .map_err(|e| SnapshotError::Gix(Box::new(gix::Error::from_error(e))))?;
        Ok(())
    }
}

pub(crate) fn stage_all(
    repo: &gix::Repository,
    worktree: &Path,
) -> Result<gix::ObjectId, SnapshotError> {
    let mut ignore_search = gix::ignore::Search::default();
    let gitignore_path = worktree.join(".gitignore");
    if gitignore_path.is_file() {
        let bytes = std::fs::read(&gitignore_path)?;
        ignore_search.add_patterns_buffer(
            &bytes,
            gitignore_path.clone(),
            Some(worktree),
            gix::ignore::search::Ignore::default(),
        );
    }

    let empty_tree = repo.empty_tree();
    let mut editor = empty_tree.edit()?;

    let mut rel_paths: Vec<PathBuf> = Vec::new();
    collect_files(worktree, worktree, &mut rel_paths)?;
    rel_paths.sort();

    for rel in &rel_paths {
        let rel_str = path_to_forward_slashes(rel);
        let rel_bstr: &gix::bstr::BStr = rel_str.as_bytes().into();
        if let Some(m) = ignore_search.pattern_matching_relative_path(
            rel_bstr,
            Some(false),
            gix::glob::pattern::Case::Sensitive,
        ) && !m.pattern.is_negative()
        {
            continue;
        }

        let abs = worktree.join(rel);
        let bytes = std::fs::read(&abs)?;
        let blob_id = repo.write_blob(&bytes)?.detach();

        #[cfg(unix)]
        let kind = {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&abs)?.permissions().mode();
            if mode & 0o111 != 0 {
                gix::object::tree::EntryKind::BlobExecutable
            } else {
                gix::object::tree::EntryKind::Blob
            }
        };
        #[cfg(not(unix))]
        let kind = gix::object::tree::EntryKind::Blob;

        editor.upsert(rel_str.as_str(), kind, blob_id)?;
    }

    let tree_id = editor.write()?.detach();
    Ok(tree_id)
}

/// Directory names never worth snapshotting, pruned during the walk.
///
/// The snapshot repo is both version control AND the artifact channel —
/// judges read the committed code, screenshots/screencasts ride along under
/// `.ololo/`, and even a `run.log` is wanted context. So this list is
/// deliberately narrow: only regenerable dependency stores and build output
/// (`npm install` / `cargo build` recreate them from the committed
/// manifests), which would otherwise balloon every push — a single
/// `node_modules` is tens of thousands of files, a Rust `target/` hundreds
/// of megabytes, enough to blow the push time cap and lose the snapshot the
/// judges were meant to see. Pruning at walk time also skips descending
/// into those trees at all. Anything else a player writes is kept; a
/// workspace `.gitignore` remains their tool for their own exclusions.
const PRUNED_DIRS: &[&str] = &[
    // dependency stores
    "node_modules",
    ".venv",
    "venv",
    // build / framework output
    "target",
    "dist",
    "build",
    "out",
    "obj",
    ".next",
    ".nuxt",
    ".svelte-kit",
    // caches and tool state
    "__pycache__",
    ".cache",
    ".parcel-cache",
    ".turbo",
    ".gradle",
    ".tox",
    ".pytest_cache",
    ".mypy_cache",
    ".ruff_cache",
    ".nyc_output",
    "coverage",
    ".idea",
];

fn collect_files(root: &Path, dir: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        let name = entry.file_name();
        if name == ".git" {
            continue;
        }
        let meta = entry.file_type()?;
        if meta.is_dir() {
            if PRUNED_DIRS.iter().any(|d| name == *d) {
                continue;
            }
            collect_files(root, &path, out)?;
        } else if meta.is_file() || meta.is_symlink() {
            if name == ".DS_Store" {
                continue;
            }
            let rel = path.strip_prefix(root).unwrap_or(&path).to_path_buf();
            out.push(rel);
        }
    }
    Ok(())
}

fn path_to_forward_slashes(p: &Path) -> String {
    p.components()
        .filter_map(|c| match c {
            std::path::Component::Normal(s) => Some(s.to_string_lossy().to_string()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/")
}

/// Build a git remote URL with the PAT embedded as HTTP Basic userinfo.
/// git-http-backend on the server side ignores the username but requires a
/// valid `Authorization: Basic` header; embedding `x:{pat}` makes the git CLI
/// emit that header automatically.
/// Wall-clock cap for one `git push` subprocess.
const PUSH_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);

/// Run a command with a hard wall-clock cap. `Ok(Some(output))` when it
/// finished in time, `Ok(None)` when it was killed at the deadline. Reader
/// threads drain stdout/stderr while the child runs, so a chatty child
/// cannot deadlock on a full pipe and a killed child cannot block the
/// readers (its pipe ends close on kill).
fn run_capped(
    mut cmd: std::process::Command,
    cap: std::time::Duration,
) -> std::io::Result<Option<std::process::Output>> {
    use std::io::Read;
    use std::process::Stdio;
    let mut child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let mut child_out = child.stdout.take().expect("stdout piped");
    let mut child_err = child.stderr.take().expect("stderr piped");
    let out_reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = child_out.read_to_end(&mut buf);
        buf
    });
    let err_reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = child_err.read_to_end(&mut buf);
        buf
    });

    let deadline = std::time::Instant::now() + cap;
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break Some(status);
        }
        if std::time::Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait(); // reap; also closes the pipes
            break None;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    };
    let stdout = out_reader.join().unwrap_or_default();
    let stderr = err_reader.join().unwrap_or_default();
    Ok(status.map(|status| std::process::Output {
        status,
        stdout,
        stderr,
    }))
}

fn build_authed_url(remote_url: &str, pat: &str) -> String {
    if let Some((_scheme, rest)) = remote_url.split_once("://")
        && rest.contains('@')
    {
        return remote_url.to_string();
    } else if let Some((scheme, rest)) = remote_url.split_once("://") {
        return format!("{scheme}://x:{pat}@{rest}");
    }
    remote_url.to_string()
}

#[cfg(test)]
mod tests;
