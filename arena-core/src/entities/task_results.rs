//! `task_results` entity.
//!
//! One row per task submission from a player. Records the player's answer
//! and the signed score change applied to the leaderboard.
use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "task_results")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub session_id_fk: Uuid,
    pub player_id_fk: Uuid,
    pub task_id: Option<Uuid>,
    pub answer: String,
    pub created_at: ChronoDateTimeUtc,
    pub point_delta: i32,
    /// Whether this row represents a bonus (or a penalty) rather than a
    /// probe outcome. Kept for every reader that filters on it; `kind`
    /// says which.
    pub is_bonus: bool,
    /// One of the `KIND_*` constants below. Joins the "one bonus per task"
    /// unique key, so each bonus kind is unique per task on its own.
    pub kind: String,
}

/// A probe outcome (`is_bonus = false`).
pub const KIND_PROBE: &str = "probe";
/// The bonus for completing every test of a task.
pub const KIND_COMPLETION_BONUS: &str = "completion_bonus";
/// The bonus for the code health of a task's final tree.
pub const KIND_HEALTH_BONUS: &str = "health_bonus";
/// The cross-session copy/paste penalty (session-level: `task_id` is NULL).
pub const KIND_SIMILARITY_PENALTY: &str = "similarity_penalty";

/// The kind rows carried before the column existed: a bonus with a task
/// was the completion bonus, everything else a probe outcome.
pub fn legacy_kind(is_bonus: bool) -> &'static str {
    if is_bonus {
        KIND_COMPLETION_BONUS
    } else {
        KIND_PROBE
    }
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
    #[sea_orm(
        belongs_to = "super::tasks::Entity",
        from = "Column::TaskId",
        to = "super::tasks::Column::Id",
        on_delete = "SetNull"
    )]
    Task,
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

impl Related<super::tasks::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Task.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
