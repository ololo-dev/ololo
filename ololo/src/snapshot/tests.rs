use super::*;
use crate::test_util::test_util::{HOME_LOCK, HomeGuard};

#[test]
fn test_sanitize_valid() {
    for s in ["default", "ABC123", "my-profile", "code_123"] {
        assert_eq!(sanitize_segment(s).unwrap(), s);
    }
}

#[test]
fn test_sanitize_rejects_dotdot() {
    assert!(sanitize_segment("../").is_err());
    assert!(sanitize_segment("..").is_err());
}

#[test]
fn test_sanitize_rejects_slash() {
    assert!(sanitize_segment("/").is_err());
    assert!(sanitize_segment("a/b").is_err());
}

#[test]
fn test_sanitize_rejects_space() {
    assert!(sanitize_segment("a b").is_err());
}

#[test]
fn test_git_dir_for_rejects_invalid() {
    let _g = HOME_LOCK.lock().unwrap();
    let _h = HomeGuard::set("/tmp");
    assert!(git_dir_for("bad/name", "code").is_err());
    assert!(git_dir_for("default", "code with space").is_err());
    assert!(git_dir_for("..", "code").is_err());
}

#[test]
fn test_new_creates_repo() {
    let _g = HOME_LOCK.lock().unwrap();
    let home = tempfile::tempdir().expect("home tempdir");
    let worktree = tempfile::tempdir().expect("worktree tempdir");
    let _h = HomeGuard::set(home.path().to_str().unwrap());

    let repo = SnapshotRepo::new("default", "ABC123", worktree.path(), None, None)
        .expect("new succeeds on fresh repo");

    let git_dir = home.path().join(".config/ololo/repos/default/ABC123");
    assert!(git_dir.exists(), "git_dir should exist after new()");
    let config = std::fs::read_to_string(git_dir.join("config")).expect("read config");
    assert!(
        config.contains("bare = false"),
        "config has bare=false: {config}"
    );
    assert!(
        config.contains(&format!("worktree = {}", worktree.path().display())),
        "config has worktree=...: {config}"
    );
    assert_eq!(
        repo.repo.workdir(),
        Some(worktree.path()),
        "repo.workdir() returns the worktree"
    );
}

#[test]
fn test_new_opens_existing_repo() {
    let _g = HOME_LOCK.lock().unwrap();
    let home = tempfile::tempdir().expect("home tempdir");
    let worktree = tempfile::tempdir().expect("worktree tempdir");
    let _h = HomeGuard::set(home.path().to_str().unwrap());

    let _first = SnapshotRepo::new("default", "ABC123", worktree.path(), None, None)
        .expect("first new() succeeds");
    // Second call with same key must open the existing repo, not error.
    let _second = SnapshotRepo::new("default", "ABC123", worktree.path(), None, None)
        .expect("second new() opens existing repo");
}

fn tree_entries(
    repo: &gix::Repository,
    tree_id: gix::ObjectId,
) -> Vec<(String, gix::object::tree::EntryKind)> {
    let tree = repo.find_object(tree_id).unwrap().into_tree();
    let decoded = tree.decode().unwrap();
    decoded
        .entries
        .iter()
        .map(|e| (e.filename.to_string(), e.mode.kind()))
        .collect()
}

fn find_entry_in_tree(
    repo: &gix::Repository,
    tree_id: gix::ObjectId,
    name: &str,
) -> Option<(gix::ObjectId, gix::object::tree::EntryKind)> {
    let tree = repo.find_object(tree_id).unwrap().into_tree();
    let decoded = tree.decode().unwrap();
    decoded
        .entries
        .iter()
        .find(|e| e.filename == name)
        .map(|e| (e.oid.to_owned(), e.mode.kind()))
}

fn read_blob(repo: &gix::Repository, id: gix::ObjectId) -> Vec<u8> {
    repo.find_object(id).unwrap().data.to_vec()
}

