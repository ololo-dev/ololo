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
    /// Whether this row represents a completion bonus rather than a probe outcome.
    pub is_bonus: bool,
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
