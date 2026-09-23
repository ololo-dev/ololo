//! `personal_projects` entity — marks a `projects` row as a user's own work
//! and keeps the request it was built from. See `crate::personal` for the
//! spec shape and the queries every "is this personal?" reader shares.
use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "personal_projects")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub project_id_fk: Uuid,
    /// The `crate::personal::PersonalSpec` the project was built from.
    #[sea_orm(column_type = "JsonBinary")]
    pub spec: Json,
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
