//! `player_memory` entity.
//!
//! One row per (session, player): structured values extracted by an LLM from
//! the player's markdown files (AGENTS.md / README.md) during the session.
//! The key set and default values come from the project's `memory_schema`;
//! extracted values override the defaults at probe-render time and are
//! available in task templates as `{memory.<key>}`. Derived from
//! player-authored content (honest-trust model), never used for scoring.
use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "player_memory")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub session_id_fk: Uuid,
    pub player_id_fk: Uuid,
    /// JSON object of extracted `key -> string value` pairs (schema keys
    /// only; unknown keys are dropped at extraction time).
    pub values_json: String,
    /// Hash of the markdown sources the values were extracted from; used to
    /// skip re-extraction when the files did not change between commits.
    pub source_hash: Option<String>,
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
