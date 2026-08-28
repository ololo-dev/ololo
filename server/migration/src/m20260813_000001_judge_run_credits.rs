use sea_orm_migration::prelude::*;

/// Purchased judge-review packs.
///
/// `users.judge_run_credits` is a balance of one-off review credits bought
/// in packs (1000/2000/5000…). The monthly tier allowance is spent first;
/// once the calendar-month usage reaches the limit, each further run
/// decrements this balance. Credits do not reset with the month.
///
/// `judge_run_ledger.source` records which pool a metered run consumed:
/// `monthly` (the tier allowance) or `pack` (purchased credits) — the
/// distinction billing and refunds will need.
#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(Users::Table)
                    .add_column(
                        ColumnDef::new(Users::JudgeRunCredits)
                            .big_integer()
                            .not_null()
                            .default(0),
                    )
                    .to_owned(),
            )
            .await?;
        manager
            .alter_table(
                Table::alter()
                    .table(JudgeRunLedger::Table)
                    .add_column(
                        ColumnDef::new(JudgeRunLedger::Source)
                            .string()
                            .not_null()
                            .default("monthly"),
                    )
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(JudgeRunLedger::Table)
                    .drop_column(JudgeRunLedger::Source)
                    .to_owned(),
            )
            .await?;
        manager
            .alter_table(
                Table::alter()
                    .table(Users::Table)
                    .drop_column(Users::JudgeRunCredits)
                    .to_owned(),
            )
            .await
    }
}

#[derive(DeriveIden)]
enum Users {
    Table,
    JudgeRunCredits,
}

#[derive(DeriveIden)]
enum JudgeRunLedger {
    Table,
    Source,
}
