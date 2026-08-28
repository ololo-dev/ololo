//! Tool + resolve_task_commit tests for the judging module.

use crate::common;
use crate::common::*;
use arena_core::judging::tools::ToolScope;

use arena_core::judging::resolve_task_commit;
use arena_core::judging::tools;
use uuid::Uuid;

#[tokio::test]
async fn tool_list_files_returns_entries() {
    let dir = tempfile::tempdir().unwrap();
    make_repo(dir.path());
    write_file(dir.path(), "a.txt", "hello");
    write_file(dir.path(), "b.txt", "world!");
    commit(dir.path(), "init");

    let files = tools::list_files(dir.path(), None, None, &ToolScope::everything())
        .await
        .expect("list_files");
    assert_eq!(files.len(), 2);
    let paths: Vec<&str> = files.iter().map(|f| f.path.as_str()).collect();
    assert!(paths.contains(&"a.txt"));
    assert!(paths.contains(&"b.txt"));
    let a = files.iter().find(|f| f.path == "a.txt").unwrap();
    assert_eq!(a.size_bytes, 5);
}

#[tokio::test]
async fn tool_read_file_returns_content() {
    let dir = tempfile::tempdir().unwrap();
    make_repo(dir.path());
    write_file(dir.path(), "hello.txt", "hello world");
    commit(dir.path(), "init");

    let content = tools::read_file(
        dir.path(),
        "hello.txt",
        None,
        None,
        &ToolScope::everything(),
    )
    .await
    .expect("read_file");
    assert_eq!(content, "hello world");
}

#[tokio::test]
async fn tool_read_file_truncates_large_file() {
    let dir = tempfile::tempdir().unwrap();
    make_repo(dir.path());
    let big = "x".repeat(40_000);
    write_file(dir.path(), "big.txt", &big);
    commit(dir.path(), "init");

    let content = tools::read_file(dir.path(), "big.txt", None, None, &ToolScope::everything())
        .await
        .expect("read_file");
    assert!(content.contains("[truncated]"));
    assert!(content.len() < big.len());
}

#[tokio::test]
async fn tool_read_file_missing_returns_error_string() {
    let dir = tempfile::tempdir().unwrap();
    make_repo(dir.path());
    write_file(dir.path(), "a.txt", "x");
    commit(dir.path(), "init");

    let content = tools::read_file(dir.path(), "nope.txt", None, None, &ToolScope::everything())
        .await
        .expect("read_file returns error string, not Err");
    assert!(content.starts_with("error:"));
}

#[tokio::test]
async fn tool_get_diff_returns_diff() {
    let dir = tempfile::tempdir().unwrap();
    make_repo(dir.path());
    write_file(dir.path(), "a.txt", "v1");
    commit(dir.path(), "c1");
    write_file(dir.path(), "a.txt", "v2");
    commit(dir.path(), "c2");

    let diff = tools::get_diff(dir.path(), None, None, None, &ToolScope::everything())
        .await
        .expect("get_diff");
    assert!(diff.contains("v1") || diff.contains("v2"));
}

#[tokio::test]
async fn tool_get_last_commit_diff_returns_diff() {
    let dir = tempfile::tempdir().unwrap();
    make_repo(dir.path());
    write_file(dir.path(), "a.txt", "v1");
    commit(dir.path(), "c1");
    write_file(dir.path(), "a.txt", "v2");
    commit(dir.path(), "c2");

    let diff = tools::get_last_commit_diff(dir.path(), None, None, &ToolScope::everything())
        .await
        .expect("get_last_commit_diff");
    assert!(diff.contains("a.txt"));
}

#[tokio::test]
async fn tool_find_task_commit_finds_matching() {
    let dir = tempfile::tempdir().unwrap();
    make_repo(dir.path());
    let task_id = Uuid::new_v4();
    write_file(dir.path(), "a.txt", "x");
    let sha = commit(dir.path(), &format!("feat({task_id}): reverse string"));

    let entries = tools::find_task_commit(dir.path(), &task_id.to_string())
        .await
        .expect("find_task_commit");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].sha, sha);
    assert!(entries[0].subject.contains(&task_id.to_string()));
}

