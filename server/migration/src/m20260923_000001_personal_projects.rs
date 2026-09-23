//! Personal projects: a user's own work, turned into a project they play in
//! their own repository.
//!
//! One row marks one `projects` row as personal and keeps what the user
//! asked for — the description, the navigation map of tasks, the judges and
//! the session length — so the project can be rebuilt (edited before its
//! first session, duplicated after) from the words the user typed rather
//! than from the briefs generated out of them. The marker lives beside
//! `projects` instead of in it: every reader that must treat personal work
//! differently (the ladder, the similarity corpus, the public profile) asks
//! this table, and nothing that constructs a project row has to change.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(PersonalProjects::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(PersonalProjects::ProjectIdFk)
                            .uuid()
                            .not_null()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(PersonalProjects::Spec)
                            .json_binary()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(PersonalProjects::CreatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(PersonalProjects::UpdatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_personal_projects_project")
                            .from(PersonalProjects::Table, PersonalProjects::ProjectIdFk)
                            .to(Projects::Table, Projects::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(PersonalProjects::Table).to_owned())
            .await
    }
}

#[derive(DeriveIden)]
enum PersonalProjects {
    Table,
    ProjectIdFk,
    Spec,
    CreatedAt,
    UpdatedAt,
}

#[derive(DeriveIden)]
enum Projects {
    Table,
    Id,
}
