use sea_orm_migration::prelude::*;

/// Judge-type discriminator. Until now every judge was an LLM evaluator;
/// `kind` distinguishes those (`"llm"`) from execution judges (`"execution"`)
/// that run the player's committed solution against the task's probes
/// server-side. Existing rows default to `"llm"`.
#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(Judges::Table)
                    .add_column(
                        ColumnDef::new(Judges::Kind)
                            .string()
                            .not_null()
                            .default("llm"),
                    )
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(Judges::Table)
                    .drop_column(Judges::Kind)
                    .to_owned(),
            )
            .await
    }
}

#[derive(DeriveIden)]
enum Judges {
    Table,
    Kind,
}