#[test]
fn test_stage_all_basic_and_ignores() {
    let _g = HOME_LOCK.lock().unwrap();
    let home = tempfile::tempdir().expect("home tempdir");
    let worktree = tempfile::tempdir().expect("worktree tempdir");
    let _h = HomeGuard::set(home.path().to_str().unwrap());

    let snapshot =
        SnapshotRepo::new("default", "STAGE1", worktree.path(), None, None).expect("new succeeds");

    std::fs::write(worktree.path().join("hello.txt"), "hello world").unwrap();
    std::fs::create_dir_all(worktree.path().join("src")).unwrap();
    std::fs::write(worktree.path().join("src/main.rs"), "fn main() {}").unwrap();
    std::fs::write(worktree.path().join(".gitignore"), "*.tmp\n").unwrap();
    std::fs::write(worktree.path().join("temp.tmp"), "should be ignored").unwrap();
    std::fs::create_dir_all(worktree.path().join(".git")).unwrap();
    std::fs::write(worktree.path().join(".git/HEAD"), "ref: refs/heads/main").unwrap();

    let tree_id = snapshot.stage_all().expect("stage_all succeeds");
    assert!(!tree_id.is_null(), "tree id is non-null");

    let repo = &snapshot.repo;
    let entries = tree_entries(repo, tree_id);
    let names: Vec<&str> = entries.iter().map(|(n, _)| n.as_str()).collect();
    assert!(
        names.contains(&"hello.txt"),
        "hello.txt in tree: {:?}",
        names
    );
    assert!(names.contains(&"src"), "src tree in tree: {:?}", names);
    assert!(
        names.contains(&".gitignore"),
        ".gitignore in tree: {:?}",
        names
    );
    assert!(
        !names.contains(&"temp.tmp"),
        "temp.tmp ignored: {:?}",
        names
    );
    assert!(!names.contains(&".git"), ".git excluded: {:?}", names);
}

#[test]
fn test_stage_all_modifications_deletions_additions() {
    let _g = HOME_LOCK.lock().unwrap();
    let home = tempfile::tempdir().expect("home tempdir");
    let worktree = tempfile::tempdir().expect("worktree tempdir");
    let _h = HomeGuard::set(home.path().to_str().unwrap());

    let snapshot =
        SnapshotRepo::new("default", "STAGE2", worktree.path(), None, None).expect("new succeeds");

    std::fs::write(worktree.path().join("hello.txt"), "v1").unwrap();
    std::fs::create_dir_all(worktree.path().join("src")).unwrap();
    std::fs::write(worktree.path().join("src/main.rs"), "fn main() {}").unwrap();
    std::fs::write(worktree.path().join(".gitignore"), "*.tmp\n").unwrap();
    std::fs::write(worktree.path().join("temp.tmp"), "ignored").unwrap();

    let _first = snapshot.stage_all().expect("first stage_all");

    std::fs::write(worktree.path().join("hello.txt"), "v2-modified").unwrap();
    std::fs::remove_file(worktree.path().join("src/main.rs")).unwrap();
    std::fs::write(worktree.path().join("new.rs"), "fn new() {}").unwrap();
    std::fs::write(worktree.path().join("temp.tmp"), "still ignored").unwrap();

    let tree_id = snapshot.stage_all().expect("second stage_all");
    let repo = &snapshot.repo;
    let entries = tree_entries(repo, tree_id);
    let names: Vec<&str> = entries.iter().map(|(n, _)| n.as_str()).collect();
    assert!(
        names.contains(&"hello.txt"),
        "hello.txt present: {:?}",
        names
    );
    assert!(names.contains(&"new.rs"), "new.rs present: {:?}", names);
    assert!(
        !names.contains(&"temp.tmp"),
        "temp.tmp ignored: {:?}",
        names
    );
    assert!(!names.contains(&".git"), ".git excluded: {:?}", names);

    let hello_blob_id = find_entry_in_tree(repo, tree_id, "hello.txt")
        .expect("hello.txt entry")
        .0;
    let blob_data = read_blob(repo, hello_blob_id);
    assert_eq!(blob_data, b"v2-modified", "hello.txt content is v2");

    assert!(
        find_entry_in_tree(repo, tree_id, "src").is_none(),
        "src dir is empty after main.rs deletion; should not be in tree"
    );
}

fn head_commit_message(repo: &gix::Repository) -> Option<String> {
    let commit = repo.head_commit().ok()?;
    let msg = commit.message_raw().ok()?;
    Some(msg.to_string())
}

