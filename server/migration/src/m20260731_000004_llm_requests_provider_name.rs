//! Record *which* provider row a telemetry call went through.
//!
//! `llm_requests.provider` holds a registry id, and every
//! `openai_compatible` row collapses to `"custom"` — so several distinct
//! endpoints were indistinguishable in the telemetry UI. This is a
//! denormalized copy of the row's display name, deliberately not a foreign
//! key: telemetry rows must outlive the providers they reference (same
//! reasoning as the session/player/task columns on this table).
//!
//! Nullable because rows written before this migration have no name to
//! backfill, and because the static last-resort configs come from no row.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(LlmRequests::Table)
                    .add_column(ColumnDef::new(LlmRequests::ProviderName).text().null())
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(LlmRequests::Table)
                    .drop_column(LlmRequests::ProviderName)
                    .to_owned(),
            )
            .await
    }
}

#[derive(DeriveIden)]
enum LlmRequests {
    Table,
    ProviderName,
}
