//! Adds `activity_event.detail`: optional structured payload for the feed.
//!
//! The activity table was built for one-line events; open-ended judge
//! verdicts carry a per-criterion sheet the session page wants to show
//! under the line. NULL everywhere else — classic events are unchanged.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(ActivityEvent::Table)
                    .add_column(ColumnDef::new(ActivityEvent::Detail).json_binary().null())
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(ActivityEvent::Table)
                    .drop_column(ActivityEvent::Detail)
                    .to_owned(),
            )
            .await
    }
}

#[derive(DeriveIden)]
enum ActivityEvent {
    Table,
    Detail,
}
