use sea_orm_migration::prelude::*;

/// Campaign projects: a parent project composed of ordered parts.
///
/// `projects.parent_project_id_fk` points at the parent project row and
/// `projects.part_ordinal` gives the part's position inside the campaign
/// (0-based). Both are NULL for standalone projects and for the parent
/// itself. No DB-level FK constraint: SQLite cannot add one via ALTER
/// TABLE, so integrity is enforced in code — seed linking resolves slugs
/// against real rows and the project delete handler unlinks children.
#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(Projects::Table)
                    .add_column(ColumnDef::new(Projects::ParentProjectIdFk).uuid().null())
                    .to_owned(),
            )
            .await?;
        manager
            .alter_table(
                Table::alter()
                    .table(Projects::Table)
                    .add_column(ColumnDef::new(Projects::PartOrdinal).integer().null())
                    .to_owned(),
            )
            .await?;
        // Doubles as the children-by-parent lookup index (parent col prefix).
        // NULLs are distinct on both SQLite and Postgres, so standalone rows
        // never collide.
        manager
            .create_index(
                Index::create()
                    .name("ux_projects_parent_part")
                    .table(Projects::Table)
                    .col(Projects::ParentProjectIdFk)
                    .col(Projects::PartOrdinal)
                    .unique()
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_index(
                Index::drop()
                    .name("ux_projects_parent_part")
                    .table(Projects::Table)
                    .to_owned(),
            )
            .await?;
        manager
            .alter_table(
                Table::alter()
                    .table(Projects::Table)
                    .drop_column(Projects::PartOrdinal)
                    .to_owned(),
            )
            .await?;
        manager
            .alter_table(
                Table::alter()
                    .table(Projects::Table)
                    .drop_column(Projects::ParentProjectIdFk)
                    .to_owned(),
            )
            .await
    }
}

#[derive(DeriveIden)]
enum Projects {
    Table,
    ParentProjectIdFk,
    PartOrdinal,
}
