//! Round-trip migration test: applies all migrations forward, rolls them
//! all back, then re-applies forward against an in-memory SQLite database.
//!
//! The Postgres path runs only when `ARENA_TEST_PG_URL` points at a disposable
//! Postgres database (the test rolls every migration back, wiping the schema).
use migration::{Migrator, MigratorTrait};
use sea_orm::Database;

#[tokio::test]
async fn migrator_up_down_up_roundtrip_sqlite() {
    let db = Database::connect("sqlite::memory:")
        .await
        .expect("connect in-memory sqlite");

    Migrator::up(&db, None).await.expect("first up");
    Migrator::down(&db, None).await.expect("down");
    Migrator::up(&db, None).await.expect("second up");
}

#[tokio::test]
async fn migrator_up_down_up_roundtrip_postgres() {
    let Ok(url) = std::env::var("ARENA_TEST_PG_URL") else {
        eprintln!("ARENA_TEST_PG_URL not set; skipping postgres roundtrip");
        return;
    };
    let db = Database::connect(&url).await.expect("connect postgres");

    Migrator::up(&db, None).await.expect("first up");
    Migrator::down(&db, None).await.expect("down");
    Migrator::up(&db, None).await.expect("second up");
}
