use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(Probes::Table)
                    .add_column(ColumnDef::new(Probes::ResolvedAnswer).text().null())
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(Probes::Table)
                    .drop_column(Probes::ResolvedAnswer)
                    .to_owned(),
            )
            .await
    }
}

#[derive(DeriveIden)]
enum Probes {
    Table,
    ResolvedAnswer,
}
