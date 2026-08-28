//! One completion bonus per (session, player, task) (DB-M1).
//!
//! `award_completion_bonus` did a SELECT-then-INSERT with no constraint, so two
//! concurrent handlers for the same player (possible on reconnect) could both
//! insert a bonus row. A partial unique index makes the second insert fail,
//! which the caller now treats as "already awarded". Existing duplicates are
//! collapsed first so the index can be created on live data.
//!
//! `WHERE is_bonus` (bare boolean predicate) is portable across SQLite and
//! Postgres — see the `WHERE public` note in the squash migration.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        // Collapse any pre-existing duplicate bonus rows, keeping the lexically
        // smallest id per (session, player, task).
        // Cast id to text before MIN(): Postgres has no `min(uuid)` aggregate,
        // and on SQLite the id is already text, so `CAST(id AS text)` is portable.
        db.execute_unprepared(
            "DELETE FROM task_results \
             WHERE is_bonus AND CAST(id AS text) NOT IN ( \
                 SELECT MIN(CAST(id AS text)) FROM task_results \
                 WHERE is_bonus \
                 GROUP BY session_id_fk, player_id_fk, task_id \
             )",
        )
        .await?;
        db.execute_unprepared(
            "CREATE UNIQUE INDEX ux_task_results_completion_bonus \
             ON task_results(session_id_fk, player_id_fk, task_id) WHERE is_bonus",
        )
        .await?;
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared("DROP INDEX ux_task_results_completion_bonus")
            .await?;
        Ok(())
    }
}
