use sea_orm_migration::prelude::*;

/// `projects.show_tasks` — whether the public project page lists the task
/// arc up front. Most projects read better with it (a player deserves to
/// see what a 30-minute commitment contains — audit UI-H2), but quiz-shaped
/// projects like the Extreme Startup family stage their questions as a
/// surprise, and printing the ladder would hand out the answers' shape
/// before the first probe fires. Defaults to visible; a project opts out in
/// its seed frontmatter (`show_tasks: false`).
#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(Projects::Table)
                    .add_column(
                        ColumnDef::new(Projects::ShowTasks)
                            .boolean()
                            .not_null()
                            .default(true),
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
                    .drop_column(Projects::ShowTasks)
                    .to_owned(),
            )
            .await
    }
}

#[derive(DeriveIden)]
enum Projects {
    Table,
    ShowTasks,
}
