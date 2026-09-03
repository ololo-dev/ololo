//! Offline list-price estimates for sessions whose logs carry token counts
//! but no cost. The table is a compact snapshot of models.dev (see
//! `scripts/update-model-pricing.py`); a model missing from it yields `None`
//! rather than a misleading zero.

use crate::types::{AgentId, TokenCounts};
use std::collections::HashMap;
use std::sync::OnceLock;

const TABLE_JSON: &str = include_str!("pricing/models.json");

/// USD per million tokens.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Price {
    pub input: f64,
    pub output: f64,
    pub cache_read: f64,
    pub cache_write: f64,
    /// Separate thinking-token rate where the vendor bills one; otherwise
    /// reasoning is billed as output.
    pub reasoning: Option<f64>,
}

#[derive(serde::Deserialize)]
struct Table {
    generated: String,
    models: HashMap<String, Vec<f64>>,
}

fn table() -> &'static Table {
    static TABLE: OnceLock<Table> = OnceLock::new();
    TABLE.get_or_init(|| serde_json::from_str(TABLE_JSON).expect("embedded pricing table is valid"))
}

/// The models.dev snapshot date the embedded table was generated from.
pub fn snapshot_date() -> &'static str {
    &table().generated
}

fn row_to_price(row: &[f64]) -> Option<Price> {
    Some(Price {
        input: *row.first()?,
        output: *row.get(1)?,
        cache_read: row.get(2).copied().unwrap_or(0.0),
        cache_write: row.get(3).copied().unwrap_or(0.0),
        reasoning: row.get(4).copied(),
    })
}

fn exact(key: &str) -> Option<Price> {
    table().models.get(key).and_then(|r| row_to_price(r))
}

/// Strip the decorations agents and gateways add around a bare model id.
fn candidates(model: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut push = |s: &str| {
        let s = s.trim();
        if !s.is_empty() && !out.iter().any(|o| o == s) {
            out.push(s.to_string());
        }
    };
    // OpenCode/Hermes report `{"id":"gpt-5.5","providerID":"openai"}`.
    let model = serde_json::from_str::<serde_json::Value>(model)
        .ok()
        .and_then(|v| v.get("id").and_then(|i| i.as_str()).map(String::from))
        .unwrap_or_else(|| model.to_string());
    let lower = model.to_ascii_lowercase();
    push(&lower);
    // "openai/gpt-5.5", "anthropic:claude-opus-5"
    let bare = lower
        .rsplit(['/', ':'])
        .next()
        .unwrap_or(&lower)
        .to_string();
    push(&bare);
    // "us.anthropic.claude-opus-5" (Bedrock), "claude-opus-5@eu" (Vertex)
    let mut stripped = bare.clone();
    for prefix in ["us.", "eu.", "jp.", "au.", "apac.", "global."] {
        if let Some(rest) = stripped.strip_prefix(prefix) {
            stripped = rest.to_string();
        }
    }
    for prefix in ["anthropic.", "meta.", "mistral.", "amazon.", "cohere."] {
        if let Some(rest) = stripped.strip_prefix(prefix) {
            stripped = rest.to_string();
        }
    }
    if let Some((head, _)) = stripped.split_once('@') {
        stripped = head.to_string();
    }
    push(&stripped);
    // "-latest" / "-preview" style aliases and YYYYMMDD date pins.
    if let Some(head) = stripped.strip_suffix("-latest") {
        push(head);
    }
    if stripped.len() > 9 {
        let (head, tail) = stripped.split_at(stripped.len() - 9);
        if tail.starts_with('-') && tail[1..].chars().all(|c| c.is_ascii_digit()) {
            push(head);
        }
    }
    out
}