fn head_commit_tree_id(repo: &gix::Repository) -> Option<gix::ObjectId> {
    let commit = repo.head_commit().ok()?;
    commit.tree_id().ok().map(|id| id.detach())
}

#[test]
fn test_commit_session_start_creates_first_commit_on_main() {
    let _g = HOME_LOCK.lock().unwrap();
    let home = tempfile::tempdir().expect("home tempdir");
    let worktree = tempfile::tempdir().expect("worktree tempdir");
    let _h = HomeGuard::set(home.path().to_str().unwrap());

    let snapshot =
        SnapshotRepo::new("default", "COM1", worktree.path(), None, None).expect("new succeeds");
    std::fs::write(worktree.path().join("a.txt"), "alpha").unwrap();

    snapshot
        .commit_session_start()
        .expect("commit_session_start");

    let repo = &snapshot.repo;
    let msg = head_commit_message(repo).expect("HEAD commit exists");
    assert!(
        msg.starts_with("ololo snapshot: session start @ "),
        "message prefix: {msg}"
    );

    let head = repo.head_commit().expect("head commit");
    let decoded = head.decode().expect("decode commit");
    assert!(decoded.parents.is_empty(), "first commit has no parents");
    let author = decoded.author().expect("author parsed");
    assert_eq!(author.name, "ololo-snapshot", "author name");
    assert_eq!(author.email, "ololo@local", "author email");
    let committer = decoded.committer().expect("committer parsed");
    assert_eq!(committer.name, "ololo-snapshot", "committer name");
    assert_eq!(committer.email, "ololo@local", "committer email");

    let main = repo
        .find_reference("refs/heads/main")
        .expect("refs/heads/main exists");
    assert_eq!(
        main.id(),
        head.id,
        "refs/heads/main points to the new commit"
    );
}

#[test]
fn test_commit_session_start_uses_parent_when_head_exists() {
    let _g = HOME_LOCK.lock().unwrap();
    let home = tempfile::tempdir().expect("home tempdir");
    let worktree = tempfile::tempdir().expect("worktree tempdir");
    let _h = HomeGuard::set(home.path().to_str().unwrap());

    let snapshot =
        SnapshotRepo::new("default", "COM2", worktree.path(), None, None).expect("new succeeds");
    std::fs::write(worktree.path().join("a.txt"), "v1").unwrap();
    snapshot.commit_session_start().expect("first commit");
    let first_head = snapshot.repo.head_commit().expect("head").id;

    std::fs::write(worktree.path().join("a.txt"), "v2").unwrap();
    std::fs::write(worktree.path().join("b.txt"), "new").unwrap();
    snapshot.commit_session_start().expect("second commit");

    let head = snapshot.repo.head_commit().expect("head after second");
    assert_ne!(head.id, first_head, "HEAD advanced");
    let decoded = head.decode().expect("decode");
    assert_eq!(decoded.parents.len(), 1, "second commit has one parent");
    assert_eq!(
        decoded.parents[0].to_string(),
        first_head.to_string(),
        "parent is first commit"
    );
}

#[test]
fn test_commit_final_message_label() {
    let _g = HOME_LOCK.lock().unwrap();
    let home = tempfile::tempdir().expect("home tempdir");
    let worktree = tempfile::tempdir().expect("worktree tempdir");
    let _h = HomeGuard::set(home.path().to_str().unwrap());

    let snapshot =
        SnapshotRepo::new("default", "COM3", worktree.path(), None, None).expect("new succeeds");
    std::fs::write(worktree.path().join("done.txt"), "finished").unwrap();

    let repo_handle = snapshot.repo.clone();
    snapshot.commit_final().expect("commit_final");

    let msg = head_commit_message(&repo_handle).expect("HEAD commit exists");
    assert!(
        msg.starts_with("ololo snapshot: final @ "),
        "final message prefix: {msg}"
    );
    let head = repo_handle.head_commit().expect("head");
    let decoded = head.decode().expect("decode");
    assert!(decoded.parents.is_empty(), "first commit no parents");
}

