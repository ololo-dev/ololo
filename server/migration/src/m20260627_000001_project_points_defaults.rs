use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // One ALTER per column — SQLite ≤3.35 chokes on multi-column ALTERs,
        // and the constitution insight pins single-column steps.
        manager
            .alter_table(
                Table::alter()
                    .table(Projects::Table)
                    .add_column(
                        ColumnDef::new(Projects::DefaultValuePoints)
                            .integer()
                            .not_null()
                            .default(10),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .alter_table(
                Table::alter()
                    .table(Projects::Table)
                    .add_column(
                        ColumnDef::new(Projects::DefaultFailPoints)
                            .integer()
                            .not_null()
                            .default(-5),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .alter_table(
                Table::alter()
                    .table(Projects::Table)
                    .add_column(
                        ColumnDef::new(Projects::DefaultNoResponsePoints)
                            .integer()
                            .not_null()
                            .default(-10),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .alter_table(
                Table::alter()
                    .table(Projects::Table)
                    .add_column(
                        ColumnDef::new(Projects::DefaultCompletionBonusPoints)
                            .integer()
                            .not_null()
                            .default(10),
                    )
                    .to_owned(),
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        for col in [
            Projects::DefaultValuePoints,
            Projects::DefaultFailPoints,
            Projects::DefaultNoResponsePoints,
            Projects::DefaultCompletionBonusPoints,
        ] {
            manager
                .alter_table(
                    Table::alter()
                        .table(Projects::Table)
                        .drop_column(col)
                        .to_owned(),
                )
                .await?;
        }
        Ok(())
    }
}

#[derive(DeriveIden)]
enum Projects {
    Table,
    DefaultValuePoints,
    DefaultFailPoints,
    DefaultNoResponsePoints,
    DefaultCompletionBonusPoints,
}
