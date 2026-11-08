//! `session_links` entity — the group a session is linked into, one row per
//! linked session. Sessions sharing a `group_id` are linked to each other;
//! a session without a row is linked to nothing. Beside `sessions` rather
//! than in it, like `session_commentators`. See `crate::session_links` for
//! the queries its readers share.
use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "session_links")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub session_id_fk: Uuid,
    pub group_id: Uuid,
    pub created_at: ChronoDateTimeUtc,
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
}

impl Related<super::sessions::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Session.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
