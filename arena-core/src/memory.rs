//! Per-player session memory.
//!
//! A project may declare a `memory_schema`: a flat JSON object mapping
//! memory keys to default values, e.g. `{"dev": "npm run dev", "port": 1234}`.
//! During a session an LLM extracts the player's actual values from their
//! markdown files (AGENTS.md / README.md); extracted values override the
//! defaults. The merged map is injected into probe templates as
//! `{memory.<key>}` alongside fixtures.
//!
//! Invariants enforced here:
//! - only scalar values (string / number / bool), stringified for rendering;
//! - only schema keys survive a merge (extraction cannot invent keys);
//! - keys and values are length-capped (player-authored content).

use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter};
use std::collections::BTreeMap;
use uuid::Uuid;

/// Maximum number of keys a memory schema may declare.
pub const MAX_MEMORY_KEYS: usize = 32;
/// Maximum length of a memory key.
pub const MAX_MEMORY_KEY_LEN: usize = 64;
/// Maximum length of a memory value (defaults and extracted alike).
pub const MAX_MEMORY_VALUE_LEN: usize = 500;

/// The player-authored files memory is extracted from, in priority order
/// (earlier files win on conflicting values — mirrored in the prompt).
///
/// Shared so the two sides cannot disagree: `game-server` reads exactly these
/// from the pushed repo, and `ololo` watches exactly these for changes worth
/// pushing. A file added here starts being both watched and read.
pub const MEMORY_SOURCE_FILES: [&str; 2] = ["AGENTS.md", "README.md"];

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum MemorySchemaError {
    #[error("memory schema must be a JSON object")]
    NotAnObject,
    #[error("memory schema declares more than {MAX_MEMORY_KEYS} keys")]
    TooManyKeys,
    #[error("invalid memory key '{0}': keys must be 1-{MAX_MEMORY_KEY_LEN} chars of [a-zA-Z0-9_]")]
    InvalidKey(String),
    #[error("invalid value for memory key '{0}': must be a string, number, or boolean")]
    NonScalarValue(String),
    #[error("value for memory key '{0}' exceeds {MAX_MEMORY_VALUE_LEN} chars")]
    ValueTooLong(String),
}

