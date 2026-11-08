//! Linked sessions: a few sessions a host runs as one event — rounds of a
//! workshop, the same brief for several rooms — read together.
//!
//! One row per linked session, beside `sessions` rather than in it (the
//! `project_repos` pattern: a real foreign key, and nothing that constructs
//! a session row has to change). Sessions sharing a `group_id` are linked
//! to each other; a session with no row is linked to nothing. The group is
//! an id and nothing more — the sessions themselves are the group's name.
use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(SessionLinks::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(SessionLinks::SessionIdFk)
                            .uuid()
                            .not_null()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(SessionLinks::GroupId).uuid().not_null())
                    .col(
                        ColumnDef::new(SessionLinks::CreatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_session_links_session")
                            .from(SessionLinks::Table, SessionLinks::SessionIdFk)
                            .to(Sessions::Table, Sessions::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name("idx_session_links_group")
                    .table(SessionLinks::Table)
                    .col(SessionLinks::GroupId)
                    .if_not_exists()
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(SessionLinks::Table).to_owned())
            .await
    }
}

#[derive(DeriveIden)]
enum SessionLinks {
    Table,
    SessionIdFk,
    GroupId,
    CreatedAt,
}

#[derive(DeriveIden)]
enum Sessions {
    Table,
    Id,
}
