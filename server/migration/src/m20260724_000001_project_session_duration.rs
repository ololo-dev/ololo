use sea_orm_migration::prelude::*;

/// Per-project session duration: `default_session_duration_secs` joins the
/// existing `default_*` family on `projects`. The schema default (3600, the
/// current env default) backfills rows that predate the migration.
#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Single add_column per alter_table call: SQLite rejects multi-column
        // ALTER TABLE at runtime even though it compiles.
        manager
            .alter_table(
                Table::alter()
                    .table(Projects::Table)
                    .add_column(
                        ColumnDef::new(Projects::DefaultSessionDurationSecs)
                            .big_integer()
                            .not_null()
                            .default(3600),
                    )
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(Projects::Table)
                    .drop_column(Projects::DefaultSessionDurationSecs)
                    .to_owned(),
            )
            .await
    }
}

#[derive(DeriveIden)]
enum Projects {
    Table,
    DefaultSessionDurationSecs,
}
