use sea_orm_migration::prelude::*;

/// Judge-scope discriminator. Until now every judge ran once per (player,
/// task): `run_task_judges_after_commit` fires each attached judge when that
/// task's snapshot commit lands. `scope = "session"` marks judges that instead
/// run once per (player, session) and emit a verdict for every task the player
/// reached — the shape anti-cheat needs, because the signal lives in the
/// trajectory across tasks rather than in any single commit.
///
/// Existing rows default to `"task"`.
#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(Judges::Table)
                    .add_column(
                        ColumnDef::new(Judges::Scope)
                            .string()
                            .not_null()
                            .default("task"),
                    )
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(Judges::Table)
                    .drop_column(Judges::Scope)
                    .to_owned(),
            )
            .await
    }
}

#[derive(DeriveIden)]
enum Judges {
    Table,
    Scope,
}
