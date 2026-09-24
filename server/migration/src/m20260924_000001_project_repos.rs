//! A project's git repository: the code its sessions start from, cloned
//! into the player's folder before a session starts there.
//!
//! One optional row per project, beside `projects` rather than in it: few
//! projects name a repository, and every reader that cares (the project
//! summary, the CLI's pre-start clone, the scoring that treats a session as
//! work on existing code) asks this table, while nothing that constructs a
//! project row has to change.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(ProjectRepos::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(ProjectRepos::ProjectIdFk)
                            .uuid()
                            .not_null()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(ProjectRepos::Url).text().not_null())
                    .col(ColumnDef::new(ProjectRepos::GitRef).text().null())
                    .col(
                        ColumnDef::new(ProjectRepos::CreatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(ProjectRepos::UpdatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_project_repos_project")
                            .from(ProjectRepos::Table, ProjectRepos::ProjectIdFk)
                            .to(Projects::Table, Projects::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(ProjectRepos::Table).to_owned())
            .await
    }
}

#[derive(DeriveIden)]
enum ProjectRepos {
    Table,
    ProjectIdFk,
    Url,
    GitRef,
    CreatedAt,
    UpdatedAt,
}

#[derive(DeriveIden)]
enum Projects {
    Table,
    Id,
}
