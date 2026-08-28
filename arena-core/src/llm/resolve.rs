//! Operation-scoped model resolution over admin-configured `llm_providers`.
//!
//! Every LLM-backed operation resolves its model through this chain:
//! 1. an explicit override (per-judge `llm_provider_id_fk` / `llm_model`),
//! 2. the operation assignment (`app_settings` key `llm_op_<operation>`),
//! 3. the default assignment (`app_settings` key `llm_default`),
//! 4. `None` — the caller applies its own last-resort fallback (an
//!    empty-model "not configured" sentinel or a static keyless config).
//!
//! Assignments are JSON strings in one of two shapes:
//! - a single model — `{"provider_id": "<uuid>", "model": "<id>"}`, or
//! - a pool — `{"pool_id": "<uuid>"}`, which expands to several candidates.
//!
//! A disabled or deleted provider makes its assignment inert (the chain
//! keeps falling through) so a stale reference can never break operations;
//! the same holds for a pool whose members all resolve to nothing.
//!
//! Resolution yields an ordered *list* of candidates
//! ([`resolve_operation_candidates`]) so callers can fail over from one to
//! the next. A single-model assignment simply produces a one-element list.

use sea_orm::{
    ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, QueryOrder, sea_query::Expr,
};
use uuid::Uuid;

use crate::entities::{app_settings, llm_pool_members, llm_pools, llm_providers};
use crate::llm::ModelConfig;
use crate::settings_encryption::SettingsEncryption;

/// Operations with a configurable model. `judge` is the default for all
/// judges; individual judges may override it. (`probe` was removed with
/// `mode: llm` rubric probes — a stored `llm_op_probe` assignment is
/// simply never read.)
pub const LLM_OPERATIONS: [&str; 4] = ["judge", "memory", "adaptation", "project_ai"];

/// `app_settings` key holding the default assignment.
pub const LLM_DEFAULT_KEY: &str = "llm_default";

/// `app_settings` key for one operation's assignment.
pub fn llm_op_key(operation: &str) -> String {
    format!("llm_op_{operation}")
}

/// A single-model assignment: which provider row and which model id.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SingleAssignment {
    pub provider_id: Uuid,
    pub model: String,
}

/// A pool assignment: the candidates come from the pool's members.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PoolAssignment {
    pub pool_id: Uuid,
}

/// A parsed assignment — either one model or a whole pool.
///
/// Untagged so the single-model shape stays byte-identical to what was
/// stored (and served to the admin UI) before pools existed; the two
/// variants are told apart by their field names, which `deny_unknown_fields`
/// on both makes unambiguous.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(untagged)]
pub enum LlmAssignment {
    Pool(PoolAssignment),
    Single(SingleAssignment),
}

impl LlmAssignment {
    /// The pool this assignment points at, if it is a pool assignment.
    pub fn pool_id(&self) -> Option<Uuid> {
        match self {
            LlmAssignment::Pool(p) => Some(p.pool_id),
            LlmAssignment::Single(_) => None,
        }
    }

    /// The provider this assignment points at, if it is a single-model one.
    pub fn provider_id(&self) -> Option<Uuid> {
        match self {
            LlmAssignment::Single(s) => Some(s.provider_id),
            LlmAssignment::Pool(_) => None,
        }
    }
}

/// Parse an assignment settings value. Empty/malformed → `None`.
pub fn parse_assignment(raw: &str) -> Option<LlmAssignment> {
    let a: LlmAssignment = serde_json::from_str(raw.trim()).ok()?;
    // An empty model id would resolve to a provider call with no model.
    if let LlmAssignment::Single(s) = &a
        && s.model.trim().is_empty()
    {
        return None;
    }
    Some(a)
}

/// Map a provider row's `kind` to the registry id that drives client
/// construction. `openai_compatible` rides the `custom` registry entry
/// (explicit base URL required).
pub fn registry_id_for_kind(kind: &str) -> Option<&'static str> {
    match kind {
        "ollama" => Some("ollama"),
        "openrouter" => Some("openrouter"),
        "openai_compatible" => Some("custom"),
        _ => None,
    }
}

