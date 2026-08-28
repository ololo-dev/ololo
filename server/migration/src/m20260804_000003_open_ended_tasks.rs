//! Open-ended tasks: additive columns only, NULL means "classic behavior".
//!
//! A task with a non-NULL `evaluation` is free-form: shown in full, finished
//! by a completion probe (or its deadline), scored by a judge panel. Every
//! other column here extends the probe/judge machinery the same way — a
//! probe may carry a config (mode/executor/schedule), an initiator, a
//! structured measurement, an artifact reference; a judge may declare its
//! own probes, the criteria it scores, and how many interactive probes it
//! may register; a verdict may be partial. Rows that predate this migration
//! keep NULL everywhere and behave exactly as before.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // One column per `alter_table` — SQLite rejects multi-column ADD.
        manager
            .alter_table(
                Table::alter()
                    .table(Tasks::Table)
                    .add_column(ColumnDef::new(Tasks::Evaluation).json_binary().null())
                    .to_owned(),
            )
            .await?;
        manager
            .alter_table(
                Table::alter()
                    .table(TaskJudges::Table)
                    .add_column(ColumnDef::new(TaskJudges::Weight).double().null())
                    .to_owned(),
            )
            .await?;
        manager
            .alter_table(
                Table::alter()
                    .table(Tests::Table)
                    .add_column(ColumnDef::new(Tests::ProbeConfig).json_binary().null())
                    .to_owned(),
            )
            .await?;
        manager
            .alter_table(
                Table::alter()
                    .table(Tests::Table)
                    .add_column(
                        ColumnDef::new(Tests::Initiator)
                            .text()
                            .not_null()
                            .default("system"),
                    )
                    .to_owned(),
            )
            .await?;
        // Plain nullable uuid, no FK constraint: SQLite cannot ADD a column
        // with one (same precedent as `judges.llm_pool_id_fk`).
        manager
            .alter_table(
                Table::alter()
                    .table(Tests::Table)
                    .add_column(ColumnDef::new(Tests::RegisteredByJudgeId).uuid().null())
                    .to_owned(),
            )
            .await?;
        manager
            .alter_table(
                Table::alter()
                    .table(Probes::Table)
                    .add_column(ColumnDef::new(Probes::ResultJson).json_binary().null())
                    .to_owned(),
            )
            .await?;
        manager
            .alter_table(
                Table::alter()
                    .table(Probes::Table)
                    .add_column(ColumnDef::new(Probes::ArtifactPath).text().null())
                    .to_owned(),
            )
            .await?;
        manager
            .alter_table(
                Table::alter()
                    .table(JudgeResults::Table)
                    .add_column(ColumnDef::new(JudgeResults::VerdictKind).text().null())
                    .to_owned(),
            )
            .await?;
        manager
            .alter_table(
                Table::alter()
                    .table(Judges::Table)
                    .add_column(ColumnDef::new(Judges::Criteria).text().null())
                    .to_owned(),
            )
            .await?;
        manager
            .alter_table(
                Table::alter()
                    .table(Judges::Table)
                    .add_column(ColumnDef::new(Judges::ProbesConfig).json_binary().null())
                    .to_owned(),
            )
            .await?;
        manager
            .alter_table(
                Table::alter()
                    .table(Judges::Table)
                    .add_column(ColumnDef::new(Judges::MaxInteractive).integer().null())
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(Judges::Table)
                    .drop_column(Judges::MaxInteractive)
                    .to_owned(),
            )
            .await?;
        manager
            .alter_table(
                Table::alter()
                    .table(Judges::Table)
                    .drop_column(Judges::ProbesConfig)
                    .to_owned(),
            )
            .await?;
        manager
            .alter_table(
                Table::alter()
                    .table(Judges::Table)
                    .drop_column(Judges::Criteria)
                    .to_owned(),
            )
            .await?;
        manager
            .alter_table(
                Table::alter()
                    .table(JudgeResults::Table)
                    .drop_column(JudgeResults::VerdictKind)
                    .to_owned(),
            )
            .await?;
        manager
            .alter_table(
                Table::alter()
                    .table(Probes::Table)
                    .drop_column(Probes::ArtifactPath)
                    .to_owned(),
            )
            .await?;
        manager
            .alter_table(
                Table::alter()
                    .table(Probes::Table)
                    .drop_column(Probes::ResultJson)
                    .to_owned(),
            )
            .await?;
        manager
            .alter_table(
                Table::alter()
                    .table(Tests::Table)
                    .drop_column(Tests::RegisteredByJudgeId)
                    .to_owned(),
            )
            .await?;
        manager
            .alter_table(
                Table::alter()
                    .table(Tests::Table)
                    .drop_column(Tests::Initiator)
                    .to_owned(),
            )
            .await?;
        manager
            .alter_table(
                Table::alter()
                    .table(Tests::Table)
                    .drop_column(Tests::ProbeConfig)
                    .to_owned(),
            )
            .await?;
        manager
            .alter_table(
                Table::alter()
                    .table(TaskJudges::Table)
                    .drop_column(TaskJudges::Weight)
                    .to_owned(),
            )
            .await?;
        manager
            .alter_table(
                Table::alter()
                    .table(Tasks::Table)
                    .drop_column(Tasks::Evaluation)
                    .to_owned(),
            )
            .await
    }
}

#[derive(DeriveIden)]
enum Tasks {
    Table,
    Evaluation,
}

#[derive(DeriveIden)]
enum TaskJudges {
    Table,
    Weight,
}

#[derive(DeriveIden)]
enum Tests {
    Table,
    ProbeConfig,
    Initiator,
    RegisteredByJudgeId,
}

#[derive(DeriveIden)]
enum Probes {
    Table,
    ResultJson,
    ArtifactPath,
}

#[derive(DeriveIden)]
enum JudgeResults {
    Table,
    VerdictKind,
}

#[derive(DeriveIden)]
enum Judges {
    Table,
    Criteria,
    ProbesConfig,
    MaxInteractive,
}