fn valid_key(key: &str) -> bool {
    !key.is_empty()
        && key.len() <= MAX_MEMORY_KEY_LEN
        && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn scalar_to_string(v: &serde_json::Value) -> Option<String> {
    match v {
        serde_json::Value::String(s) => Some(s.clone()),
        serde_json::Value::Number(n) => Some(n.to_string()),
        serde_json::Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

/// Validate and flatten a `memory_schema` JSON value into an ordered
/// `key -> default` map. Used both by project CRUD/seed validation and at
/// probe-render time.
pub fn parse_memory_schema(
    schema: &serde_json::Value,
) -> Result<BTreeMap<String, String>, MemorySchemaError> {
    let obj = schema.as_object().ok_or(MemorySchemaError::NotAnObject)?;
    if obj.len() > MAX_MEMORY_KEYS {
        return Err(MemorySchemaError::TooManyKeys);
    }
    let mut out = BTreeMap::new();
    for (k, v) in obj {
        if !valid_key(k) {
            return Err(MemorySchemaError::InvalidKey(k.clone()));
        }
        let s = scalar_to_string(v).ok_or_else(|| MemorySchemaError::NonScalarValue(k.clone()))?;
        if s.len() > MAX_MEMORY_VALUE_LEN {
            return Err(MemorySchemaError::ValueTooLong(k.clone()));
        }
        out.insert(k.clone(), s);
    }
    Ok(out)
}

/// Parse the `projects.memory_schema` column. `None`/empty → empty map;
/// malformed JSON → empty map (logged by callers), so a bad schema can never
/// break probe dispatch.
pub fn parse_memory_schema_column(column: Option<&str>) -> BTreeMap<String, String> {
    let Some(raw) = column else {
        return BTreeMap::new();
    };
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return BTreeMap::new();
    }
    serde_json::from_str::<serde_json::Value>(trimmed)
        .ok()
        .and_then(|v| parse_memory_schema(&v).ok())
        .unwrap_or_default()
}

/// Filter an extraction result down to usable entries: only schema keys,
/// only scalars, trimmed, non-empty, length-capped. Returns just the
/// extracted entries (no defaults mixed in) so the stored row stays a pure
/// record of what was extracted.
pub fn filter_extracted_values(
    defaults: &BTreeMap<String, String>,
    extracted: &serde_json::Value,
) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    if let Some(obj) = extracted.as_object() {
        for (k, v) in obj {
            if !defaults.contains_key(k) {
                continue;
            }
            if let Some(s) = scalar_to_string(v) {
                let s = s.trim().to_string();
                if !s.is_empty() && s.len() <= MAX_MEMORY_VALUE_LEN {
                    out.insert(k.clone(), s);
                }
            }
        }
    }
    out
}

/// Merge extracted values over schema defaults. Only schema keys survive;
/// non-scalar or over-long extracted values fall back to the default.
pub fn merge_memory(
    defaults: &BTreeMap<String, String>,
    extracted: &serde_json::Value,
) -> BTreeMap<String, String> {
    let mut merged = defaults.clone();
    merged.extend(filter_extracted_values(defaults, extracted));
    merged
}

/// `memory.<key>` tokens for `normalize_brace_placeholders`, so authors can
/// write `{memory.key}` in command templates.
pub fn memory_placeholder_names(memory: &BTreeMap<String, String>) -> Vec<String> {
    memory.keys().map(|k| format!("memory.{k}")).collect()
}

/// The merged memory map as a JSON object value (for validation/answer
/// contexts where raw — unquoted — values are wanted).
pub fn memory_json_object(
    memory: &BTreeMap<String, String>,
) -> serde_json::Map<String, serde_json::Value> {
    memory
        .iter()
        .map(|(k, v)| (k.clone(), serde_json::Value::String(v.clone())))
        .collect()
}

/// Load the merged memory map for `(session, player)`: project defaults
/// overlaid with the latest extracted values. Empty when the project
/// declares no memory schema. Never fails — errors degrade to defaults.
pub async fn load_memory_map(
    db: &DatabaseConnection,
    session_id: Uuid,
    player_id: Uuid,
    memory_schema: Option<&str>,
) -> BTreeMap<String, String> {
    let defaults = parse_memory_schema_column(memory_schema);
    if defaults.is_empty() {
        return defaults;
    }
    let row = crate::entities::player_memory::Entity::find()
        .filter(crate::entities::player_memory::Column::SessionIdFk.eq(session_id))
        .filter(crate::entities::player_memory::Column::PlayerIdFk.eq(player_id))
        .one(db)
        .await;
    match row {
        Ok(Some(m)) => match serde_json::from_str::<serde_json::Value>(&m.values_json) {
            Ok(extracted) => merge_memory(&defaults, &extracted),
            Err(e) => {
                tracing::warn!(session_id = %session_id, player_id = %player_id, error = %e, "session_memory: bad values_json, using defaults");
                defaults
            }
        },
        Ok(None) => defaults,
        Err(e) => {
            tracing::warn!(session_id = %session_id, player_id = %player_id, error = %e, "session_memory: load failed, using defaults");
            defaults
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_scalar_schema() {
        let m =
            parse_memory_schema(&json!({"dev": "npm run dev", "port": 1234, "on": true})).unwrap();
        assert_eq!(m.get("dev").unwrap(), "npm run dev");
        assert_eq!(m.get("port").unwrap(), "1234");
        assert_eq!(m.get("on").unwrap(), "true");
    }

    #[test]
    fn rejects_non_scalar_values_and_bad_keys() {
        assert_eq!(
            parse_memory_schema(&json!({"a": [1]})),
            Err(MemorySchemaError::NonScalarValue("a".into()))
        );
        assert_eq!(
            parse_memory_schema(&json!({"bad key": "x"})),
            Err(MemorySchemaError::InvalidKey("bad key".into()))
        );
        assert_eq!(
            parse_memory_schema(&json!([1, 2])),
            Err(MemorySchemaError::NotAnObject)
        );
    }

    #[test]
    fn column_parse_is_forgiving() {
        assert!(parse_memory_schema_column(None).is_empty());
        assert!(parse_memory_schema_column(Some("")).is_empty());
        assert!(parse_memory_schema_column(Some("not json")).is_empty());
        let m = parse_memory_schema_column(Some(r#"{"port": 8080}"#));
        assert_eq!(m.get("port").unwrap(), "8080");
    }

    #[test]
    fn merge_keeps_only_schema_keys_and_falls_back() {
        let defaults = parse_memory_schema(&json!({"dev": "npm run dev", "port": 1234})).unwrap();
        let merged = merge_memory(
            &defaults,
            &json!({"dev": "bun dev", "port": {"nested": true}, "invented": "x"}),
        );
        assert_eq!(merged.get("dev").unwrap(), "bun dev");
        assert_eq!(merged.get("port").unwrap(), "1234"); // non-scalar → default
        assert!(!merged.contains_key("invented"));
    }

    #[test]
    fn merge_caps_and_trims_values() {
        let defaults = parse_memory_schema(&json!({"cmd": "default"})).unwrap();
        let long = "x".repeat(MAX_MEMORY_VALUE_LEN + 1);
        let merged = merge_memory(&defaults, &json!({"cmd": long}));
        assert_eq!(merged.get("cmd").unwrap(), "default");
        let merged = merge_memory(&defaults, &json!({"cmd": "  spaced  "}));
        assert_eq!(merged.get("cmd").unwrap(), "spaced");
    }

    #[test]
    fn placeholder_names_are_dotted() {
        let m = parse_memory_schema(&json!({"dev": "x"})).unwrap();
        assert_eq!(memory_placeholder_names(&m), vec!["memory.dev".to_string()]);
    }
}