#[test]
fn test_commit_session_start_and_final_chain() {
    let _g = HOME_LOCK.lock().unwrap();
    let home = tempfile::tempdir().expect("home tempdir");
    let worktree = tempfile::tempdir().expect("worktree tempdir");
    let _h = HomeGuard::set(home.path().to_str().unwrap());

    let snapshot =
        SnapshotRepo::new("default", "COM4", worktree.path(), None, None).expect("new succeeds");
    std::fs::write(worktree.path().join("f.txt"), "start").unwrap();
    snapshot.commit_session_start().expect("start commit");
    let start_tree = head_commit_tree_id(&snapshot.repo).expect("start tree");

    std::fs::write(worktree.path().join("f.txt"), "end").unwrap();
    let repo_handle = snapshot.repo.clone();
    snapshot.commit_final().expect("final commit");
    let final_tree = head_commit_tree_id(&repo_handle).expect("final tree");

    assert_ne!(final_tree, start_tree, "trees differ across commits");
    let head = repo_handle.head_commit().expect("head");
    let decoded = head.decode().expect("decode");
    assert_eq!(decoded.parents.len(), 1, "final has start as parent");
    let msg = decoded.message.to_string();
    assert!(
        msg.starts_with("ololo snapshot: final @ "),
        "final label: {msg}"
    );
}

#[test]
fn test_commit_task_message_format() {
    let _g = HOME_LOCK.lock().unwrap();
    let home = tempfile::tempdir().expect("home tempdir");
    let worktree = tempfile::tempdir().expect("worktree tempdir");
    let _h = HomeGuard::set(home.path().to_str().unwrap());

    let snapshot =
        SnapshotRepo::new("default", "TASK1", worktree.path(), None, None).expect("new succeeds");
    std::fs::write(worktree.path().join("a.txt"), "alpha").unwrap();
    snapshot.commit_session_start().expect("start commit");

    std::fs::write(worktree.path().join("b.txt"), "beta").unwrap();
    let task_id = uuid::Uuid::new_v4();
    snapshot
        .commit_task(task_id, "Implement greeting endpoint")
        .expect("commit_task");

    let msg = head_commit_message(&snapshot.repo).expect("HEAD commit exists");
    assert_eq!(
        msg,
        format!("feat({task_id}): Implement greeting endpoint"),
        "task commit message: {msg}"
    );

    let head = snapshot.repo.head_commit().expect("head");
    let decoded = head.decode().expect("decode");
    assert_eq!(decoded.parents.len(), 1, "task commit has start as parent");
}

#[test]
fn test_commit_wip_message_never_matches_the_feat_grep() {
    let _g = HOME_LOCK.lock().unwrap();
    let home = tempfile::tempdir().expect("home tempdir");
    let worktree = tempfile::tempdir().expect("worktree tempdir");
    let _h = HomeGuard::set(home.path().to_str().unwrap());

    let snapshot =
        SnapshotRepo::new("default", "WIP1", worktree.path(), None, None).expect("new succeeds");
    std::fs::write(worktree.path().join("a.txt"), "alpha").unwrap();
    snapshot.commit_session_start().expect("start commit");

    std::fs::write(worktree.path().join("b.txt"), "beta").unwrap();
    let task_id = uuid::Uuid::new_v4();
    snapshot.commit_wip(task_id).expect("commit_wip");

    let msg = head_commit_message(&snapshot.repo).expect("HEAD commit exists");
    assert_eq!(msg, format!("wip({task_id}): checkpoint"));
    // The server resolves the task's FINAL snapshot by grepping `feat(` —
    // a checkpoint must never satisfy that grep.
    assert!(!msg.starts_with("feat("));
}

#[test]
fn test_push_to_remote_no_op_when_disabled() {
    let _g = HOME_LOCK.lock().unwrap();
    let home = tempfile::tempdir().expect("home tempdir");
    let worktree = tempfile::tempdir().expect("worktree tempdir");
    let _h = HomeGuard::set(home.path().to_str().unwrap());
    let snapshot = SnapshotRepo::new("default", "PUSH1", worktree.path(), None, None).expect("new");
    std::fs::write(worktree.path().join("a.txt"), "a").unwrap();
    snapshot.commit_session_start().expect("start");
    // No remote/pat configured — push_to_remote is a silent no-op.
    snapshot.push_to_remote().expect("no-op push");
}

