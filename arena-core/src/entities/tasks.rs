//! `tasks` entity.
//!
//! `test_template` is `serde_json::Value` so the entity layer stays
//! dialect-agnostic; the strongly-typed `TestTemplate` struct in
//! `crate::task_template` is the canonical shape.
//!
//! Tasks are project-owned (not session-owned).
use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "tasks")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub project_id_fk: Uuid,
    pub ordinal: i32,
    pub title: String,
    pub content: String,
    #[sea_orm(column_type = "Json")]
    pub test_template: Json,
    pub created_at: ChronoDateTimeUtc,
    pub tags: String,
    pub point_value: i32,
    /// Seconds until a dispatched probe is considered `no_response`. NULL = inherit project default.
    pub deadline_secs: Option<i64>,
    /// Minimum probe dispatch interval in seconds. NULL = inherit project default.
    pub min_interval_secs: Option<i32>,
    /// Increment applied to interval after each probe cycle. NULL = inherit project default.
    pub interval_increment_secs: Option<i32>,
    /// Maximum probe dispatch interval in seconds. NULL = inherit project default.
    pub max_interval_secs: Option<i32>,
    /// Points applied on a failing probe response.
    pub fail_points: i32,
    /// Points applied when no response is received before deadline.
    pub no_response_points: i32,
    /// Bonus points awarded when a player completes all tests for this task.
    pub completion_bonus_points: i32,
    /// Open-ended evaluation contract (`crate::evaluation::EvaluationContract`).
    /// NULL = classic probe-verified task; nothing about it changes.
    #[sea_orm(column_type = "Json", nullable)]
    pub evaluation: Option<Json>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "super::projects::Entity",
        from = "Column::ProjectIdFk",
        to = "super::projects::Column::Id",
        on_delete = "Cascade"
    )]
    Project,
}

impl Related<super::projects::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Project.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
