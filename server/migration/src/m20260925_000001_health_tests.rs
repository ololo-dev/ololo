//! The project's own tests as part of code health.
//!
//! `player_test_commands` holds the commands that run a player's tests and
//! coverage, read from their `AGENTS.md` / `README.md` whenever those files
//! change (one row per session player). Each checkpoint gains the client's
//! run of those tests after its probe: how it ended and what it measured,
//! NULL when no run was reported for that commit.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(PlayerTestCommands::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(PlayerTestCommands::Id)
                            .uuid()
                            .not_null()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(PlayerTestCommands::SessionIdFk)
                            .uuid()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(PlayerTestCommands::PlayerIdFk)
                            .uuid()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(PlayerTestCommands::TestCommand)
                            .text()
                            .null(),
                    )
                    .col(
                        ColumnDef::new(PlayerTestCommands::CoverageCommand)
                            .text()
                            .null(),
                    )
                    .col(
                        ColumnDef::new(PlayerTestCommands::Sources)
                            .text()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(PlayerTestCommands::SourceHash)
                            .text()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(PlayerTestCommands::CreatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(PlayerTestCommands::UpdatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_player_test_commands_session")
                            .from(PlayerTestCommands::Table, PlayerTestCommands::SessionIdFk)
                            .to(Sessions::Table, Sessions::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_player_test_commands_player")
                            .from(PlayerTestCommands::Table, PlayerTestCommands::PlayerIdFk)
                            .to(Players::Table, Players::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name("ux_player_test_commands_session_player")
                    .table(PlayerTestCommands::Table)
                    .col(PlayerTestCommands::SessionIdFk)
                    .col(PlayerTestCommands::PlayerIdFk)
                    .unique()
                    .to_owned(),
            )
            .await?;
        // One column per statement: SQLite's ALTER TABLE adds one at a time.
        manager
            .alter_table(
                Table::alter()
                    .table(HealthCheckpoints::Table)
                    .add_column(ColumnDef::new(HealthCheckpoints::TestsStatus).text().null())
                    .to_owned(),
            )
            .await?;
        manager
            .alter_table(
                Table::alter()
                    .table(HealthCheckpoints::Table)
                    .add_column(
                        ColumnDef::new(HealthCheckpoints::TestsResult)
                            .json_binary()
                            .null(),
                    )
                    .to_owned(),
            )
            .await?;
        manager
            .alter_table(
                Table::alter()
                    .table(HealthCheckpoints::Table)
                    .add_column(
                        ColumnDef::new(HealthCheckpoints::TestsReportedAt)
                            .timestamp_with_time_zone()
                            .null(),
                    )
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        for column in [
            HealthCheckpoints::TestsReportedAt,
            HealthCheckpoints::TestsResult,
            HealthCheckpoints::TestsStatus,
        ] {
            manager
                .alter_table(
                    Table::alter()
                        .table(HealthCheckpoints::Table)
                        .drop_column(column)
                        .to_owned(),
                )
                .await?;
        }
        manager
            .drop_table(
                Table::drop()
                    .table(PlayerTestCommands::Table)
                    .if_exists()
                    .to_owned(),
            )
            .await
    }
}

#[derive(DeriveIden)]
enum PlayerTestCommands {
    Table,
    Id,
    SessionIdFk,
    PlayerIdFk,
    TestCommand,
    CoverageCommand,
    Sources,
    SourceHash,
    CreatedAt,
    UpdatedAt,
}

#[derive(DeriveIden)]
enum HealthCheckpoints {
    Table,
    TestsStatus,
    TestsResult,
    TestsReportedAt,
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