/// Build a [`ModelConfig`] from a provider row + model id, decrypting the
/// stored API key. Returns `None` when the row's kind is unknown or the key
/// fails to decrypt (logged by callers via the chain falling through).
pub fn model_config_from_provider(
    row: &llm_providers::Model,
    model: &str,
    enc: &SettingsEncryption,
) -> Option<ModelConfig> {
    let registry_id = registry_id_for_kind(&row.kind)?;
    let api_key = match &row.api_key_enc {
        Some(stored) if !stored.is_empty() => Some(enc.decrypt(stored).ok()?),
        _ => None,
    };
    Some(ModelConfig {
        provider: registry_id.to_string(),
        // The one place a provider row becomes a config, so the only place
        // its name can be captured before the row is dropped.
        provider_name: Some(row.name.clone()),
        model: model.to_string(),
        base_url: row.base_url.clone().filter(|s| !s.trim().is_empty()),
        api_key,
    })
}

async fn read_setting(db: &DatabaseConnection, key: &str) -> Option<String> {
    app_settings::Entity::find_by_id(key.to_string())
        .one(db)
        .await
        .ok()
        .flatten()
        .map(|r| r.value)
        .filter(|v| !v.trim().is_empty())
}

async fn load_enabled_provider(db: &DatabaseConnection, id: Uuid) -> Option<llm_providers::Model> {
    llm_providers::Entity::find_by_id(id)
        .filter(llm_providers::Column::Enabled.eq(true))
        .one(db)
        .await
        .ok()
        .flatten()
}

async fn config_from_single(
    db: &DatabaseConnection,
    enc: &SettingsEncryption,
    assignment: &SingleAssignment,
) -> Option<ModelConfig> {
    let row = load_enabled_provider(db, assignment.provider_id).await?;
    model_config_from_provider(&row, &assignment.model, enc)
}

/// Advance a pool's round-robin cursor by one. Best-effort: a failed bump
/// only means the next resolve reuses this start position, never a failed
/// resolution.
async fn bump_rr_cursor(db: &DatabaseConnection, pool_id: Uuid) {
    let _ = llm_pools::Entity::update_many()
        .col_expr(
            llm_pools::Column::RrCursor,
            Expr::col(llm_pools::Column::RrCursor).add(1),
        )
        .filter(llm_pools::Column::Id.eq(pool_id))
        .exec(db)
        .await;
}

