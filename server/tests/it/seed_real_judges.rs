//! Seeds the repository's real `judges/` directory.
//!
//! The unit tests for the seeder use synthetic fixtures, so a typo in an actual
//! judge file — a misspelled `scope:`, an unknown value, a scope/kind
//! combination the seeder rejects — would be skipped at boot with only a log
//! line to show for it. That failure is invisible until a session runs and the
//! judge silently never fires. This asserts the shipped files load as intended.

use arena_core::entities::judges;
use migration::{Migrator, MigratorTrait};
use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter};

async fn setup_db() -> DatabaseConnection {
    let db = sea_orm::Database::connect("sqlite::memory:")
        .await
        .expect("sqlite connect");
    Migrator::up(&db, None).await.expect("migrate up");
    db
}

/// `judges/` at the workspace root, from this crate's manifest dir.
fn judges_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root")
        .join("judges")
}

#[tokio::test]
async fn shipped_judge_files_all_seed_successfully() {
    let db = setup_db().await;
    let dir = judges_dir();
    let on_disk = std::fs::read_dir(&dir)
        .expect("judges dir readable")
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|x| x == "md"))
        .count();
    assert!(on_disk > 0, "no judge markdown found in {}", dir.display());

    server::seed::judges::seed_judges_with_dir(&db, &dir)
        .await
        .expect("seed judges");

    let seeded = judges::Entity::find().all(&db).await.expect("query judges");
    assert_eq!(
        seeded.len(),
        on_disk,
        "every shipped judge file must seed; the seeder logs and skips bad ones. \
         Seeded slugs: {:?}",
        seeded.iter().map(|j| &j.slug).collect::<Vec<_>>()
    );
    for j in &seeded {
        assert!(
            j.scope == "task" || j.scope == "session",
            "judge '{}' seeded with an unexpected scope '{}'",
            j.slug,
            j.scope
        );
    }
}

#[tokio::test]
async fn task_anti_cheat_is_task_scoped() {
    // The session-scoped `anti-cheater` was retired 2026-08-14: it
    // double-punished the same replay offense task-anti-cheat already
    // penalizes per task (6LA7TX took -40 AND -400 for one replay). The
    // per-task judge carries the session-wide replay signature in its
    // decide program, so scope=task covers both.
    let db = setup_db().await;
    server::seed::judges::seed_judges_with_dir(&db, &judges_dir())
        .await
        .expect("seed judges");

    // This checkout may ship a subset of the judge library.
    let Some(anti_cheat) = judges::Entity::find()
        .filter(judges::Column::Slug.eq("task-anti-cheat"))
        .one(&db)
        .await
        .expect("query")
    else {
        return;
    };

    assert_eq!(
        anti_cheat.scope, "task",
        "the per-task judge is the one anti-cheat control left standing"
    );
    assert_eq!(anti_cheat.kind, "llm");
    assert!(
        judges::Entity::find()
            .filter(judges::Column::Slug.eq("anti-cheater"))
            .one(&db)
            .await
            .expect("query")
            .is_none(),
        "the retired session-scope judge must not reappear in judges/"
    );
}
