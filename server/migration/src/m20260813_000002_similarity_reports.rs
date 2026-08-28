use sea_orm_migration::prelude::*;

/// Persisted cross-session copy/paste scan results, one row per
/// (session, player) scan at finish — clean runs included, so the player
/// page can show "checked, N%" instead of silence. `sources_json` holds
/// the top matching sessions (`[{join_code, player, matched_lines}]`).
/// Before this table the report lived only in the game server's on-disk
/// event log, invisible to every player surface.
#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(SimilarityReports::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(SimilarityReports::Id)
                            .uuid()
                            .not_null()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(SimilarityReports::SessionIdFk)
                            .uuid()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(SimilarityReports::PlayerIdFk)
                            .uuid()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(SimilarityReports::DuplicatedPct)
                            .double()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(SimilarityReports::DuplicatedLines)
                            .big_integer()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(SimilarityReports::TotalLines)
                            .big_integer()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(SimilarityReports::CorpusRepos)
                            .integer()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(SimilarityReports::Penalty)
                            .big_integer()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(SimilarityReports::SourcesJson)
                            .json_binary()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(SimilarityReports::CreatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_similarity_reports_session")
                            .from(SimilarityReports::Table, SimilarityReports::SessionIdFk)
                            .to(Sessions::Table, Sessions::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_similarity_reports_player")
                            .from(SimilarityReports::Table, SimilarityReports::PlayerIdFk)
                            .to(Players::Table, Players::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name("ux_similarity_reports_session_player")
                    .table(SimilarityReports::Table)
                    .col(SimilarityReports::SessionIdFk)
                    .col(SimilarityReports::PlayerIdFk)
                    .unique()
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(SimilarityReports::Table).to_owned())
            .await
    }
}

#[derive(DeriveIden)]
enum SimilarityReports {
    Table,
    Id,
    SessionIdFk,
    PlayerIdFk,
    DuplicatedPct,
    DuplicatedLines,
    TotalLines,
    CorpusRepos,
    Penalty,
    SourcesJson,
    CreatedAt,
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
