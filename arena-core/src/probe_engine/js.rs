//! JavaScript fixture/validation evaluation via boa-engine, and the
//! `SecretMeta` redaction helper.

use std::collections::HashMap;

use boa_engine::{
    Context as JsContext, JsResult, JsValue, NativeFunction, Source, js_string, property::Attribute,
};
use serde::{Deserialize, Serialize};

use crate::probe_engine::error::ProbeEngineError;
use crate::probe_engine::js_wrap::{normalize_validation_script, wrap_js};

/// A boa context with runtime limits. Fixture, validation and judge-decision
/// scripts are authored by task and judge creators (untrusted), and boa's
/// `eval` is a blocking
/// interpreter loop — without a bound, `while (true) {}` would hang the thread
/// forever. The loop/recursion limits surface as a `JsEval` error instead. The
/// caps are far above anything a legitimate fixture generator needs.
pub(crate) fn bounded_js_context() -> JsContext {
    let mut ctx = JsContext::default();
    // 1M loop iterations is ~1000x more than any legitimate fixture generator
    // needs, while bounding a runaway to well under a second.
    ctx.runtime_limits_mut().set_loop_iteration_limit(1_000_000);
    ctx.runtime_limits_mut().set_recursion_limit(2_000);
    ctx
}

/// Result of evaluating a `js fixtures` block, including any `__secret`
/// metadata declared inline.
///
/// The `__secret` array names fixture keys to redact from clients (the
/// values stay in `fixtures` for command rendering + validation, but the
/// caller must strip them before persisting `fixture_values` JSON or
/// shipping `TestPush`/`PlayerProbeEntry` to the player). The
/// `__secret_expected` flag suppresses shipping `expected_answer`,
/// `resolved_answer`, and `answer_template` to the player — the server
/// still grades with them, but the client never sees the expected value.
///
/// Both metadata keys are removed from `fixtures` before return so the
/// validation script never sees them. Use double-underscore prefixes to
/// avoid collisions with normal fixture names.
#[derive(Debug, Clone, Default)]
pub struct JsFixturesOutcome {
    pub fixtures: serde_json::Map<String, serde_json::Value>,
    pub secret_keys: Vec<String>,
    pub secret_expected: bool,
}

/// Rewrite integral floats as integers, recursively.
///
/// Every JS number is an f64, so boa's `to_json` hands back `382.0` for a
/// fixture the script computed as `Math.floor(...) + 1`. Left alone, that
/// float leaks into every Rust-side stringification — the rendered command
/// carried `"382.0"` while the validation (running in JS, where
/// `String(382)` is `"382"`) expected `382`, failing a correct answer on
/// the platform's own formatting. JS itself cannot tell 382 from 382.0, so
/// collapsing the representation changes nothing for validation scripts.
fn normalize_json_numbers(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Number(n) => {
            if let Some(f) = n.as_f64()
                && n.as_i64().is_none()
                && f.fract() == 0.0
                // i64-exact range for f64 integers; beyond it the float is
                // the honest representation.
                && (-9_007_199_254_740_992.0..=9_007_199_254_740_992.0).contains(&f)
            {
                *n = serde_json::Number::from(f as i64);
            }
        }
        serde_json::Value::Array(items) => items.iter_mut().for_each(normalize_json_numbers),
        serde_json::Value::Object(map) => map.values_mut().for_each(normalize_json_numbers),
        _ => {}
    }
}

