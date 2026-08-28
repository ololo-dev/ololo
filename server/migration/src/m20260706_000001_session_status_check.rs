use sea_orm::DatabaseBackend;
use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

const CHECK_CLAUSE: &str = "(status IN ('lobby','running','paused','finished','cancelled'))";

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        match manager.get_database_backend() {
            DatabaseBackend::Postgres => {
                db.execute_unprepared(&format!(
                    "ALTER TABLE sessions ADD CONSTRAINT sessions_status_check CHECK {}",
                    CHECK_CLAUSE
                ))
                .await?;
            }
            DatabaseBackend::Sqlite => {
                sqlite_rebuild_status(manager, db, true).await?;
            }
            _ => {
                return Err(DbErr::Custom(
                    "session_status_check: unsupported backend".into(),
                ));
            }
        }
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        match manager.get_database_backend() {
            DatabaseBackend::Postgres => {
                db.execute_unprepared(
                    "ALTER TABLE sessions DROP CONSTRAINT IF EXISTS sessions_status_check",
                )
                .await?;
            }
            DatabaseBackend::Sqlite => {
                sqlite_rebuild_status(manager, db, false).await?;
            }
            _ => {
                return Err(DbErr::Custom(
                    "session_status_check: unsupported backend".into(),
                ));
            }
        }
        Ok(())
    }
}

async fn sqlite_rebuild_status(
    manager: &SchemaManager<'_>,
    db: &SchemaManagerConnection<'_>,
    with_check: bool,
) -> Result<(), DbErr> {
    // SQLite cannot ALTER TABLE ADD CONSTRAINT — rebuild via temp table.
    // Columns mirror m20260524_000001_squash sessions table exactly.
    let check_col = if with_check {
        format!(
            " status TEXT NOT NULL DEFAULT 'lobby' CHECK {}",
            CHECK_CLAUSE
        )
    } else {
        " status TEXT NOT NULL DEFAULT 'lobby'".to_owned()
    };
    let create_new = format!(
        "CREATE TABLE sessions_new (\n\
         id TEXT NOT NULL PRIMARY KEY,\n\
         name TEXT NOT NULL,\n\
         created_at TEXT NOT NULL,\n\
         owner_id_fk TEXT,\n\
         {},\n\
         join_code TEXT NOT NULL,\n\
         started_at TEXT,\n\
         finished_at TEXT,\n\
         project_id_fk TEXT NOT NULL,\n\
         game_server_id TEXT,\n\
         FOREIGN KEY (project_id_fk) REFERENCES projects(id) ON DELETE CASCADE,\n\
         FOREIGN KEY (game_server_id) REFERENCES game_servers(id) ON DELETE SET NULL\n\
         )",
        check_col
    );
    db.execute_unprepared(&create_new).await?;
    db.execute_unprepared(
        "INSERT INTO sessions_new (id, name, created_at, owner_id_fk, status, join_code, \
         started_at, finished_at, project_id_fk, game_server_id) \
         SELECT id, name, created_at, owner_id_fk, status, join_code, \
         started_at, finished_at, project_id_fk, game_server_id FROM sessions",
    )
    .await?;
    db.execute_unprepared("DROP TABLE sessions").await?;
    db.execute_unprepared("ALTER TABLE sessions_new RENAME TO sessions")
        .await?;

    // Recreate indices dropped with the old table.
    manager
        .create_index(
            Index::create()
                .if_not_exists()
                .name("uq_sessions_join_code")
                .table(Sessions::Table)
                .col(Sessions::JoinCode)
                .unique()
                .to_owned(),
        )
        .await?;
    manager
        .create_index(
            Index::create()
                .if_not_exists()
                .name("idx_sessions_project_status")
                .table(Sessions::Table)
                .col(Sessions::ProjectIdFk)
                .col(Sessions::Status)
                .to_owned(),
        )
        .await?;
    Ok(())
}

#[derive(DeriveIden)]
enum Sessions {
    Table,
    JoinCode,
    ProjectIdFk,
    Status,
}
