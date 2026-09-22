//! Integration tests for ololo-health: real jscpd runs over small trees
//! written into temp dirs (outside any git checkout, like the snapshot
//! trees the client and the server materialize).

use std::path::Path;
use std::time::Duration;

use ololo_health::{HEALTH_SCHEMA, HealthConfig, HealthError, JSCPD_CORE_VERSION, Level, analyze};

/// A JavaScript function with control flow: well over jscpd's default
/// 50-token / 5-line clone floor, so two copies are one clone pair.
const DUP: &str = r#"export function summarize(items, options) {
  let total = 0;
  let count = 0;
  for (const item of items) {
    if (item.price > 0 && item.quantity > 0) {
      total += item.price * item.quantity;
      count += 1;
    } else if (options.strict) {
      throw new Error("invalid item: " + item.id);
    }
  }
  const average = count > 0 ? total / count : 0;
  return { total, count, average, currency: options.currency || "USD" };
}
"#;

/// Also past the 50-token floor: jscpd drops smaller files from its sources
/// altogether, so they would not count as files of the tree.
const UNIQUE: &str = r#"export function greet(name, mood) {
  if (!name) {
    return mood === "grumpy" ? "go away" : "hello, stranger";
  }
  const parts = [];
  for (let i = 0; i < 3; i += 1) {
    parts.push(name.charAt(i) || "?");
  }
  if (mood === "loud") {
    return ("hello, " + name + parts.join("")).toUpperCase();
  }
  return "hello, " + name;
}
"#;

fn write_tree(root: &Path, files: &[(&str, &str)]) {
    for (rel, content) in files {
        let path = root.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, content).unwrap();
    }
}

fn copy_tree(from: &Path, to: &Path) {
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            std::fs::create_dir_all(&target).unwrap();
            copy_tree(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), target).unwrap();
        }
    }
}

/// A module with a used and an unused export, past the 50-token floor.
const LIBRARY: &str = r#"export function used(list) {
  const out = [];
  for (const item of list) {
    if (item % 15 === 0) out.push("FizzBuzz");
    else if (item % 3 === 0) out.push("Fizz");
    else if (item % 5 === 0) out.push("Buzz");
    else out.push(String(item));
  }
  return out;
}

export function neverCalled(n) {
  let total = 0;
  for (let i = 0; i < n; i++) {
    total += i * 2 + 1;
  }
  return total;
}
"#;

const ENTRY: &str = r#"import { used } from "./library.js";

export function main(argv) {
  const numbers = argv.map((value) => Number(value)).filter((value) => !Number.isNaN(value));
  const lines = used(numbers);
  for (const line of lines) {
    console.log(line);
  }
  return lines.length;
}

main(process.argv.slice(2));
"#;

#[test]
fn measures_complexity_and_dead_code_where_the_language_allows() {
    let dir = tempfile::tempdir().unwrap();
    write_tree(
        dir.path(),
        &[
            (
                "package.json",
                r#"{ "name": "fixture", "main": "src/index.js", "type": "module" }"#,
            ),
            ("src/index.js", ENTRY),
            ("src/library.js", LIBRARY),
        ],
    );
    let result = analyze(dir.path(), &HealthConfig::default()).unwrap();
    let m = &result.metrics;
    assert_eq!(m.files, 2);
    assert!(
        m.complexity_pct.is_some(),
        "complexity is measured: {:?}",
        result.health
    );
    assert!(
        m.dead_code_pct.is_some() && m.dead_lines.is_some(),
        "JavaScript is a language the dead-code analyzer reads: {:?}",
        result.health
    );
    assert!(
        m.dead_lines.unwrap_or(0) > 0 && m.dead_symbols.unwrap_or(0) >= 1,
        "the unused export is dead code: {:?}",
        result.health
    );

    // A tree in a language it does not read: complexity still measured,
    // dead code honestly absent.
    let dir = tempfile::tempdir().unwrap();
    write_tree(dir.path(), &[("src/a.go", DUP), ("src/b.go", UNIQUE)]);
    let result = analyze(dir.path(), &HealthConfig::default()).unwrap();
    assert!(result.metrics.complexity_pct.is_some());
    assert!(result.metrics.dead_code_pct.is_none());
    assert!(result.metrics.dead_lines.is_none());
}