#[test]
fn build_authed_url_embeds_pat() {
    let u = build_authed_url("https://arena.dev/git/s/p.git", "ololo_secret");
    assert_eq!(u, "https://x:ololo_secret@arena.dev/git/s/p.git");
}

#[test]
fn build_authed_url_preserves_existing_userinfo() {
    let u = build_authed_url("https://u:p@arena.dev/git/s/p.git", "ololo_secret");
    assert_eq!(u, "https://u:p@arena.dev/git/s/p.git");
}

#[test]
fn build_authed_url_handles_no_scheme() {
    let u = build_authed_url("not-a-url", "ololo_secret");
    assert_eq!(u, "not-a-url");
}

// ─────────────────── Task-addressed auxiliary commits ────────────────────────

/// Auxiliary commits (artifacts, flags) address the current task in their
/// message so the frontend can attribute their diffs; without a known task
/// they keep the plain legacy form.
#[test]
fn test_auxiliary_commits_address_the_current_task() {
    let _g = HOME_LOCK.lock().unwrap();
    let home = tempfile::tempdir().expect("home tempdir");
    let worktree = tempfile::tempdir().expect("worktree tempdir");
    let _h = HomeGuard::set(home.path().to_str().unwrap());

    let snapshot =
        SnapshotRepo::new("default", "ADDR1", worktree.path(), None, None).expect("new succeeds");

    std::fs::write(worktree.path().join("a.txt"), "1").unwrap();
    snapshot.commit_artifacts_sync().expect("commit");
    assert_eq!(
        snapshot.head_commit_message().as_deref(),
        Some("artifact: sync"),
        "no current task → legacy message"
    );

    let task_id = uuid::Uuid::new_v4();
    snapshot.set_current_task(Some(task_id));
    std::fs::write(worktree.path().join("a.txt"), "2").unwrap();
    snapshot.commit_artifacts_sync().expect("commit");
    assert_eq!(
        snapshot.head_commit_message(),
        Some(format!("artifact({task_id}): sync")),
    );

    std::fs::write(worktree.path().join("b.txt"), "3").unwrap();
    snapshot.commit_completion_flag("done.md").expect("commit");
    assert_eq!(
        snapshot.head_commit_message(),
        Some(format!("flag({task_id}): done.md")),
    );
}

// ─────────────────── Memory-source commits (AGENTS.md / README.md) ───────────

/// The whole point of a memory-source commit: publish what the server reads
/// memory from, without dragging the player's in-progress task code with it.
#[test]
fn test_commit_memory_sources_touches_only_memory_files() {
    let _g = HOME_LOCK.lock().unwrap();
    let home = tempfile::tempdir().expect("home tempdir");
    let worktree = tempfile::tempdir().expect("worktree tempdir");
    let _h = HomeGuard::set(home.path().to_str().unwrap());

    let snapshot =
        SnapshotRepo::new("default", "MEM1", worktree.path(), None, None).expect("new succeeds");

    // Baseline commit carrying both a memory file and ordinary work.
    std::fs::write(worktree.path().join("AGENTS.md"), "agent v1").unwrap();
    std::fs::write(worktree.path().join("solution.rs"), "fn main() {}").unwrap();
    snapshot.commit_session_start().expect("baseline commit");

    // Player edits both: their memory file AND their unfinished code.
    std::fs::write(worktree.path().join("AGENTS.md"), "agent v2").unwrap();
    std::fs::write(worktree.path().join("solution.rs"), "fn main() { todo!() }").unwrap();
    std::fs::write(worktree.path().join("README.md"), "readme v1").unwrap();

    assert!(
        snapshot.commit_memory_sources().expect("commit succeeds"),
        "changed memory files must produce a commit"
    );

    let repo = gix::open(snapshot.git_dir()).expect("open repo");
    let tree_id = repo.head_commit().unwrap().tree_id().unwrap().detach();
    let entry = |name: &str| find_entry_in_tree(&repo, tree_id, name);

    assert_eq!(
        read_blob(&repo, entry("AGENTS.md").expect("AGENTS.md present").0),
        b"agent v2",
        "the edited memory file is published"
    );
    assert_eq!(
        read_blob(&repo, entry("README.md").expect("README.md present").0),
        b"readme v1",
        "a newly created memory file is published"
    );
    assert_eq!(
        read_blob(&repo, entry("solution.rs").expect("solution.rs present").0),
        b"fn main() {}",
        "in-progress work stays at its previous committed state"
    );
}

