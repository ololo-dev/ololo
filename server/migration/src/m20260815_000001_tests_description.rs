use sea_orm_migration::prelude::*;

/// `tests.description` — the probe section's prose from the task markdown
/// (the author's explanation of what the check verifies). The player chat
/// shows it under the probe's title so a check bubble explains itself.
/// Nullable: legacy rows and judge-registered probes have none.
#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(Tests::Table)
                    .add_column(ColumnDef::new(Tests::Description).text().null())
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(Tests::Table)
                    .drop_column(Tests::Description)
                    .to_owned(),
            )
            .await
    }
}

#[derive(DeriveIden)]
enum Tests {
    Table,
    Description,
}
