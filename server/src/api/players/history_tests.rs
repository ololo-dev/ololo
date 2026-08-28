use super::*;

#[test]
fn read_history_empty_repo_returns_empty() {
    let tmp = tempfile::tempdir().expect("tmp");
    let repo = tmp.path().join("test.git");
    std::fs::create_dir_all(&repo).unwrap();
    let git_bin = which::which("git").unwrap();
    let out = std::process::Command::new(git_bin)
        .arg("init")
        .arg("--bare")
        .arg(&repo)
        .output()
        .unwrap();
    assert!(out.status.success(), "git init bare failed");
    let commits = read_history(&repo).expect("read_history on empty repo");
    assert!(commits.is_empty(), "empty bare repo has no commits");
}

#[test]
fn read_history_parses_commit() {
    let tmp = tempfile::tempdir().expect("tmp");
    let work = tmp.path().join("work");
    let repo = tmp.path().join("test.git");
    std::fs::create_dir_all(&work).unwrap();
    let git_bin = which::which("git").unwrap();

    // Init a real worktree repo, commit, then clone --bare.
    let init = std::process::Command::new(&git_bin)
        .arg("init")
        .arg(&work)
        .output()
        .unwrap();
    assert!(init.status.success());
    // set author
    std::process::Command::new(&git_bin)
        .arg("-C")
        .arg(&work)
        .args(["config", "user.name", "tester"])
        .status()
        .unwrap();
    std::process::Command::new(&git_bin)
        .arg("-C")
        .arg(&work)
        .args(["config", "user.email", "t@t"])
        .status()
        .unwrap();
    std::fs::write(work.join("hello.txt"), "hello\n").unwrap();
    std::process::Command::new(&git_bin)
        .arg("-C")
        .arg(&work)
        .args(["add", "."])
        .status()
        .unwrap();
    std::process::Command::new(&git_bin)
        .arg("-C")
        .arg(&work)
        .args(["commit", "-m", "feat(test-uuid): add hello"])
        .status()
        .unwrap();
    // clone bare
    let clone = std::process::Command::new(&git_bin)
        .arg("clone")
        .arg("--bare")
        .arg(&work)
        .arg(&repo)
        .output()
        .unwrap();
    assert!(clone.status.success(), "git clone --bare failed");

    let commits = read_history(&repo).expect("read_history");
    assert_eq!(commits.len(), 1, "one commit");
    let c = &commits[0];
    assert_eq!(c.author_name, "tester");
    assert!(c.message.starts_with("feat(test-uuid): add hello"));
    assert_eq!(c.files.len(), 1, "one file changed");
    assert_eq!(c.files[0].path, "hello.txt");
    assert_eq!(c.files[0].status, "added");
    assert!(c.files[0].patch.contains("+hello"), "patch has add line");
}