pub fn eval_js_fixtures_with_meta(script: &str) -> Result<JsFixturesOutcome, ProbeEngineError> {
    let mut ctx = bounded_js_context();
    let wrapped = wrap_js(script);
    let value = ctx
        .eval(Source::from_bytes(&wrapped))
        .map_err(|e| ProbeEngineError::JsEval(e.to_string()))?;
    let mut json = value
        .to_json(&mut ctx)
        .map_err(|e| ProbeEngineError::JsEval(e.to_string()))?
        .ok_or(ProbeEngineError::JsFixturesNotObject)?;
    normalize_json_numbers(&mut json);
    let mut map = match json {
        serde_json::Value::Object(map) => map,
        _ => return Err(ProbeEngineError::JsFixturesNotObject),
    };

    // Extract `__secret` (array of fixture key names to redact) and
    // `__secret_expected` (bool: suppress expected value shipping).
    // Both are removed from the fixture map so the validation script
    // never sees them.
    let secret_keys = match map.remove("__secret") {
        Some(serde_json::Value::Array(arr)) => arr
            .into_iter()
            .filter_map(|v| v.as_str().map(String::from))
            .collect(),
        _ => Vec::new(),
    };
    let secret_expected = matches!(
        map.remove("__secret_expected"),
        Some(serde_json::Value::Bool(true))
    );

    Ok(JsFixturesOutcome {
        fixtures: map,
        secret_keys,
        secret_expected,
    })
}

/// Parsed `probes.secret_meta` JSON: which fixture keys and the expected
/// value are secret (redacted from clients). Stored on the probe row so
/// both the game-server (WS dispatch) and the REST API can redact from
/// the same source of truth.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SecretMeta {
    /// Fixture key names to redact from `fixture_values` JSON before
    /// shipping to the client. The values stay in the fixture map for
    /// command rendering + validation; only the client-visible JSON is
    /// stripped.
    #[serde(default)]
    pub fixtures: Vec<String>,
    /// When true, suppress shipping `expected_answer`, `resolved_answer`,
    /// and `answer_template` to the client. The server still grades with
    /// them; the client never sees the expected value.
    #[serde(default)]
    pub expected: bool,
}

impl SecretMeta {
    /// Parse a `secret_meta` JSON blob. Returns `None` on missing/invalid
    /// input — callers treat `None` as "nothing is secret" (back-compat).
    pub fn parse(raw: &Option<String>) -> Option<Self> {
        let s = raw.as_ref()?;
        if s.trim().is_empty() {
            return None;
        }
        serde_json::from_str(s).ok()
    }

    /// True when there is nothing to redact (empty fixtures list and
    /// expected=false). Callers can skip redaction work entirely.
    pub fn is_empty(&self) -> bool {
        self.fixtures.is_empty() && !self.expected
    }

    /// Serialize to the storage format. `None` when empty (so the column
    /// stays NULL for back-compat probes with no secrets).
    pub fn to_json_string(&self) -> Option<String> {
        if self.is_empty() {
            None
        } else {
            serde_json::to_string(self).ok()
        }
    }

    /// Strip secret fixture keys from a `fixture_values` JSON string,
    /// returning a new JSON string with those keys removed (and their
    /// values replaced with the sentinel "<redacted>"). Returns the
    /// input unchanged when no secrets are declared or parsing fails.
    // ponytail: redact-in-place via serde_json round-trip; upgrade to a
    // streaming JSON editor if fixture_values blobs grow large.
    pub fn redact_fixture_values(&self, json: &str) -> String {
        if self.fixtures.is_empty() {
            return json.to_string();
        }
        let mut value: serde_json::Value = match serde_json::from_str(json) {
            Ok(v) => v,
            Err(_) => return json.to_string(),
        };
        if let Some(obj) = value.as_object_mut() {
            for key in &self.fixtures {
                if obj.contains_key(key) {
                    obj.insert(
                        key.clone(),
                        serde_json::Value::String("<redacted>".to_string()),
                    );
                }
            }
        }
        serde_json::to_string(&value).unwrap_or_else(|_| json.to_string())
    }
}

pub fn js_fixture_scalars(
    fixtures: &serde_json::Map<String, serde_json::Value>,
) -> HashMap<String, String> {
    fixtures
        .iter()
        .map(|(k, v)| {
            let scalar = match v {
                serde_json::Value::String(s) => s.clone(),
                serde_json::Value::Number(n) => n.to_string(),
                serde_json::Value::Bool(b) => b.to_string(),
                serde_json::Value::Null => String::new(),
                _ => v.to_string(),
            };
            (k.clone(), scalar)
        })
        .collect()
}

