use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(LlmRequests::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(LlmRequests::Id)
                            .uuid()
                            .not_null()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(LlmRequests::Operation).text().not_null())
                    .col(ColumnDef::new(LlmRequests::Provider).text().not_null())
                    .col(ColumnDef::new(LlmRequests::Model).text().not_null())
                    .col(ColumnDef::new(LlmRequests::Status).text().not_null())
                    .col(ColumnDef::new(LlmRequests::Error).text().null())
                    .col(
                        ColumnDef::new(LlmRequests::TokensInput)
                            .big_integer()
                            .not_null()
                            .default(0),
                    )
                    .col(
                        ColumnDef::new(LlmRequests::TokensOutput)
                            .big_integer()
                            .not_null()
                            .default(0),
                    )
                    .col(
                        ColumnDef::new(LlmRequests::TokensCacheRead)
                            .big_integer()
                            .not_null()
                            .default(0),
                    )
                    .col(
                        ColumnDef::new(LlmRequests::TokensCacheWrite)
                            .big_integer()
                            .not_null()
                            .default(0),
                    )
                    .col(
                        ColumnDef::new(LlmRequests::DurationMs)
                            .big_integer()
                            .not_null(),
                    )
                    // Intentionally NO foreign keys: telemetry rows must
                    // outlive the sessions/players/tasks they reference.
                    .col(ColumnDef::new(LlmRequests::SessionId).uuid().null())
                    .col(ColumnDef::new(LlmRequests::PlayerId).uuid().null())
                    .col(ColumnDef::new(LlmRequests::TaskId).uuid().null())
                    .col(ColumnDef::new(LlmRequests::JudgeSlug).text().null())
                    .col(ColumnDef::new(LlmRequests::DetailJson).text().null())
                    .col(
                        ColumnDef::new(LlmRequests::CreatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("ix_llm_requests_created_at")
                    .table(LlmRequests::Table)
                    .col(LlmRequests::CreatedAt)
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("ix_llm_requests_operation")
                    .table(LlmRequests::Table)
                    .col(LlmRequests::Operation)
                    .to_owned(),
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(
                Table::drop()
                    .table(LlmRequests::Table)
                    .if_exists()
                    .to_owned(),
            )
            .await?;
        Ok(())
    }
}

#[derive(DeriveIden)]
enum LlmRequests {
    Table,
    Id,
    Operation,
    Provider,
    Model,
    Status,
    Error,
    TokensInput,
    TokensOutput,
    TokensCacheRead,
    TokensCacheWrite,
    DurationMs,
    SessionId,
    PlayerId,
    TaskId,
    JudgeSlug,
    DetailJson,
    CreatedAt,
}
