//! `sessions` entity.
use crate::session_status::SessionStatus;
use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "sessions")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub name: String,
    pub created_at: ChronoDateTimeUtc,
    pub owner_id_fk: Option<Uuid>,
    pub status: SessionStatus,
    pub join_code: String,
    pub started_at: Option<ChronoDateTimeUtc>,
    pub finished_at: Option<ChronoDateTimeUtc>,
    pub paused_at: Option<ChronoDateTimeUtc>,
    pub paused_duration_secs: Option<i64>,
    pub project_id_fk: Uuid,
    pub game_server_id: Option<Uuid>,
    /// Why the session was cancelled: `"user"` or `"idle_timeout"`. NULL for
    /// sessions that finished normally (or predate the column).
    pub cancel_reason: Option<String>,
    /// Display name of whoever cancelled it, for `cancel_reason = "user"`.
    pub cancelled_by: Option<String>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "super::users::Entity",
        from = "Column::OwnerIdFk",
        to = "super::users::Column::Id"
    )]
    Owner,
    #[sea_orm(
        belongs_to = "super::projects::Entity",
        from = "Column::ProjectIdFk",
        to = "super::projects::Column::Id"
    )]
    Project,
    #[sea_orm(
        belongs_to = "super::game_servers::Entity",
        from = "Column::GameServerId",
        to = "super::game_servers::Column::Id"
    )]
    GameServer,
}

impl Related<super::projects::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Project.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