#[tokio::test]
async fn tool_find_task_commit_no_match_returns_empty() {
    let dir = tempfile::tempdir().unwrap();
    make_repo(dir.path());
    write_file(dir.path(), "a.txt", "x");
    commit(dir.path(), "unrelated");

    let entries = tools::find_task_commit(dir.path(), "no-such-uuid")
        .await
        .expect("find_task_commit");
    assert!(entries.is_empty());
}

#[tokio::test]
async fn tool_get_commit_log_returns_entries() {
    let dir = tempfile::tempdir().unwrap();
    make_repo(dir.path());
    write_file(dir.path(), "a.txt", "1");
    commit(dir.path(), "c1");
    write_file(dir.path(), "a.txt", "2");
    commit(dir.path(), "c2");

    let entries = tools::get_commit_log(dir.path(), None, Some(5), None)
        .await
        .expect("get_commit_log");
    assert_eq!(entries.len(), 2);
}

#[tokio::test]
async fn tool_list_files_empty_repo_returns_empty() {
    let dir = tempfile::tempdir().unwrap();
    make_repo(dir.path());
    // no commits
    let files = tools::list_files(dir.path(), None, None, &ToolScope::everything())
        .await
        .expect("list_files");
    assert!(files.is_empty());
}

#[tokio::test]
async fn tool_list_files_missing_repo_returns_empty() {
    let dir = tempfile::tempdir().unwrap();
    // not a git repo
    let files = tools::list_files(dir.path(), None, None, &ToolScope::everything())
        .await
        .expect("list_files");
    assert!(files.is_empty());
}

#[tokio::test]
async fn resolve_task_commit_finds_commit() {
    let dir = tempfile::tempdir().unwrap();
    make_repo(dir.path());
    let task_id = Uuid::new_v4();
    write_file(dir.path(), "a.txt", "x");
    let sha = commit(dir.path(), &format!("feat({task_id}): reverse"));

    let res = resolve_task_commit(dir.path(), task_id)
        .await
        .expect("resolve_task_commit");
    let (found_sha, subject) = res.expect("some");
    assert_eq!(found_sha, sha);
    assert!(subject.contains(&task_id.to_string()));
}

#[tokio::test]
async fn resolve_task_commit_ignores_auxiliary_commits_carrying_the_id() {
    // wip()/artifact()/flag() commits carry the task id too, but they are
    // mid-task trees — the resolver must return the feat() snapshot even
    // when an auxiliary commit is more recent.
    let dir = tempfile::tempdir().unwrap();
    make_repo(dir.path());
    let task_id = Uuid::new_v4();
    write_file(dir.path(), "a.txt", "x");
    let feat_sha = commit(dir.path(), &format!("feat({task_id}): reverse"));
    write_file(dir.path(), "b.txt", "y");
    commit(dir.path(), &format!("artifact({task_id}): sync"));
    write_file(dir.path(), "c.txt", "z");
    commit(dir.path(), &format!("wip({task_id}): checkpoint"));

    let res = resolve_task_commit(dir.path(), task_id)
        .await
        .expect("resolve_task_commit");
    let (found_sha, _) = res.expect("some");
    assert_eq!(found_sha, feat_sha);
}

#[tokio::test]
async fn resolve_task_commit_no_match_returns_none() {
    let dir = tempfile::tempdir().unwrap();
    make_repo(dir.path());
    write_file(dir.path(), "a.txt", "x");
    commit(dir.path(), "unrelated");

    let res = resolve_task_commit(dir.path(), Uuid::new_v4())
        .await
        .expect("resolve_task_commit");
    assert!(res.is_none());
}

// ── Per-judge blind spots ────────────────────────────────────────────────
//
// `.ololo/` is the platform's own tree inside the player's snapshot: probe
// scratch, completion flags, delivered artifacts. The UX review lives off it;
// a judge reading only the player's code must not pay for it.

