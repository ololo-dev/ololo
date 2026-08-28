use sea_orm_migration::prelude::*;

/// Judges attached to another judge's open artifact request.
///
/// The interactive requests themselves are `tests` rows (the registry of
/// requested artifacts, instruction = description). When a judge's ask
/// matches an already-open request, no second request is sent to the
/// participant — the judge is recorded here as a watcher and re-driven
/// alongside the registrant when the artifact lands or expires.
#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(ArtifactRequestWatchers::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(ArtifactRequestWatchers::TestId)
                            .uuid()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(ArtifactRequestWatchers::JudgeId)
                            .uuid()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(ArtifactRequestWatchers::CreatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .primary_key(
                        Index::create()
                            .col(ArtifactRequestWatchers::TestId)
                            .col(ArtifactRequestWatchers::JudgeId),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_artifact_watchers_test")
                            .from(
                                ArtifactRequestWatchers::Table,
                                ArtifactRequestWatchers::TestId,
                            )
                            .to(Tests::Table, Tests::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_artifact_watchers_judge")
                            .from(
                                ArtifactRequestWatchers::Table,
                                ArtifactRequestWatchers::JudgeId,
                            )
                            .to(Judges::Table, Judges::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(
                Table::drop()
                    .table(ArtifactRequestWatchers::Table)
                    .to_owned(),
            )
            .await
    }
}

#[derive(DeriveIden)]
enum ArtifactRequestWatchers {
    Table,
    TestId,
    JudgeId,
    CreatedAt,
}

#[derive(DeriveIden)]
enum Tests {
    Table,
    Id,
}

#[derive(DeriveIden)]
enum Judges {
    Table,
    Id,
}
