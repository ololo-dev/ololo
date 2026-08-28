//! Admin cost analytics over the `llm_requests` telemetry.
//!
//! Every judge/memory/adaptation LLM call already records its tokens and its
//! (session, player, task, judge) context; this module turns those rows into
//! money. Prices live in the `llm_model_prices` app setting — a JSON map of
//! `model → {input, output, cache_read, cache_write}` in USD per million
//! tokens, editable on the analytics page (with a models.dev assist).
//! A model missing from the map contributes tokens but a `null` cost, and is
//! counted in `unpriced_requests` so the page can say what the totals miss.

use std::collections::HashMap;

use arena_core::entities::{app_settings, llm_requests, players, sessions};
use axum::Json;
use axum::extract::{Path, Query, State};
use chrono::{Duration, Timelike, Utc};
use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, QueryOrder};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::api::error::impl_api_error;
use crate::api::settings::AdminUser;
use crate::state::AppState;

/// Errors of the analytics endpoints.
#[derive(Debug)]
pub enum AnalyticsError {
    NotFound,
    Db(String),
}

impl From<sea_orm::DbErr> for AnalyticsError {
    fn from(e: sea_orm::DbErr) -> Self {
        Self::Db(e.to_string())
    }
}

impl_api_error!(AnalyticsError {
    Self::NotFound => (NOT_FOUND, "not_found"),
    Self::Db(_) => (INTERNAL_SERVER_ERROR, "database_error"),
});

/// App-settings key holding the price map.
pub const PRICES_KEY: &str = "llm_model_prices";
/// App-settings key for the "show costs on session pages" switch.
pub const SHOW_COSTS_KEY: &str = "show_llm_costs_in_session";

/// USD per million tokens, by kind.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct ModelPrice {
    #[serde(default)]
    pub input: f64,
    #[serde(default)]
    pub output: f64,
    #[serde(default)]
    pub cache_read: f64,
    #[serde(default)]
    pub cache_write: f64,
}

impl ModelPrice {
    /// USD for one request's token counts.
    pub fn cost(&self, input: i64, output: i64, cache_read: i64, cache_write: i64) -> f64 {
        (input as f64 * self.input
            + output as f64 * self.output
            + cache_read as f64 * self.cache_read
            + cache_write as f64 * self.cache_write)
            / 1_000_000.0
    }
}

pub(crate) async fn load_prices(db: &DatabaseConnection) -> HashMap<String, ModelPrice> {
    let raw = app_settings::Entity::find_by_id(PRICES_KEY.to_string())
        .one(db)
        .await
        .ok()
        .flatten()
        .map(|r| r.value);
    raw.and_then(|v| serde_json::from_str(&v).ok())
        .unwrap_or_default()
}

/// One aggregated row of spending, grouped by whatever `key`/`label` name.
#[derive(Debug, Default, Serialize)]
pub struct CostBucket {
    /// Group key: model id, judge slug, session id, player id, operation.
    pub key: String,
    /// Human label where the key alone is opaque (session name, player name).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Secondary line under the label (a session's start date).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sublabel: Option<String>,
    pub requests: i64,
    pub failed_requests: i64,
    pub tokens_input: i64,
    pub tokens_output: i64,
    pub tokens_cache_read: i64,
    pub tokens_cache_write: i64,
    /// USD, summed over priced models only; `null` when NOTHING was priced.
    pub cost: Option<f64>,
    /// Requests whose model has no price entry — the cost above misses them.
    pub unpriced_requests: i64,
}

impl CostBucket {
    fn add(&mut self, row: &llm_requests::Model, prices: &HashMap<String, ModelPrice>) {
        self.requests += 1;
        if row.status != "ok" {
            self.failed_requests += 1;
        }
        self.tokens_input += row.tokens_input;
        self.tokens_output += row.tokens_output;
        self.tokens_cache_read += row.tokens_cache_read;
        self.tokens_cache_write += row.tokens_cache_write;
        match prices.get(&row.model) {
            Some(p) => {
                *self.cost.get_or_insert(0.0) += p.cost(
                    row.tokens_input,
                    row.tokens_output,
                    row.tokens_cache_read,
                    row.tokens_cache_write,
                );
            }
            None => self.unpriced_requests += 1,
        }
    }
}

