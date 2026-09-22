//! The snapshot repo: the player's working tree, committed on `main` of a
//! bare repository that lives outside the worktree, and pushed to the
//! per-player store on the ololo server.
//!
//! One branch, one linear line, every commit made by this module — the
//! agent never sees the repository, so nothing rebases or amends it. The
//! commit messages follow `arena_core::snapshot_message`: a subject that
//! names the kind and the task, and a trailer block with the session, the
//! participant, the task title, the probe and a timestamp, so `git log`
//! alone tells which task was in progress at every commit.
//!
//! Pushing never forces: the server's repository is the line of record,
//! and a push it cannot fast-forward (a second client, a re-initialised
//! snapshot) is rejected, after which [`SnapshotRepo::recover_from_remote`]
//! puts the working tree back on top of the server's line as a new commit.

use std::collections::{HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use arena_core::protocol::{PushState, PushStatus};
use arena_core::snapshot_message::{self as message, Kind, Trailers};
use thiserror::Error;

pub mod pusher;

pub use pusher::PusherHandle;

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
    #[error("push is not configured (no remote)")]
    PushDisabled,
    #[error("git {op} failed: {detail}")]
    Git { op: &'static str, detail: String },
}

/// The task the player is working on, as the probe stream told us.
#[derive(Debug, Clone, PartialEq, Eq)]
struct CurrentTask {
    id: uuid::Uuid,
    title: String,
}

/// Who this snapshot belongs to, once the resolve endpoint has said.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Identity {
    session: Option<uuid::Uuid>,
    participant: Option<uuid::Uuid>,
}

/// What the pushes so far amount to, for reports on later commits.
#[derive(Debug, Default)]
struct PushLedger {
    /// Heads that reached the server, newest last (bounded).
    pushed_heads: VecDeque<gix::ObjectId>,
    /// Local heads a recovery replaced after a rejected push, newest last.
    rewritten_heads: VecDeque<gix::ObjectId>,
    /// The last push error, cleared by the next successful push.
    last_error: Option<String>,
}

