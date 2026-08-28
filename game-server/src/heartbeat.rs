use anyhow::Result;
use arena_core::entities::game_servers;
use chrono::Utc;
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter, Set};
use uuid::Uuid;

const ADMIN_OVERRIDDEN_STATUSES: &[&str] = &["inactive", "draining"];

pub async fn register(
    db: &sea_orm::DatabaseConnection,
    server_id: Uuid,
    advertise_url: &str,
    capacity: i32,
    zmq_url: Option<&str>,
) -> Result<()> {
    let existing_by_id = game_servers::Entity::find_by_id(server_id).one(db).await?;

    if let Some(model) = existing_by_id {
        let current_status = model.status.as_str();
        let preserve_status = ADMIN_OVERRIDDEN_STATUSES.contains(&current_status);
        let mut am: game_servers::ActiveModel = model.into();
        am.url = Set(advertise_url.to_string());
        am.capacity = Set(capacity);
        am.zmq_url = Set(zmq_url.map(|s| s.to_string()));
        if !preserve_status {
            am.status = Set("active".to_string());
        }
        am.last_heartbeat = Set(Utc::now());
        am.updated_at = Set(Utc::now());
        am.update(db).await?;
    } else {
        let existing_by_url = game_servers::Entity::find()
            .filter(game_servers::Column::Url.eq(advertise_url))
            .one(db)
            .await?;

        if let Some(model) = existing_by_url {
            let old_id = model.id;
            let preserved_display_name = model.display_name.clone();
            let preserved_status = model.status.clone();
            game_servers::Entity::delete_by_id(old_id).exec(db).await?;
            let status = if ADMIN_OVERRIDDEN_STATUSES.contains(&preserved_status.as_str()) {
                preserved_status
            } else {
                "active".to_string()
            };
            let am = game_servers::ActiveModel {
                id: Set(server_id),
                url: Set(advertise_url.to_string()),
                zmq_url: Set(zmq_url.map(|s| s.to_string())),
                display_name: Set(preserved_display_name),
                capacity: Set(capacity),
                active_sessions: Set(0),
                status: Set(status),
                last_heartbeat: Set(Utc::now()),
                created_at: Set(Utc::now()),
                updated_at: Set(Utc::now()),
            };
            am.insert(db).await?;
        } else {
            let am = game_servers::ActiveModel {
                id: Set(server_id),
                url: Set(advertise_url.to_string()),
                zmq_url: Set(zmq_url.map(|s| s.to_string())),
                display_name: Set(None),
                capacity: Set(capacity),
                active_sessions: Set(0),
                status: Set("active".to_string()),
                last_heartbeat: Set(Utc::now()),
                created_at: Set(Utc::now()),
                updated_at: Set(Utc::now()),
            };
            am.insert(db).await?;
        }
    }

    tracing::info!(
        "registered game server id={} url={}",
        server_id,
        advertise_url
    );
    Ok(())
}

pub async fn send_heartbeat(db: &sea_orm::DatabaseConnection, server_id: Uuid) -> Result<()> {
    let active_count = arena_core::entities::sessions::Entity::find()
        .filter(arena_core::entities::sessions::Column::GameServerId.eq(server_id))
        .filter(
            arena_core::entities::sessions::Column::Status
                .eq(arena_core::session_status::SessionStatus::Running),
        )
        .count(db)
        .await? as i32;

    let existing = game_servers::Entity::find_by_id(server_id).one(db).await?;

    if let Some(model) = existing {
        let mut am: game_servers::ActiveModel = model.into();
        am.last_heartbeat = Set(Utc::now());
        am.active_sessions = Set(active_count);
        am.updated_at = Set(Utc::now());
        am.update(db).await?;
    }

    Ok(())
}

pub async fn run_heartbeat(db: sea_orm::DatabaseConnection, server_id: Uuid) {
    let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(5));
    loop {
        interval.tick().await;
        if let Err(e) = send_heartbeat(&db, server_id).await {
            tracing::warn!("heartbeat failed: {}", e);
        }
    }
}
