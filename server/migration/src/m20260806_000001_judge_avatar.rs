//! Adds `judges.avatar_url`: an optional face for the judge, set by an
//! admin in settings. NULL keeps today's initial-letter fallback.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(Judges::Table)
                    .add_column(ColumnDef::new(Judges::AvatarUrl).text().null())
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(Judges::Table)
                    .drop_column(Judges::AvatarUrl)
                    .to_owned(),
            )
            .await
    }
}

#[derive(DeriveIden)]
enum Judges {
    Table,
    AvatarUrl,
}
