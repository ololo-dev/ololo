use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(JudgeResults::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(JudgeResults::Id)
                            .uuid()
                            .not_null()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(JudgeResults::SessionIdFk).uuid().not_null())
                    .col(ColumnDef::new(JudgeResults::PlayerIdFk).uuid().not_null())
                    .col(ColumnDef::new(JudgeResults::TaskJudgeId).uuid().not_null())
                    .col(
                        ColumnDef::new(JudgeResults::Rating)
                            .json_binary()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(JudgeResults::PointDelta)
                            .integer()
                            .not_null(),
                    )
                    .col(ColumnDef::new(JudgeResults::Feedback).text().not_null())
                    .col(ColumnDef::new(JudgeResults::Model).text().not_null())
                    .col(ColumnDef::new(JudgeResults::RawOutput).text().not_null())
                    .col(
                        ColumnDef::new(JudgeResults::CreatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(JudgeResults::UpdatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_judge_results_session")
                            .from(JudgeResults::Table, JudgeResults::SessionIdFk)
                            .to(Sessions::Table, Sessions::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_judge_results_player")
                            .from(JudgeResults::Table, JudgeResults::PlayerIdFk)
                            .to(Players::Table, Players::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_judge_results_task_judge")
                            .from(JudgeResults::Table, JudgeResults::TaskJudgeId)
                            .to(TaskJudges::Table, TaskJudges::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("ux_judge_results_task_judge_player")
                    .table(JudgeResults::Table)
                    .col(JudgeResults::TaskJudgeId)
                    .col(JudgeResults::PlayerIdFk)
                    .unique()
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("ix_judge_results_session_player")
                    .table(JudgeResults::Table)
                    .col(JudgeResults::SessionIdFk)
                    .col(JudgeResults::PlayerIdFk)
                    .to_owned(),
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(
                Table::drop()
                    .table(JudgeResults::Table)
                    .if_exists()
                    .to_owned(),
            )
            .await?;
        Ok(())
    }
}

#[derive(DeriveIden)]
enum JudgeResults {
    Table,
    Id,
    SessionIdFk,
    PlayerIdFk,
    TaskJudgeId,
    Rating,
    PointDelta,
    Feedback,
    Model,
    RawOutput,
    CreatedAt,
    UpdatedAt,
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
enum TaskJudges {
    Table,
    Id,
}