#[test]
fn scores_a_tree_and_is_deterministic_across_runs_and_paths() {
    let dir = tempfile::tempdir().unwrap();
    write_tree(
        dir.path(),
        &[
            ("src/a.js", DUP),
            ("src/b.js", DUP),
            ("src/unique.js", UNIQUE),
        ],
    );
    let cfg = HealthConfig::default();

    let first = analyze(dir.path(), &cfg).unwrap();
    assert_eq!(first.schema, HEALTH_SCHEMA);
    assert_eq!(first.jscpd_version, JSCPD_CORE_VERSION);
    assert!(
        first.score.is_some(),
        "a code tree scores: {:?}",
        first.health
    );
    assert!(first.grade.is_some());
    assert_ne!(first.level, Level::Unknown);
    assert_eq!(first.metrics.files, 3);
    assert!(first.metrics.code_lines > 0);
    assert!(
        first.metrics.clones >= 1,
        "the two copies are a clone: {:?}",
        first.health
    );
    assert!(first.metrics.duplicated_lines.unwrap_or(0) > 0);
    assert!(first.metrics.duplication_pct.unwrap_or(0.0) > 0.0);
    assert_eq!(first.metrics.ignore_markers, 0);
    assert!(!first.metrics.jscpd_config_present);
    // jscpd's own JSON carries the dimensions the reader will want.
    assert_eq!(
        first.health["score"],
        serde_json::json!(first.score.unwrap())
    );
    assert!(
        first.health["dimensions"]
            .as_array()
            .is_some_and(|d| !d.is_empty())
    );

    let second = analyze(dir.path(), &cfg).unwrap();
    assert_eq!(second.health, first.health, "same tree, same jscpd result");
    assert_eq!(second.score, first.score);
    assert_eq!(second.metrics, first.metrics);

    // The same files under another path score identically: nothing in the
    // result depends on where the tree lives.
    let elsewhere = tempfile::tempdir().unwrap();
    copy_tree(dir.path(), elsewhere.path());
    let moved = analyze(elsewhere.path(), &cfg).unwrap();
    assert_eq!(moved.health, first.health);
    assert_eq!(moved.metrics, first.metrics);
}

#[test]
fn a_clean_tree_scores_higher_than_a_duplicated_one() {
    let clean = tempfile::tempdir().unwrap();
    write_tree(
        clean.path(),
        &[("src/a.js", DUP), ("src/unique.js", UNIQUE)],
    );
    let dirty = tempfile::tempdir().unwrap();
    write_tree(
        dirty.path(),
        &[
            ("src/a.js", DUP),
            ("src/b.js", DUP),
            ("src/unique.js", UNIQUE),
        ],
    );
    let cfg = HealthConfig::default();
    let clean = analyze(clean.path(), &cfg).unwrap();
    let dirty = analyze(dirty.path(), &cfg).unwrap();
    assert_eq!(clean.metrics.clones, 0);
    assert!(
        clean.score.unwrap() > dirty.score.unwrap(),
        "{clean:?} vs {dirty:?}"
    );
}

#[test]
fn ignores_platform_and_dependency_directories() {
    let dir = tempfile::tempdir().unwrap();
    write_tree(
        dir.path(),
        &[
            (".ololo/artifacts/a.js", DUP),
            ("node_modules/pkg/b.js", DUP),
            ("deep/target/c.js", DUP),
            (".git/hooks/d.js", DUP),
            ("src/unique.js", UNIQUE),
        ],
    );
    let result = analyze(dir.path(), &HealthConfig::default()).unwrap();
    assert_eq!(result.metrics.clones, 0, "{:?}", result.health);
    assert_eq!(
        result.metrics.files, 1,
        "only src/unique.js is the player's code"
    );
}

