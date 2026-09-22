//! `task_results.kind` — what a row is: a probe outcome, the completion
//! bonus, the similarity penalty, or (new) a health bonus.
//!
//! The partial unique index guarding "one completion bonus per task"
//! (`ux_task_results_completion_bonus`, keyed on session/player/task
//! `WHERE is_bonus`) left no room for a second bonus row on the same task.
//! The kind joins the key, so each bonus kind is unique per task on its own
//! and the health bonus can live in `task_results` like every other score
//! change — one `SUM(point_delta)` still totals a player.
//!
//! Existing rows are classified from what the writers always wrote:
//! `is_bonus` with a task is the completion bonus, `is_bonus` without one
//! and a `similarity-penalty…` answer is the copy/paste penalty, the rest
//! are probe outcomes.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(TaskResults::Table)
                    .add_column(
                        ColumnDef::new(TaskResults::Kind)
                            .string()
                            .not_null()
                            .default("probe"),
                    )
                    .to_owned(),
            )
            .await?;
        let db = manager.get_connection();
        db.execute_unprepared(
            "UPDATE task_results SET kind = 'completion_bonus' \
             WHERE is_bonus AND task_id IS NOT NULL",
        )
        .await?;
        db.execute_unprepared(
            "UPDATE task_results SET kind = 'similarity_penalty' \
             WHERE is_bonus AND task_id IS NULL AND answer LIKE 'similarity-penalty%'",
        )
        .await?;
        db.execute_unprepared("DROP INDEX ux_task_results_completion_bonus")
            .await?;
        // `WHERE is_bonus` (bare boolean predicate) is portable across SQLite
        // and Postgres — same shape as the index it replaces.
        db.execute_unprepared(
            "CREATE UNIQUE INDEX ux_task_results_bonus_kind \
             ON task_results(session_id_fk, player_id_fk, task_id, kind) WHERE is_bonus",
        )
        .await?;
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        db.execute_unprepared("DROP INDEX ux_task_results_bonus_kind")
            .await?;
        // Health bonus rows would collide with the completion bonus under the
        // old index; they belong to the feature being rolled back.
        db.execute_unprepared("DELETE FROM task_results WHERE kind = 'health_bonus'")
            .await?;
        db.execute_unprepared(
            "CREATE UNIQUE INDEX ux_task_results_completion_bonus \
             ON task_results(session_id_fk, player_id_fk, task_id) WHERE is_bonus",
        )
        .await?;
        manager
            .alter_table(
                Table::alter()
                    .table(TaskResults::Table)
                    .drop_column(TaskResults::Kind)
                    .to_owned(),
            )
            .await
    }
}

#[derive(DeriveIden)]
enum TaskResults {
    Table,
    Kind,
}
