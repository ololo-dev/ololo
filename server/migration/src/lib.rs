pub use sea_orm_migration::prelude::*;

pub mod m20260524_000001_squash;
pub mod m20260525_000001_session_execution_engine;
pub mod m20260525_000002_fix_task_results_schema;
pub mod m20260526_000001_drop_adapted_tests_command;
pub mod m20260527_000001_simplify_adapted_schema;
pub mod m20260528_000001_scoring_refactor;
pub mod m20260528_000002_game_servers;
pub mod m20260529_000001_probes_updated_at;
pub mod m20260531_000001_scheduler_next_probe_at;
pub mod m20260603_000001_game_servers_zmq_url;
pub mod m20260627_000001_project_points_defaults;
pub mod m20260627_000002_drop_pass_points;
pub mod m20260627_000003_project_intervals_defaults;
pub mod m20260706_000001_session_status_check;
pub mod m20260706_000002_session_pause_columns;
pub mod m20260707_000001_probes_resolved_answer;
pub mod m20260707_000002_probes_secret_meta;
pub mod m20260708_000001_judges;
pub mod m20260708_000002_judge_results;
pub mod m20260716_000001_activity_events;
pub mod m20260719_000001_judge_results_duration;
pub mod m20260719_000002_task_agent_stats;
pub mod m20260721_000001_judge_results_status;
pub mod m20260723_000001_judge_results_run_detail;
pub mod m20260724_000001_judge_results_cache_tokens;
pub mod m20260724_000001_project_session_duration;
pub mod m20260726_000001_judge_kind;
pub mod m20260726_000002_judge_scope;
pub mod m20260727_000001_agent_presence_idle_timeout;
pub mod m20260727_000002_session_cancel_reason;
pub mod m20260729_000001_perf_indexes;
pub mod m20260729_000002_completion_bonus_unique;
pub mod m20260730_000001_player_memory;
pub mod m20260730_000002_llm_providers;
pub mod m20260730_000003_llm_requests;
pub mod m20260730_000004_llm_requests_events;
pub mod m20260731_000001_judges_evidence_mode;
pub mod m20260731_000002_llm_pools;
pub mod m20260731_000003_judges_llm_pool;
pub mod m20260731_000004_llm_requests_provider_name;
mod m20260803_000001_judges_evidence_needs;
mod m20260804_000003_open_ended_tasks;
mod m20260805_000001_activity_detail;
mod m20260806_000001_judge_avatar;
mod m20260812_000001_account_plans;
mod m20260813_000001_judge_run_credits;
mod m20260813_000002_similarity_reports;
mod m20260813_000003_artifact_watchers;
mod m20260815_000001_tests_description;
mod m20260818_000001_projects_show_tasks;
mod m20260820_000001_projects_campaign_parts;
mod m20260822_000001_judges_ignore_paths;
mod m20260904_000001_judge_run_transcripts;

/// Advisory-lock key serialising `Migrator::up` across processes on Postgres.
/// Arbitrary but must be identical in every binary that runs migrations.
const MIGRATION_LOCK_KEY: i64 = 0x6f_6c6f_6c6f; // "ololo"

/// Connect to `url` and run all pending migrations, then return the pool.
///
/// `server` and `game-server` both migrate on boot. SQLite serialises writes
/// on its own, but on Postgres two containers starting together would race
/// each other's DDL — so there the migration runs under a session advisory
/// lock held on a dedicated single-connection pool (session locks live on one
/// specific connection; the shared pool can't guarantee that).
pub async fn connect_and_migrate(url: &str) -> Result<sea_orm::DatabaseConnection, sea_orm::DbErr> {
    use sea_orm::{ConnectOptions, ConnectionTrait, Database, DatabaseBackend, Statement};

    // Pool size. sqlx's default is 10; that starves the per-second pollers,
    // session timers, and request handlers on Postgres, so raise it there.
    // SQLite keeps the default — extra connections only pile up on its single
    // writer lock. `ARENA_DB_MAX_CONNECTIONS` overrides either default.
    let is_sqlite = url.trim_start().starts_with("sqlite");
    let max_connections: u32 = std::env::var("ARENA_DB_MAX_CONNECTIONS")
        .ok()
        .and_then(|v| v.parse().ok())
        .filter(|n: &u32| *n > 0)
        .unwrap_or(if is_sqlite { 10 } else { 30 });

    let mut opts = ConnectOptions::new(url.to_owned());
    opts.sqlx_logging(false).max_connections(max_connections);
    let db = Database::connect(opts).await?;

    if db.get_database_backend() == DatabaseBackend::Postgres {
        let mut lock_opts = ConnectOptions::new(url.to_owned());
        lock_opts.sqlx_logging(false).max_connections(1);
        let lock_conn = Database::connect(lock_opts).await?;
        lock_conn
            .execute(Statement::from_string(
                DatabaseBackend::Postgres,
                format!("SELECT pg_advisory_lock({MIGRATION_LOCK_KEY})"),
            ))
            .await?;
        let result = Migrator::up(&db, None).await;
        // Closing the connection releases the advisory lock even when the
        // migration itself failed.
        let _ = lock_conn.close().await;
        result?;
    } else {
        // Migrate over a dedicated single-connection pool. A migration is
        // not one statement: the sessions-rebuild (m20260706) mixes
        // transaction-scoped `execute_unprepared` with `manager` calls that
        // draw from the pool, and on a multi-connection SQLite pool those
        // land on DIFFERENT connections — the rebuild's DROP/RENAME raced
        // its own index recreation and every fresh file-backed boot died
        // with "there is already another table or index with this name:
        // sessions". `sqlite::memory:` never showed it (a memory DB forces
        // one connection per pool), which is why the test suite stayed
        // green while `dev-play.sh up --fresh` was broken.
        let mut mig_opts = ConnectOptions::new(url.to_owned());
        mig_opts.sqlx_logging(false).max_connections(1);
        let mig_conn = Database::connect(mig_opts).await?;
        let result = Migrator::up(&mig_conn, None).await;
        let _ = mig_conn.close().await;
        result?;
    }
    Ok(db)
}

