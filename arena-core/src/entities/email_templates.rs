use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "email_templates")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub r#type: String,
    pub subject: String,
    pub body_html: String,
    pub body_text: String,
    pub updated_at: ChronoDateTimeUtc,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
