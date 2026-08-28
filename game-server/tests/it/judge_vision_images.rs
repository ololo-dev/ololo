//! The vision loader's top-up must attach only images this session added.
//!
//! A campaign part imports the previous part's whole workspace — stale
//! `.ololo/artifacts/` deliveries included — and the old top-up listed every
//! image at HEAD and called it "participant-delivered". A judge then scored
//! part 3 against part 1's screenshots (plum NJXDD5). The root snapshot is
//! the boundary: what was already in it is inherited, not delivered.

use sea_orm::DatabaseConnection;
use std::path::Path;
use std::process::Command;
use uuid::Uuid;

async fn setup_db() -> DatabaseConnection {
    let db = sea_orm::Database::connect("sqlite::memory:")
        .await
        .expect("sqlite connect");
    use migration::MigratorTrait;
    migration::Migrator::up(&db, None).await.expect("migrate");
    db
}

fn git(dir: &Path, args: &[&str]) {
    let out = Command::new(which::which("git").expect("git"))
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .expect("git run");
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// A one-pixel PNG, enough for the loader's byte checks.
const PNG: &[u8] = &[
    0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1F, 0x15, 0xC4,
    0x89, 0x00, 0x00, 0x00, 0x0A, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9C, 0x63, 0x00, 0x01, 0x00, 0x00,
    0x05, 0x00, 0x01, 0x0D, 0x0A, 0x2D, 0xB4, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE,
    0x42, 0x60, 0x82,
];

#[tokio::test]
async fn top_up_attaches_only_images_the_session_added() {
    let db = setup_db().await;
    let session_id = Uuid::new_v4();
    let player_id = Uuid::new_v4();
    let task_id = Uuid::new_v4();

    // Build the player's bare repo: an inherited screenshot in the root
    // snapshot (a previous part's delivery), and one committed in-session.
    let repos_root = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    git(work.path(), &["init", "-q"]);
    git(work.path(), &["config", "user.email", "t@t"]);
    git(work.path(), &["config", "user.name", "T"]);
    std::fs::create_dir_all(work.path().join(".ololo/artifacts/old-request")).unwrap();
    std::fs::write(
        work.path().join(".ololo/artifacts/old-request/desktop.png"),
        PNG,
    )
    .unwrap();
    git(work.path(), &["add", "-A"]);
    git(
        work.path(),
        &["commit", "-q", "-m", "ololo snapshot: session start"],
    );
    std::fs::write(work.path().join("fresh-shot.png"), PNG).unwrap();
    git(work.path(), &["add", "-A"]);
    git(
        work.path(),
        &["commit", "-q", "-m", "wip: screenshot during play"],
    );

    let bare = arena_core::git_store::player_repo_path(repos_root.path(), session_id, player_id);
    std::fs::create_dir_all(bare.parent().unwrap()).unwrap();
    git(
        work.path(),
        &["clone", "-q", "--bare", ".", bare.to_str().unwrap()],
    );

    unsafe { std::env::set_var("OLOLO_GIT_REPOS_DIR", repos_root.path()) };
    let images =
        game_server::judge_queue::load_artifact_images(&db, session_id, player_id, task_id).await;
    unsafe { std::env::remove_var("OLOLO_GIT_REPOS_DIR") };

    assert_eq!(
        images.len(),
        1,
        "one in-session image, no inherited ones: {:?}",
        images.iter().map(|i| i.label.clone()).collect::<Vec<_>>()
    );
    assert!(
        images[0].label.contains("fresh-shot.png"),
        "{}",
        images[0].label
    );
    assert!(
        !images.iter().any(|i| i.label.contains("old-request")),
        "an imported artifact from a previous part must never be attached as delivered"
    );
}
