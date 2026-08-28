//! Player-scoped HTTP handlers and helpers.
//!
//! Public surface (consumed by `lib.rs` router assembly and `ws/player.rs`):
//! - [`get_player_snapshot`], [`get_player_history`] — axum route handlers
//! - [`build_snapshot`] — used by WS player feed
//! - [`PlayerError`] — handler error type

pub(crate) use artifacts::{artifact_reference, read_artifact_blob};
pub use artifacts::{get_player_artifact, get_player_repo_file};
pub use error::PlayerError;
pub use history::get_player_history;
pub use memory::get_player_memory;
pub(crate) use snapshot::build_snapshot;
pub use snapshot::get_player_snapshot;

pub(crate) mod artifacts;
pub(crate) mod error;
pub(crate) mod history;
mod memory;
pub(crate) mod snapshot;

use arena_core::entities::{players, users};
use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter};
use uuid::Uuid;

/// Resolve the `:player` path segment within a session: a UUID matches the
/// player id directly; anything else is treated as the account username
/// (the same identifier `/u/<username>` profiles use) and resolved to that
/// user's player in the session.
pub(crate) async fn find_player_by_param(
    db: &DatabaseConnection,
    session_id: Uuid,
    param: &str,
) -> Result<Option<players::Model>, sea_orm::DbErr> {
    let query = players::Entity::find().filter(players::Column::SessionIdFk.eq(session_id));
    match Uuid::parse_str(param) {
        Ok(id) => query.filter(players::Column::Id.eq(id)).one(db).await,
        Err(_) => {
            let Some(user) = users::Entity::find()
                .filter(users::Column::Username.eq(param))
                .one(db)
                .await?
            else {
                return Ok(None);
            };
            query
                .filter(players::Column::UserIdFk.eq(user.id))
                .one(db)
                .await
        }
    }
}
