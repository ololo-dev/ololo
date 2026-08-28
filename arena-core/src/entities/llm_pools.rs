//! `llm_pools` entity — a named, reusable set of model candidates.
//!
//! An assignment (`llm_default`, `llm_op_<operation>`, or a per-judge
//! override) may point at a pool instead of a single provider+model. Members
//! are grouped into priority tiers: a tier shares the load between its members,
//! and the next tier is only reached when the current one fails. See
//! [`crate::llm::resolve`].
use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "llm_pools")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    /// Display name, unique.
    pub name: String,
    pub description: String,
    /// Round-robin cursor, bumped once per resolve. Rotating each tier's
    /// start position by this value is what splits the load between members
    /// of one tier; persisting it (rather than counting in memory) keeps the
    /// `server` and `game-server` processes on a single shared sequence.
    pub rr_cursor: i64,
    pub created_at: ChronoDateTimeUtc,
    pub updated_at: ChronoDateTimeUtc,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(has_many = "super::llm_pool_members::Entity")]
    Members,
}

impl Related<super::llm_pool_members::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Members.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
