//! `task_judges` entity — attaches judges to project tasks.
use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "task_judges")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub task_id: Uuid,
    pub judge_id: Uuid,
    pub ordinal: i32,
    #[sea_orm(column_type = "Json", nullable)]
    pub rating_scale_override: Option<Json>,
    /// Panel weight for open-ended tasks: this judge's share of the task's
    /// point budget relative to the other attached judges. NULL = 1.0.
    pub weight: Option<f64>,
    pub created_at: ChronoDateTimeUtc,
    pub updated_at: ChronoDateTimeUtc,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "super::tasks::Entity",
        from = "Column::TaskId",
        to = "super::tasks::Column::Id",
        on_delete = "Cascade"
    )]
    Task,
    #[sea_orm(
        belongs_to = "super::judges::Entity",
        from = "Column::JudgeId",
        to = "super::judges::Column::Id",
        on_delete = "Restrict"
    )]
    Judge,
}

impl Related<super::tasks::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Task.def()
    }
}

impl Related<super::judges::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Judge.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
