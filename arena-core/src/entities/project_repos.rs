//! `project_repos` entity — the git repository a project's sessions start
//! from, one row per project that names one. See `crate::project_repo` for
//! what may be stored and the queries its readers share.
use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "project_repos")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub project_id_fk: Uuid,
    /// What `git clone` is given: an https or ssh remote.
    pub url: String,
    /// Branch, tag or commit to check out after the clone; `None` = the
    /// remote's default branch.
    pub git_ref: Option<String>,
    pub created_at: ChronoDateTimeUtc,
    pub updated_at: ChronoDateTimeUtc,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "super::projects::Entity",
        from = "Column::ProjectIdFk",
        to = "super::projects::Column::Id"
    )]
    Project,
}

impl Related<super::projects::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Project.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
