use anyhow::Result;
use sea_orm::DatabaseConnection;

pub async fn connect() -> Result<DatabaseConnection> {
    let url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "sqlite://./arena.db?mode=rwc".to_string());
    Ok(migration::connect_and_migrate(&url).await?)
}
