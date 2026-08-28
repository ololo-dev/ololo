//! Integration tests for `arena_core::probe_engine` — JS fixture/validation
//! evaluation, secret metadata, and `wrap_js` regression coverage.

use arena_core::probe_engine::*;

#[test]
fn eval_js_fixtures_object() {
    let out = eval_js_fixtures_with_meta("({ a: 2, b: 'x' })")
        .unwrap()
        .fixtures;
    assert_eq!(out.get("a"), Some(&serde_json::json!(2)));
    assert_eq!(out.get("b"), Some(&serde_json::json!("x")));
}

// --- eval_js_fixtures_with_meta: __secret / __secret_expected ---

#[test]
fn eval_js_fixtures_extracts_secret_metadata() {
    // The __secret array and __secret_expected flag are pulled out of
    // the fixture map; the remaining fixtures (including `answer`)
    // stay available for the validation script to reference.
    let outcome = eval_js_fixtures_with_meta(
        "({ answer: 'Paris', question: 'q', __secret: ['answer'], __secret_expected: true })",
    )
    .unwrap();
    assert_eq!(outcome.secret_keys, vec!["answer".to_string()]);
    assert!(outcome.secret_expected);
    // Metadata keys removed from fixtures; answer kept for validation.
    assert!(outcome.fixtures.get("__secret").is_none());
    assert!(outcome.fixtures.get("__secret_expected").is_none());
    assert_eq!(
        outcome.fixtures.get("answer"),
        Some(&serde_json::json!("Paris"))
    );
    assert_eq!(
        outcome.fixtures.get("question"),
        Some(&serde_json::json!("q"))
    );
}

#[test]
fn eval_js_fixtures_no_secret_metadata_defaults_empty() {
    // Back-compat: fixtures without __secret/__secret_expected parse
    // fine and yield empty secret metadata.
    let outcome = eval_js_fixtures_with_meta("({ a: 1, b: 2 })").unwrap();
    assert!(outcome.secret_keys.is_empty());
    assert!(!outcome.secret_expected);
    assert_eq!(outcome.fixtures.get("a"), Some(&serde_json::json!(1)));
}

