//! `app_settings` entity. Spec: FR-007, AC-016.
//!
//! A simple key-value store for application-wide configuration. Keys and
//! values are plain strings. Seeded by migration with:
//!   - `ai_provider = "ollama"`
//!   - `ai_model    = "llama3.2"`
use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "app_settings")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub key: String,
    pub value: String,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
