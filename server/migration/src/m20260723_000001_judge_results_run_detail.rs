use sea_orm_migration::prelude::*;

/// Full judge-run observability for the admin judges tab: the LLM
/// `provider` used, a chronological `run_log` (JSON array of LLM turns and
/// tool calls with timestamps, durations, and per-turn token usage), and
/// total `tokens_input`/`tokens_output`. Pre-existing rows keep an empty
/// provider and null detail columns.
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
                        ColumnDef::new(JudgeResults::Provider)
                            .string()
                            .not_null()
                            .default(""),
                    )
                    .to_owned(),
            )
            .await?;
        manager
            .alter_table(
                Table::alter()
                    .table(JudgeResults::Table)
                    .add_column(ColumnDef::new(JudgeResults::RunLog).json_binary().null())
                    .to_owned(),
            )
            .await?;
        manager
            .alter_table(
                Table::alter()
                    .table(JudgeResults::Table)
                    .add_column(
                        ColumnDef::new(JudgeResults::TokensInput)
                            .big_integer()
                            .null(),
                    )
                    .to_owned(),
            )
            .await?;
        manager
            .alter_table(
                Table::alter()
                    .table(JudgeResults::Table)
                    .add_column(
                        ColumnDef::new(JudgeResults::TokensOutput)
                            .big_integer()
                            .null(),
                    )
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        for col in [
            JudgeResults::Provider,
            JudgeResults::RunLog,
            JudgeResults::TokensInput,
            JudgeResults::TokensOutput,
        ] {
            manager
                .alter_table(
                    Table::alter()
                        .table(JudgeResults::Table)
                        .drop_column(col)
                        .to_owned(),
                )
                .await?;
        }
        Ok(())
    }
}

#[derive(DeriveIden, Clone, Copy)]
enum JudgeResults {
    Table,
    Provider,
    RunLog,
    TokensInput,
    TokensOutput,
}
