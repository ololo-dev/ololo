//! Sea-orm entity modules. One module per database table.
pub mod activity_event;
pub mod app_settings;
pub mod artifact_request_watchers;
pub mod auth_tokens;
pub mod categories;
pub mod cli_tokens;
pub mod email_templates;
pub mod game_servers;
pub mod judge_results;
pub mod judge_run_ledger;
pub mod judge_run_transcripts;
pub mod judges;
pub mod llm_pool_members;
pub mod llm_pools;
pub mod llm_providers;
pub mod llm_requests;
pub mod player_memory;
pub mod players;
pub mod probes;
pub mod projects;
pub mod refresh_tokens;
pub mod session_scheduler_state;
pub mod sessions;
pub mod similarity_reports;
pub mod task_agent_stats;
pub mod task_judges;
pub mod task_results;
pub mod tasks;
pub mod tests;
pub mod users;

/// Backward-compatible alias for code that still imports `entities::session`.
pub mod session {
    pub use super::sessions::*;
}
