//! Integration tests for `arena_core::probe_engine` — answer-template
//! evaluation (minijinja) and pre-evaluation.

use std::collections::HashMap;

use arena_core::probe_engine::*;
use arena_core::task_template::{FixtureDef, FixtureKind};

fn make_numeric_def(name: &str, min: i64, max: i64) -> FixtureDef {
    FixtureDef {
        name: name.to_string(),
        kind: FixtureKind::NumericRange { min, max },
    }
}

fn make_kv_def(name: &str, pairs: &[(&str, &str)]) -> FixtureDef {
    FixtureDef {
        name: name.to_string(),
        kind: FixtureKind::KeyValue {
            pairs: pairs
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        },
    }
}

fn fixed_samples(pairs: &[(&str, Option<&str>, &str)]) -> HashMap<String, FixtureSample> {
    pairs
        .iter()
        .map(|(name, key, val)| {
            (
                name.to_string(),
                FixtureSample {
                    key: key.map(str::to_string),
                    value: val.to_string(),
                },
            )
        })
        .collect()
}

// --- evaluate_answer: value assertions (equality) ---

#[test]
fn eval_addition_pass() {
    let defs = vec![make_numeric_def("a", 6, 6), make_numeric_def("b", 2, 2)];
    let samples = fixed_samples(&[("a", None, "6"), ("b", None, "2")]);
    assert!(evaluate_answer("a + b", &defs, &samples, "8").unwrap());
}

#[test]
fn eval_addition_fail() {
    let defs = vec![make_numeric_def("a", 6, 6), make_numeric_def("b", 2, 2)];
    let samples = fixed_samples(&[("a", None, "6"), ("b", None, "2")]);
    assert!(!evaluate_answer("a + b", &defs, &samples, "9").unwrap());
}

#[test]
fn eval_subtraction() {
    let defs = vec![make_numeric_def("a", 6, 6), make_numeric_def("b", 2, 2)];
    let samples = fixed_samples(&[("a", None, "6"), ("b", None, "2")]);
    assert!(evaluate_answer("a - b", &defs, &samples, "4").unwrap());
}

#[test]
fn eval_multiplication() {
    let defs = vec![make_numeric_def("a", 6, 6), make_numeric_def("b", 2, 2)];
    let samples = fixed_samples(&[("a", None, "6"), ("b", None, "2")]);
    assert!(evaluate_answer("a * b", &defs, &samples, "12").unwrap());
}

#[test]
fn eval_integer_division() {
    let defs = vec![make_numeric_def("a", 7, 7), make_numeric_def("b", 2, 2)];
    let samples = fixed_samples(&[("a", None, "7"), ("b", None, "2")]);
    // Integer (floor) division: 7 // 2 = 3
    assert!(evaluate_answer("a // b", &defs, &samples, "3").unwrap());
}

#[test]
fn eval_operator_precedence() {
    let defs = vec![make_numeric_def("x", 6, 6), make_numeric_def("y", 2, 2)];
    let samples = fixed_samples(&[("x", None, "6"), ("y", None, "2")]);
    assert!(evaluate_answer("x + y * 3", &defs, &samples, "12").unwrap());
    assert!(evaluate_answer("(x + y) * 3", &defs, &samples, "24").unwrap());
}

#[test]
fn eval_string_literal_pass() {
    let defs = vec![];
    let samples = fixed_samples(&[]);
    assert!(evaluate_answer("\"Passed\"", &defs, &samples, "Passed").unwrap());
}

#[test]
fn eval_string_literal_fail() {
    let defs = vec![];
    let samples = fixed_samples(&[]);
    assert!(!evaluate_answer("\"Passed\"", &defs, &samples, "Failed").unwrap());
}

#[test]
fn eval_string_literal_single_quote() {
    let defs = vec![];
    let samples = fixed_samples(&[]);
    assert!(evaluate_answer("'some text'", &defs, &samples, "some text").unwrap());
}