/// Outcome of a JS validation that may carry display values for `expected`
/// and `actual` (used by the frontend/TUI probe panel). When the validation
/// script does not call `assertEqual`, both fields are `None` and only
/// `pass` is meaningful — preserving back-compat with predicate templates.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ValidationOutcome {
    pub pass: bool,
    pub expected: Option<String>,
    pub actual: Option<String>,
}

/// Evaluate a `js validation` block and return a [`ValidationOutcome`] with
/// display values when the script calls `assertEqual(actual, expected)`.
///
/// `assertEqual` stashes `{expected, actual}` as a JS string on the global
/// object under `__assertion` and returns `actual === expected`. After
/// evaluation the engine reads that stash and clears it. Predicate
/// templates that never call `assertEqual` get `expected: None,
/// actual: None` (back-compat).
// ponytail: global-slot stash avoids Trace capture issues with Rc<RefCell>;
// upgrade to per-context captures if assertion helpers grow beyond one.
pub fn eval_js_validation_outcome(
    script: &str,
    fixtures: &serde_json::Map<String, serde_json::Value>,
    result: &str,
) -> Result<ValidationOutcome, ProbeEngineError> {
    eval_js_validation_outcome_with_memory(script, fixtures, &Default::default(), result)
}

/// Like [`eval_js_validation_outcome`], additionally registering `memory` as
/// a global object (raw — unquoted — values) so `{memory.key}` / `memory.key`
/// resolve in validation scripts.
pub fn eval_js_validation_outcome_with_memory(
    script: &str,
    fixtures: &serde_json::Map<String, serde_json::Value>,
    memory: &std::collections::BTreeMap<String, String>,
    result: &str,
) -> Result<ValidationOutcome, ProbeEngineError> {
    eval_js_validation_outcome_full(script, fixtures, memory, result, None)
}

/// Like [`eval_js_validation_outcome_with_memory`], additionally registering
/// `exit_code` (a number, 0 when the runner reported none) — judge-authored
/// validations reach for it, and without the global the natural
/// `exit_code === 0` can never hold.
pub fn eval_js_validation_outcome_full(
    script: &str,
    fixtures: &serde_json::Map<String, serde_json::Value>,
    memory: &std::collections::BTreeMap<String, String>,
    result: &str,
    exit_code: Option<i64>,
) -> Result<ValidationOutcome, ProbeEngineError> {
    let normalized = normalize_validation_script(script);
    let wrapped = wrap_js(&normalized);
    let mut ctx = bounded_js_context();

    for (k, v) in fixtures {
        let js_val =
            JsValue::from_json(v, &mut ctx).map_err(|e| ProbeEngineError::JsEval(e.to_string()))?;
        ctx.register_global_property(js_string!(k.as_str()), js_val, Attribute::all())
            .map_err(|e| ProbeEngineError::JsEval(e.to_string()))?;
    }
    if !memory.is_empty() {
        let obj = serde_json::Value::Object(crate::memory::memory_json_object(memory));
        let js_val = JsValue::from_json(&obj, &mut ctx)
            .map_err(|e| ProbeEngineError::JsEval(e.to_string()))?;
        ctx.register_global_property(js_string!("memory"), js_val, Attribute::all())
            .map_err(|e| ProbeEngineError::JsEval(e.to_string()))?;
    }
    ctx.register_global_property(js_string!("result"), js_string!(result), Attribute::all())
        .map_err(|e| ProbeEngineError::JsEval(e.to_string()))?;
    ctx.register_global_property(
        js_string!("exit_code"),
        JsValue::from(exit_code.unwrap_or(0) as f64),
        Attribute::all(),
    )
    .map_err(|e| ProbeEngineError::JsEval(e.to_string()))?;

    // Register `assertEqual(actual, expected)`: stashes both operands as
    // strings on the global object and returns the boolean comparison.
    ctx.register_global_builtin_callable(
        js_string!("assertEqual"),
        2,
        NativeFunction::from_fn_ptr(assert_equal_fn),
    )
    .map_err(|e| ProbeEngineError::JsEval(e.to_string()))?;

    let value = ctx
        .eval(Source::from_bytes(&wrapped))
        .map_err(|e| ProbeEngineError::JsEval(e.to_string()))?;
    let json = value
        .to_json(&mut ctx)
        .map_err(|e| ProbeEngineError::JsEval(e.to_string()))?
        .ok_or(ProbeEngineError::JsValidationNotBool)?;
    let pass = match json {
        serde_json::Value::Bool(b) => b,
        _ => return Err(ProbeEngineError::JsValidationNotBool),
    };

    // Read the assertion stash left by `assertEqual`, if any. A missing
    // global property reads back as `undefined` — treat that as "no
    // assertion was made" (predicate-template back-compat).
    let actual = ctx
        .global_object()
        .get(js_string!("__assertion_actual"), &mut ctx)
        .ok()
        .filter(|v| !v.is_undefined())
        .and_then(|v| v.to_string(&mut ctx).ok())
        .map(|s| s.to_std_string_escaped())
        .filter(|s| !s.is_empty());
    let expected = ctx
        .global_object()
        .get(js_string!("__assertion_expected"), &mut ctx)
        .ok()
        .filter(|v| !v.is_undefined())
        .and_then(|v| v.to_string(&mut ctx).ok())
        .map(|s| s.to_std_string_escaped())
        .filter(|s| !s.is_empty());

    Ok(ValidationOutcome {
        pass,
        expected,
        actual,
    })
}

