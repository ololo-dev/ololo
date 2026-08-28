//! Answer-template evaluation (minijinja) and fixture-context builders.

use std::collections::HashMap;

use serde_json::Value;

use crate::probe_engine::error::ProbeEngineError;
use crate::probe_engine::fixtures::FixtureSample;
use crate::probe_engine::render::minijinja_err;
use crate::task_template::{FixtureDef, FixtureKind};

/// Evaluate `template` as a minijinja expression against the fixture context plus
/// the actual probe `result` (trimmed stdout).
///
/// The context contains:
/// - `result` — the actual probe output (trimmed), available for predicate assertions.
/// - `{{X}}` — scalar string: the sampled key for `KeyValue` fixtures, the number
///   for `NumericRange` fixtures (as a JSON integer for arithmetic).
/// - `{{X_map}}` — the full key→value pairs map, for `KeyValue` fixtures only.
///
/// ## Return value
/// Returns `true` (pass) or `false` (fail):
/// - If the expression evaluates to a **boolean** → used directly as pass/fail.
/// - If the expression evaluates to a **string or number** → compared (trimmed) to
///   `result` for equality.
pub fn evaluate_answer(
    template: &str,
    defs: &[FixtureDef],
    samples: &HashMap<String, FixtureSample>,
    result: &str,
) -> Result<bool, ProbeEngineError> {
    evaluate_answer_with_memory(template, defs, samples, &Default::default(), result)
}

/// Like [`evaluate_answer`], additionally binding `memory` as a nested object
/// (raw — unquoted — values) so `{{memory.key}}` resolves in answer templates.
pub fn evaluate_answer_with_memory(
    template: &str,
    defs: &[FixtureDef],
    samples: &HashMap<String, FixtureSample>,
    memory: &std::collections::BTreeMap<String, String>,
    result: &str,
) -> Result<bool, ProbeEngineError> {
    let trimmed = template.trim();
    if trimmed.is_empty() {
        return Err(ProbeEngineError::SyntaxError("empty template".into()));
    }
    let mut ctx = build_full_context(defs, samples);
    if !memory.is_empty()
        && let Some(obj) = ctx.as_object_mut()
    {
        obj.insert(
            "memory".to_string(),
            Value::Object(crate::memory::memory_json_object(memory)),
        );
    }
    eval_answer_with_ctx(trimmed, &ctx, result)
}

/// Evaluate an answer template expression against a **pre-built fixture context**
/// and the actual probe `result`.
///
/// Callers build and store the context at dispatch time, then evaluate lazily
/// when the probe result arrives.
pub fn eval_answer_with_ctx(
    template: &str,
    fixture_ctx: &Value,
    result: &str,
) -> Result<bool, ProbeEngineError> {
    // Clone and inject `result` so predicate expressions can reference it.
    let mut ctx = fixture_ctx.clone();
    if let Some(obj) = ctx.as_object_mut() {
        obj.insert("result".to_string(), Value::String(result.to_string()));
    }

    let mut env = minijinja::Environment::new();
    env.set_auto_escape_callback(|_| minijinja::AutoEscape::None);
    env.set_undefined_behavior(minijinja::UndefinedBehavior::Strict);

    let expression = env.compile_expression(template).map_err(minijinja_err)?;
    let value = expression.eval(&ctx).map_err(minijinja_err)?;

    // Serialize to serde_json::Value to detect boolean vs string/number.
    let json_val =
        serde_json::to_value(&value).unwrap_or_else(|_| Value::String(value.to_string()));

    match json_val {
        // Boolean: predicate result — use directly.
        Value::Bool(b) => Ok(b),
        // String: value assertion — compare trimmed to result.
        Value::String(s) => Ok(s.trim() == result.trim()),
        // Number: value assertion — compare decimal string to result.
        Value::Number(n) => Ok(n.to_string() == result.trim()),
        // Other (array, object, null): convert to string and compare.
        #[allow(clippy::cmp_owned)]
        other => Ok(other.to_string() == result.trim()),
    }
}

/// Build the full context for answer evaluation:
/// - `X` as a JSON integer for `NumericRange` (enables arithmetic), or the sampled
///   key string for `KeyValue`.
/// - `X_map` as a JSON object for `KeyValue` fixtures.
///
/// Note: `result` is NOT injected here — callers inject it via [`eval_answer_with_ctx`].
pub fn build_full_context(defs: &[FixtureDef], samples: &HashMap<String, FixtureSample>) -> Value {
    let mut map = serde_json::Map::new();
    for def in defs {
        if let Some(s) = samples.get(&def.name) {
            match &def.kind {
                FixtureKind::NumericRange { .. } => {
                    // Store as JSON number so minijinja arithmetic works without casting.
                    let n: i64 = s.value.parse().unwrap_or(0);
                    map.insert(def.name.clone(), Value::Number(n.into()));
                }
                FixtureKind::KeyValue { pairs } => {
                    // Scalar slot: the sampled key.
                    let key = s.key.as_deref().unwrap_or(&s.value);
                    map.insert(def.name.clone(), Value::String(key.to_string()));
                    // Dict slot: full pairs map for lookup formulas.
                    let pairs_obj: serde_json::Map<String, Value> = pairs
                        .iter()
                        .map(|(k, v)| (k.clone(), Value::String(v.clone())))
                        .collect();
                    map.insert(format!("{}_map", def.name), Value::Object(pairs_obj));
                }
            }
        }
    }
    Value::Object(map)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::probe_engine::test_util::{fixed, numeric};

    #[test]
    fn numeric_addition_value_assertion() {
        let defs = vec![numeric("a", 6, 6), numeric("b", 2, 2)];
        let samples = fixed("a", "6");
        let samples2 = fixed("b", "2");
        let mut s = samples;
        s.extend(samples2);
        assert!(evaluate_answer("a + b", &defs, &s, "8").unwrap());
        assert!(!evaluate_answer("a + b", &defs, &s, "9").unwrap());
    }

    #[test]
    fn predicate_result_not_empty() {
        let defs = vec![];
        let samples = HashMap::new();
        assert!(evaluate_answer("result != \"\"", &defs, &samples, "x").unwrap());
        assert!(!evaluate_answer("result != \"\"", &defs, &samples, "").unwrap());
    }

    #[test]
    fn empty_template_is_error() {
        let defs = vec![];
        let samples = HashMap::new();
        assert!(evaluate_answer("", &defs, &samples, "").is_err());
    }

    #[test]
    fn ctx_evaluation_injects_result() {
        let ctx = serde_json::json!({});
        assert!(eval_answer_with_ctx("result != \"\"", &ctx, "x").unwrap());
        assert!(!eval_answer_with_ctx("result != \"\"", &ctx, "").unwrap());
    }
}
