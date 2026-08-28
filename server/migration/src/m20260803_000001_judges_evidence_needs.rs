//! Adds `judges.evidence_needs`: what a judge declares it needs to see.
//!
//! The evidence snapshot is assembled whole on every judge run — including
//! the cross-task view, which walks git twice per task the player reached.
//! Most judges never read it: an execution judge re-runs probes, and a
//! `tools` judge pulls what it wants through tool calls. Only a judge's own
//! `js decide` / `js review` programs read the snapshot, and each reads a
//! slice of it.
//!
//! So a judge lists what it needs, and only that is materialized. NULL means
//! "not declared" and keeps the whole snapshot, so existing rows behave
//! exactly as before until their file says otherwise.

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
                    .add_column(ColumnDef::new(Judges::EvidenceNeeds).text().null())
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(Judges::Table)
                    .drop_column(Judges::EvidenceNeeds)
                    .to_owned(),
            )
            .await
    }
}

#[derive(DeriveIden)]
enum Judges {
    Table,
    EvidenceNeeds,
}
