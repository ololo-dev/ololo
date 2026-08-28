use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(LlmProviders::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(LlmProviders::Id)
                            .uuid()
                            .not_null()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(LlmProviders::Name)
                            .string()
                            .not_null()
                            .unique_key(),
                    )
                    .col(ColumnDef::new(LlmProviders::Kind).string().not_null())
                    .col(ColumnDef::new(LlmProviders::BaseUrl).text().null())
                    .col(ColumnDef::new(LlmProviders::ApiKeyEnc).text().null())
                    .col(
                        ColumnDef::new(LlmProviders::Enabled)
                            .boolean()
                            .not_null()
                            .default(true),
                    )
                    .col(ColumnDef::new(LlmProviders::CatalogId).string().null())
                    .col(
                        ColumnDef::new(LlmProviders::CreatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(LlmProviders::UpdatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .to_owned(),
            )
            .await?;

        // Per-judge model override. One alter_table call per column (SQLite).
        manager
            .alter_table(
                Table::alter()
                    .table(Judges::Table)
                    .add_column(ColumnDef::new(Judges::LlmProviderIdFk).uuid().null())
                    .to_owned(),
            )
            .await?;
        manager
            .alter_table(
                Table::alter()
                    .table(Judges::Table)
                    .add_column(ColumnDef::new(Judges::LlmModel).string().null())
                    .to_owned(),
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(Judges::Table)
                    .drop_column(Judges::LlmModel)
                    .to_owned(),
            )
            .await?;
        manager
            .alter_table(
                Table::alter()
                    .table(Judges::Table)
                    .drop_column(Judges::LlmProviderIdFk)
                    .to_owned(),
            )
            .await?;
        manager
            .drop_table(
                Table::drop()
                    .table(LlmProviders::Table)
                    .if_exists()
                    .to_owned(),
            )
            .await?;
        Ok(())
    }
}

#[derive(DeriveIden)]
enum LlmProviders {
    Table,
    Id,
    Name,
    Kind,
    BaseUrl,
    ApiKeyEnc,
    Enabled,
    CatalogId,
    CreatedAt,
    UpdatedAt,
}

#[derive(DeriveIden)]
enum Judges {
    Table,
    LlmProviderIdFk,
    LlmModel,
}
