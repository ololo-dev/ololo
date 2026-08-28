//! `judge_results` entity.
//!
//! One row per judge evaluation of a player's task submission. Records the
//! LLM judge's rating, point delta, feedback, and raw output.
use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "judge_results")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub session_id_fk: Uuid,
    pub player_id_fk: Uuid,
    pub task_judge_id: Uuid,
    #[sea_orm(column_type = "Json")]
    pub rating: Json,
    pub point_delta: i32,
    pub feedback: String,
    pub model: String,
    /// LLM provider id (e.g. "openrouter", "ollama"). Empty for rows
    /// predating provider tracking.
    pub provider: String,
    pub raw_output: String,
    /// Wall-clock duration of the judge run (LLM + tool calls), if recorded.
    pub duration_ms: Option<i64>,
    /// Chronological run log (JSON array of `JudgeLogEvent`): LLM turns and
    /// tool calls with timestamps, durations, and per-turn token usage.
    /// Admin-only surface — never exposed on public payloads.
    pub run_log: Option<Json>,
    /// Total input/output tokens across the run's LLM turns, when the
    /// provider reported usage. Sums span every attempt of the run
    /// (retries included) — the basis for per-session/per-player cost.
    pub tokens_input: Option<i64>,
    pub tokens_output: Option<i64>,
    /// Provider-cache token totals (read = billed at cache rate,
    /// write = cache creation).
    pub tokens_cache_read: Option<i64>,
    pub tokens_cache_write: Option<i64>,
    /// Run lifecycle: "running" | "scored" | "failed". Only "scored" rows
    /// carry a meaningful rating/feedback/point_delta.
    pub status: String,
    /// Why a "failed" run produced no verdict. Full provider/system error —
    /// admin-only surface; public payloads carry a generic message.
    pub error: Option<String>,
    /// "full" | "partial" — a `partial` verdict was issued without data the
    /// judge asked for (an interactive probe timed out, or the task was cut
    /// short). NULL = full.
    pub verdict_kind: Option<String>,
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
        belongs_to = "super::task_judges::Entity",
        from = "Column::TaskJudgeId",
        to = "super::task_judges::Column::Id",
        on_delete = "Cascade"
    )]
    TaskJudge,
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

impl Related<super::task_judges::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::TaskJudge.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
