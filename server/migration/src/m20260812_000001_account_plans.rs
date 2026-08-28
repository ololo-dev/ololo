use sea_orm_migration::prelude::*;

/// Account plans and the monthly judge-run quota.
///
/// `users.plan` ("free" | "premium") selects which tier limit from
/// `app_settings` applies; `users.judge_run_limit` is an optional per-user
/// override that wins over the tier limit. Existing and new users default to
/// "premium" — enforcement starts permissive until billing exists.
///
/// `judge_run_ledger` is an append-only record of metered judge runs: one row
/// per run of the judge pipeline, charged to the judged player's user account.
/// It is deliberately NOT foreign-keyed to sessions/players — deleting a
/// session must not refund the month's usage — only to `users`, whose
/// deletion takes the ledger with it.
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
                        ColumnDef::new(Users::Plan)
                            .string()
                            .not_null()
                            .default("premium"),
                    )
                    .to_owned(),
            )
            .await?;
        manager
            .alter_table(
                Table::alter()
                    .table(Users::Table)
                    .add_column(ColumnDef::new(Users::JudgeRunLimit).integer().null())
                    .to_owned(),
            )
            .await?;

        manager
            .create_table(
                Table::create()
                    .table(JudgeRunLedger::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(JudgeRunLedger::Id)
                            .uuid()
                            .not_null()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(JudgeRunLedger::UserIdFk).uuid().not_null())
                    .col(ColumnDef::new(JudgeRunLedger::SessionId).uuid().not_null())
                    .col(ColumnDef::new(JudgeRunLedger::PlayerId).uuid().not_null())
                    .col(ColumnDef::new(JudgeRunLedger::JudgeId).uuid().not_null())
                    .col(
                        ColumnDef::new(JudgeRunLedger::CreatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_judge_run_ledger_user")
                            .from(JudgeRunLedger::Table, JudgeRunLedger::UserIdFk)
                            .to(Users::Table, Users::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("ix_judge_run_ledger_user_created")
                    .table(JudgeRunLedger::Table)
                    .col(JudgeRunLedger::UserIdFk)
                    .col(JudgeRunLedger::CreatedAt)
                    .to_owned(),
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(JudgeRunLedger::Table).to_owned())
            .await?;
        manager
            .alter_table(
                Table::alter()
                    .table(Users::Table)
                    .drop_column(Users::JudgeRunLimit)
                    .to_owned(),
            )
            .await?;
        manager
            .alter_table(
                Table::alter()
                    .table(Users::Table)
                    .drop_column(Users::Plan)
                    .to_owned(),
            )
            .await
    }
}

#[derive(DeriveIden)]
enum Users {
    Table,
    Id,
    Plan,
    JudgeRunLimit,
}

#[derive(DeriveIden)]
enum JudgeRunLedger {
    Table,
    Id,
    UserIdFk,
    SessionId,
    PlayerId,
    JudgeId,
    CreatedAt,
}