#[test]
fn eval_dict_lookup() {
    let defs = vec![make_kv_def(
        "country",
        &[("France", "Paris"), ("Germany", "Berlin")],
    )];
    let samples = fixed_samples(&[("country", Some("France"), "Paris")]);
    assert!(evaluate_answer("country_map[country]", &defs, &samples, "Paris").unwrap());
    assert!(!evaluate_answer("country_map[country]", &defs, &samples, "Berlin").unwrap());
}

// --- evaluate_answer: predicate assertions (boolean) ---

#[test]
fn eval_predicate_not_empty_passes_for_nonempty_result() {
    let defs = vec![];
    let samples = fixed_samples(&[]);
    assert!(evaluate_answer("result != \"\"", &defs, &samples, "hello").unwrap());
}

#[test]
fn eval_predicate_not_empty_fails_for_empty_result() {
    let defs = vec![];
    let samples = fixed_samples(&[]);
    assert!(!evaluate_answer("result != \"\"", &defs, &samples, "").unwrap());
}

#[test]
fn eval_predicate_not_equal() {
    let defs = vec![];
    let samples = fixed_samples(&[]);
    assert!(evaluate_answer("result != \"Failed\"", &defs, &samples, "Passed").unwrap());
    assert!(!evaluate_answer("result != \"Failed\"", &defs, &samples, "Failed").unwrap());
}

#[test]
fn eval_predicate_contains() {
    let defs = vec![];
    let samples = fixed_samples(&[]);
    assert!(
        evaluate_answer(
            "\"0 tests failed\" in result",
            &defs,
            &samples,
            "All 0 tests failed"
        )
        .unwrap()
    );
    assert!(
        !evaluate_answer(
            "\"0 tests failed\" in result",
            &defs,
            &samples,
            "1 test failed"
        )
        .unwrap()
    );
}

#[test]
fn eval_predicate_not_contains() {
    let defs = vec![];
    let samples = fixed_samples(&[]);
    assert!(evaluate_answer("\"errors\" not in result", &defs, &samples, "all clear").unwrap());
    assert!(
        !evaluate_answer(
            "\"errors\" not in result",
            &defs,
            &samples,
            "2 errors found"
        )
        .unwrap()
    );
}

#[test]
fn eval_predicate_with_fixture_and_result() {
    // Combining: result must not equal the fixture value
    let defs = vec![make_kv_def("word", &[("hello", "world")])];
    let samples = fixed_samples(&[("word", Some("hello"), "world")]);
    // result != word (the key "hello")
    assert!(evaluate_answer("result != word", &defs, &samples, "goodbye").unwrap());
    assert!(!evaluate_answer("result != word", &defs, &samples, "hello").unwrap());
}

// --- evaluate_answer: edge cases ---

#[test]
fn eval_empty_template_errors() {
    let defs = vec![];
    let samples = fixed_samples(&[]);
    assert!(evaluate_answer("", &defs, &samples, "").is_err());
    assert!(evaluate_answer("   ", &defs, &samples, "").is_err());
}

#[test]
fn eval_undefined_var_errors() {
    let defs = vec![];
    let samples = fixed_samples(&[]);
    assert!(evaluate_answer("missing_var + 1", &defs, &samples, "").is_err());
}

#[test]
fn eval_result_trimmed_comparison() {
    // Value assertions trim the result before comparing.
    let defs = vec![];
    let samples = fixed_samples(&[]);
    assert!(evaluate_answer("\"Passed\"", &defs, &samples, "  Passed  ").unwrap());
}

// --- eval_answer_with_ctx ---

#[test]
fn eval_with_ctx_works_for_predicate() {
    let ctx = serde_json::json!({});
    assert!(eval_answer_with_ctx("result != \"\"", &ctx, "non-empty").unwrap());
    assert!(!eval_answer_with_ctx("result != \"\"", &ctx, "").unwrap());
}

#[test]
fn eval_with_ctx_contains_check() {
    let ctx = serde_json::json!({});
    assert!(eval_answer_with_ctx("\"PASS\" in result", &ctx, "PASS: all good").unwrap());
    assert!(!eval_answer_with_ctx("\"PASS\" in result", &ctx, "FAIL: something broke").unwrap());
}
