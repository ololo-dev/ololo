use arena_core::session_status::SessionStatus;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use chrono::Utc;
use migration::{Migrator, MigratorTrait};
use sea_orm::{ActiveModelTrait, Database, Set};

use super::*;

async fn fresh_state() -> AppState {
    let db = Database::connect("sqlite::memory:")
        .await
        .expect("db connect");
    Migrator::up(&db, None).await.expect("migrate");
    let cfg = crate::AuthConfig {
        jwt_signing_key: b"players-tests-secret-key-32-bytes!!".to_vec(),
        frontend_origins: vec!["http://localhost:5173".to_string()],
        access_ttl: std::time::Duration::from_secs(900),
        refresh_ttl: std::time::Duration::from_secs(86400),
        max_agents_per_session: 16,
    };
    AppState::new(db, cfg)
}

mod build_snapshot_a;
mod build_snapshot_b;
mod build_snapshot_c;
mod completion_status;
mod errors;
mod session_duration;
