//! Admin settings API. Spec: FR-007–FR-017.
//!
//! Routes (mounted by `lib.rs`):
//!   GET  /api/admin/settings            — list all settings (admin only)
//!   PUT  /api/admin/settings            — update a setting (admin only)
//!
//! LLM provider configuration lives in `api::llm_admin`
//! (`/api/admin/llm/*`); this module keeps the live model-listing helpers
//! it uses.
//!
//! # Ollama HTTP abstraction
//! [`OllamaHttp`] is injected through [`AppState`] so tests can swap in a
//! deterministic stub without spinning up a real Ollama server
//! (Commandment 2 compliance).

pub mod common;
pub mod read;
pub mod write;

/// Whether the session replay bar is offered at all.
///
/// The replay is an admin tool — it rewinds a finished session and re-reveals
/// its events up to a playhead — and this switch is how an instance turns it
/// off entirely, admins included: absent or `"true"` shows it, `"false"`
/// hides it everywhere.
pub const SESSION_REPLAY_KEY: &str = "session_replay_enabled";

/// The setting's value as a bool, defaulting to on when it was never set.
pub fn session_replay_enabled(value: Option<&str>) -> bool {
    !matches!(value.map(str::trim), Some("false"))
}

pub use common::{
    AdminUser, AdminUserDto, CreateAdminUserBody, OllamaClientError, OllamaHttp, OllamaHttpHandle,
    PutSettingsBody, ReqwestOllamaHttp, SettingsError, UpdateAdminUserBody,
};
pub use read::{
    get_admin_users, get_settings, list_models_for, list_openai_compatible_models,
    list_openrouter_models,
};
pub use write::{
    delete_admin_user, is_project_creation_allowed, patch_admin_user, post_admin_user, put_settings,
};

#[cfg(test)]
mod tests;
