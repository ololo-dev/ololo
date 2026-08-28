//! A judge's model override can name a pool as well as a single model.
//!
//! Both may be set at once: `llm_source_order` decides which set of
//! candidates goes first, and the other becomes the failover behind it. That
//! is why this is an order column rather than a "use pool / use model" flag
//! — the two sources compose instead of excluding each other.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // One column per `alter_table` — SQLite rejects multi-column ADD.
        manager
            .alter_table(
                Table::alter()
                    .table(Judges::Table)
                    .add_column(ColumnDef::new(Judges::LlmPoolIdFk).uuid().null())
                    .to_owned(),
            )
            .await?;
        manager
            .alter_table(
                Table::alter()
                    .table(Judges::Table)
                    .add_column(
                        ColumnDef::new(Judges::LlmSourceOrder)
                            .text()
                            .not_null()
                            .default("pool_first"),
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
                    .drop_column(Judges::LlmSourceOrder)
                    .to_owned(),
            )
            .await?;
        manager
            .alter_table(
                Table::alter()
                    .table(Judges::Table)
                    .drop_column(Judges::LlmPoolIdFk)
                    .to_owned(),
            )
            .await
    }
}

#[derive(DeriveIden)]
enum Judges {
    Table,
    LlmPoolIdFk,
    LlmSourceOrder,
}
