//! `task_agent_stats` entity.
//!
//! One row per (session, player, task): client-reported statistics about how
//! the player's AI agent implemented the task — token usage, message counts,
//! tool usage and skill loads. Reported by the ololo CLI when a task
//! completes. Trusted client data (honest-trust model), never used for
//! scoring.
use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "task_agent_stats")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub session_id_fk: Uuid,
    pub player_id_fk: Uuid,
    /// Nullable: the project task may be deleted after the session ran.
    pub task_id_fk: Option<Uuid>,
    pub task_ordinal: i32,
    /// Capture window: first probe for the task → task completion.
    pub window_started_at: Option<ChronoDateTimeUtc>,
    pub window_ended_at: Option<ChronoDateTimeUtc>,
    /// Aggregated across every agent session active in the window.
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cache_read_tokens: i64,
    pub cache_write_tokens: i64,
    pub reasoning_tokens: i64,
    pub cost: Option<f64>,
    pub user_messages: i64,
    pub assistant_messages: i64,
    pub tool_calls: i64,
    /// JSON array of per-agent-session detail
    /// (`Vec<arena_core::protocol::AgentSessionStats>`).
    pub agents_json: String,
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
    #[sea_orm(
        belongs_to = "super::tasks::Entity",
        from = "Column::TaskIdFk",
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
