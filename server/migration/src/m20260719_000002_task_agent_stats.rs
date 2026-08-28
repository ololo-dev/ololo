use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(TaskAgentStats::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(TaskAgentStats::Id)
                            .uuid()
                            .not_null()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(TaskAgentStats::SessionIdFk)
                            .uuid()
                            .not_null(),
                    )
                    .col(ColumnDef::new(TaskAgentStats::PlayerIdFk).uuid().not_null())
                    .col(ColumnDef::new(TaskAgentStats::TaskIdFk).uuid().null())
                    .col(
                        ColumnDef::new(TaskAgentStats::TaskOrdinal)
                            .integer()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(TaskAgentStats::WindowStartedAt)
                            .timestamp_with_time_zone()
                            .null(),
                    )
                    .col(
                        ColumnDef::new(TaskAgentStats::WindowEndedAt)
                            .timestamp_with_time_zone()
                            .null(),
                    )
                    .col(
                        ColumnDef::new(TaskAgentStats::InputTokens)
                            .big_integer()
                            .not_null()
                            .default(0),
                    )
                    .col(
                        ColumnDef::new(TaskAgentStats::OutputTokens)
                            .big_integer()
                            .not_null()
                            .default(0),
                    )
                    .col(
                        ColumnDef::new(TaskAgentStats::CacheReadTokens)
                            .big_integer()
                            .not_null()
                            .default(0),
                    )
                    .col(
                        ColumnDef::new(TaskAgentStats::CacheWriteTokens)
                            .big_integer()
                            .not_null()
                            .default(0),
                    )
                    .col(
                        ColumnDef::new(TaskAgentStats::ReasoningTokens)
                            .big_integer()
                            .not_null()
                            .default(0),
                    )
                    .col(ColumnDef::new(TaskAgentStats::Cost).double().null())
                    .col(
                        ColumnDef::new(TaskAgentStats::UserMessages)
                            .big_integer()
                            .not_null()
                            .default(0),
                    )
                    .col(
                        ColumnDef::new(TaskAgentStats::AssistantMessages)
                            .big_integer()
                            .not_null()
                            .default(0),
                    )
                    .col(
                        ColumnDef::new(TaskAgentStats::ToolCalls)
                            .big_integer()
                            .not_null()
                            .default(0),
                    )
                    .col(ColumnDef::new(TaskAgentStats::AgentsJson).text().not_null())
                    .col(
                        ColumnDef::new(TaskAgentStats::CreatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(TaskAgentStats::UpdatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_task_agent_stats_session")
                            .from(TaskAgentStats::Table, TaskAgentStats::SessionIdFk)
                            .to(Sessions::Table, Sessions::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_task_agent_stats_player")
                            .from(TaskAgentStats::Table, TaskAgentStats::PlayerIdFk)
                            .to(Players::Table, Players::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_task_agent_stats_task")
                            .from(TaskAgentStats::Table, TaskAgentStats::TaskIdFk)
                            .to(Tasks::Table, Tasks::Id)
                            .on_delete(ForeignKeyAction::SetNull),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("ux_task_agent_stats_session_player_ordinal")
                    .table(TaskAgentStats::Table)
                    .col(TaskAgentStats::SessionIdFk)
                    .col(TaskAgentStats::PlayerIdFk)
                    .col(TaskAgentStats::TaskOrdinal)
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
                    .table(TaskAgentStats::Table)
                    .if_exists()
                    .to_owned(),
            )
            .await?;
        Ok(())
    }
}

#[derive(DeriveIden)]
enum TaskAgentStats {
    Table,
    Id,
    SessionIdFk,
    PlayerIdFk,
    TaskIdFk,
    TaskOrdinal,
    WindowStartedAt,
    WindowEndedAt,
    InputTokens,
    OutputTokens,
    CacheReadTokens,
    CacheWriteTokens,
    ReasoningTokens,
    Cost,
    UserMessages,
    AssistantMessages,
    ToolCalls,
    AgentsJson,
    CreatedAt,
    UpdatedAt,
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

#[derive(DeriveIden)]
enum Tasks {
    Table,
    Id,
}