#[test]
fn test_commit_memory_sources_is_a_no_op_when_unchanged() {
    let _g = HOME_LOCK.lock().unwrap();
    let home = tempfile::tempdir().expect("home tempdir");
    let worktree = tempfile::tempdir().expect("worktree tempdir");
    let _h = HomeGuard::set(home.path().to_str().unwrap());

    let snapshot =
        SnapshotRepo::new("default", "MEM2", worktree.path(), None, None).expect("new succeeds");
    std::fs::write(worktree.path().join("AGENTS.md"), "stable").unwrap();
    snapshot.commit_session_start().expect("baseline commit");
    let before = snapshot.head_commit_message();

    // Probes fire every few seconds; an unchanged file must not add a commit
    // each time.
    assert!(
        !snapshot.commit_memory_sources().expect("no-op succeeds"),
        "identical content reports no change"
    );
    assert_eq!(
        snapshot.head_commit_message(),
        before,
        "HEAD is untouched when nothing changed"
    );

    // Editing other files is likewise not a memory change.
    std::fs::write(worktree.path().join("solution.rs"), "changed").unwrap();
    assert!(
        !snapshot.commit_memory_sources().expect("no-op succeeds"),
        "non-memory edits do not trigger a memory commit"
    );
}

#[test]
fn test_commit_memory_sources_records_a_deletion() {
    let _g = HOME_LOCK.lock().unwrap();
    let home = tempfile::tempdir().expect("home tempdir");
    let worktree = tempfile::tempdir().expect("worktree tempdir");
    let _h = HomeGuard::set(home.path().to_str().unwrap());

    let snapshot =
        SnapshotRepo::new("default", "MEM3", worktree.path(), None, None).expect("new succeeds");
    std::fs::write(worktree.path().join("AGENTS.md"), "will go").unwrap();
    std::fs::write(worktree.path().join("keep.rs"), "kept").unwrap();
    snapshot.commit_session_start().expect("baseline commit");

    // Deleting AGENTS.md must retract it, or the server keeps extracting
    // memory from a file the player removed.
    std::fs::remove_file(worktree.path().join("AGENTS.md")).unwrap();
    assert!(
        snapshot.commit_memory_sources().expect("commit succeeds"),
        "a deletion is a change"
    );

    let repo = gix::open(snapshot.git_dir()).expect("open repo");
    let tree_id = repo.head_commit().unwrap().tree_id().unwrap().detach();
    assert!(
        find_entry_in_tree(&repo, tree_id, "AGENTS.md").is_none(),
        "AGENTS.md removed"
    );
    assert!(
        find_entry_in_tree(&repo, tree_id, "keep.rs").is_some(),
        "other files kept"
    );
}

#[test]
fn test_commit_memory_sources_works_without_a_baseline_commit() {
    let _g = HOME_LOCK.lock().unwrap();
    let home = tempfile::tempdir().expect("home tempdir");
    let worktree = tempfile::tempdir().expect("worktree tempdir");
    let _h = HomeGuard::set(home.path().to_str().unwrap());

    let snapshot =
        SnapshotRepo::new("default", "MEM4", worktree.path(), None, None).expect("new succeeds");
    std::fs::write(worktree.path().join("README.md"), "first").unwrap();
    std::fs::write(worktree.path().join("other.rs"), "not committed").unwrap();

    assert!(
        snapshot.commit_memory_sources().expect("commit succeeds"),
        "first memory commit lands even with no parent"
    );
    let repo = gix::open(snapshot.git_dir()).expect("open repo");
    let tree_id = repo.head_commit().unwrap().tree_id().unwrap().detach();
    assert!(find_entry_in_tree(&repo, tree_id, "README.md").is_some());
    assert!(
        find_entry_in_tree(&repo, tree_id, "other.rs").is_none(),
        "still only the memory files"
    );
}

