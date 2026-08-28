use std::path::Path;
use uuid::Uuid;

use arena_core::git_store;

#[test]
fn player_repo_path_layout() {
    let base = Path::new("/tmp/arena-repos");
    let sid = Uuid::nil();
    let pid = Uuid::nil();
    let p = git_store::player_repo_path(base, sid, pid);
    assert!(p.starts_with("/tmp/arena-repos"));
    assert!(p.to_string_lossy().ends_with(".git"));
}

#[test]
fn repos_base_dir_env_behavior() {
    // Override path is used verbatim.
    unsafe {
        std::env::set_var("OLOLO_GIT_REPOS_DIR", "/custom/repos");
    }
    let base = git_store::repos_base_dir().expect("env set");
    assert_eq!(base, Path::new("/custom/repos"));

    // Empty override falls through to $HOME-based default.
    unsafe {
        std::env::set_var("OLOLO_GIT_REPOS_DIR", "");
        std::env::set_var("HOME", "/home/arena-test");
    }
    let base = git_store::repos_base_dir().expect("home set");
    assert!(base.ends_with(".local/share/ololo/repos"));
    assert!(base.starts_with("/home/arena-test"));
}
