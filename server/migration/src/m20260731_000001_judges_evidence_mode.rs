//! Adds `judges.evidence_mode`, selecting how an LLM judge receives its facts.
//!
//! `"tools"` (the default, and what every existing judge keeps) runs the
//! agentic loop: the judge starts from a thin briefing and pulls git evidence
//! through tool calls. `"dossier"` runs a single completion against an
//! evidence pack assembled server-side, with no tools offered — the same shape
//! the session-scoped judge already uses, which avoids paying a provider
//! round-trip per tool call and re-sending the tool transcript every turn.

use sea_orm_migration::prelude::*;

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
                        ColumnDef::new(Judges::EvidenceMode)
                            .text()
                            .not_null()
                            .default("tools"),
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
                    .drop_column(Judges::EvidenceMode)
                    .to_owned(),
            )
            .await
    }
}

#[derive(DeriveIden)]
enum Judges {
    Table,
    EvidenceMode,
}