const LEDGER_CAP: usize = 64;

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
    current_task: Mutex<Option<CurrentTask>>,
    identity: Mutex<Identity>,
    /// Tasks whose `start(<task>)` marker is on `main` — seeded from the
    /// log when an existing repo is reopened, so a reconnect does not write
    /// a second marker.
    started_tasks: Mutex<HashSet<uuid::Uuid>>,
    push: Mutex<PushLedger>,
    /// The background pusher, once attached; without it pushes run inline.
    pusher: Mutex<Option<PusherHandle>>,
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

        let started_tasks = started_tasks_on_main(&repo);

        Ok(Self {
            repo,
            worktree: worktree_abs,
            git_remote_url,
            pat,
            current_task: Mutex::new(None),
            identity: Mutex::new(Identity::default()),
            started_tasks: Mutex::new(started_tasks),
            push: Mutex::new(PushLedger::default()),
            pusher: Mutex::new(None),
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

/// How many commits back a log scan looks (start markers, ancestry).
const LOG_SCAN_LIMIT: usize = 2_000;

/// Walk `main` first-parent from the head, newest first, up to
/// [`LOG_SCAN_LIMIT`] commits.
fn walk_main(repo: &gix::Repository) -> Vec<gix::ObjectId> {
    let Ok(head) = repo.head_commit() else {
        return Vec::new();
    };
    let Ok(walk) = head.ancestors().first_parent_only().all() else {
        return Vec::new();
    };
    walk.take(LOG_SCAN_LIMIT)
        .filter_map(|info| info.ok().map(|i| i.id))
        .collect()
}

/// The tasks with a `start(<task>)` marker on `main`.
fn started_tasks_on_main(repo: &gix::Repository) -> HashSet<uuid::Uuid> {
    let mut out = HashSet::new();
    for id in walk_main(repo) {
        let Ok(commit) = repo.find_commit(id) else {
            continue;
        };
        let Ok(raw) = commit.message_raw() else {
            continue;
        };
        let parsed = message::SnapshotMessage::parse(&raw.to_string());
        if parsed.is_task_start()
            && let Some(task) = parsed.task_id()
        {
            out.insert(task);
        }
    }
    out
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

    /// The commit `main` points at, when there is one.
    pub fn head_id(&self) -> Option<gix::ObjectId> {
        self.repo.head_commit().ok().map(|c| c.id)
    }

    /// Whether `commit` is on the first-parent line of `head` (or is it).
    fn is_on_main_line(&self, head: gix::ObjectId, commit: gix::ObjectId) -> bool {
        if head == commit {
            return true;
        }
        let Ok(head_commit) = self.repo.find_commit(head) else {
            return false;
        };
        let Ok(walk) = head_commit.ancestors().first_parent_only().all() else {
            return false;
        };
        walk.take(LOG_SCAN_LIMIT)
            .filter_map(|info| info.ok())
            .any(|info| info.id == commit)
    }

    /// Who this snapshot belongs to — stamped into every commit from now on.
    pub fn set_identity(&self, session: Option<uuid::Uuid>, participant: Option<uuid::Uuid>) {
        if let Ok(mut id) = self.identity.lock() {
            if session.is_some() {
                id.session = session;
            }
            if participant.is_some() {
                id.participant = participant;
            }
        }
    }

    /// Remember the task the player is currently working on, so auxiliary
    /// commits (artifacts, flags, memory) can address it in their message.
    pub fn set_current_task(&self, task: Option<(uuid::Uuid, &str)>) {
        if let Ok(mut cur) = self.current_task.lock() {
            *cur = task.map(|(id, title)| CurrentTask {
                id,
                title: title.to_string(),
            });
        }
    }

    fn current_task(&self) -> Option<CurrentTask> {
        self.current_task.lock().ok().and_then(|c| c.clone())
    }

    /// The trailer block every commit carries: format, identity, the task
    /// (when known), the probe (for probe commits), the outcome (for the
    /// task-final commit) and the time.
    fn trailers(
        &self,
        task: Option<&CurrentTask>,
        probe: Option<(uuid::Uuid, u32)>,
        outcome: Option<&str>,
    ) -> Trailers {
        let identity = self.identity.lock().map(|i| i.clone()).unwrap_or_default();
        Trailers {
            format: Some(message::FORMAT_VERSION),
            session: identity.session,
            participant: identity.participant,
            task: task.map(|t| t.id),
            task_title: task.map(|t| t.title.clone()).filter(|t| !t.is_empty()),
            probe: probe.map(|(id, _)| id),
            probe_seq: probe.map(|(_, seq)| seq),
            outcome: outcome.map(str::to_string),
            timestamp: Some(chrono::Utc::now()),
        }
    }

    /// `"{kind}({task_id}): {subject}"` when a task is known, plain
    /// `"{kind}: {subject}"` otherwise, plus the trailer block. Deliberately
    /// NOT the `feat(` prefix — `resolve_task_commit` matches `feat(<task_id>)`
    /// to find the task's *final* snapshot, and these auxiliary commits must
    /// never be mistaken for it. The frontend attributes any `kind(task_id):`
    /// message to the task's Changes view.
    fn addressed_message(&self, kind: &Kind, subject: &str) -> String {
        let task = self.current_task();
        let head = message::subject(kind, task.as_ref().map(|t| t.id), subject);
        message::format(&head, &self.trailers(task.as_ref(), None, None))
    }

    fn task_message(
        &self,
        kind: &Kind,
        task: &CurrentTask,
        subject: &str,
        probe: Option<(uuid::Uuid, u32)>,
        outcome: Option<&str>,
    ) -> String {
        let head = message::subject(kind, Some(task.id), subject);
        message::format(&head, &self.trailers(Some(task), probe, outcome))
    }

    pub fn commit_session_start(&self) -> Result<gix::ObjectId, SnapshotError> {
        self.commit_with_label("session start")
    }

    pub fn commit_final(&self) -> Result<gix::ObjectId, SnapshotError> {
        self.commit_with_label("final")
    }

    /// Commit the current worktree state with a per-task message:
    /// `feat({task_id}): {title}` — the task-done marker. Called when a
    /// task's probes are done; `outcome` names why (`completed`, `deadline`).
    pub fn commit_task(
        &self,
        task_id: uuid::Uuid,
        title: &str,
        outcome: &str,
    ) -> Result<gix::ObjectId, SnapshotError> {
        let task = CurrentTask {
            id: task_id,
            title: title.to_string(),
        };
        let msg = self.task_message(&Kind::Feat, &task, title, None, Some(outcome));
        self.commit_raw(&msg)
    }

    /// The task-start marker: an empty commit `start({task_id}): {title}`
    /// (the tree of HEAD, unchanged) that opens the task's range in the
    /// history. Idempotent per task: `Ok(None)` when the marker is already
    /// on `main`. Also makes the task current.
    pub fn commit_task_start(
        &self,
        task_id: uuid::Uuid,
        title: &str,
    ) -> Result<Option<gix::ObjectId>, SnapshotError> {
        self.set_current_task(Some((task_id, title)));
        if self
            .started_tasks
            .lock()
            .map(|s| s.contains(&task_id))
            .unwrap_or(false)
        {
            return Ok(None);
        }
        let task = CurrentTask {
            id: task_id,
            title: title.to_string(),
        };
        let msg = self.task_message(&Kind::Start, &task, title, None, None);
        let tree_id = match self.repo.head_commit() {
            Ok(c) => c
                .tree_id()
                .map_err(|e| SnapshotError::CommitRead(e.into()))?
                .detach(),
            Err(_) => self.repo.empty_tree().id,
        };
        let id = self.commit_tree(&msg, tree_id)?;
        if let Ok(mut s) = self.started_tasks.lock() {
            s.insert(task_id);
        }
        Ok(Some(id))
    }

    /// The tree at a probe dispatch: `probe({task_id}): #{seq} {title}`.
    /// Commits even when nothing changed since the last commit, so every
    /// probe maps to exactly one commit. Makes the task current.
    pub fn commit_probe(
        &self,
        task_id: uuid::Uuid,
        title: &str,
        probe_id: uuid::Uuid,
        seq: u32,
    ) -> Result<gix::ObjectId, SnapshotError> {
        self.set_current_task(Some((task_id, title)));
        let task = CurrentTask {
            id: task_id,
            title: title.to_string(),
        };
        let subject = format!("#{seq} {title}");
        let msg = self.task_message(&Kind::Probe, &task, &subject, Some((probe_id, seq)), None);
        self.commit_raw(&msg)
    }

    /// Commit the whole working tree when a completion flag file appears:
    /// `flag({task_id}): {file_name}`. This commit is what makes the flagged
    /// tree visible to the judges the moment the player declares done.
    pub fn commit_completion_flag(&self, file_name: &str) -> Result<gix::ObjectId, SnapshotError> {
        self.commit_raw(&self.addressed_message(&Kind::Flag, file_name))
    }

    /// Commit whatever sits under `.ololo/artifacts/**`:
    /// `artifact({task_id}): sync`. The server reads the pushed tree by
    /// folder — the task id in the message is for frontend attribution only.
    pub fn commit_artifacts_sync(&self) -> Result<gix::ObjectId, SnapshotError> {
        self.commit_raw(&self.addressed_message(&Kind::Artifact, "sync"))
    }

    /// Commit a work-in-progress checkpoint for an open-ended task:
    /// `wip({task_id}): checkpoint`. Deliberately NOT the `feat(` prefix —
    /// `resolve_task_commit` greps for that to find the task's *final*
    /// snapshot, and a checkpoint must never be mistaken for it. Server-side
    /// probes read HEAD, so this is what keeps their measurements fresh.
    pub fn commit_wip(&self, task_id: uuid::Uuid) -> Result<gix::ObjectId, SnapshotError> {
        let task = match self.current_task() {
            Some(t) if t.id == task_id => t,
            _ => CurrentTask {
                id: task_id,
                title: String::new(),
            },
        };
        let msg = self.task_message(&Kind::Wip, &task, "checkpoint", None, None);
        self.commit_raw(&msg)
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

        let iso = now_iso();
        self.commit_tree(
            &self.addressed_message(&Kind::Memory, &format!("sources @ {iso}")),
            tree_id,
        )?;
        Ok(true)
    }

    fn commit_with_label(&self, label: &str) -> Result<gix::ObjectId, SnapshotError> {
        let subject = message::subject(&Kind::Session, None, &format!("{label} @ {}", now_iso()));
        let msg = message::format(&subject, &self.trailers(None, None, None));
        self.commit_raw(&msg)
    }

    fn commit_raw(&self, msg: &str) -> Result<gix::ObjectId, SnapshotError> {
        let tree_id = self.stage_all()?;
        self.commit_tree(msg, tree_id)
    }

    /// Commit an already-built tree onto `refs/heads/main`.
    fn commit_tree(
        &self,
        msg: &str,
        tree_id: gix::ObjectId,
    ) -> Result<gix::ObjectId, SnapshotError> {
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

        let id = self
            .repo
            .commit_as(sig_ref, sig_ref, "refs/heads/main", msg, tree_id, parents)
            .map_err(|e| SnapshotError::Gix(Box::new(gix::Error::from_error(e))))?;
        Ok(id.detach())
    }

    /// Write the tree of `commit` under `dest` (which must exist and be
    /// empty), leaving out the directories in `skip_dirs` at any depth and
    /// symbolic links. This is what the health scan reads: the committed
    /// tree, not the live working tree, so the client and the server score
    /// the same bytes.
    pub fn export_tree(
        &self,
        commit: gix::ObjectId,
        dest: &Path,
        skip_dirs: &[&str],
    ) -> Result<(), SnapshotError> {
        let commit = self
            .repo
            .find_commit(commit)
            .map_err(|e| SnapshotError::Git {
                op: "export",
                detail: e.to_string(),
            })?;
        let tree = commit.tree()?;
        self.write_tree(&tree, dest, skip_dirs)
    }

    fn write_tree(
        &self,
        tree: &gix::Tree<'_>,
        dest: &Path,
        skip_dirs: &[&str],
    ) -> Result<(), SnapshotError> {
        let decoded = tree.decode().map_err(|e| SnapshotError::Git {
            op: "export",
            detail: e.to_string(),
        })?;
        for entry in decoded.entries {
            let name = entry.filename.to_string();
            // Tree entries are relative names; anything else is not a tree
            // this module wrote.
            if name.is_empty()
                || name == "."
                || name == ".."
                || name.contains('/')
                || name.contains('\\')
            {
                continue;
            }
            let path = dest.join(&name);
            if entry.mode.is_tree() {
                if skip_dirs.contains(&name.as_str()) {
                    continue;
                }
                std::fs::create_dir_all(&path)?;
                let sub = self
                    .repo
                    .find_tree(entry.oid)
                    .map_err(|e| SnapshotError::Git {
                        op: "export",
                        detail: e.to_string(),
                    })?;
                self.write_tree(&sub, &path, skip_dirs)?;
            } else if entry.mode.is_blob() || entry.mode.is_executable() {
                let blob = self
                    .repo
                    .find_blob(entry.oid)
                    .map_err(|e| SnapshotError::Git {
                        op: "export",
                        detail: e.to_string(),
                    })?;
                std::fs::write(&path, &blob.data)?;
                #[cfg(unix)]
                if entry.mode.is_executable() {
                    use std::os::unix::fs::PermissionsExt;
                    let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755));
                }
            }
            // Links and submodules are not the player's code; skipped.
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
}

fn now_iso() -> String {
    let time = gix::date::Time::now_local_or_utc();
    time.format(gix::date::time::format::ISO8601_STRICT)
        .unwrap_or_else(|_| time.seconds.to_string())
}

// ─────────────────────────────── pushing ───────────────────────────────

/// How one push ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PushOutcome {
    /// `head` is on the server now.
    Pushed { head: gix::ObjectId },
    /// The server refused a non-fast-forward: its `main` has commits ours
    /// does not. `recover_from_remote` is the way forward.
    Rejected { head: gix::ObjectId, detail: String },
    Failed {
        head: Option<gix::ObjectId>,
        error: String,
    },
    /// Killed at [`PUSH_TIMEOUT`]; the next push retries.
    Timeout { head: Option<gix::ObjectId> },
    /// No remote configured.
    Disabled,
}

impl PushOutcome {
    pub fn is_pushed(&self) -> bool {
        matches!(self, PushOutcome::Pushed { .. })
    }
}

/// Everything a push needs, cloned out of the repo so the push itself runs
/// without holding the repo lock (git reads the ref at start; a commit made
/// meanwhile simply rides the next push).
#[derive(Debug, Clone)]
pub struct PushTarget {
    git_dir: PathBuf,
    url: String,
    pat: Option<String>,
}

impl PushTarget {
    fn authed_url(&self) -> String {
        match &self.pat {
            Some(pat) => build_authed_url(&self.url, pat),
            None => self.url.clone(),
        }
    }

    fn git(&self) -> Result<std::process::Command, SnapshotError> {
        let git_bin = which::which("git").map_err(|e| SnapshotError::ConfigWrite(e.to_string()))?;
        let mut cmd = std::process::Command::new(git_bin);
        cmd.arg("-C").arg(&self.git_dir);
        Ok(cmd)
    }

    fn local_head(&self) -> Option<gix::ObjectId> {
        let out = self
            .git()
            .ok()?
            .arg("rev-parse")
            .arg("--verify")
            .arg("refs/heads/main")
            .output()
            .ok()?;
        if !out.status.success() {
            return None;
        }
        gix::ObjectId::from_hex(String::from_utf8_lossy(&out.stdout).trim().as_bytes()).ok()
    }

    /// Push the local `refs/heads/main` to the remote using the system `git`
    /// binary over the HTTP smart protocol. Auth is HTTP Basic with the PAT
    /// as password (git-http-backend accepts either field — convention is
    /// username "x", password = PAT). Never forced: a non-fast-forward is
    /// reported as [`PushOutcome::Rejected`], not overwritten.
    pub fn push(&self) -> PushOutcome {
        let head = self.local_head();
        let mut cmd = match self.git() {
            Ok(c) => c,
            Err(e) => {
                return PushOutcome::Failed {
                    head,
                    error: e.to_string(),
                };
            }
        };
        cmd
            // Keep the whole pack in one buffered request with a
            // Content-Length. Past git's default 1MiB postBuffer the client
            // switches to a streamed chunked body, which the server chain
            // (Cloudflare → proxy → CGI bridge) corrupts — every push of
            // session KN5JHB died with "unexpected disconnect while reading
            // sideband packet" while buffered pushes sail through.
            .arg("-c")
            .arg("http.postBuffer=536870912")
            .arg("push")
            .arg(self.authed_url())
            .arg("refs/heads/main:refs/heads/main");
        // A git that never returns — a network stall mid-request, a proxy
        // eating the response — used to freeze the whole headless session
        // narration while probes kept running in the background (session
        // 3XDEWR sat mute for 15 minutes). A push this size finishes in
        // seconds; anything past the cap is killed and logged.
        match run_capped(cmd, PUSH_TIMEOUT) {
            Ok(Some(out)) if out.status.success() => {
                tracing::info!("snapshot pushed to remote store");
                match head {
                    Some(head) => PushOutcome::Pushed { head },
                    None => PushOutcome::Failed {
                        head: None,
                        error: "pushed, but the local head could not be read".into(),
                    },
                }
            }
            Ok(Some(out)) => {
                let stderr = String::from_utf8_lossy(&out.stderr).trim_end().to_string();
                let code = out.status.code().unwrap_or(-1);
                if is_non_fast_forward(&stderr)
                    && let Some(head) = head
                {
                    tracing::warn!("git push rejected (non-fast-forward): {stderr}");
                    return PushOutcome::Rejected {
                        head,
                        detail: stderr,
                    };
                }
                tracing::warn!("git push to remote failed (exit {code}): {stderr}");
                PushOutcome::Failed {
                    head,
                    error: format!("exit {code}: {stderr}"),
                }
            }
            Ok(None) => {
                tracing::warn!(
                    "git push killed after {}s (hung network?); will retry",
                    PUSH_TIMEOUT.as_secs()
                );
                PushOutcome::Timeout { head }
            }
            Err(e) => PushOutcome::Failed {
                head,
                error: e.to_string(),
            },
        }
    }

    /// Fetch the remote `main` into `refs/remotes/ololo/main` and return it.
    fn fetch_remote_main(&self) -> Result<gix::ObjectId, SnapshotError> {
        let mut cmd = self.git()?;
        cmd.arg("-c")
            .arg("http.postBuffer=536870912")
            .arg("fetch")
            .arg("--no-tags")
            .arg(self.authed_url())
            .arg("+refs/heads/main:refs/remotes/ololo/main");
        match run_capped(cmd, PUSH_TIMEOUT)? {
            Some(out) if out.status.success() => {}
            Some(out) => {
                return Err(SnapshotError::Git {
                    op: "fetch",
                    detail: String::from_utf8_lossy(&out.stderr).trim_end().to_string(),
                });
            }
            None => {
                return Err(SnapshotError::Git {
                    op: "fetch",
                    detail: format!("killed after {}s", PUSH_TIMEOUT.as_secs()),
                });
            }
        }
        let out = self
            .git()?
            .arg("rev-parse")
            .arg("--verify")
            .arg("refs/remotes/ololo/main")
            .output()?;
        if !out.status.success() {
            return Err(SnapshotError::Git {
                op: "fetch",
                detail: "remote main not found after fetch".into(),
            });
        }
        gix::ObjectId::from_hex(String::from_utf8_lossy(&out.stdout).trim().as_bytes()).map_err(
            |e| SnapshotError::Git {
                op: "fetch",
                detail: format!("unreadable remote head: {e}"),
            },
        )
    }
}

fn is_non_fast_forward(stderr: &str) -> bool {
    stderr.contains("non-fast-forward")
        || stderr.contains("fetch first")
        || stderr.contains("[rejected]")
        || stderr.contains("[remote rejected]")
}

impl SnapshotRepo {
    /// The push configuration, when pushing is on.
    pub fn push_target(&self) -> Option<PushTarget> {
        let url = self.git_remote_url.clone()?;
        Some(PushTarget {
            git_dir: self.git_dir().to_path_buf(),
            url,
            pat: self.pat.clone(),
        })
    }

    /// Push `main` now, on the calling thread, and record the outcome.
    /// `Ok(Disabled)` when no remote is configured. Failures are outcomes,
    /// not errors: a push glitch never blocks the session flow — the local
    /// commit is persisted, and the next successful push catches up.
    pub fn push_to_remote(&self) -> Result<PushOutcome, SnapshotError> {
        let Some(target) = self.push_target() else {
            return Ok(PushOutcome::Disabled);
        };
        let outcome = target.push();
        self.record_push_outcome(&outcome);
        Ok(outcome)
    }

    pub fn record_push_outcome(&self, outcome: &PushOutcome) {
        let Ok(mut ledger) = self.push.lock() else {
            return;
        };
        match outcome {
            PushOutcome::Pushed { head } => {
                ledger.pushed_heads.push_back(*head);
                if ledger.pushed_heads.len() > LEDGER_CAP {
                    ledger.pushed_heads.pop_front();
                }
                ledger.last_error = None;
            }
            PushOutcome::Rejected { detail, .. } => {
                ledger.last_error = Some(format!("rejected: {detail}"));
            }
            PushOutcome::Failed { error, .. } => ledger.last_error = Some(error.clone()),
            PushOutcome::Timeout { .. } => {
                ledger.last_error = Some(format!("killed after {}s", PUSH_TIMEOUT.as_secs()));
            }
            PushOutcome::Disabled => {}
        }
    }

    /// Where the push of `commit` stands, for a report about it: pushed
    /// when it is on (or behind) a head the server accepted; rejected when
    /// a recovery replaced the line it was on; failed when the last push
    /// error is still standing; pending otherwise.
    // Read by the health report (the next commit wires it in).
    #[allow(dead_code)]
    pub fn push_status_for(&self, commit: gix::ObjectId) -> PushStatus {
        let Ok(ledger) = self.push.lock() else {
            return PushStatus {
                state: PushState::Pending,
                error: None,
                pushed_commit: None,
            };
        };
        let latest_pushed = ledger.pushed_heads.back().copied();
        let pushed = ledger.pushed_heads.contains(&commit)
            || latest_pushed.is_some_and(|head| self.is_on_main_line(head, commit));
        if pushed {
            return PushStatus {
                state: PushState::Pushed,
                error: None,
                pushed_commit: latest_pushed.map(|h| h.to_string()),
            };
        }
        let rewritten = ledger
            .rewritten_heads
            .iter()
            .any(|head| *head == commit || self.is_on_main_line(*head, commit));
        let state = if rewritten {
            PushState::RejectedNonFastForward
        } else if ledger.last_error.is_some() {
            PushState::Failed
        } else {
            PushState::Pending
        };
        PushStatus {
            state,
            error: ledger.last_error.clone(),
            pushed_commit: latest_pushed.map(|h| h.to_string()),
        }
    }

    /// After a rejected push: take the server's `main` as ours and put the
    /// working tree on top of it as a fresh commit, so the line stays the
    /// server's and nothing the player has is lost. The head being replaced
    /// is remembered so reports about commits on it say `history_rewritten`.
    pub fn recover_from_remote(&self) -> Result<gix::ObjectId, SnapshotError> {
        let target = self.push_target().ok_or(SnapshotError::PushDisabled)?;
        let remote_head = target.fetch_remote_main()?;
        let old_head = self.head_id();
        self.repo
            .reference(
                "refs/heads/main",
                remote_head,
                gix::refs::transaction::PreviousValue::Any,
                "ololo: resync to the server's main after a rejected push",
            )
            .map_err(|e| SnapshotError::Git {
                op: "update-ref",
                detail: e.to_string(),
            })?;
        if let Some(old) = old_head
            && let Ok(mut ledger) = self.push.lock()
        {
            ledger.rewritten_heads.push_back(old);
            if ledger.rewritten_heads.len() > LEDGER_CAP {
                ledger.rewritten_heads.pop_front();
            }
        }
        // Start markers on the server's line may differ from ours.
        if let Ok(mut s) = self.started_tasks.lock() {
            *s = started_tasks_on_main(&self.repo);
        }
        let id =
            self.commit_raw(&self.addressed_message(&Kind::Wip, "resync after a rejected push"))?;
        tracing::warn!(
            "snapshot history resynced to the server's main ({} → {})",
            old_head.map(|h| h.to_string()).unwrap_or_default(),
            id
        );
        Ok(id)
    }

    pub fn attach_pusher(&self, handle: PusherHandle) {
        if let Ok(mut p) = self.pusher.lock() {
            *p = Some(handle);
        }
    }

    pub fn pusher(&self) -> Option<PusherHandle> {
        self.pusher.lock().ok().and_then(|p| p.clone())
    }

    /// Ask for `main` to be pushed: through the background pusher when one
    /// is attached (never blocks the caller), inline otherwise.
    pub fn request_push(&self) {
        match self.pusher() {
            Some(pusher) => pusher.request(),
            None => {
                if let Err(e) = self.push_to_remote() {
                    tracing::warn!("snapshot push failed: {e}");
                }
            }
        }
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
///
/// The health scan skips the same list (`ololo_health::PRUNED_DIRS` is the
/// one source), so what is never committed is never scored either.
const PRUNED_DIRS: &[&str] = ololo_health::PRUNED_DIRS;

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

/// Wall-clock cap for one `git push` / `git fetch` subprocess.
pub const PUSH_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);

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

/// Build a git remote URL with the PAT embedded as HTTP Basic userinfo.
/// git-http-backend on the server side ignores the username but requires a
/// valid `Authorization: Basic` header; embedding `x:{pat}` makes the git CLI
/// emit that header automatically. Local (`file://`) remotes carry no auth.
fn build_authed_url(remote_url: &str, pat: &str) -> String {
    if let Some((scheme, rest)) = remote_url.split_once("://") {
        if scheme == "file" || rest.contains('@') {
            return remote_url.to_string();
        }
        return format!("{scheme}://x:{pat}@{rest}");
    }
    remote_url.to_string()
}

#[cfg(test)]
mod tests;