/// Expand a pool into an ordered candidate list: tiers by ascending
/// `priority`, and within a tier the members rotated by the pool's
/// round-robin cursor so consecutive resolves start on different members.
///
/// Every member of a tier stays in the list after the rotation point, so a
/// tier is exhausted (each of its members tried) before the next tier is
/// reached. Members that are disabled, or whose provider is disabled,
/// deleted, or fails to decrypt, are dropped — a pool of only such members
/// yields an empty list and the assignment falls through as inert.
///
/// The read and the bump are two statements, not one atomic operation: two
/// processes resolving at the same instant can share a start position. That
/// costs a little evenness, never correctness, so it is not worth a lock on
/// this path.
async fn pool_candidates(
    db: &DatabaseConnection,
    enc: &SettingsEncryption,
    pool_id: Uuid,
) -> Vec<ModelConfig> {
    let Some(pool) = llm_pools::Entity::find_by_id(pool_id)
        .one(db)
        .await
        .ok()
        .flatten()
    else {
        return Vec::new();
    };

    let members = llm_pool_members::Entity::find()
        .filter(llm_pool_members::Column::PoolIdFk.eq(pool_id))
        .filter(llm_pool_members::Column::Enabled.eq(true))
        .order_by_asc(llm_pool_members::Column::Priority)
        .order_by_asc(llm_pool_members::Column::CreatedAt)
        .all(db)
        .await
        .unwrap_or_default();
    if members.is_empty() {
        return Vec::new();
    }

    // One query for every provider the pool references, rather than one per
    // member: a pool is resolved on the hot path of each judge attempt.
    let provider_ids: Vec<Uuid> = {
        let mut ids: Vec<Uuid> = members.iter().map(|m| m.provider_id_fk).collect();
        ids.sort_unstable();
        ids.dedup();
        ids
    };
    let providers: std::collections::HashMap<Uuid, llm_providers::Model> =
        llm_providers::Entity::find()
            .filter(llm_providers::Column::Id.is_in(provider_ids))
            .filter(llm_providers::Column::Enabled.eq(true))
            .all(db)
            .await
            .unwrap_or_default()
            .into_iter()
            .map(|p| (p.id, p))
            .collect();

    let cursor = pool.rr_cursor;
    bump_rr_cursor(db, pool_id).await;

    // `members` is already ordered by priority, so equal-priority runs are
    // contiguous and one pass groups them into tiers.
    let mut out: Vec<ModelConfig> = Vec::new();
    let mut tier: Vec<ModelConfig> = Vec::new();
    let mut tier_priority: Option<i32> = None;
    for m in &members {
        if tier_priority != Some(m.priority) {
            push_rotated(&mut out, std::mem::take(&mut tier), cursor);
            tier_priority = Some(m.priority);
        }
        if let Some(row) = providers.get(&m.provider_id_fk)
            && let Some(cfg) = model_config_from_provider(row, &m.model, enc)
        {
            tier.push(cfg);
        }
    }
    push_rotated(&mut out, tier, cursor);
    out
}

/// Append one tier's candidates, rotated left by `cursor` positions.
fn push_rotated(out: &mut Vec<ModelConfig>, tier: Vec<ModelConfig>, cursor: i64) {
    if tier.is_empty() {
        return;
    }
    let start = cursor.rem_euclid(tier.len() as i64) as usize;
    out.extend(tier[start..].iter().cloned());
    out.extend(tier[..start].iter().cloned());
}

/// `llm_source_order` value: the pool's candidates lead, the pinned model is
/// the failover behind them. The default.
pub const SOURCE_ORDER_POOL_FIRST: &str = "pool_first";
/// `llm_source_order` value: the pinned model leads, the pool backs it up.
pub const SOURCE_ORDER_MODEL_FIRST: &str = "model_first";

/// An explicit override of the assignment chain — today only judges set one.
///
/// Both halves are optional and **compose**: a judge may pin a pool, a
/// single provider+model, or both. With both set, `pool_first` says which
/// half leads; the other trails it as failover. An override that resolves to
/// no usable candidate is inert, and resolution falls through to the normal
/// `llm_op_<operation>` → `llm_default` chain rather than failing.
///
/// A model without a provider is **not** an override — the pair travels
/// together, so a model id can never be silently grafted onto whichever
/// provider the chain happened to resolve.
#[derive(Debug, Clone)]
pub struct LlmOverride<'a> {
    pub pool_id: Option<Uuid>,
    pub provider_id: Option<Uuid>,
    pub model: Option<&'a str>,
    /// Which half leads when both are set. See [`SOURCE_ORDER_POOL_FIRST`].
    pub pool_first: bool,
}

impl Default for LlmOverride<'_> {
    fn default() -> Self {
        Self {
            pool_id: None,
            provider_id: None,
            model: None,
            pool_first: true,
        }
    }
}

impl<'a> LlmOverride<'a> {
    /// No override — resolution uses the assignment chain alone.
    pub fn none() -> Self {
        Self::default()
    }

    /// Build from a judge row's columns.
    pub fn for_judge(
        pool_id: Option<Uuid>,
        provider_id: Option<Uuid>,
        model: Option<&'a str>,
        source_order: &str,
    ) -> Self {
        Self {
            pool_id,
            provider_id,
            model,
            pool_first: source_order != SOURCE_ORDER_MODEL_FIRST,
        }
    }

