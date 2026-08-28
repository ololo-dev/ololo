use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(Judges::Table)
                    .if_not_exists()
                    .col(ColumnDef::new(Judges::Id).uuid().not_null().primary_key())
                    .col(
                        ColumnDef::new(Judges::Slug)
                            .string()
                            .not_null()
                            .unique_key(),
                    )
                    .col(ColumnDef::new(Judges::Name).string().not_null())
                    .col(ColumnDef::new(Judges::Description).text().not_null())
                    .col(ColumnDef::new(Judges::Prompt).text().not_null())
                    .col(ColumnDef::new(Judges::RatingScale).json_binary().not_null())
                    .col(
                        ColumnDef::new(Judges::CreatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(Judges::UpdatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("ux_judges_slug")
                    .table(Judges::Table)
                    .col(Judges::Slug)
                    .unique()
                    .to_owned(),
            )
            .await?;

        manager
            .create_table(
                Table::create()
                    .table(TaskJudges::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(TaskJudges::Id)
                            .uuid()
                            .not_null()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(TaskJudges::TaskId).uuid().not_null())
                    .col(ColumnDef::new(TaskJudges::JudgeId).uuid().not_null())
                    .col(ColumnDef::new(TaskJudges::Ordinal).integer().not_null())
                    .col(
                        ColumnDef::new(TaskJudges::RatingScaleOverride)
                            .json_binary()
                            .null(),
                    )
                    .col(
                        ColumnDef::new(TaskJudges::CreatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(TaskJudges::UpdatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_task_judges_task")
                            .from(TaskJudges::Table, TaskJudges::TaskId)
                            .to(Tasks::Table, Tasks::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_task_judges_judge")
                            .from(TaskJudges::Table, TaskJudges::JudgeId)
                            .to(Judges::Table, Judges::Id)
                            .on_delete(ForeignKeyAction::Restrict),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("ux_task_judges_task_judge")
                    .table(TaskJudges::Table)
                    .col(TaskJudges::TaskId)
                    .col(TaskJudges::JudgeId)
                    .unique()
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("ux_task_judges_task_ordinal")
                    .table(TaskJudges::Table)
                    .col(TaskJudges::TaskId)
                    .col(TaskJudges::Ordinal)
                    .unique()
                    .to_owned(),
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(
                Table::drop()
                    .table(TaskJudges::Table)
                    .if_exists()
                    .to_owned(),
            )
            .await?;
        manager
            .drop_table(Table::drop().table(Judges::Table).if_exists().to_owned())
            .await?;
        Ok(())
    }
}

#[derive(DeriveIden)]
enum Judges {
    Table,
    Id,
    Slug,
    Name,
    Description,
    Prompt,
    RatingScale,
    CreatedAt,
    UpdatedAt,
}

#[derive(DeriveIden)]
enum TaskJudges {
    Table,
    Id,
    TaskId,
    JudgeId,
    Ordinal,
    RatingScaleOverride,
    CreatedAt,
    UpdatedAt,
}

#[derive(DeriveIden)]
enum Tasks {
    Table,
    Id,
}
