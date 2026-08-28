//! Fresh file-backed SQLite must migrate end to end over the DEFAULT pool.
//!
//! `sqlite::memory:` (what every other test uses) forces one connection per
//! pool, so it can never catch a migration whose statements straddle two
//! pooled connections — which is exactly how the sessions-rebuild migration
//! broke every fresh `dev-play.sh` boot while 1900+ tests stayed green.
//! This test runs the real `connect_and_migrate` against a temp file with
//! the production pool settings.

#[tokio::test]
async fn fresh_file_backed_sqlite_migrates_with_the_default_pool() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("fresh.db");
    let url = format!("sqlite://{}?mode=rwc", path.display());
    let db = migration::connect_and_migrate(&url)
        .await
        .expect("all migrations apply on a fresh file-backed sqlite");
    // The last table this suite grew — proves the run reached the tail.
    use sea_orm::ConnectionTrait;
    let row = db
        .query_one(sea_orm::Statement::from_string(
            db.get_database_backend(),
            "SELECT show_tasks FROM projects LIMIT 1".to_owned(),
        ))
        .await
        .expect("projects.show_tasks column exists");
    let _ = row; // no rows on a fresh db — the query compiling is the assertion
}