    /// True when nothing is pinned, so the chain applies unchanged.
    pub fn is_empty(&self) -> bool {
        self.pool_id.is_none() && self.provider_id.is_none()
    }

    /// The override's own candidates, in the configured order.
    async fn candidates(
        &self,
        db: &DatabaseConnection,
        enc: &SettingsEncryption,
    ) -> Vec<ModelConfig> {
        if self.is_empty() {
            return Vec::new();
        }
        let from_pool = match self.pool_id {
            Some(id) => pool_candidates(db, enc, id).await,
            None => Vec::new(),
        };
        let model = self.model.map(str::trim).filter(|m| !m.is_empty());
        let from_model = match (self.provider_id, model) {
            (Some(pid), Some(model)) => config_from_single(
                db,
                enc,
                &SingleAssignment {
                    provider_id: pid,
                    model: model.to_string(),
                },
            )
            .await
            .into_iter()
            .collect(),
            _ => Vec::new(),
        };
        let (lead, trail) = if self.pool_first {
            (from_pool, from_model)
        } else {
            (from_model, from_pool)
        };
        lead.into_iter().chain(trail).collect()
    }
}

async fn candidates_from_assignment(
    db: &DatabaseConnection,
    enc: &SettingsEncryption,
    assignment: &LlmAssignment,
) -> Vec<ModelConfig> {
    match assignment {
        LlmAssignment::Pool(p) => pool_candidates(db, enc, p.pool_id).await,
        LlmAssignment::Single(s) => config_from_single(db, enc, s).await.into_iter().collect(),
    }
}

/// Resolve the ordered candidate list for `operation`, applying an optional
/// explicit override (e.g. a judge row's `llm_provider_id_fk` /
/// `llm_model`).
///
/// The list is what makes failover possible: the caller tries candidates in
/// order and stops at the first that answers. A single-model assignment
/// yields one candidate; a pool yields its members, tier by tier. An empty
/// list means nothing is configured — the caller applies its own
/// last-resort fallback.
///
/// Override semantics match [`resolve_operation_model`]: an override
/// override (a pool, a provider+model pair, or both) replaces the
/// assignment chain entirely; when both are set, `pool_first` decides which
/// half leads and the other becomes the failover behind it.
pub async fn resolve_operation_candidates(
    db: &DatabaseConnection,
    enc: &SettingsEncryption,
    operation: &str,
    over: &LlmOverride<'_>,
) -> Vec<ModelConfig> {
    // 1. Explicit override. Both halves are optional and compose; an
    //    override that resolves to nothing (disabled provider, emptied pool)
    //    is inert and the chain below still runs.
    let overridden = over.candidates(db, enc).await;
    if !overridden.is_empty() {
        return overridden;
    }

    // 2./3. Operation assignment, then default assignment. An assignment
    //    that expands to nothing (deleted pool, all members disabled) is
    //    inert and the chain keeps falling through.
    for key in [llm_op_key(operation), LLM_DEFAULT_KEY.to_string()] {
        if let Some(raw) = read_setting(db, &key).await
            && let Some(a) = parse_assignment(&raw)
        {
            let candidates = candidates_from_assignment(db, enc, &a).await;
            if !candidates.is_empty() {
                return candidates;
            }
        }
    }
    Vec::new()
}

