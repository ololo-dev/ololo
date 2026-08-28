use sea_orm_migration::prelude::*;

/// Cost accounting for judge LLM usage: provider-cache token counts join
/// the existing `tokens_input`/`tokens_output` so per-session, per-player
/// spend is answerable from `judge_results` alone (cache reads bill at a
/// different rate than fresh input tokens).
#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        for col in [
            JudgeResults::TokensCacheRead,
            JudgeResults::TokensCacheWrite,
        ] {
            manager
                .alter_table(
                    Table::alter()
                        .table(JudgeResults::Table)
                        .add_column(ColumnDef::new(col).big_integer().null())
                        .to_owned(),
                )
                .await?;
        }
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        for col in [
            JudgeResults::TokensCacheRead,
            JudgeResults::TokensCacheWrite,
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
    TokensCacheRead,
    TokensCacheWrite,
}
