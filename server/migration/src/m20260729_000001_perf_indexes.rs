//! Indexes on hot predicates (DB-H2).
//!
//! - `sessions.game_server_id`: read every ~5s by the heartbeat/session-count
//!   path and by session assignment; previously a full scan.
//! - `sessions.finished_at`: filtered by finished-session queries.
//! - `game_servers.url` unique: the heartbeat lookup-by-url branch was racy
//!   without a unique constraint (ARCH-M2); this also enforces one row per URL.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_index(
                Index::create()
                    .if_not_exists()
                    .name("idx_sessions_game_server_id")
                    .table(Sessions::Table)
                    .col(Sessions::GameServerId)
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .if_not_exists()
                    .name("idx_sessions_finished_at")
                    .table(Sessions::Table)
                    .col(Sessions::FinishedAt)
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .if_not_exists()
                    .name("ux_game_servers_url")
                    .table(GameServers::Table)
                    .col(GameServers::Url)
                    .unique()
                    .to_owned(),
            )
            .await?;
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_index(
                Index::drop()
                    .name("ux_game_servers_url")
                    .table(GameServers::Table)
                    .to_owned(),
            )
            .await?;
        manager
            .drop_index(
                Index::drop()
                    .name("idx_sessions_finished_at")
                    .table(Sessions::Table)
                    .to_owned(),
            )
            .await?;
        manager
            .drop_index(
                Index::drop()
                    .name("idx_sessions_game_server_id")
                    .table(Sessions::Table)
                    .to_owned(),
            )
            .await?;
        Ok(())
    }
}

#[derive(DeriveIden)]
enum Sessions {
    Table,
    GameServerId,
    FinishedAt,
}

#[derive(DeriveIden)]
enum GameServers {
    Table,
    Url,
}
