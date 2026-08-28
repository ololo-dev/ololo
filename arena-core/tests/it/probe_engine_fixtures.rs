//! Integration tests for `arena_core::probe_engine` — fixture sampling and
//! command rendering.

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

#[test]
fn sample_numeric_range_has_no_key() {
    let defs = vec![make_numeric_def("n", 1, 10)];
    let mut rng = rand::thread_rng();
    let map = sample_fixtures(&defs, &mut rng).unwrap();
    let s = &map["n"];
    assert!(s.key.is_none());
    let n: i64 = s.value.parse().unwrap();
    assert!((1..=10).contains(&n));
}

#[test]
fn sample_key_value_exposes_key_and_value() {
    let defs = vec![make_kv_def("kv", &[("foo", "bar")])];
    let mut rng = rand::thread_rng();
    let map = sample_fixtures(&defs, &mut rng).unwrap();
    let s = &map["kv"];
    assert_eq!(s.key.as_deref(), Some("foo"));
    assert_eq!(s.value, "bar");
}

#[test]
fn sample_empty_pool_errors() {
    let defs = vec![make_kv_def("empty", &[])];
    let mut rng = rand::thread_rng();
    assert!(sample_fixtures(&defs, &mut rng).is_err());
}

#[test]
fn render_substitutes_numeric_value() {
    let defs = vec![make_numeric_def("n", 5, 5)];
    let samples = fixed_samples(&[("n", None, "5")]);
    assert_eq!(
        render_command("echo {{n}}", &defs, &samples).unwrap(),
        "echo 5"
    );
}

#[test]
fn render_kv_substitutes_key_not_value() {
    let defs = vec![make_kv_def("country", &[("France", "Paris")])];
    let samples = fixed_samples(&[("country", Some("France"), "Paris")]);
    let out = render_command("curl http://localhost/?q={{country}}", &defs, &samples).unwrap();
    assert_eq!(out, "curl http://localhost/?q=France");
}

#[test]
fn render_undefined_var_errors() {
    let defs = vec![make_numeric_def("n", 1, 1)];
    let samples = fixed_samples(&[("n", None, "1")]);
    assert!(render_command("echo {{missing}}", &defs, &samples).is_err());
}

#[test]
fn render_multiple_vars() {
    let defs = vec![make_numeric_def("a", 1, 1), make_numeric_def("b", 2, 2)];
    let samples = fixed_samples(&[("a", None, "1"), ("b", None, "2")]);
    assert_eq!(
        render_command("{{a}} + {{b}}", &defs, &samples).unwrap(),
        "1 + 2"
    );
}

#[test]
fn normalize_single_brace_placeholders() {
    let names = vec!["a".to_string(), "b".to_string()];
    let out = normalize_brace_placeholders("echo {a}-{b}", &names);
    assert_eq!(out, "echo {{a}}-{{b}}");
}

#[test]
fn normalize_then_render_single_brace_template() {
    // Regression: single-brace {x} placeholders in command_template must be
    // substituted, not passed through as literal text.
    let template = "test -f {htmlFile} && test -f {cssFile}";
    let names: Vec<String> = vec!["htmlFile".into(), "cssFile".into()];
    let normalized = normalize_brace_placeholders(template, &names);
    let mut scalars = HashMap::new();
    scalars.insert("htmlFile".to_string(), "./index.html".to_string());
    scalars.insert("cssFile".to_string(), "./styles.css".to_string());
    let rendered = render_command_quoted(&normalized, &scalars).unwrap();
    assert_eq!(rendered, "test -f ./index.html && test -f ./styles.css");
}
