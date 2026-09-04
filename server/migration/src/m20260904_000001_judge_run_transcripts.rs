use sea_orm_migration::prelude::*;

/// The conversation of a judge run paused on a participant request.
///
/// A judge that asks the participant for a capture (or to run a command)
/// no longer writes a provisional verdict and re-investigates from scratch
/// when the answer lands: its run stops at that tool call, the transcript
/// so far is kept here, and the re-drive resumes the same conversation
/// with the answer appended. One row per waiting `judge_results` row;
/// deleted with it, and cleared once the resumed run scores.
#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(JudgeRunTranscripts::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(JudgeRunTranscripts::JudgeResultId)
                            .uuid()
                            .not_null()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(JudgeRunTranscripts::Transcript)
                            .json_binary()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(JudgeRunTranscripts::CreatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(JudgeRunTranscripts::UpdatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_judge_run_transcripts_result")
                            .from(
                                JudgeRunTranscripts::Table,
                                JudgeRunTranscripts::JudgeResultId,
                            )
                            .to(JudgeResults::Table, JudgeResults::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(JudgeRunTranscripts::Table).to_owned())
            .await
    }
}

#[derive(DeriveIden)]
enum JudgeRunTranscripts {
    Table,
    JudgeResultId,
    Transcript,
    CreatedAt,
    UpdatedAt,
}

#[derive(DeriveIden)]
enum JudgeResults {
    Table,
    Id,
}
