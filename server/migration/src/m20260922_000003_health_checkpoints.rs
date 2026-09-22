//! Code-health checkpoints: one row per snapshot commit the game scores —
//! a `probe(<task>)` commit the agent made when a probe arrived, or the
//! `feat(<task>)` commit that closed a task. Both sides' numbers live on
//! the row: the client's report (advisory) and the server's own
//! verification of the same commit (authoritative), with the flags a
//! reader needs before comparing them.
//!
//! Keyed by (session, player, commit): a commit is scored once, however
//! many reports mention it. The `server_status` index serves the recovery
//! sweep that re-drives rows left `pending` by a restart.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(HealthCheckpoints::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(HealthCheckpoints::Id)
                            .uuid()
                            .not_null()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(HealthCheckpoints::SessionIdFk)
                            .uuid()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(HealthCheckpoints::PlayerIdFk)
                            .uuid()
                            .not_null(),
                    )
                    .col(ColumnDef::new(HealthCheckpoints::TaskIdFk).uuid().null())
                    .col(ColumnDef::new(HealthCheckpoints::ProbeIdFk).uuid().null())
                    .col(ColumnDef::new(HealthCheckpoints::Kind).string().not_null())
                    .col(
                        ColumnDef::new(HealthCheckpoints::ProbeSeq)
                            .integer()
                            .not_null()
                            .default(0),
                    )
                    .col(
                        ColumnDef::new(HealthCheckpoints::CommitSha)
                            .string()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(HealthCheckpoints::DerivedTaskId)
                            .uuid()
                            .null(),
                    )
                    .col(
                        ColumnDef::new(HealthCheckpoints::ScoreMismatch)
                            .boolean()
                            .not_null()
                            .default(false),
                    )
                    .col(
                        ColumnDef::new(HealthCheckpoints::VersionMismatch)
                            .boolean()
                            .not_null()
                            .default(false),
                    )
                    .col(
                        ColumnDef::new(HealthCheckpoints::TaskMismatch)
                            .boolean()
                            .not_null()
                            .default(false),
                    )
                    .col(
                        ColumnDef::new(HealthCheckpoints::Late)
                            .boolean()
                            .not_null()
                            .default(false),
                    )
                    .col(
                        ColumnDef::new(HealthCheckpoints::HistoryRewritten)
                            .boolean()
                            .not_null()
                            .default(false),
                    )
                    .col(
                        ColumnDef::new(HealthCheckpoints::ClientStatus)
                            .string()
                            .null(),
                    )
                    .col(
                        ColumnDef::new(HealthCheckpoints::ClientScore)
                            .double()
                            .null(),
                    )
                    .col(
                        ColumnDef::new(HealthCheckpoints::ClientGrade)
                            .string()
                            .null(),
                    )
                    .col(
                        ColumnDef::new(HealthCheckpoints::ClientResult)
                            .json_binary()
                            .null(),
                    )
                    .col(
                        ColumnDef::new(HealthCheckpoints::ClientDurationMs)
                            .big_integer()
                            .null(),
                    )
                    .col(
                        ColumnDef::new(HealthCheckpoints::ClientJscpdVersion)
                            .string()
                            .null(),
                    )
                    .col(ColumnDef::new(HealthCheckpoints::ClientError).text().null())
                    .col(
                        ColumnDef::new(HealthCheckpoints::ClientReportedAt)
                            .timestamp_with_time_zone()
                            .null(),
                    )
                    .col(
                        ColumnDef::new(HealthCheckpoints::ServerStatus)
                            .string()
                            .not_null()
                            .default("pending"),
                    )
                    .col(
                        ColumnDef::new(HealthCheckpoints::ServerScore)
                            .double()
                            .null(),
                    )
                    .col(
                        ColumnDef::new(HealthCheckpoints::ServerGrade)
                            .string()
                            .null(),
                    )
                    .col(
                        ColumnDef::new(HealthCheckpoints::ServerResult)
                            .json_binary()
                            .null(),
                    )
                    .col(
                        ColumnDef::new(HealthCheckpoints::ServerDurationMs)
                            .big_integer()
                            .null(),
                    )
                    .col(
                        ColumnDef::new(HealthCheckpoints::ServerJscpdVersion)
                            .string()
                            .null(),
                    )
                    .col(ColumnDef::new(HealthCheckpoints::ServerError).text().null())
                    .col(
                        ColumnDef::new(HealthCheckpoints::ServerVerifiedAt)
                            .timestamp_with_time_zone()
                            .null(),
                    )
                    .col(
                        ColumnDef::new(HealthCheckpoints::CreatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(HealthCheckpoints::UpdatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_health_checkpoints_session")
                            .from(HealthCheckpoints::Table, HealthCheckpoints::SessionIdFk)
                            .to(Sessions::Table, Sessions::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_health_checkpoints_player")
                            .from(HealthCheckpoints::Table, HealthCheckpoints::PlayerIdFk)
                            .to(Players::Table, Players::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_health_checkpoints_task")
                            .from(HealthCheckpoints::Table, HealthCheckpoints::TaskIdFk)
                            .to(Tasks::Table, Tasks::Id)
                            .on_delete(ForeignKeyAction::SetNull),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_health_checkpoints_probe")
                            .from(HealthCheckpoints::Table, HealthCheckpoints::ProbeIdFk)
                            .to(Probes::Table, Probes::Id)
                            .on_delete(ForeignKeyAction::SetNull),
                    )
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name("ux_health_checkpoints_commit")
                    .table(HealthCheckpoints::Table)
                    .col(HealthCheckpoints::SessionIdFk)
                    .col(HealthCheckpoints::PlayerIdFk)
                    .col(HealthCheckpoints::CommitSha)
                    .unique()
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name("ix_health_checkpoints_player_time")
                    .table(HealthCheckpoints::Table)
                    .col(HealthCheckpoints::SessionIdFk)
                    .col(HealthCheckpoints::PlayerIdFk)
                    .col(HealthCheckpoints::CreatedAt)
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name("ix_health_checkpoints_server_status")
                    .table(HealthCheckpoints::Table)
                    .col(HealthCheckpoints::ServerStatus)
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(HealthCheckpoints::Table).to_owned())
            .await
    }
}

#[derive(DeriveIden)]
enum HealthCheckpoints {
    Table,
    Id,
    SessionIdFk,
    PlayerIdFk,
    TaskIdFk,
    ProbeIdFk,
    Kind,
    ProbeSeq,
    CommitSha,
    DerivedTaskId,
    ScoreMismatch,
    VersionMismatch,
    TaskMismatch,
    Late,
    HistoryRewritten,
    ClientStatus,
    ClientScore,
    ClientGrade,
    ClientResult,
    ClientDurationMs,
    ClientJscpdVersion,
    ClientError,
    ClientReportedAt,
    ServerStatus,
    ServerScore,
    ServerGrade,
    ServerResult,
    ServerDurationMs,
    ServerJscpdVersion,
    ServerError,
    ServerVerifiedAt,
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
enum Tasks {
    Table,
    Id,
}

#[derive(DeriveIden)]
enum Probes {
    Table,
    Id,
}
