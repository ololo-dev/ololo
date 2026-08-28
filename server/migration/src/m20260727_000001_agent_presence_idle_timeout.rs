use sea_orm_migration::prelude::*;

/// Agent presence + idle timeout.
///
/// `players.agent_connected` / `players.agent_last_seen_at`: whether the
/// player's ololo agent socket is currently connected to the game server, and
/// when it was last seen. The web player page used to derive its "Live" badge
/// purely from the session status, so it kept claiming Live long after the
/// client dropped. Persisted (not just broadcast) so the snapshot builder in
/// the `server` process — which cannot see the game server's in-memory
/// registry — renders the truth on page load, and so the idle sweep below has
/// something durable to reason over across restarts.
///
/// `projects.idle_timeout_secs`: cancel a running session once it has had no
/// connected agents for this long. 0 disables the sweep for the project.
///
/// One `alter_table` per column — SQLite rejects multi-column alters at
/// runtime (constitution: sqlite-alter-table-one-column-per-call).
#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(Players::Table)
                    .add_column(
                        ColumnDef::new(Players::AgentConnected)
                            .boolean()
                            .not_null()
                            .default(false),
                    )
                    .to_owned(),
            )
            .await?;
        manager
            .alter_table(
                Table::alter()
                    .table(Players::Table)
                    .add_column(
                        ColumnDef::new(Players::AgentLastSeenAt)
                            .timestamp_with_time_zone()
                            .null(),
                    )
                    .to_owned(),
            )
            .await?;
        manager
            .alter_table(
                Table::alter()
                    .table(Projects::Table)
                    .add_column(
                        ColumnDef::new(Projects::IdleTimeoutSecs)
                            .integer()
                            .not_null()
                            .default(300),
                    )
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(Players::Table)
                    .drop_column(Players::AgentConnected)
                    .to_owned(),
            )
            .await?;
        manager
            .alter_table(
                Table::alter()
                    .table(Players::Table)
                    .drop_column(Players::AgentLastSeenAt)
                    .to_owned(),
            )
            .await?;
        manager
            .alter_table(
                Table::alter()
                    .table(Projects::Table)
                    .drop_column(Projects::IdleTimeoutSecs)
                    .to_owned(),
            )
            .await
    }
}

#[derive(DeriveIden)]
enum Players {
    Table,
    AgentConnected,
    AgentLastSeenAt,
}

#[derive(DeriveIden)]
enum Projects {
    Table,
    IdleTimeoutSecs,
}
