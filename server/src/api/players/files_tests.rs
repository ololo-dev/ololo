use super::*;

fn run_git(dir: &FsPath, args: &[&str]) {
    let git_bin = which::which("git").unwrap();
    let out = std::process::Command::new(git_bin)
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .unwrap();
    assert!(out.status.success(), "git {args:?}: {out:?}");
}

/// A bare repo whose HEAD holds a text file, a nested file and a binary.
fn bare_repo_with_files(tmp: &FsPath) -> PathBuf {
    let work = tmp.join("work");
    let repo = tmp.join("test.git");
    std::fs::create_dir_all(work.join("src")).unwrap();
    run_git(&work, &["init"]);
    run_git(&work, &["config", "user.name", "tester"]);
    run_git(&work, &["config", "user.email", "t@t"]);
    std::fs::write(work.join("README.md"), "# hi\n").unwrap();
    std::fs::write(work.join("src/main.rs"), "fn main() {}\n").unwrap();
    std::fs::write(work.join("logo.bin"), [0u8, 1, 2, 3]).unwrap();
    run_git(&work, &["add", "."]);
    run_git(&work, &["commit", "-m", "first files"]);
    run_git(
        tmp,
        &[
            "clone",
            "--bare",
            work.to_str().unwrap(),
            repo.to_str().unwrap(),
        ],
    );
    repo
}

#[test]
fn an_empty_repo_has_no_head_tree() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("test.git");
    std::fs::create_dir_all(&repo).unwrap();
    run_git(&repo, &["init", "--bare"]);
    assert!(read_head_tree(&repo).unwrap().is_none());
}

#[test]
fn the_head_tree_lists_every_file_with_its_size() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = bare_repo_with_files(tmp.path());
    let listing = read_head_tree(&repo).unwrap().expect("HEAD exists");
    let head = listing.head.expect("head");
    assert!(is_object_id(&head.sha), "sha: {}", head.sha);
    assert_eq!(head.subject, "first files");
    let files: Vec<(&str, u64)> = listing
        .files
        .iter()
        .map(|f| (f.path.as_str(), f.size))
        .collect();
    assert_eq!(
        files,
        vec![("README.md", 5), ("logo.bin", 4), ("src/main.rs", 13)]
    );
    assert!(!listing.truncated);
}

#[test]
fn a_file_reads_as_text_at_head_and_at_its_commit() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = bare_repo_with_files(tmp.path());
    let file = read_file(&repo, "HEAD", "src/main.rs")
        .unwrap()
        .expect("file");
    assert_eq!(file.content.as_deref(), Some("fn main() {}\n"));
    assert!(!file.binary && !file.too_large);

    let sha = read_head_tree(&repo).unwrap().unwrap().head.unwrap().sha;
    let pinned = read_file(&repo, &sha, "README.md").unwrap().expect("file");
    assert_eq!(pinned.content.as_deref(), Some("# hi\n"));
}

#[test]
fn a_binary_file_is_flagged_not_sent() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = bare_repo_with_files(tmp.path());
    let file = read_file(&repo, "HEAD", "logo.bin").unwrap().expect("file");
    assert!(file.binary);
    assert_eq!(file.content, None);
    assert_eq!(file.size, 4);
}

#[test]
fn only_an_exact_file_path_resolves() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = bare_repo_with_files(tmp.path());
    for path in [
        "src",
        "src/",
        "missing.txt",
        "../README.md",
        "src/../README.md",
        "-h",
    ] {
        assert!(
            read_file(&repo, "HEAD", path).unwrap().is_none(),
            "{path} must not resolve"
        );
    }
}

#[test]
fn only_bare_object_ids_pass_as_sha() {
    assert!(is_object_id(&"a".repeat(40)));
    assert!(is_object_id(&"0".repeat(64)));
    for bad in [
        "HEAD",
        "HEAD~1",
        &"a".repeat(39),
        &format!("{}~", "a".repeat(39)),
        "--all",
    ] {
        assert!(!is_object_id(bad), "{bad}");
    }
}
