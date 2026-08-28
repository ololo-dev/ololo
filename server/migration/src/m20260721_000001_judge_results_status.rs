use sea_orm_migration::prelude::*;

/// Judge run lifecycle visibility: `status` tracks the run state
/// ("running" | "scored" | "failed") and `error` records why a failed run
/// produced no verdict. Pre-existing rows are all successful verdicts, so
/// the column defaults to "scored".
#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(JudgeResults::Table)
                    .add_column(
                        ColumnDef::new(JudgeResults::Status)
                            .string()
                            .not_null()
                            .default("scored"),
                    )
                    .to_owned(),
            )
            .await?;
        manager
            .alter_table(
                Table::alter()
                    .table(JudgeResults::Table)
                    .add_column(ColumnDef::new(JudgeResults::Error).text().null())
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(JudgeResults::Table)
                    .drop_column(JudgeResults::Status)
                    .to_owned(),
            )
            .await?;
        manager
            .alter_table(
                Table::alter()
                    .table(JudgeResults::Table)
                    .drop_column(JudgeResults::Error)
                    .to_owned(),
            )
            .await
    }
}

#[derive(DeriveIden)]
enum JudgeResults {
    Table,
    Status,
    Error,
}
