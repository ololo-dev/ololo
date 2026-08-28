use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(ActivityEvent::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(ActivityEvent::Id)
                            .uuid()
                            .not_null()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(ActivityEvent::SessionIdFk).uuid().not_null())
                    .col(ColumnDef::new(ActivityEvent::PlayerIdFk).uuid().not_null())
                    .col(ColumnDef::new(ActivityEvent::TaskIdFk).uuid().not_null())
                    .col(ColumnDef::new(ActivityEvent::EventKind).text().not_null())
                    .col(
                        ColumnDef::new(ActivityEvent::TaskOrdinal)
                            .integer()
                            .not_null(),
                    )
                    .col(ColumnDef::new(ActivityEvent::TaskTitle).text().not_null())
                    .col(
                        ColumnDef::new(ActivityEvent::PlayerDisplayName)
                            .text()
                            .not_null(),
                    )
                    .col(ColumnDef::new(ActivityEvent::JudgeName).text())
                    .col(ColumnDef::new(ActivityEvent::PointDelta).integer())
                    .col(
                        ColumnDef::new(ActivityEvent::Timestamp)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(ActivityEvent::Version)
                            .big_unsigned()
                            .not_null(),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_activity_event_session")
                            .from(ActivityEvent::Table, ActivityEvent::SessionIdFk)
                            .to(Sessions::Table, Sessions::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_activity_event_player")
                            .from(ActivityEvent::Table, ActivityEvent::PlayerIdFk)
                            .to(Players::Table, Players::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_activity_event_task")
                            .from(ActivityEvent::Table, ActivityEvent::TaskIdFk)
                            .to(Tasks::Table, Tasks::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("ix_activity_event_session_created")
                    .table(ActivityEvent::Table)
                    .col(ActivityEvent::SessionIdFk)
                    .col(ActivityEvent::Timestamp)
                    .to_owned(),
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(
                Table::drop()
                    .table(ActivityEvent::Table)
                    .if_exists()
                    .to_owned(),
            )
            .await?;
        Ok(())
    }
}

// ponytail: no retention policy; add periodic cleanup if session volume grows.

#[derive(DeriveIden)]
enum ActivityEvent {
    Table,
    Id,
    SessionIdFk,
    PlayerIdFk,
    TaskIdFk,
    EventKind,
    TaskOrdinal,
    TaskTitle,
    PlayerDisplayName,
    JudgeName,
    PointDelta,
    Timestamp,
    Version,
}

#[derive(DeriveIden)]
enum Sessions {
    Table,
    Id,
}

#[derive(DeriveIden)]
enum Players {
    Table,
    Id,
}

#[derive(DeriveIden)]
enum Tasks {
    Table,
    Id,
}
