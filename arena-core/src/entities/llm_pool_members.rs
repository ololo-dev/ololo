//! `llm_pool_members` entity — one `(provider, model)` candidate in a pool.
use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "llm_pool_members")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub pool_id_fk: Uuid,
    pub provider_id_fk: Uuid,
    /// Model id as the provider names it.
    pub model: String,
    /// Lower value is tried first. Members sharing a value form one tier and
    /// split the load; the next tier is reached only when this one fails.
    pub priority: i32,
    /// A disabled member is skipped without being deleted.
    pub enabled: bool,
    pub created_at: ChronoDateTimeUtc,
    pub updated_at: ChronoDateTimeUtc,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "super::llm_pools::Entity",
        from = "Column::PoolIdFk",
        to = "super::llm_pools::Column::Id",
        on_delete = "Cascade"
    )]
    Pool,
    #[sea_orm(
        belongs_to = "super::llm_providers::Entity",
        from = "Column::ProviderIdFk",
        to = "super::llm_providers::Column::Id",
        on_delete = "Cascade"
    )]
    Provider,
}

impl Related<super::llm_pools::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Pool.def()
    }
}

impl Related<super::llm_providers::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Provider.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