/// Look a model id up, tolerating provider prefixes, region/date suffixes and
/// a trailing variant the table does not know (longest known prefix on a
/// `-`/`.`/`@` boundary wins, so `gpt-5` never claims `gpt-5o-mini`).
pub fn lookup(model: &str) -> Option<Price> {
    let cands = candidates(model);
    for c in &cands {
        if let Some(p) = exact(c) {
            return Some(p);
        }
    }
    let mut best: Option<(&str, &Vec<f64>)> = None;
    for c in &cands {
        for (key, row) in &table().models {
            if key.len() < 5 || !c.starts_with(key.as_str()) {
                continue;
            }
            let boundary = c[key.len()..]
                .chars()
                .next()
                .is_none_or(|ch| matches!(ch, '-' | '.' | '@' | ':'));
            if boundary && best.is_none_or(|(k, _)| key.len() > k.len()) {
                best = Some((key, row));
            }
        }
        if best.is_some() {
            break;
        }
    }
    best.and_then(|(_, row)| row_to_price(row))
}

/// Whether the agent's `output` count already contains its `reasoning`
/// count (OpenAI-style accounting) rather than reporting thinking tokens
/// beside it (Gemini/OpenCode-style).
fn reasoning_inside_output(agent: AgentId) -> bool {
    matches!(agent, AgentId::Codex | AgentId::Grok)
}

/// Estimated USD cost of `counts` at the model's list price, or `None` when
/// the model is unknown.
pub fn estimate_cost(agent: AgentId, model: &str, counts: &TokenCounts) -> Option<f64> {
    let p = lookup(model)?;
    let reasoning = if reasoning_inside_output(agent) {
        0.0
    } else {
        counts.reasoning as f64 * p.reasoning.unwrap_or(p.output)
    };
    let usd = counts.input as f64 * p.input
        + counts.output as f64 * p.output
        + counts.cache_read as f64 * p.cache_read
        + counts.cache_write as f64 * p.cache_write
        + reasoning;
    Some(usd / 1_000_000.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_loads_and_has_flagship_models() {
        for m in ["claude-opus-5", "gpt-5", "gemini-2.5-pro"] {
            assert!(exact(m).is_some(), "{m} missing from the embedded table");
        }
        assert!(snapshot_date().starts_with("20"));
    }

    #[test]
    fn lookup_strips_provider_region_and_date_decorations() {
        let base = lookup("claude-opus-5").unwrap();
        for alias in [
            "anthropic/claude-opus-5",
            "us.anthropic.claude-opus-5",
            "claude-opus-5@default",
            "Claude-Opus-5",
            "claude-opus-5-20260724",
            r#"{"id":"claude-opus-5","providerID":"anthropic"}"#,
        ] {
            assert_eq!(lookup(alias), Some(base), "{alias}");
        }
    }

    #[test]
    fn lookup_falls_back_to_longest_known_prefix_on_a_boundary() {
        let base = lookup("gpt-5").unwrap();
        assert_eq!(lookup("gpt-5-unknown-variant"), Some(base));
        // A bare prefix must not swallow a different model family member.
        assert_ne!(lookup("gpt-5.5"), Some(base));
    }

    #[test]
    fn unknown_model_is_none_not_zero() {
        assert!(lookup("totally-unknown-model-9000").is_none());
        assert!(lookup("").is_none());
        assert!(lookup("llama3.2").is_none());
    }

    #[test]
    fn cost_bills_reasoning_as_output_except_where_output_already_holds_it() {
        let counts = TokenCounts {
            input: 1_000_000,
            output: 1_000_000,
            cache_read: 0,
            cache_write: 0,
            reasoning: 1_000_000,
        };
        let p = lookup("claude-opus-5").unwrap();
        let gemini_style = estimate_cost(AgentId::Gemini, "claude-opus-5", &counts).unwrap();
        let codex_style = estimate_cost(AgentId::Codex, "claude-opus-5", &counts).unwrap();
        assert!((gemini_style - (p.input + 2.0 * p.output)).abs() < 1e-9);
        assert!((codex_style - (p.input + p.output)).abs() < 1e-9);
    }
}