fn assert_equal_fn(_: &JsValue, args: &[JsValue], ctx: &mut JsContext) -> JsResult<JsValue> {
    let a = args.first().cloned().unwrap_or(JsValue::undefined());
    let b = args.get(1).cloned().unwrap_or(JsValue::undefined());
    let a_str = a.to_string(ctx)?.to_std_string_escaped();
    let b_str = b.to_string(ctx)?.to_std_string_escaped();
    let _ = ctx.global_object().set(
        js_string!("__assertion_actual"),
        js_string!(a_str.as_str()),
        false,
        ctx,
    );
    let _ = ctx.global_object().set(
        js_string!("__assertion_expected"),
        js_string!(b_str.as_str()),
        false,
        ctx,
    );
    Ok(JsValue::from(a.strict_equals(&b)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn fixtures_object_evaluates() {
        let out = eval_js_fixtures_with_meta("({ a: 2, b: 'x' })")
            .unwrap()
            .fixtures;
        assert_eq!(out.get("a"), Some(&json!(2)));
        assert_eq!(out.get("b"), Some(&json!("x")));
    }

    #[test]
    fn infinite_loop_is_bounded_not_hung() {
        // PERF-H2: a runaway loop in an (untrusted) fixture script must return an
        // error via the runtime loop-iteration limit rather than spin forever.
        let err = eval_js_fixtures_with_meta("while (true) {}");
        assert!(
            matches!(err, Err(ProbeEngineError::JsEval(_))),
            "expected a bounded JsEval error, got {err:?}"
        );
    }

    #[test]
    fn secret_meta_extracted_and_removed() {
        let out =
            eval_js_fixtures_with_meta("({ v: 1, __secret: ['v'], __secret_expected: true })")
                .unwrap();
        assert_eq!(out.secret_keys, vec!["v".to_string()]);
        assert!(out.secret_expected);
        assert!(out.fixtures.get("v").is_some());
        assert!(out.fixtures.get("__secret").is_none());
    }

    #[test]
    fn secret_meta_redacts_and_roundtrips() {
        let meta = SecretMeta {
            fixtures: vec!["answer".into()],
            expected: false,
        };
        let redacted = meta.redact_fixture_values(r#"{"answer":"Paris","q":"?"}"#);
        assert!(redacted.contains(r#""answer":"<redacted>""#));
        assert!(redacted.contains(r#""q":"?""#));

        let s = meta.to_json_string().unwrap();
        assert!(SecretMeta::parse(&Some(s)).is_some());
        assert!(SecretMeta::default().to_json_string().is_none());
    }

    #[test]
    fn validation_sees_exit_code_global() {
        let fx = serde_json::Map::new();
        let ok = eval_js_validation_outcome_full(
            "exit_code === 0 && result.includes(\"pass\")",
            &fx,
            &Default::default(),
            "16 tests pass",
            Some(0),
        )
        .unwrap();
        assert!(ok.pass);
        let no = eval_js_validation_outcome_full(
            "exit_code === 0",
            &fx,
            &Default::default(),
            "",
            Some(2),
        )
        .unwrap();
        assert!(!no.pass);
        // Judge-authored `return result.exit_code === 0` normalizes onto the
        // real global instead of reading `undefined` off the stdout string.
        let judge_style = eval_js_validation_outcome_full(
            "return result.exit_code === 0",
            &fx,
            &Default::default(),
            "all good",
            Some(0),
        )
        .unwrap();
        assert!(judge_style.pass);
        // Legacy entry points default the exit code to 0.
        let legacy = eval_js_validation_outcome("exit_code === 0", &fx, "x").unwrap();
        assert!(legacy.pass);
    }

    #[test]
    fn validation_captures_assert_equal_display() {
        let mut fx = serde_json::Map::new();
        fx.insert("n".to_string(), json!(7));
        let out =
            eval_js_validation_outcome("assertEqual(Number(result.trim()), n)", &fx, "7").unwrap();
        assert!(out.pass);
        assert_eq!(out.expected.as_deref(), Some("7"));
        assert_eq!(out.actual.as_deref(), Some("7"));
    }

    #[test]
    fn predicate_without_assert_equal_has_no_display() {
        let mut fx = serde_json::Map::new();
        fx.insert("n".to_string(), json!(7));
        let out = eval_js_validation_outcome("Number(result.trim()) === n", &fx, "7").unwrap();
        assert!(out.pass);
        assert!(out.expected.is_none());
        assert!(out.actual.is_none());
    }
}

#[cfg(test)]
mod integral_number_tests {
    use super::*;

    /// The fizzbuzz "plain number" regression: `Math.floor(...) + 1` is an
    /// integer to JS but an f64 to boa, and the old passthrough rendered it
    /// into the command as `"382.0"` while the JS-side validation expected
    /// `382`. Integral fixture numbers must stringify without the float tail.
    #[test]
    fn integral_js_numbers_stringify_without_float_tail() {
        let out = eval_js_fixtures_with_meta(
            r#"({ n: Math.floor(381.9) + 1, half: 3.5, neg: -(2 ** 10), big: 1e300 })"#,
        )
        .expect("fixtures eval");
        let scalars = js_fixture_scalars(&out.fixtures);
        assert_eq!(scalars["n"], "382");
        assert_eq!(scalars["neg"], "-1024");
        assert_eq!(scalars["half"], "3.5", "true fractions keep their point");
        assert!(
            scalars["big"].contains('e')
                || scalars["big"].ends_with(".0")
                || scalars["big"].len() > 100,
            "out-of-i64-range floats stay floats: {}",
            scalars["big"]
        );
    }

    /// Nested containers are normalized too — a fixture list of computed
    /// integers must not ship `[1.0, 2.0]` to persisted JSON.
    #[test]
    fn nested_numbers_are_normalized() {
        let out = eval_js_fixtures_with_meta(r#"({ list: [1, 2, 3].map((x) => x * 10) })"#)
            .expect("fixtures eval");
        assert_eq!(
            serde_json::to_string(&out.fixtures["list"]).unwrap(),
            "[10,20,30]"
        );
    }
}
