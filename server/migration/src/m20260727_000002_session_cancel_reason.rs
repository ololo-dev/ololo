use sea_orm_migration::prelude::*;

/// Why a session was cancelled, and by whom.
///
/// `cancel_reason` is a machine code: `"user"` (someone cancelled it from the
/// dashboard) or `"idle_timeout"` (the idle sweep found no connected agents).
/// `cancelled_by` carries the display name for the `"user"` case. Persisted so
/// the end notification can say *why* — "cancelled by Andrey" reads very
/// differently from "cancelled automatically: nobody was connected" — and so
/// the answer survives past the live broadcast.
///
/// One `alter_table` per column (SQLite).
#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(Sessions::Table)
                    .add_column(ColumnDef::new(Sessions::CancelReason).string().null())
                    .to_owned(),
            )
            .await?;
        manager
            .alter_table(
                Table::alter()
                    .table(Sessions::Table)
                    .add_column(ColumnDef::new(Sessions::CancelledBy).string().null())
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(Sessions::Table)
                    .drop_column(Sessions::CancelReason)
                    .to_owned(),
            )
            .await?;
        manager
            .alter_table(
                Table::alter()
                    .table(Sessions::Table)
                    .drop_column(Sessions::CancelledBy)
                    .to_owned(),
            )
            .await
    }
}

#[derive(DeriveIden)]
enum Sessions {
    Table,
    CancelReason,
    CancelledBy,
}