/// `run_capped` is what stands between a hung `git push` and a frozen
/// session loop: a child that outlives the cap is killed and reported as
/// `None`; a quick one returns its full output.
#[test]
fn run_capped_kills_at_the_deadline_and_passes_quick_commands() {
    let started = std::time::Instant::now();
    let mut slow = std::process::Command::new("sleep");
    slow.arg("30");
    let out = super::run_capped(slow, std::time::Duration::from_millis(300)).expect("spawn sleep");
    assert!(out.is_none(), "a hung child is killed, not awaited");
    assert!(
        started.elapsed() < std::time::Duration::from_secs(5),
        "the kill happens at the cap, not at the child's leisure"
    );

    let mut quick = std::process::Command::new("sh");
    quick.arg("-c").arg("echo out; echo err >&2");
    let out = super::run_capped(quick, std::time::Duration::from_secs(10))
        .expect("spawn sh")
        .expect("finished in time");
    assert!(out.status.success());
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "out");
    assert_eq!(String::from_utf8_lossy(&out.stderr).trim(), "err");
}

/// Like [`find_entry_in_tree`] but walks a `/`-separated path through
/// nested trees.
fn find_path_in_tree(
    repo: &gix::Repository,
    tree_id: gix::ObjectId,
    path: &str,
) -> Option<gix::ObjectId> {
    let mut current = tree_id;
    let mut parts = path.split('/').peekable();
    while let Some(part) = parts.next() {
        let (oid, _) = find_entry_in_tree(repo, current, part)?;
        if parts.peek().is_none() {
            return Some(oid);
        }
        current = oid;
    }
    None
}

/// The snapshot is version control AND the artifact channel: player files
/// (`run.log` included), `.ololo/artifacts/**` and dotfiles all ship — but
/// regenerable dependency stores and build output must not. A single
/// `node_modules` or Rust `target/` is big enough to blow the push cap and
/// cost the judges the very snapshot they were meant to read.
#[test]
fn stage_all_prunes_dependency_and_build_dirs_but_keeps_artifacts() {
    let _g = HOME_LOCK.lock().unwrap();
    let home = tempfile::tempdir().expect("home tempdir");
    let worktree = tempfile::tempdir().expect("worktree tempdir");
    let _h = HomeGuard::set(home.path().to_str().unwrap());

    let snapshot =
        SnapshotRepo::new("default", "PRUNE1", worktree.path(), None, None).expect("new succeeds");

    let w = worktree.path();
    // Wanted: sources, logs, artifacts, manifests.
    std::fs::create_dir_all(w.join("src")).unwrap();
    std::fs::write(w.join("src/main.js"), "console.log(1)").unwrap();
    std::fs::write(w.join("package.json"), "{}").unwrap();
    std::fs::write(w.join("run.log"), "session narration").unwrap();
    std::fs::create_dir_all(w.join(".ololo/artifacts/req-1")).unwrap();
    std::fs::write(w.join(".ololo/artifacts/req-1/shot.png"), "png").unwrap();
    // Unwanted: dependency stores, build output, caches, Finder junk.
    for dir in [
        "node_modules/lodash",
        "target/debug",
        "build",
        "dist/assets",
        "__pycache__",
        ".venv/lib",
        "coverage",
    ] {
        std::fs::create_dir_all(w.join(dir)).unwrap();
        std::fs::write(w.join(dir).join("payload.bin"), "junk").unwrap();
    }
    std::fs::write(w.join(".DS_Store"), "finder").unwrap();
    std::fs::write(w.join("src/.DS_Store"), "finder").unwrap();

    let tree_id = snapshot.stage_all().expect("stage_all succeeds");
    let repo = &snapshot.repo;

    for kept in [
        "src/main.js",
        "package.json",
        "run.log",
        ".ololo/artifacts/req-1/shot.png",
    ] {
        assert!(
            find_path_in_tree(repo, tree_id, kept).is_some(),
            "{kept} must be in the snapshot"
        );
    }
    for pruned in [
        "node_modules/lodash/payload.bin",
        "target/debug/payload.bin",
        "build/payload.bin",
        "dist/assets/payload.bin",
        "__pycache__/payload.bin",
        ".venv/lib/payload.bin",
        "coverage/payload.bin",
        ".DS_Store",
        "src/.DS_Store",
    ] {
        assert!(
            find_path_in_tree(repo, tree_id, pruned).is_none(),
            "{pruned} must NOT be in the snapshot"
        );
    }
}
