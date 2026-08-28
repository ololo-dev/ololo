//! `artifact_request_watchers` entity — judges attached to another judge's
//! open artifact request. The request rows themselves live in `tests`
//! (initiator = judge, mode = interactive); a watcher is re-driven with the
//! registrant when the artifact lands or the request expires.
use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "artifact_request_watchers")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub test_id: Uuid,
    #[sea_orm(primary_key, auto_increment = false)]
    pub judge_id: Uuid,
    pub created_at: ChronoDateTimeUtc,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "super::tests::Entity",
        from = "Column::TestId",
        to = "super::tests::Column::Id",
        on_delete = "Cascade"
    )]
    Test,
    #[sea_orm(
        belongs_to = "super::judges::Entity",
        from = "Column::JudgeId",
        to = "super::judges::Column::Id",
        on_delete = "Cascade"
    )]
    Judge,
}

impl ActiveModelBehavior for ActiveModel {}
