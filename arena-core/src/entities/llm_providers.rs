//! `llm_providers` entity — admin-configured LLM providers.
//!
//! Multiple providers can be configured (Ollama, OpenRouter, any
//! OpenAI-compatible endpoint); operations pick a `(provider, model)` pair
//! via `app_settings` assignments (`llm_default`, `llm_op_<operation>`) or a
//! per-judge override on the `judges` table. API keys are stored encrypted
//! with [`crate::settings_encryption::SettingsEncryption`].
//!
//! `judges.llm_provider_id_fk` references this table without a DB-level FK
//! (SQLite cannot add constrained columns via ALTER TABLE); provider
//! deletion clears referencing assignments in the API layer.
use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "llm_providers")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    /// Display name, unique (e.g. "OpenRouter", "Local Ollama").
    pub name: String,
    /// Client kind: `ollama` | `openrouter` | `openai_compatible`.
    pub kind: String,
    /// Endpoint base URL; NULL uses the kind's default.
    pub base_url: Option<String>,
    /// API key encrypted with `SettingsEncryption`; NULL for keyless.
    pub api_key_enc: Option<String>,
    pub enabled: bool,
    /// models.dev provider id (e.g. "openrouter") powering model
    /// suggestions in the admin UI; informational only.
    pub catalog_id: Option<String>,
    pub created_at: ChronoDateTimeUtc,
    pub updated_at: ChronoDateTimeUtc,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