fn bucketize<'a, K, F>(
    rows: impl Iterator<Item = &'a llm_requests::Model>,
    prices: &HashMap<String, ModelPrice>,
    mut key_of: F,
) -> Vec<CostBucket>
where
    K: Into<String>,
    F: FnMut(&llm_requests::Model) -> Option<K>,
{
    let mut map: HashMap<String, CostBucket> = HashMap::new();
    for row in rows {
        let Some(key) = key_of(row) else { continue };
        let key: String = key.into();
        let bucket = map.entry(key.clone()).or_insert_with(|| CostBucket {
            key,
            ..Default::default()
        });
        bucket.add(row, prices);
    }
    let mut out: Vec<CostBucket> = map.into_values().collect();
    out.sort_by(|a, b| {
        b.cost
            .unwrap_or(0.0)
            .total_cmp(&a.cost.unwrap_or(0.0))
            .then_with(|| {
                (b.tokens_input + b.tokens_output).cmp(&(a.tokens_input + a.tokens_output))
            })
    });
    out
}

/// One day×hour cell of the usage heatmap (UTC).
#[derive(Debug, Serialize)]
pub struct HeatCell {
    /// `YYYY-MM-DD`, UTC.
    pub day: String,
    /// 0–23, UTC.
    pub hour: u32,
    pub requests: i64,
    pub tokens: i64,
    /// USD over priced models; `null` when nothing in the cell was priced.
    pub cost: Option<f64>,
}