#[test]
fn does_not_honour_the_trees_own_jscpd_config() {
    let dir = tempfile::tempdir().unwrap();
    write_tree(
        dir.path(),
        &[
            ("src/a.js", DUP),
            ("src/b.js", DUP),
            (
                ".jscpd.json",
                r#"{ "minTokens": 100000, "minLines": 1000, "ignore": ["**"] }"#,
            ),
        ],
    );
    let result = analyze(dir.path(), &HealthConfig::default()).unwrap();
    assert!(result.metrics.jscpd_config_present);
    assert!(
        result.metrics.clones >= 1,
        "the config must not switch detection off"
    );

    let via_package = tempfile::tempdir().unwrap();
    write_tree(
        via_package.path(),
        &[
            ("src/a.js", UNIQUE),
            (
                "package.json",
                r#"{ "name": "x", "jscpd": { "ignore": ["**"] } }"#,
            ),
        ],
    );
    assert!(
        analyze(via_package.path(), &HealthConfig::default())
            .unwrap()
            .metrics
            .jscpd_config_present
    );

    let plain = tempfile::tempdir().unwrap();
    write_tree(
        plain.path(),
        &[("src/a.js", UNIQUE), ("package.json", r#"{ "name": "x" }"#)],
    );
    assert!(
        !analyze(plain.path(), &HealthConfig::default())
            .unwrap()
            .metrics
            .jscpd_config_present
    );
}

#[test]
fn counts_files_that_carry_ignore_markers() {
    let dir = tempfile::tempdir().unwrap();
    let hidden = format!("// jscpd:ignore-start\n{DUP}// jscpd:ignore-end\n");
    write_tree(
        dir.path(),
        &[
            ("src/a.js", &hidden),
            ("src/b.js", DUP),
            ("src/c.js", UNIQUE),
        ],
    );
    let result = analyze(dir.path(), &HealthConfig::default()).unwrap();
    assert_eq!(result.metrics.ignore_markers, 1);
}

#[test]
fn skips_files_over_the_size_cap() {
    let dir = tempfile::tempdir().unwrap();
    write_tree(dir.path(), &[("src/a.js", DUP), ("src/b.js", DUP)]);
    let cfg = HealthConfig {
        max_file_bytes: 64,
        ..HealthConfig::default()
    };
    let result = analyze(dir.path(), &cfg).unwrap();
    assert_eq!(result.metrics.clones, 0);
    assert_eq!(result.metrics.files, 0);
    assert!(result.score.is_none());
    assert_eq!(result.level, Level::Unknown);
}

#[test]
fn refuses_trees_over_the_caps() {
    let dir = tempfile::tempdir().unwrap();
    write_tree(
        dir.path(),
        &[
            ("src/a.js", DUP),
            ("src/b.js", DUP),
            ("node_modules/x/huge.js", DUP),
        ],
    );

    let by_files = HealthConfig {
        max_files: 1,
        ..HealthConfig::default()
    };
    let err = analyze(dir.path(), &by_files).unwrap_err();
    assert!(
        matches!(err, HealthError::TreeTooLarge { max_files: 1, .. }),
        "{err}"
    );

    let by_bytes = HealthConfig {
        max_total_bytes: 10,
        ..HealthConfig::default()
    };
    let err = analyze(dir.path(), &by_bytes).unwrap_err();
    assert!(
        matches!(err, HealthError::TreeTooLarge { max_bytes: 10, .. }),
        "{err}"
    );

    // Excluded directories do not count towards the caps.
    let two_files = HealthConfig {
        max_files: 2,
        ..HealthConfig::default()
    };
    assert!(analyze(dir.path(), &two_files).is_ok());
}

#[test]
fn an_empty_tree_has_no_score() {
    let dir = tempfile::tempdir().unwrap();
    let result = analyze(dir.path(), &HealthConfig::default()).unwrap();
    assert_eq!(result.score, None);
    assert_eq!(result.grade, None);
    assert_eq!(result.level, Level::Unknown);
    assert_eq!(result.metrics.files, 0);
    assert_eq!(result.metrics.clones, 0);
}

#[test]
fn rejects_missing_paths_and_files() {
    let dir = tempfile::tempdir().unwrap();
    let missing = analyze(&dir.path().join("nope"), &HealthConfig::default()).unwrap_err();
    assert!(matches!(missing, HealthError::Io(_)), "{missing}");
    let file = dir.path().join("f.js");
    std::fs::write(&file, UNIQUE).unwrap();
    let not_dir = analyze(&file, &HealthConfig::default()).unwrap_err();
    assert!(
        matches!(not_dir, HealthError::NotADirectory(_)),
        "{not_dir}"
    );
}

#[test]
fn rejects_an_invalid_config_before_scanning() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = HealthConfig {
        timeout: Duration::ZERO,
        ..HealthConfig::default()
    };
    assert!(matches!(
        analyze(dir.path(), &cfg),
        Err(HealthError::Config(_))
    ));
}

#[tokio::test]
async fn analyze_async_agrees_with_analyze() {
    let dir = tempfile::tempdir().unwrap();
    write_tree(
        dir.path(),
        &[
            ("src/a.js", DUP),
            ("src/b.js", DUP),
            ("src/unique.js", UNIQUE),
        ],
    );
    let cfg = HealthConfig::default();
    let sync = analyze(dir.path(), &cfg).unwrap();
    let asynchronous = ololo_health::analyze_async(dir.path().to_path_buf(), cfg)
        .await
        .unwrap();
    assert_eq!(asynchronous.health, sync.health);
    assert_eq!(asynchronous.score, sync.score);
}