#[test]
fn secret_meta_redacts_fixture_values_json() {
    // The redactor replaces secret keys with "<redacted>" in the
    // client-visible JSON. Non-secret keys pass through unchanged.
    let meta = SecretMeta {
        fixtures: vec!["answer".into()],
        expected: false,
    };
    let json = r#"{"question":"q","answer":"Paris","port":"8088"}"#;
    let redacted = meta.redact_fixture_values(json);
    assert!(redacted.contains(r#""answer":"<redacted>""#));
    assert!(redacted.contains(r#""question":"q""#));
    assert!(redacted.contains(r#""port":"8088""#));
}

#[test]
fn secret_meta_no_fixtures_returns_input_unchanged() {
    // No secret fixture keys → no redaction work; input returned as-is.
    let meta = SecretMeta {
        fixtures: vec![],
        expected: true,
    };
    let json = r#"{"a":"b"}"#;
    assert_eq!(meta.redact_fixture_values(json), json);
}

#[test]
fn secret_meta_roundtrip_json() {
    // to_json_string + parse round-trip; empty meta yields None.
    let meta = SecretMeta {
        fixtures: vec!["answer".into()],
        expected: true,
    };
    let s = meta.to_json_string().unwrap();
    let parsed = SecretMeta::parse(&Some(s)).unwrap();
    assert_eq!(parsed.fixtures, vec!["answer".to_string()]);
    assert!(parsed.expected);
    assert!(SecretMeta::default().to_json_string().is_none());
    assert!(SecretMeta::parse(&None).is_none());
}

// --- wrap_js / eval_js_fixtures: multi-line object regression ---

#[test]
fn wrap_js_multiline_object_uses_expression_body() {
    // A multi-line object literal has '\n' but no 'return' or ';'.
    // Must produce an expression body so the object is returned.
    let script = "({\n  a: 1,\n  b: 2\n})";
    let wrapped = wrap_js(script);
    assert!(
        wrapped.starts_with("(() => ("),
        "expected expression body, got: {wrapped}"
    );
}

#[test]
fn eval_js_fixtures_multiline_object() {
    // Regression: multi-line JS object literal stored in fixture_definitions
    // must parse correctly (was broken by block-body wrapping).
    let script = "({\n  htmlFile: \"./index.html\",\n  cssFile: \"./styles.css\"\n})";
    let out = eval_js_fixtures_with_meta(script).unwrap().fixtures;
    assert_eq!(
        out.get("htmlFile"),
        Some(&serde_json::json!("./index.html"))
    );
    assert_eq!(out.get("cssFile"), Some(&serde_json::json!("./styles.css")));
}

#[test]
fn wrap_js_nested_return_still_returns_trailing_expr() {
    // Regression: a template whose `return` statements live inside
    // nested arrow-function bodies must NOT be treated as a top-level
    // block body. The trailing predicate (`got === expected`) must be
    // returned by the wrapper IIFE, else eval yields undefined and
    // every probe is misgraded as fail.
    let script = "const isSquare = x => { const r = Math.round(Math.sqrt(x)); return r*r === x; }; const isCube = x => { const r = Math.round(Math.cbrt(x)); return r*r*r === x; }; const nums = list.split(\",\").map(s => Number(s.trim())); const expected = nums.filter(x => isSquare(x) && isCube(x)).sort((a,b)=>a-b).join(\", \"); const got = result.trim().split(\",\").map(s => Number(s.trim())).sort((a,b)=>a-b).join(\", \"); got === expected";
    let wrapped = wrap_js(script);
    assert!(
        wrapped.contains("return (got === expected)"),
        "wrapper must return the trailing predicate, got: {wrapped}"
    );
}

#[test]
fn wrap_js_top_level_return_uses_block_body() {
    // A script with a genuine top-level `return` (rare legacy form)
    // must still wrap as a block body without injecting another return.
    let script = "return got === expected";
    let wrapped = wrap_js(script);
    assert!(
        wrapped.starts_with("(() => {"),
        "top-level return must use block body, got: {wrapped}"
    );
    assert!(
        !wrapped.contains("return ("),
        "must not inject a synthetic return, got: {wrapped}"
    );
}

#[test]
fn wrap_js_nested_semicolon_does_not_split() {
    // Regression: a fixtures script with a `;` nested inside an inner
    // arrow/IIFE body must NOT be treated as the top-level split point.
    // Previously `rsplit_once(';')` grabbed the inner `;` after
    // `return { n, ordinal };`, producing `})()` as the trailing
    // expression and a SyntaxError on eval.
    let script = "({\n  baseDir: \".\",\n  ...(() => {\n    const n = Math.floor(Math.random() * 20) + 4;\n    return { n, ordinal: n + \"th\" };\n  })()\n})";
    let wrapped = wrap_js(script);
    assert!(
        wrapped.starts_with("(() => ("),
        "object-literal fixtures must wrap as expression body, got: {wrapped}"
    );
    assert!(
        !wrapped.contains("return ("),
        "must not inject a synthetic return into a pure object literal, got: {wrapped}"
    );
}

#[test]
fn eval_js_fixtures_nested_iife_semicolons() {
    // End-to-end: the fibonacci fixtures script (real-world case) must
    // eval to an object, not a SyntaxError. Inner `;` inside the IIFE
    // must not be mistaken for a top-level statement terminator.
    let script = r#"({
  baseDir: ".",
  port: "8088",
  qid: Math.random().toString(16).slice(2,10),
  ...(() => {
    const n = Math.floor(Math.random() * 20) + 4;
    const s = ["th","st","nd","rd"], v = n % 100;
    const ordinal = n + (s[(v-20)%10] || s[v] || "th");
    return { n, ordinal };
  })()
})"#;
    let out = eval_js_fixtures_with_meta(script)
        .expect("fibonacci fixtures must eval")
        .fixtures;
    assert_eq!(out.get("baseDir"), Some(&serde_json::json!(".")));
    assert_eq!(out.get("port"), Some(&serde_json::json!("8088")));
    assert!(out.get("n").is_some(), "n must be present");
    assert!(out.get("ordinal").is_some(), "ordinal must be present");
}

#[test]
fn wrap_js_for_loop_header_semicolons_not_split() {
    // Regression: a validation script with a `for(;;)` header must NOT
    // split on the `;` inside the parens — those are statement
    // separators within the for-header, not top-level terminators.
    // Previously `last_top_level_semicolon` only tracked `{}` depth,
    // so `for(let i=0; i<n; i++)` contributed two bogus top-level `;`
    // and the wrapper injected `return (i++) ...)` producing garbage.
    let script = "let a = 0, b = 1; for (let i = 0; i < n; i++) { [a, b] = [b, a + b]; } assertEqual(Number(result.trim()), a)";
    let wrapped = wrap_js(script);
    assert!(
        wrapped.contains("return (assertEqual(Number(result.trim()), a))"),
        "wrapper must return the trailing assertEqual call, got: {wrapped}"
    );
    assert!(
        !wrapped.contains("return (i++)"),
        "must not split inside the for-header, got: {wrapped}"
    );
}

#[test]
fn eval_js_validation_fibonacci_loop() {
    // End-to-end: the fibonacci validation script (real-world case).
    // Uses a `for` loop with `;` in the header and an `assertEqual`
    // trailing call. Must grade a correct answer as Pass and surface
    // the computed expected value (F(n)) via the assertion stash.
    let script = "let a = 0, b = 1; for (let i = 0; i < n; i++) { [a, b] = [b, a + b]; } assertEqual(Number(result.trim()), a)";
    let mut fixtures = serde_json::Map::new();
    // n = 10 → F(10) = 55 (0-indexed: 0,1,1,2,3,5,8,13,21,34,55)
    fixtures.insert("n".to_string(), serde_json::json!(10));
    let outcome = eval_js_validation_outcome(script, &fixtures, "55")
        .expect("fibonacci validation must eval without error");
    assert!(outcome.pass, "F(10)=55 must grade as Pass");
    assert_eq!(outcome.expected.as_deref(), Some("55"));
    assert_eq!(outcome.actual.as_deref(), Some("55"));

    let fail = eval_js_validation_outcome(script, &fixtures, "99")
        .expect("wrong answer must eval without error");
    assert!(!fail.pass, "99 != 55 must grade as Fail");
    assert_eq!(fail.expected.as_deref(), Some("55"));
    assert_eq!(fail.actual.as_deref(), Some("99"));
}

#[test]
fn eval_js_validation_nested_return_predicate() {
    // End-to-end: the square-and-cube template (returns nested inside
    // arrow bodies, trailing predicate at top level) must grade a
    // correct answer as true and a wrong answer as false.
    let script = "const isSquare = x => { const r = Math.round(Math.sqrt(x)); return r*r === x; }; const isCube = x => { const r = Math.round(Math.cbrt(x)); return r*r*r === x; }; const nums = list.split(\",\").map(s => Number(s.trim())); const expected = nums.filter(x => isSquare(x) && isCube(x)).sort((a,b)=>a-b).join(\", \"); const got = result.trim().split(\",\").map(s => Number(s.trim())).sort((a,b)=>a-b).join(\", \"); got === expected";
    let mut fixtures = serde_json::Map::new();
    fixtures.insert(
        "list".to_string(),
        serde_json::json!("1, 9, 4, 64, 16, 25, 4096, 49, 729, 36"),
    );
    let pass = eval_js_validation_outcome(script, &fixtures, "1, 64, 4096, 729")
        .expect("correct answer must eval without error")
        .pass;
    assert!(pass, "correct sixth powers must grade as Pass");
    let fail = eval_js_validation_outcome(script, &fixtures, "WRONG")
        .expect("wrong answer must eval without error")
        .pass;
    assert!(!fail, "wrong answer must grade as Fail");
}

// --- eval_js_validation_outcome + assertEqual ---

#[test]
fn assert_equal_captures_display_values_on_pass() {
    // Multiplication task: assertEqual(Number(result.trim()), n1 * n2).
    // Pass path: result="42", n1=6, n2=7 → expected="42", actual="42".
    let mut fixtures = serde_json::Map::new();
    fixtures.insert("n1".to_string(), serde_json::json!(6));
    fixtures.insert("n2".to_string(), serde_json::json!(7));
    let outcome = eval_js_validation_outcome(
        "assertEqual(Number(result.trim()), n1 * n2)",
        &fixtures,
        "42",
    )
    .expect("assertEqual pass must eval without error");
    assert!(outcome.pass);
    assert_eq!(outcome.expected.as_deref(), Some("42"));
    assert_eq!(outcome.actual.as_deref(), Some("42"));
}

#[test]
fn assert_equal_captures_display_values_on_fail_with_nan_actual() {
    // Fail path: result="ok" → Number("ok") = NaN, displayed as "NaN".
    // expected is the real product, actual is "NaN" — far more useful
    // to the participant than the raw predicate string.
    let mut fixtures = serde_json::Map::new();
    fixtures.insert("n1".to_string(), serde_json::json!(6));
    fixtures.insert("n2".to_string(), serde_json::json!(7));
    let outcome = eval_js_validation_outcome(
        "assertEqual(Number(result.trim()), n1 * n2)",
        &fixtures,
        "ok",
    )
    .expect("assertEqual fail must eval without error");
    assert!(!outcome.pass);
    assert_eq!(outcome.expected.as_deref(), Some("42"));
    assert_eq!(outcome.actual.as_deref(), Some("NaN"));
}

#[test]
fn predicate_template_without_assert_equal_yield_no_display_values() {
    // Back-compat: predicate templates that never call assertEqual
    // return None for both expected and actual (only pass matters).
    let mut fixtures = serde_json::Map::new();
    fixtures.insert("n1".to_string(), serde_json::json!(6));
    fixtures.insert("n2".to_string(), serde_json::json!(7));
    let outcome = eval_js_validation_outcome("Number(result.trim()) === n1 * n2", &fixtures, "42")
        .expect("predicate must eval without error");
    assert!(outcome.pass);
    assert_eq!(outcome.expected, None);
    assert_eq!(outcome.actual, None);
}

#[test]
fn assert_equal_string_operands_display_as_strings() {
    // Non-numeric operands: assertEqual compares by strict equals but
    // displays both operands as their JS stringification.
    let fixtures = serde_json::Map::new();
    let outcome =
        eval_js_validation_outcome("assertEqual(result.trim(), \"hello\")", &fixtures, "hello")
            .unwrap();
    assert!(outcome.pass);
    assert_eq!(outcome.expected.as_deref(), Some("hello"));
    assert_eq!(outcome.actual.as_deref(), Some("hello"));
}

#[test]
fn eval_js_fixtures_iife_card_pool() {
    // The extreme-startup general-knowledge tasks use an IIFE that picks a
    // random card from a per-type pool. Guard that wrap_js + boa evaluate
    // this shape and that the picked pair stays consistent.
    let script = r#"(() => {
  const cards = [
    ["which city is the Eiffel tower in", "Paris"],
    ["which city is Big Ben in", "London"],
    ["which city is the Colosseum in", "Rome"],
    ["which city is the Brandenburg Gate in", "Berlin"]
  ];
  const pick = cards[Math.floor(Math.random() * cards.length)];
  return {
    baseDir: ".",
    id: Math.random().toString(36).slice(2, 10),
    question: pick[0],
    answer: pick[1],
    __secret: ["answer"],
    __secret_expected: true
  };
})()"#;
    let pairs = [
        ("which city is the Eiffel tower in", "Paris"),
        ("which city is Big Ben in", "London"),
        ("which city is the Colosseum in", "Rome"),
        ("which city is the Brandenburg Gate in", "Berlin"),
    ];
    for _ in 0..10 {
        let outcome = eval_js_fixtures_with_meta(script).unwrap();
        assert_eq!(outcome.secret_keys, vec!["answer".to_string()]);
        assert!(outcome.secret_expected);
        let question = outcome.fixtures["question"].as_str().unwrap();
        let answer = outcome.fixtures["answer"].as_str().unwrap();
        let expected = pairs
            .iter()
            .find(|(q, _)| *q == question)
            .map(|(_, a)| *a)
            .expect("question drawn from the pool");
        assert_eq!(answer, expected, "answer matches the picked question");
        assert!(!outcome.fixtures["id"].as_str().unwrap().is_empty());
    }
}