/// Resolve the single best model for `operation` — the first candidate from
/// [`resolve_operation_candidates`].
///
/// Kept for callers that cannot fail over (they only ever issue one call).
/// Anything that can retry should resolve the full candidate list instead,
/// otherwise a pool assignment silently degrades to its first member.
pub async fn resolve_operation_model(
    db: &DatabaseConnection,
    enc: &SettingsEncryption,
    operation: &str,
    over: &LlmOverride<'_>,
) -> Option<ModelConfig> {
    resolve_operation_candidates(db, enc, operation, over)
        .await
        .into_iter()
        .next()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_assignment_and_rejects_junk() {
        let id = Uuid::new_v4();
        let raw = format!(r#"{{"provider_id": "{id}", "model": "llama3.2"}}"#);
        let a = parse_assignment(&raw).unwrap();
        assert_eq!(a.provider_id(), Some(id));
        assert_eq!(a.pool_id(), None);
        assert!(matches!(&a, LlmAssignment::Single(s) if s.model == "llama3.2"));
        assert!(parse_assignment("").is_none());
        assert!(parse_assignment("not json").is_none());
        assert!(parse_assignment(&format!(r#"{{"provider_id": "{id}", "model": ""}}"#)).is_none());
    }

    #[test]
    fn parses_pool_assignment_and_keeps_shapes_distinct() {
        let pool = Uuid::new_v4();
        let a = parse_assignment(&format!(r#"{{"pool_id": "{pool}"}}"#)).unwrap();
        assert_eq!(a.pool_id(), Some(pool));
        assert_eq!(a.provider_id(), None);

        // The single-model shape must still serialize exactly as it did
        // before pools existed — the admin UI reads these fields directly.
        let single = LlmAssignment::Single(SingleAssignment {
            provider_id: pool,
            model: "m".into(),
        });
        assert_eq!(
            serde_json::to_value(&single).unwrap(),
            serde_json::json!({"provider_id": pool, "model": "m"})
        );
        assert_eq!(
            serde_json::to_value(LlmAssignment::Pool(PoolAssignment { pool_id: pool })).unwrap(),
            serde_json::json!({"pool_id": pool})
        );

        // A blend of both shapes is not a valid assignment either way.
        assert!(
            parse_assignment(&format!(r#"{{"pool_id": "{pool}", "model": "m"}}"#)).is_none(),
            "mixed pool+model must not parse"
        );
    }

    #[test]
    fn tier_rotation_walks_every_member_from_a_moving_start() {
        let cfg = |m: &str| ModelConfig {
            provider_name: None,
            provider: "ollama".into(),
            model: m.into(),
            base_url: None,
            api_key: None,
        };
        let tier = vec![cfg("a"), cfg("b"), cfg("c")];

        // The cursor only moves the start; the whole tier is still covered,
        // which is what lets failover exhaust a tier before dropping down.
        for (cursor, expected) in [
            (0i64, ["a", "b", "c"]),
            (1, ["b", "c", "a"]),
            (2, ["c", "a", "b"]),
            (3, ["a", "b", "c"]),
        ] {
            let mut out = Vec::new();
            push_rotated(&mut out, tier.clone(), cursor);
            let got: Vec<&str> = out.iter().map(|c| c.model.as_str()).collect();
            assert_eq!(got, expected, "cursor {cursor}");
        }
    }

    #[test]
    fn kind_mapping_covers_supported_kinds() {
        assert_eq!(registry_id_for_kind("ollama"), Some("ollama"));
        assert_eq!(registry_id_for_kind("openrouter"), Some("openrouter"));
        assert_eq!(registry_id_for_kind("openai_compatible"), Some("custom"));
        assert_eq!(registry_id_for_kind("weird"), None);
    }

    #[test]
    fn provider_row_builds_config_with_decrypted_key() {
        let enc = SettingsEncryption::new(b"0123456789abcdef0123456789abcdef");
        let row = llm_providers::Model {
            id: Uuid::new_v4(),
            name: "router".into(),
            kind: "openrouter".into(),
            base_url: None,
            api_key_enc: Some(enc.encrypt("sk-test")),
            enabled: true,
            catalog_id: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };
        let cfg = model_config_from_provider(&row, "some/model", &enc).unwrap();
        assert_eq!(cfg.provider, "openrouter");
        assert_eq!(cfg.model, "some/model");
        assert_eq!(cfg.api_key.as_deref(), Some("sk-test"));
    }
}
