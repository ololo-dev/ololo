//! `player_test_commands` entity — the commands that run a player's tests
//! and their coverage, as the player's `AGENTS.md` / `README.md` name them.
//!
//! One row per (session, player), read by the model that reads session
//! memory whenever those files change, and handed to the player's CLI,
//! which runs the command after each probe's health analysis. Derived from
//! player-authored docs (honest-trust model); both commands NULL means the
//! docs name none.
use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "player_test_commands")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub session_id_fk: Uuid,
    pub player_id_fk: Uuid,
    pub test_command: Option<String>,
    pub coverage_command: Option<String>,
    /// JSON array of the doc files the commands were read from.
    pub sources: String,
    /// Hash of the docs the commands were read from; an unchanged hash
    /// skips the next read.
    pub source_hash: String,
    pub created_at: ChronoDateTimeUtc,
    pub updated_at: ChronoDateTimeUtc,
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
        belongs_to = "super::players::Entity",
        from = "Column::PlayerIdFk",
        to = "super::players::Column::Id",
        on_delete = "Cascade"
    )]
    Player,
}

impl Related<super::sessions::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Session.def()
    }
}

impl Related<super::players::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Player.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}

impl Model {
    /// The doc files, as stored.
    pub fn source_list(&self) -> Vec<String> {
        serde_json::from_str(&self.sources).unwrap_or_default()
    }
}
