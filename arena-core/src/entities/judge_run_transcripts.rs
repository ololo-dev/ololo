//! `judge_run_transcripts` entity — the conversation of a judge run paused
//! on a participant request (a capture, a command to run). One row per
//! `waiting` judge result; the re-drive resumes the conversation from it
//! with the answer appended, instead of re-investigating from scratch.
use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "judge_run_transcripts")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub judge_result_id: Uuid,
    /// The rig message history (user, assistant, tool results) up to and
    /// including the tool result that registered the request. The system
    /// prompt is not stored: the resume supplies the current one.
    pub transcript: Json,
    pub created_at: ChronoDateTimeUtc,
    pub updated_at: ChronoDateTimeUtc,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "super::judge_results::Entity",
        from = "Column::JudgeResultId",
        to = "super::judge_results::Column::Id",
        on_delete = "Cascade"
    )]
    JudgeResult,
}

impl ActiveModelBehavior for ActiveModel {}
