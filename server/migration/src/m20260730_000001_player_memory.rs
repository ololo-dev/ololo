use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(Projects::Table)
                    .add_column(ColumnDef::new(Projects::MemorySchema).text().null())
                    .to_owned(),
            )
            .await?;

        manager
            .create_table(
                Table::create()
                    .table(PlayerMemory::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(PlayerMemory::Id)
                            .uuid()
                            .not_null()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(PlayerMemory::SessionIdFk).uuid().not_null())
                    .col(ColumnDef::new(PlayerMemory::PlayerIdFk).uuid().not_null())
                    .col(ColumnDef::new(PlayerMemory::ValuesJson).text().not_null())
                    .col(ColumnDef::new(PlayerMemory::SourceHash).text().null())
                    .col(
                        ColumnDef::new(PlayerMemory::CreatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(PlayerMemory::UpdatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_player_memory_session")
                            .from(PlayerMemory::Table, PlayerMemory::SessionIdFk)
                            .to(Sessions::Table, Sessions::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_player_memory_player")
                            .from(PlayerMemory::Table, PlayerMemory::PlayerIdFk)
                            .to(Players::Table, Players::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("ux_player_memory_session_player")
                    .table(PlayerMemory::Table)
                    .col(PlayerMemory::SessionIdFk)
                    .col(PlayerMemory::PlayerIdFk)
                    .unique()
                    .to_owned(),
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(
                Table::drop()
                    .table(PlayerMemory::Table)
                    .if_exists()
                    .to_owned(),
            )
            .await?;
        manager
            .alter_table(
                Table::alter()
                    .table(Projects::Table)
                    .drop_column(Projects::MemorySchema)
                    .to_owned(),
            )
            .await?;
        Ok(())
    }
}

#[derive(DeriveIden)]
enum PlayerMemory {
    Table,
    Id,
    SessionIdFk,
    PlayerIdFk,
    ValuesJson,
    SourceHash,
    CreatedAt,
    UpdatedAt,
}

#[derive(DeriveIden)]
enum Projects {
    Table,
    MemorySchema,
}

#[derive(DeriveIden)]
enum Sessions {
    Table,
    Id,
}

#[derive(DeriveIden)]
enum Players {
    Table,
    Id,
}
