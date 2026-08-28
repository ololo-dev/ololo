//! `players` entity.
//!
//! One row per player (PAT-authenticated agent) connected to a session.
//!
//! `user_id_fk` is nullable: a player whose PAT is not tied to a registered
//! user account will have `None` here.
//!
//! `fingerprint` enables reconnect matching — it is defense-in-depth, not a
//! security trust boundary.
use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "players")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub session_id_fk: Uuid,
    pub user_id_fk: Option<Uuid>,
    pub display_name: String,
    pub fingerprint: Option<String>,
    pub metadata_json: Option<String>,
    pub joined_at: ChronoDateTimeUtc,
    pub reconnected_at: Option<ChronoDateTimeUtc>,
    pub revoked_at: Option<ChronoDateTimeUtc>,
    /// Whether the player's ololo agent socket is connected to the game server
    /// right now. Written by the game server on connect/disconnect and reset
    /// on its startup; the web "Live" indicator reads this, not the session
    /// status.
    pub agent_connected: bool,
    /// Last moment the agent socket was known alive (connect and disconnect
    /// both stamp it). The idle sweep measures "empty for how long" from the
    /// max of these across the session's players.
    pub agent_last_seen_at: Option<ChronoDateTimeUtc>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "super::sessions::Entity",
        from = "Column::SessionIdFk",
        to = "super::sessions::Column::Id",
        on_delete = "Cascade"
    )]
    Session,
    #[sea_orm(
        belongs_to = "super::users::Entity",
        from = "Column::UserIdFk",
        to = "super::users::Column::Id"
    )]
    User,
    #[sea_orm(has_many = "super::task_results::Entity")]
    Results,
}

impl Related<super::sessions::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Session.def()
    }
}

impl Related<super::users::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::User.def()
    }
}

impl Related<super::task_results::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Results.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
