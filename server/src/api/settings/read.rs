//! Read handlers for admin settings: listing settings and users, plus live
//! model-listing helpers for the `llm_providers` admin API.

use crate::api::settings::common::{AdminUser, AdminUserDto, OllamaClientError, SettingsError};
use crate::state::AppState;
use arena_core::entities::{app_settings, judge_run_ledger, users};
use axum::Json;
use axum::extract::State;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect};

/// `GET /api/admin/users` — list all registered users (admin only).
pub async fn get_admin_users(
    _admin: AdminUser,
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, SettingsError> {
    let rows = users::Entity::find()
        .order_by_asc(users::Column::CreatedAt)
        .all(&state.db)
        .await
        .map_err(|_| SettingsError::Database)?;

    // This month's metered judge runs, grouped per account in one query;
    // tier limits fetched once instead of per user.
    let month_start = arena_core::quota::month_start_utc(chrono::Utc::now());
    let usage: std::collections::HashMap<uuid::Uuid, i64> = judge_run_ledger::Entity::find()
        .select_only()
        .column(judge_run_ledger::Column::UserIdFk)
        .column_as(judge_run_ledger::Column::Id.count(), "runs")
        .filter(judge_run_ledger::Column::CreatedAt.gte(month_start))
        .group_by(judge_run_ledger::Column::UserIdFk)
        .into_tuple::<(uuid::Uuid, i64)>()
        .all(&state.db)
        .await
        .map_err(|_| SettingsError::Database)?
        .into_iter()
        .collect();
    let free_limit =
        arena_core::quota::plan_judge_run_limit(&state.db, arena_core::quota::PLAN_FREE)
            .await
            .map_err(|_| SettingsError::Database)?;
    let premium_limit =
        arena_core::quota::plan_judge_run_limit(&state.db, arena_core::quota::PLAN_PREMIUM)
            .await
            .map_err(|_| SettingsError::Database)?;

    let dtos: Vec<AdminUserDto> = rows
        .into_iter()
        .map(|u| {
            let tier_limit = if u.plan == arena_core::quota::PLAN_FREE {
                free_limit
            } else {
                premium_limit
            };
            AdminUserDto {
                judge_runs_this_month: usage.get(&u.id).copied().unwrap_or(0),
                judge_run_limit_effective: u.judge_run_limit.map_or(tier_limit, i64::from),
                judge_run_credits: u.judge_run_credits,
                id: u.id,
                email: u.email,
                display_name: u.display_name,
                username: u.username,
                is_admin: u.is_admin,
                avatar_url: u.avatar_url,
                created_at: u.created_at,
                plan: u.plan,
                judge_run_limit: u.judge_run_limit,
            }
        })
        .collect();

    Ok(Json(serde_json::json!({ "users": dtos })))
}

/// `GET /api/admin/settings` — returns all settings as a JSON object.
/// Spec: FR-008, AC-008, AC-009.
pub async fn get_settings(
    _admin: crate::api::settings::common::AdminUser,
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, SettingsError> {
    let rows = app_settings::Entity::find()
        .all(&state.db)
        .await
        .map_err(|_| SettingsError::Database)?;

    let obj: serde_json::Map<String, serde_json::Value> = rows
        .into_iter()
        .map(|r| {
            let value = if crate::api::settings::common::is_secret_setting(&r.key) {
                "[redacted]".to_string()
            } else {
                r.value
            };
            (r.key, serde_json::Value::String(value))
        })
        .collect();

    Ok(Json(serde_json::Value::Object(obj)))
}

/// Live model listing for a dynamic `llm_providers` row: explicit kind,
/// base URL, and (already decrypted) key. Unreachable endpoints → `[]` so
/// the admin UI degrades to a free-form model input.
pub async fn list_models_for(
    state: &AppState,
    kind: &str,
    base_url: Option<&str>,
    api_key: Option<&str>,
) -> Vec<String> {
    let result = match kind {
        "openrouter" => list_openrouter_models(api_key).await,
        "openai_compatible" => match base_url.filter(|b| !b.trim().is_empty()) {
            Some(base) => list_openai_compatible_models(api_key, base).await,
            None => return vec![],
        },
        // ollama — honor a custom base URL when set, else the shared client.
        _ => match base_url.filter(|b| !b.trim().is_empty()) {
            Some(base) => {
                list_openai_compatible_models(
                    api_key,
                    &format!("{}/v1", base.trim_end_matches('/')),
                )
                .await
            }
            None => state.ollama_http.list_models().await,
        },
    };
    result.unwrap_or_default()
}

/// Live fetch of OpenRouter model ids. The endpoint is public, so no key is
/// required for listing; a key is only attached when supplied (preview or
/// stored/env). Returns `Err` when the HTTP call fails.
// ponytail: direct reqwest call; no rig ModelLister needed for a simple GET.
pub async fn list_openrouter_models(key: Option<&str>) -> Result<Vec<String>, OllamaClientError> {
    let client = reqwest::Client::new();
    let mut builder = client
        .get("https://openrouter.ai/api/v1/models")
        .timeout(std::time::Duration::from_secs(10));
    if let Some(k) = key {
        builder = builder.bearer_auth(k);
    }
    let resp = builder
        .send()
        .await
        .map_err(|_| OllamaClientError::Unreachable)?
        .error_for_status()
        .map_err(|_| OllamaClientError::BadResponse)?;

    #[derive(serde::Deserialize)]
    struct OrModel {
        id: String,
    }
    #[derive(serde::Deserialize)]
    struct OrList {
        data: Vec<OrModel>,
    }
    let body: OrList = resp
        .json()
        .await
        .map_err(|_| OllamaClientError::BadResponse)?;
    Ok(body.data.into_iter().map(|m| m.id).collect())
}

/// Live fetch of OpenCode Zen model ids (and any OpenAI-compatible
/// provider). The public catalog lives at `<base_url>/models` (default
/// `https://opencode.ai/zen/v1`); a key from the `OPENCODE_API_KEY`
/// env var or the stored `opencode_api_key` setting is attached as a
/// bearer token when supplied. Returns `Err` when the base URL is
/// unreachable or the response is malformed.
// ponytail: OpenAI-shaped /models response; direct reqwest, no rig needed.
pub async fn list_openai_compatible_models(
    key: Option<&str>,
    base: &str,
) -> Result<Vec<String>, OllamaClientError> {
    let url = format!("{}/models", base.trim_end_matches('/'));
    let client = reqwest::Client::new();
    let mut builder = client.get(&url).timeout(std::time::Duration::from_secs(10));
    if let Some(k) = key {
        builder = builder.bearer_auth(k);
    }
    let resp = builder
        .send()
        .await
        .map_err(|_| OllamaClientError::Unreachable)?
        .error_for_status()
        .map_err(|_| OllamaClientError::BadResponse)?;

    #[derive(serde::Deserialize)]
    struct OcModel {
        id: String,
    }
    #[derive(serde::Deserialize)]
    struct OcList {
        data: Vec<OcModel>,
    }
    let body: OcList = resp
        .json()
        .await
        .map_err(|_| OllamaClientError::BadResponse)?;
    Ok(body.data.into_iter().map(|m| m.id).collect())
}
