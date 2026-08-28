//! `activity_event` entity.
//!
//! One row per activity log event (task started, task scored). Persisted for
//! finished-session replay via getSessionReport.
use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "activity_event")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub session_id_fk: Uuid,
    pub player_id_fk: Uuid,
    pub task_id_fk: Uuid,
    pub event_kind: String,
    pub task_ordinal: i32,
    pub task_title: String,
    pub player_display_name: String,
    pub judge_name: Option<String>,
    pub point_delta: Option<i32>,
    /// Optional structured payload: a criteria-judge verdict stores its
    /// per-criterion sheet summary here for the session activity feed.
    #[sea_orm(column_type = "Json", nullable)]
    pub detail: Option<Json>,
    pub timestamp: ChronoDateTimeUtc,
    pub version: i64,
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
        from = "Column::TaskIdFk",
        to = "super::tasks::Column::Id",
        on_delete = "Cascade"
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