pub struct Migrator;

#[async_trait::async_trait]
impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![
            Box::new(m20260524_000001_squash::Migration),
            Box::new(m20260525_000001_session_execution_engine::Migration),
            Box::new(m20260525_000002_fix_task_results_schema::Migration),
            Box::new(m20260526_000001_drop_adapted_tests_command::Migration),
            Box::new(m20260527_000001_simplify_adapted_schema::Migration),
            Box::new(m20260528_000001_scoring_refactor::Migration),
            Box::new(m20260528_000002_game_servers::Migration),
            Box::new(m20260529_000001_probes_updated_at::Migration),
            Box::new(m20260531_000001_scheduler_next_probe_at::Migration),
            Box::new(m20260603_000001_game_servers_zmq_url::Migration),
            Box::new(m20260627_000001_project_points_defaults::Migration),
            Box::new(m20260627_000002_drop_pass_points::Migration),
            Box::new(m20260627_000003_project_intervals_defaults::Migration),
            Box::new(m20260706_000001_session_status_check::Migration),
            Box::new(m20260706_000002_session_pause_columns::Migration),
            Box::new(m20260707_000001_probes_resolved_answer::Migration),
            Box::new(m20260707_000002_probes_secret_meta::Migration),
            Box::new(m20260708_000001_judges::Migration),
            Box::new(m20260708_000002_judge_results::Migration),
            Box::new(m20260716_000001_activity_events::Migration),
            Box::new(m20260719_000001_judge_results_duration::Migration),
            Box::new(m20260719_000002_task_agent_stats::Migration),
            Box::new(m20260721_000001_judge_results_status::Migration),
            Box::new(m20260723_000001_judge_results_run_detail::Migration),
            Box::new(m20260724_000001_judge_results_cache_tokens::Migration),
            Box::new(m20260724_000001_project_session_duration::Migration),
            Box::new(m20260726_000001_judge_kind::Migration),
            Box::new(m20260726_000002_judge_scope::Migration),
            Box::new(m20260727_000001_agent_presence_idle_timeout::Migration),
            Box::new(m20260727_000002_session_cancel_reason::Migration),
            Box::new(m20260729_000001_perf_indexes::Migration),
            Box::new(m20260729_000002_completion_bonus_unique::Migration),
            Box::new(m20260730_000001_player_memory::Migration),
            Box::new(m20260730_000002_llm_providers::Migration),
            Box::new(m20260730_000003_llm_requests::Migration),
            Box::new(m20260730_000004_llm_requests_events::Migration),
            Box::new(m20260731_000001_judges_evidence_mode::Migration),
            Box::new(m20260731_000002_llm_pools::Migration),
            Box::new(m20260731_000003_judges_llm_pool::Migration),
            Box::new(m20260731_000004_llm_requests_provider_name::Migration),
            Box::new(m20260803_000001_judges_evidence_needs::Migration),
            Box::new(m20260804_000003_open_ended_tasks::Migration),
            Box::new(m20260805_000001_activity_detail::Migration),
            Box::new(m20260806_000001_judge_avatar::Migration),
            Box::new(m20260812_000001_account_plans::Migration),
            Box::new(m20260813_000001_judge_run_credits::Migration),
            Box::new(m20260813_000002_similarity_reports::Migration),
            Box::new(m20260813_000003_artifact_watchers::Migration),
            Box::new(m20260815_000001_tests_description::Migration),
            Box::new(m20260818_000001_projects_show_tasks::Migration),
            Box::new(m20260820_000001_projects_campaign_parts::Migration),
            Box::new(m20260822_000001_judges_ignore_paths::Migration),
            Box::new(m20260904_000001_judge_run_transcripts::Migration),
        ]
    }
}
