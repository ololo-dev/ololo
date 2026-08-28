use sea_orm_migration::prelude::*;

/// `judges.ignore_paths` — repo paths a judge's git tools must not open.
///
/// The player's snapshot carries the platform's own runtime tree under
/// `.ololo/` (delivered artifacts, completion flags, scratch data-dirs a
/// probe's fixtures wrote). Some judges need it — the UX review reads the
/// screenshots and screencasts delivered there. Others must not spend a
/// single token on it: the From Scratch judge asks only whether the player
/// implemented the tool or wrapped the real one, and in a five-part campaign
/// `.ololo/tmp` was 468 of the repo's 494 files, so the file listing it
/// re-read on every task was 93% scratch data.
///
/// JSON array of path prefixes, NULL when the judge sees the whole snapshot.
#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(Judges::Table)
                    .add_column(ColumnDef::new(Judges::IgnorePaths).text().null())
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(Judges::Table)
                    .drop_column(Judges::IgnorePaths)
                    .to_owned(),
            )
            .await
    }
}

#[derive(DeriveIden)]
enum Judges {
    Table,
    IgnorePaths,
}