fn ololo_blind() -> ToolScope {
    ToolScope::from_json(Some(r#"[".ololo/"]"#))
}

fn seed_repo_with_runtime_tree(dir: &std::path::Path) {
    make_repo(dir);
    // `write_file` does not create parent directories.
    std::fs::create_dir_all(dir.join(".ololo/tmp/pg-1")).expect("mkdir");
    std::fs::create_dir_all(dir.join(".ololo/tmp/pg-2")).expect("mkdir");
    std::fs::create_dir_all(dir.join("src")).expect("mkdir");
    write_file(dir, "serve.sh", "cargo run\n");
    write_file(dir, ".ololo/tmp/pg-1/wal.log", "scratch");
    write_file(dir, ".ololo/server-done.md", "done");
    write_file(dir, ".ololorc", "not the platform's file");
    commit(dir, "init");
}

#[tokio::test]
async fn a_blind_spot_keeps_the_platform_tree_out_of_the_listing() {
    let dir = tempfile::tempdir().unwrap();
    seed_repo_with_runtime_tree(dir.path());

    let files = tools::list_files(dir.path(), None, None, &ololo_blind())
        .await
        .expect("list_files");
    let paths: Vec<&str> = files.iter().map(|f| f.path.as_str()).collect();
    assert!(paths.contains(&"serve.sh"), "{paths:?}");
    // A file that merely starts with the same letters is not the tree.
    assert!(paths.contains(&".ololorc"), "{paths:?}");
    assert!(
        !paths.iter().any(|p| p.starts_with(".ololo/")),
        "the runtime tree is hidden: {paths:?}"
    );

    // The same repo, a judge that declared nothing: everything is there.
    let all = tools::list_files(dir.path(), None, None, &ToolScope::everything())
        .await
        .expect("list_files");
    assert_eq!(all.len(), 4, "{all:?}");
}

#[tokio::test]
async fn reading_inside_a_blind_spot_is_refused_in_words() {
    // Not an empty string and not an empty repo: the judge must be able to
    // tell a rule from missing evidence, because its prompt says to score 0
    // when it cannot read something rather than guess.
    let dir = tempfile::tempdir().unwrap();
    seed_repo_with_runtime_tree(dir.path());

    let out = tools::read_file(
        dir.path(),
        ".ololo/tmp/pg-1/wal.log",
        None,
        None,
        &ololo_blind(),
    )
    .await
    .expect("read_file");
    assert!(out.starts_with("error:"), "{out}");
    assert!(out.contains("outside this judge's scope"), "{out}");
    assert!(!out.contains("scratch"), "the content never leaks: {out}");

    // The player's own file is untouched.
    let ok = tools::read_file(dir.path(), "serve.sh", None, None, &ololo_blind())
        .await
        .expect("read_file");
    assert_eq!(ok, "cargo run\n");
}

#[tokio::test]
async fn a_diff_skips_the_blind_spot_but_keeps_the_rest() {
    let dir = tempfile::tempdir().unwrap();
    seed_repo_with_runtime_tree(dir.path());
    write_file(dir.path(), "src/main.rs", "fn main() {}\n");
    write_file(dir.path(), ".ololo/tmp/pg-2/wal.log", "more scratch");
    commit(dir.path(), "work");

    let diff = tools::get_last_commit_diff(dir.path(), None, None, &ololo_blind())
        .await
        .expect("diff");
    assert!(diff.contains("src/main.rs"), "{diff}");
    assert!(!diff.contains(".ololo/"), "{diff}");

    let full = tools::get_last_commit_diff(dir.path(), None, None, &ToolScope::everything())
        .await
        .expect("diff");
    assert!(full.contains(".ololo/tmp/pg-2/wal.log"), "{full}");
}

#[test]
fn an_undeclared_or_unreadable_blind_spot_hides_nothing() {
    // A judge that cannot say what to skip still gets to do its job.
    assert!(ToolScope::from_json(None).is_empty());
    assert!(ToolScope::from_json(Some("not json")).is_empty());
    assert!(ToolScope::from_json(Some("[]")).is_empty());
    assert!(!ToolScope::from_json(Some(r#"[".ololo"]"#)).is_empty());
    // With or without the trailing slash, the whole directory goes.
    assert!(ToolScope::from_json(Some(r#"[".ololo"]"#)).hides(".ololo/tmp/x"));
    assert!(ToolScope::from_json(Some(r#"[".ololo/"]"#)).hides(".ololo/tmp/x"));
    assert!(!ToolScope::from_json(Some(r#"[".ololo/"]"#)).hides(".ololorc"));
}
