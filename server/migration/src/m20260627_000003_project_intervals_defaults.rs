use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();

        // --- Project default interval columns (NOT NULL, baselines 60/5/5/60) ---
        manager
            .alter_table(
                Table::alter()
                    .table(Projects::Table)
                    .add_column(
                        ColumnDef::new(Projects::DefaultDeadlineSecs)
                            .big_integer()
                            .not_null()
                            .default(60),
                    )
                    .to_owned(),
            )
            .await?;
        manager
            .alter_table(
                Table::alter()
                    .table(Projects::Table)
                    .add_column(
                        ColumnDef::new(Projects::DefaultMinIntervalSecs)
                            .integer()
                            .not_null()
                            .default(5),
                    )
                    .to_owned(),
            )
            .await?;
        manager
            .alter_table(
                Table::alter()
                    .table(Projects::Table)
                    .add_column(
                        ColumnDef::new(Projects::DefaultIntervalIncrementSecs)
                            .integer()
                            .not_null()
                            .default(5),
                    )
                    .to_owned(),
            )
            .await?;
        manager
            .alter_table(
                Table::alter()
                    .table(Projects::Table)
                    .add_column(
                        ColumnDef::new(Projects::DefaultMaxIntervalSecs)
                            .integer()
                            .not_null()
                            .default(60),
                    )
                    .to_owned(),
            )
            .await?;

        // --- Task interval columns: NOT NULL → NULLable ---
        // SQLite lacks MODIFY COLUMN; use add-copy-drop-rename per column.
        // deadline_secs (i64)
        manager
            .alter_table(
                Table::alter()
                    .table(Tasks::Table)
                    .add_column(ColumnDef::new(Tasks::DeadlineSecsNew).big_integer().null())
                    .to_owned(),
            )
            .await?;
        db.execute_unprepared("UPDATE tasks SET deadline_secs_new = deadline_secs")
            .await?;
        manager
            .alter_table(
                Table::alter()
                    .table(Tasks::Table)
                    .drop_column(Tasks::DeadlineSecs)
                    .to_owned(),
            )
            .await?;
        manager
            .alter_table(
                Table::alter()
                    .table(Tasks::Table)
                    .rename_column(Tasks::DeadlineSecsNew, Tasks::DeadlineSecs)
                    .to_owned(),
            )
            .await?;

        // min_interval_secs (i32)
        manager
            .alter_table(
                Table::alter()
                    .table(Tasks::Table)
                    .add_column(ColumnDef::new(Tasks::MinIntervalSecsNew).integer().null())
                    .to_owned(),
            )
            .await?;
        db.execute_unprepared("UPDATE tasks SET min_interval_secs_new = min_interval_secs")
            .await?;
        manager
            .alter_table(
                Table::alter()
                    .table(Tasks::Table)
                    .drop_column(Tasks::MinIntervalSecs)
                    .to_owned(),
            )
            .await?;
        manager
            .alter_table(
                Table::alter()
                    .table(Tasks::Table)
                    .rename_column(Tasks::MinIntervalSecsNew, Tasks::MinIntervalSecs)
                    .to_owned(),
            )
            .await?;

        // interval_increment_secs (i32)
        manager
            .alter_table(
                Table::alter()
                    .table(Tasks::Table)
                    .add_column(
                        ColumnDef::new(Tasks::IntervalIncrementSecsNew)
                            .integer()
                            .null(),
                    )
                    .to_owned(),
            )
            .await?;
        db.execute_unprepared(
            "UPDATE tasks SET interval_increment_secs_new = interval_increment_secs",
        )
        .await?;
        manager
            .alter_table(
                Table::alter()
                    .table(Tasks::Table)
                    .drop_column(Tasks::IntervalIncrementSecs)
                    .to_owned(),
            )
            .await?;
        manager
            .alter_table(
                Table::alter()
                    .table(Tasks::Table)
                    .rename_column(
                        Tasks::IntervalIncrementSecsNew,
                        Tasks::IntervalIncrementSecs,
                    )
                    .to_owned(),
            )
            .await?;

        // max_interval_secs (i32)
        manager
            .alter_table(
                Table::alter()
                    .table(Tasks::Table)
                    .add_column(ColumnDef::new(Tasks::MaxIntervalSecsNew).integer().null())
                    .to_owned(),
            )
            .await?;
        db.execute_unprepared("UPDATE tasks SET max_interval_secs_new = max_interval_secs")
            .await?;
        manager
            .alter_table(
                Table::alter()
                    .table(Tasks::Table)
                    .drop_column(Tasks::MaxIntervalSecs)
                    .to_owned(),
            )
            .await?;
        manager
            .alter_table(
                Table::alter()
                    .table(Tasks::Table)
                    .rename_column(Tasks::MaxIntervalSecsNew, Tasks::MaxIntervalSecs)
                    .to_owned(),
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();

        // Drop project default columns
        for col in [
            Projects::DefaultDeadlineSecs,
            Projects::DefaultMinIntervalSecs,
            Projects::DefaultIntervalIncrementSecs,
            Projects::DefaultMaxIntervalSecs,
        ] {
            manager
                .alter_table(
                    Table::alter()
                        .table(Projects::Table)
                        .drop_column(col)
                        .to_owned(),
                )
                .await?;
        }

        // Restore task columns to NOT NULL (best-effort: values preserved, NULL → 0)
        // deadline_secs
        manager
            .alter_table(
                Table::alter()
                    .table(Tasks::Table)
                    .add_column(
                        ColumnDef::new(Tasks::DeadlineSecsNew)
                            .big_integer()
                            .not_null()
                            .default(0),
                    )
                    .to_owned(),
            )
            .await?;
        db.execute_unprepared("UPDATE tasks SET deadline_secs_new = COALESCE(deadline_secs, 0)")
            .await?;
        manager
            .alter_table(
                Table::alter()
                    .table(Tasks::Table)
                    .drop_column(Tasks::DeadlineSecs)
                    .to_owned(),
            )
            .await?;
        manager
            .alter_table(
                Table::alter()
                    .table(Tasks::Table)
                    .rename_column(Tasks::DeadlineSecsNew, Tasks::DeadlineSecs)
                    .to_owned(),
            )
            .await?;

        // min_interval_secs
        manager
            .alter_table(
                Table::alter()
                    .table(Tasks::Table)
                    .add_column(
                        ColumnDef::new(Tasks::MinIntervalSecsNew)
                            .integer()
                            .not_null()
                            .default(0),
                    )
                    .to_owned(),
            )
            .await?;
        db.execute_unprepared(
            "UPDATE tasks SET min_interval_secs_new = COALESCE(min_interval_secs, 0)",
        )
        .await?;
        manager
            .alter_table(
                Table::alter()
                    .table(Tasks::Table)
                    .drop_column(Tasks::MinIntervalSecs)
                    .to_owned(),
            )
            .await?;
        manager
            .alter_table(
                Table::alter()
                    .table(Tasks::Table)
                    .rename_column(Tasks::MinIntervalSecsNew, Tasks::MinIntervalSecs)
                    .to_owned(),
            )
            .await?;

        // interval_increment_secs
        manager
            .alter_table(
                Table::alter()
                    .table(Tasks::Table)
                    .add_column(
                        ColumnDef::new(Tasks::IntervalIncrementSecsNew)
                            .integer()
                            .not_null()
                            .default(0),
                    )
                    .to_owned(),
            )
            .await?;
        db.execute_unprepared(
            "UPDATE tasks SET interval_increment_secs_new = COALESCE(interval_increment_secs, 0)",
        )
        .await?;
        manager
            .alter_table(
                Table::alter()
                    .table(Tasks::Table)
                    .drop_column(Tasks::IntervalIncrementSecs)
                    .to_owned(),
            )
            .await?;
        manager
            .alter_table(
                Table::alter()
                    .table(Tasks::Table)
                    .rename_column(
                        Tasks::IntervalIncrementSecsNew,
                        Tasks::IntervalIncrementSecs,
                    )
                    .to_owned(),
            )
            .await?;

        // max_interval_secs
        manager
            .alter_table(
                Table::alter()
                    .table(Tasks::Table)
                    .add_column(
                        ColumnDef::new(Tasks::MaxIntervalSecsNew)
                            .integer()
                            .not_null()
                            .default(0),
                    )
                    .to_owned(),
            )
            .await?;
        db.execute_unprepared(
            "UPDATE tasks SET max_interval_secs_new = COALESCE(max_interval_secs, 0)",
        )
        .await?;
        manager
            .alter_table(
                Table::alter()
                    .table(Tasks::Table)
                    .drop_column(Tasks::MaxIntervalSecs)
                    .to_owned(),
            )
            .await?;
        manager
            .alter_table(
                Table::alter()
                    .table(Tasks::Table)
                    .rename_column(Tasks::MaxIntervalSecsNew, Tasks::MaxIntervalSecs)
                    .to_owned(),
            )
            .await?;

        Ok(())
    }
}

#[derive(DeriveIden)]
enum Projects {
    Table,
    DefaultDeadlineSecs,
    DefaultMinIntervalSecs,
    DefaultIntervalIncrementSecs,
    DefaultMaxIntervalSecs,
}

#[derive(DeriveIden)]
enum Tasks {
    Table,
    DeadlineSecs,
    DeadlineSecsNew,
    MinIntervalSecs,
    MinIntervalSecsNew,
    IntervalIncrementSecs,
    IntervalIncrementSecsNew,
    MaxIntervalSecs,
    MaxIntervalSecsNew,
}
