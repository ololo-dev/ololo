//! `judge_run_ledger` entity — append-only metering of judge runs.
//!
//! One row per run of the judge pipeline, charged to the judged player's
//! user account (`players.user_id_fk`); runs by players without an account
//! are unmetered and leave no row. `session_id`/`player_id`/`judge_id` are
//! plain columns, not FKs: deleting a session must not refund the month's
//! usage. Only `user_id_fk` is a real FK (cascade on user deletion).
use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "judge_run_ledger")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub user_id_fk: Uuid,
    pub session_id: Uuid,
    pub player_id: Uuid,
    pub judge_id: Uuid,
    pub created_at: ChronoDateTimeUtc,
    /// Which pool the run consumed: `"monthly"` (tier allowance) or
    /// `"pack"` (purchased credits).
    pub source: String,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "super::users::Entity",
        from = "Column::UserIdFk",
        to = "super::users::Column::Id",
        on_delete = "Cascade"
    )]
    User,
}

impl Related<super::users::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::User.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