#[derive(Debug, Deserialize)]
pub struct CostsQuery {
    /// Look-back window; default 30, capped to a year.
    #[serde(default)]
    pub days: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct CostsSummaryResponse {
    pub days: i64,
    pub totals: CostBucket,
    pub by_model: Vec<CostBucket>,
    pub by_judge: Vec<CostBucket>,
    pub by_operation: Vec<CostBucket>,
    pub by_session: Vec<CostBucket>,
    /// Spending per player account, across every session in the window.
    pub by_player: Vec<CostBucket>,
    /// Day×hour usage grid over the window (UTC); empty cells are omitted.
    pub heatmap: Vec<HeatCell>,
    /// The stored price map, so the page can render and edit it in place.
    pub prices: HashMap<String, ModelPrice>,
    /// Models seen in the window with no price entry.
    pub unpriced_models: Vec<String>,
}

/// `GET /api/admin/analytics/costs?days=N` — platform LLM spending, grouped
/// every way the page needs. Loads the window's rows once and buckets in
/// memory: the telemetry table is capped and pruned, and one admin page load
/// does not justify five GROUP BY round-trips.
pub async fn get_costs_summary(
    _admin: AdminUser,
    State(state): State<AppState>,
    Query(q): Query<CostsQuery>,
) -> Result<Json<CostsSummaryResponse>, AnalyticsError> {
    let days = q.days.unwrap_or(30).clamp(1, 366);
    let since = Utc::now() - Duration::days(days);
    let prices = load_prices(&state.db).await;

    let rows = llm_requests::Entity::find()
        .filter(llm_requests::Column::CreatedAt.gte(since))
        .order_by_desc(llm_requests::Column::CreatedAt)
        .all(&state.db)
        .await?;

    let mut totals = CostBucket {
        key: "total".to_string(),
        ..Default::default()
    };
    for row in &rows {
        totals.add(row, &prices);
    }

    let by_model = bucketize(rows.iter(), &prices, |r| Some(r.model.clone()));
    let by_judge = bucketize(rows.iter(), &prices, |r| r.judge_slug.clone());
    let by_operation = bucketize(rows.iter(), &prices, |r| Some(r.operation.clone()));
    let mut by_session = bucketize(rows.iter(), &prices, |r| {
        r.session_id.map(|id| id.to_string())
    });

    // Session labels: join code + started date read better than a UUID.
    let session_ids: Vec<Uuid> = by_session
        .iter()
        .filter_map(|b| b.key.parse().ok())
        .collect();
    if !session_ids.is_empty() {
        let labels: HashMap<String, (String, String)> = sessions::Entity::find()
            .filter(sessions::Column::Id.is_in(session_ids))
            .all(&state.db)
            .await?
            .into_iter()
            .map(|s| {
                let at = s.started_at.unwrap_or(s.created_at);
                (
                    s.id.to_string(),
                    (s.join_code, at.format("%-d %b %Y, %H:%M").to_string()),
                )
            })
            .collect();
        for b in &mut by_session {
            if let Some((code, date)) = labels.get(&b.key) {
                b.label = Some(code.clone());
                b.sublabel = Some(date.clone());
            }
        }
    }

    // Player spending across sessions: telemetry rows carry the per-session
    // player id — fold them onto the owning account so one person's play
    // across many sessions lands in one row (account-less players stand
    // alone under their player id).
    let player_ids: Vec<Uuid> = {
        let mut ids: Vec<Uuid> = rows.iter().filter_map(|r| r.player_id).collect();
        ids.sort_unstable();
        ids.dedup();
        ids
    };
    let mut by_player = Vec::new();
    if !player_ids.is_empty() {
        let player_rows = players::Entity::find()
            .filter(players::Column::Id.is_in(player_ids))
            .all(&state.db)
            .await?;
        let mut key_of_player: HashMap<Uuid, String> = HashMap::new();
        let mut label_of_key: HashMap<String, String> = HashMap::new();
        for p in &player_rows {
            let key = p.user_id_fk.unwrap_or(p.id).to_string();
            key_of_player.insert(p.id, key.clone());
            label_of_key.insert(key, p.display_name.clone());
        }
        by_player = bucketize(rows.iter(), &prices, |r| {
            r.player_id.and_then(|id| key_of_player.get(&id).cloned())
        });
        for b in &mut by_player {
            b.label = label_of_key.get(&b.key).cloned();
        }
    }

    // Pseudo-models (execution sandbox runs, gate decisions) carry no
    // tokens — pricing them is meaningless, so they stay off the nag list.
    let mut unpriced_models: Vec<String> = by_model
        .iter()
        .filter(|b| b.unpriced_requests > 0 && b.tokens_input + b.tokens_output > 0)
        .map(|b| b.key.clone())
        .collect();
    unpriced_models.sort();

    // Day×hour usage grid: fold every request into its UTC hour cell.
    let mut heat: HashMap<(String, u32), HeatCell> = HashMap::new();
    for row in &rows {
        let day = row.created_at.format("%Y-%m-%d").to_string();
        let hour = row.created_at.hour();
        let cell = heat.entry((day.clone(), hour)).or_insert_with(|| HeatCell {
            day,
            hour,
            requests: 0,
            tokens: 0,
            cost: None,
        });
        cell.requests += 1;
        cell.tokens +=
            row.tokens_input + row.tokens_output + row.tokens_cache_read + row.tokens_cache_write;
        if let Some(p) = prices.get(&row.model) {
            *cell.cost.get_or_insert(0.0) += p.cost(
                row.tokens_input,
                row.tokens_output,
                row.tokens_cache_read,
                row.tokens_cache_write,
            );
        }
    }
    let mut heatmap: Vec<HeatCell> = heat.into_values().collect();
    heatmap.sort_by(|a, b| a.day.cmp(&b.day).then(a.hour.cmp(&b.hour)));

    Ok(Json(CostsSummaryResponse {
        days,
        totals,
        by_model,
        by_judge,
        by_operation,
        by_session,
        by_player,
        heatmap,
        prices,
        unpriced_models,
    }))
}

#[derive(Debug, Serialize)]
pub struct SessionCostsResponse {
    pub session_id: Uuid,
    pub join_code: String,
    pub totals: CostBucket,
    pub by_player: Vec<CostBucket>,
    pub by_judge: Vec<CostBucket>,
    pub by_operation: Vec<CostBucket>,
}

/// `GET /api/admin/sessions/:id/costs` — one session's LLM spending, per
/// player and per judge. This is both the analytics drill-down and the
/// source for the in-session cost display (admin viewers, when the
/// `show_llm_costs_in_session` switch is on).
pub async fn get_session_costs(
    _admin: AdminUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<SessionCostsResponse>, AnalyticsError> {
    let session = sessions::Entity::find_by_id(id)
        .one(&state.db)
        .await?
        .ok_or(AnalyticsError::NotFound)?;
    let prices = load_prices(&state.db).await;

    let rows = llm_requests::Entity::find()
        .filter(llm_requests::Column::SessionId.eq(id))
        .all(&state.db)
        .await?;

    let mut totals = CostBucket {
        key: "total".to_string(),
        ..Default::default()
    };
    for row in &rows {
        totals.add(row, &prices);
    }
    let mut by_player = bucketize(rows.iter(), &prices, |r| {
        r.player_id.map(|id| id.to_string())
    });
    let by_judge = bucketize(rows.iter(), &prices, |r| r.judge_slug.clone());
    let by_operation = bucketize(rows.iter(), &prices, |r| Some(r.operation.clone()));

    let player_ids: Vec<Uuid> = by_player
        .iter()
        .filter_map(|b| b.key.parse().ok())
        .collect();
    if !player_ids.is_empty() {
        let names: HashMap<String, String> = players::Entity::find()
            .filter(players::Column::Id.is_in(player_ids))
            .all(&state.db)
            .await?
            .into_iter()
            .map(|p| (p.id.to_string(), p.display_name))
            .collect();
        for b in &mut by_player {
            b.label = names.get(&b.key).cloned();
        }
    }

    Ok(Json(SessionCostsResponse {
        session_id: id,
        join_code: session.join_code,
        totals,
        by_player,
        by_judge,
        by_operation,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(model: &str, ti: i64, to: i64, status: &str) -> llm_requests::Model {
        llm_requests::Model {
            id: Uuid::new_v4(),
            operation: "judge".into(),
            provider: "custom".into(),
            provider_name: None,
            model: model.into(),
            status: status.into(),
            error: None,
            tokens_input: ti,
            tokens_output: to,
            tokens_cache_read: 0,
            tokens_cache_write: 0,
            duration_ms: 100,
            session_id: None,
            player_id: None,
            task_id: None,
            judge_slug: Some("correctness".into()),
            detail_json: None,
            events_json: None,
            created_at: Utc::now(),
        }
    }

    #[test]
    fn priced_and_unpriced_models_are_kept_apart() {
        let mut prices = HashMap::new();
        prices.insert(
            "gpt-priced".to_string(),
            ModelPrice {
                input: 2.0,
                output: 10.0,
                ..Default::default()
            },
        );
        let rows = vec![
            row("gpt-priced", 1_000_000, 100_000, "ok"),
            row("mystery-model", 500_000, 50_000, "ok"),
            row("gpt-priced", 0, 0, "failed"),
        ];
        let mut totals = CostBucket::default();
        for r in &rows {
            totals.add(r, &prices);
        }
        // 1M in * $2/M + 100k out * $10/M = 2 + 1 = 3
        assert_eq!(totals.requests, 3);
        assert_eq!(totals.failed_requests, 1);
        assert_eq!(totals.unpriced_requests, 1);
        assert!((totals.cost.unwrap() - 3.0).abs() < 1e-9);

        let by_model = bucketize(rows.iter(), &prices, |r| Some(r.model.clone()));
        assert_eq!(by_model[0].key, "gpt-priced");
        assert_eq!(by_model[1].key, "mystery-model");
        assert_eq!(by_model[1].cost, None);
    }
}
