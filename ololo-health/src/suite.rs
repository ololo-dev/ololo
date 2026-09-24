//! The project's own test suite, as two more health dimensions.
//!
//! jscpd scores what it can read in the tree. Whether the tests pass and
//! how much of the code they cover can only be learned by running them —
//! on the player's machine, where their dependencies and toolchain live
//! (the server never runs player code). The client runs the command the
//! project's docs name; this module turns what that run printed, or the
//! coverage report it wrote, into two of jscpd's external metrics:
//!
//! - `tests` — the share of tests failing, lower is healthier, half-life
//!   [`TESTS_HALF_LIFE`]: nothing failing scores 100, one test in ten 50.
//! - `coverage` — line coverage, higher is healthier, half-life
//!   [`COVERAGE_HALF_LIFE`] (jscpd's own documented example): 81% scores 72.
//!
//! Nothing is invented. A run that printed no counts is judged by its exit
//! code; a suite that ran no tests at all adds no `tests` dimension; a run
//! that measured no coverage adds no `coverage` one; and a project whose
//! docs name no test command adds neither — jscpd's rule for a dimension
//! it cannot measure: left out and named, never scored as perfect.

use std::path::Path;
use std::sync::LazyLock;
use std::time::SystemTime;

use cpd_core::health::{Direction, ExternalMetric};
use regex::Regex;
use serde::{Deserialize, Serialize};

/// Failing share, in percent, at which the `tests` sub-score halves.
pub const TESTS_HALF_LIFE: f64 = 10.0;
/// Distance from full coverage, in percent, at which the `coverage`
/// sub-score halves — jscpd's documented example value.
pub const COVERAGE_HALF_LIFE: f64 = 40.0;
/// jscpd metric ids of the two dimensions.
pub const TESTS_ID: &str = "tests";
pub const COVERAGE_ID: &str = "coverage";

/// Output lines kept as the evidence of a result.
const MAX_SUMMARY_LINES: usize = 6;
const MAX_SUMMARY_LINE_CHARS: usize = 160;
/// Largest coverage report read; bigger ones are skipped.
const MAX_REPORT_BYTES: u64 = 32 * 1024 * 1024;

/// Tests a runner reported.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TestCounts {
    pub passed: u64,
    pub failed: u64,
    #[serde(default)]
    pub skipped: u64,
}

impl TestCounts {
    fn add(&mut self, other: TestCounts) {
        self.passed += other.passed;
        self.failed += other.failed;
        self.skipped += other.skipped;
    }
}

/// What one run of the project's test command measured.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SuiteResult {
    /// The command's exit code; `None` when a signal ended it.
    pub exit_code: Option<i32>,
    /// The tests the output reported, summed over every runner that
    /// printed a summary; `None` when no summary was recognised.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub counts: Option<TestCounts>,
    /// Line coverage, in percent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub coverage_pct: Option<f64>,
    /// Where `coverage_pct` was read: a report's path relative to the
    /// workspace, or `output`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub coverage_source: Option<String>,
    /// The output lines the numbers were read from — the evidence, a few
    /// lines at most.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub summary: Vec<String>,
}

/// Share of the tests failing, in percent: from the counts when the output
/// had them, else from the exit code (0 → none failing, anything else →
/// all of them). `None` when the suite reported running no tests at all —
/// there is no share to measure.
pub fn failing_pct(result: &SuiteResult) -> Option<f64> {
    match result.counts {
        Some(c) if c.passed + c.failed > 0 => Some(round1(
            c.failed as f64 / (c.passed + c.failed) as f64 * 100.0,
        )),
        Some(_) => None,
        None => Some(if result.exit_code == Some(0) {
            0.0
        } else {
            100.0
        }),
    }
}

/// The run as jscpd external metrics: `tests` when there is a failing
/// share, `coverage` when coverage was measured.
pub fn metrics(result: &SuiteResult) -> Vec<ExternalMetric> {
    let mut out = Vec::new();
    if let Some(failing) = failing_pct(result) {
        out.push(ExternalMetric {
            id: TESTS_ID.to_string(),
            value: Some(failing),
            direction: Direction::Lower,
            half_life: Some(TESTS_HALF_LIFE),
            ..ExternalMetric::default()
        });
    }
    if let Some(coverage) = result.coverage_pct.filter(|c| c.is_finite()) {
        out.push(ExternalMetric {
            id: COVERAGE_ID.to_string(),
            value: Some(round1(coverage.clamp(0.0, 100.0))),
            direction: Direction::Higher,
            max: Some(100.0),
            half_life: Some(COVERAGE_HALF_LIFE),
            ..ExternalMetric::default()
        });
    }
    out
}

fn round1(value: f64) -> f64 {
    (value * 10.0).round() / 10.0
}

// ───────────────────────────── output ─────────────────────────────

/// How sure a coverage number read from the output is: a summary table's
/// total row beats a generic "coverage: N%" line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum CoverageKind {
    Generic,
    GoPackage,
    Line,
    Total,
    GoTotal,
    Table,
}

/// Reads a test run's output line by line — stdout and stderr alike, as
/// they arrive, so nothing has to be buffered — and recognises the summary
/// lines of the common runners: Jest, Vitest, Mocha, node:test, Bun, Deno,
/// AVA, Jasmine, pytest, unittest, cargo test, cargo nextest, go test,
/// RSpec, Minitest, PHPUnit, dotnet test, Gradle, ExUnit; and the coverage
/// summaries of Istanbul (Jest, Vitest, nyc, c8), Bun, Deno, coverage.py,
/// cargo llvm-cov, cargo tarpaulin, go, SimpleCov, PHPUnit and coverlet.
#[derive(Debug, Default)]
pub struct OutputParser {
    counts: TestCounts,
    counted: bool,
    go_pass: u64,
    go_fail: u64,
    go_skip: u64,
    go_verbose: bool,
    unittest_total: Option<u64>,
    /// The column of line coverage in the last summary-table header seen.
    table_column: Option<usize>,
    coverage: Option<(CoverageKind, f64)>,
    go_packages: Vec<f64>,
    summary: Vec<String>,
}

/// What an [`OutputParser`] recognised.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ParsedOutput {
    pub counts: Option<TestCounts>,
    pub coverage_pct: Option<f64>,
    pub summary: Vec<String>,
}

static ANSI: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\x1b\[[0-9;?]*[ -/]*[@-~]|\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)").unwrap()
});

fn re(pattern: &str) -> Regex {
    Regex::new(pattern).expect("static pattern")
}

static COUNT_PARTS: LazyLock<Regex> = LazyLock::new(|| {
    re(r"(\d+)\s+(passed|failed|skipped|todo|pending|errors?|xfailed|xpassed|deselected)\b")
});
static JEST_VITEST: LazyLock<Regex> = LazyLock::new(|| re(r"^Tests:?\s+(\d+\s+\w+.*)$"));
static PYTEST: LazyLock<Regex> = LazyLock::new(|| {
    re(
        r"^=*\s*(\d+\s+(?:passed|failed|errors?|skipped|xfailed|xpassed|deselected|warnings?|rerun)\b.*?)\s+in\s+[\d.]+m?s\b",
    )
});
static NO_TESTS: LazyLock<Regex> = LazyLock::new(|| {
    re(
        r"^(?:=*\s*no tests ran\b|No tests found\b|No test files found\b|Error: no test specified\b|running 0 tests$)",
    )
});
static CARGO: LazyLock<Regex> = LazyLock::new(|| {
    re(r"^test result: (?:ok|FAILED)\.\s+(\d+) passed;\s+(\d+) failed;\s+(\d+) ignored")
});
static NEXTEST: LazyLock<Regex> =
    LazyLock::new(|| re(r"^Summary\s+\[[^\]]*\]\s+\d+\s+tests?\s+run:\s+(.*)$"));
static MOCHA: LazyLock<Regex> = LazyLock::new(|| re(r"^(\d+)\s+(passing|failing|pending)\b"));
static NODE_TEST: LazyLock<Regex> =
    LazyLock::new(|| re(r"^(?:#|ℹ)\s+(pass|fail|skipped|todo|cancelled)\s+(\d+)$"));
static BUN: LazyLock<Regex> = LazyLock::new(|| re(r"^(\d+)\s+(pass|fail|skip|todo)$"));
static AVA: LazyLock<Regex> =
    LazyLock::new(|| re(r"^(\d+)\s+tests?\s+(passed|failed|skipped|todo)$"));
static DENO: LazyLock<Regex> = LazyLock::new(|| {
    re(
        r"^(?:ok|FAILED)\s+\|\s+(\d+)\s+passed(?:\s+\(\d+\s+steps?\))?\s+\|\s+(\d+)\s+failed(?:\s+\(\d+\s+steps?\))?(?:\s+\|\s+(\d+)\s+ignored)?",
    )
});
static RSPEC: LazyLock<Regex> =
    LazyLock::new(|| re(r"^(\d+)\s+examples?,\s+(\d+)\s+failures?(?:,\s+(\d+)\s+pending)?"));
static JASMINE: LazyLock<Regex> =
    LazyLock::new(|| re(r"^(\d+)\s+specs?,\s+(\d+)\s+failures?(?:,\s+(\d+)\s+pending\s+specs?)?"));
static EXUNIT: LazyLock<Regex> = LazyLock::new(|| {
    re(r"^(\d+)\s+tests?,\s+(\d+)\s+failures?(?:,\s+(\d+)\s+(?:skipped|excluded|invalid))?")
});
static GRADLE: LazyLock<Regex> =
    LazyLock::new(|| re(r"^(\d+)\s+tests?\s+completed,\s+(\d+)\s+failed(?:,\s+(\d+)\s+skipped)?"));
static MINITEST: LazyLock<Regex> = LazyLock::new(|| {
    re(
        r"^(\d+)\s+runs?,\s+\d+\s+assertions?,\s+(\d+)\s+failures?,\s+(\d+)\s+errors?,\s+(\d+)\s+skips?",
    )
});
static PHPUNIT_OK: LazyLock<Regex> = LazyLock::new(|| re(r"^OK\s+\((\d+)\s+tests?,"));
static PHPUNIT: LazyLock<Regex> =
    LazyLock::new(|| re(r"^Tests:\s+(\d+),\s+Assertions:\s+\d+(.*)$"));
static PHPUNIT_PARTS: LazyLock<Regex> =
    LazyLock::new(|| re(r"(Errors|Failures|Skipped|Incomplete):\s+(\d+)"));
static DOTNET: LazyLock<Regex> = LazyLock::new(|| {
    re(
        r"(?:Passed|Failed)!\s+-\s+Failed:\s+(\d+),\s+Passed:\s+(\d+),\s+Skipped:\s+(\d+),\s+Total:\s+\d+",
    )
});
static UNITTEST_RAN: LazyLock<Regex> = LazyLock::new(|| re(r"^Ran\s+(\d+)\s+tests?\s+in\s+"));
static UNITTEST_END: LazyLock<Regex> = LazyLock::new(|| re(r"^(OK|FAILED)(?:\s+\((.*)\))?$"));
static UNITTEST_PARTS: LazyLock<Regex> =
    LazyLock::new(|| re(r"(failures|errors|skipped|expected failures|unexpected successes)=(\d+)"));
static GO_RESULT: LazyLock<Regex> = LazyLock::new(|| re(r"^--- (PASS|FAIL|SKIP):\s"));
static GO_RUN: LazyLock<Regex> = LazyLock::new(|| re(r"^=== RUN\s"));

static PERCENT: LazyLock<Regex> = LazyLock::new(|| re(r"(\d+(?:\.\d+)?)\s*%"));
static LINES_SUMMARY: LazyLock<Regex> = LazyLock::new(|| re(r"^Lines\s*:\s*(\d+(?:\.\d+)?)\s*%"));
static TOTAL_ROW: LazyLock<Regex> = LazyLock::new(|| re(r"^TOTAL\s"));
static GO_PACKAGE: LazyLock<Regex> =
    LazyLock::new(|| re(r"coverage:\s+(\d+(?:\.\d+)?)%\s+of\s+statements"));
static GO_TOTAL: LazyLock<Regex> =
    LazyLock::new(|| re(r"^total:\s+\(statements\)\s+(\d+(?:\.\d+)?)%"));
static TARPAULIN: LazyLock<Regex> =
    LazyLock::new(|| re(r"^(\d+(?:\.\d+)?)%\s+coverage,\s+\d+/\d+\s+lines\s+covered"));
static SIMPLECOV: LazyLock<Regex> =
    LazyLock::new(|| re(r"(?:Line Coverage:\s+(\d+(?:\.\d+)?)%|\((\d+(?:\.\d+)?)%\)\s+covered)"));

fn num(s: &str) -> u64 {
    s.parse().unwrap_or(0)
}

/// `1 failed, 12 passed, 2 skipped` (and the Vitest/pytest variants) as
/// counts; `total`, warnings and anything unnamed are ignored.
fn count_parts(text: &str) -> TestCounts {
    let mut counts = TestCounts::default();
    for cap in COUNT_PARTS.captures_iter(text) {
        let n = num(&cap[1]);
        match &cap[2] {
            "passed" | "xpassed" => counts.passed += n,
            "failed" | "error" | "errors" => counts.failed += n,
            _ => counts.skipped += n,
        }
    }
    counts
}

/// Only the line's last carriage-return segment (progress bars redraw with
/// `\r`), without terminal escape sequences, trimmed.
fn clean(line: &str) -> String {
    let line = line.rsplit('\r').next().unwrap_or(line);
    ANSI.replace_all(line, "").trim().to_string()
}

impl OutputParser {
    pub fn new() -> Self {
        Self::default()
    }

    /// Feed one line of output.
    pub fn feed_line(&mut self, raw: &str) {
        let line = clean(raw);
        if line.is_empty() {
            return;
        }
        if self.tests_line(&line) || self.coverage_line(&line) {
            self.note(&line);
        }
    }

    fn count(&mut self, counts: TestCounts) {
        self.counts.add(counts);
        self.counted = true;
    }

    fn note(&mut self, line: &str) {
        let line: String = line.chars().take(MAX_SUMMARY_LINE_CHARS).collect();
        if self.summary.contains(&line) {
            return;
        }
        if self.summary.len() == MAX_SUMMARY_LINES {
            self.summary.remove(0);
        }
        self.summary.push(line);
    }

    /// A test-count line; true when the line counted.
    fn tests_line(&mut self, line: &str) -> bool {
        // Go's per-test lines are only a count with `-v`, which prints a
        // `=== RUN` for every test; without it only failures are listed.
        if GO_RUN.is_match(line) {
            self.go_verbose = true;
            return false;
        }
        if let Some(cap) = GO_RESULT.captures(line) {
            match &cap[1] {
                "PASS" => self.go_pass += 1,
                "FAIL" => self.go_fail += 1,
                _ => self.go_skip += 1,
            }
            return false;
        }
        if NO_TESTS.is_match(line) {
            self.count(TestCounts::default());
            return true;
        }
        if let Some(cap) = PHPUNIT.captures(line) {
            let total = num(&cap[1]);
            let (mut failed, mut skipped) = (0, 0);
            for part in PHPUNIT_PARTS.captures_iter(&cap[2]) {
                match &part[1] {
                    "Errors" | "Failures" => failed += num(&part[2]),
                    _ => skipped += num(&part[2]),
                }
            }
            self.count(TestCounts {
                passed: total.saturating_sub(failed + skipped),
                failed,
                skipped,
            });
            return true;
        }
        if let Some(cap) = PHPUNIT_OK.captures(line) {
            self.count(TestCounts {
                passed: num(&cap[1]),
                ..TestCounts::default()
            });
            return true;
        }
        if let Some(cap) = JEST_VITEST.captures(line) {
            let counts = count_parts(&cap[1]);
            if counts != TestCounts::default() {
                self.count(counts);
                return true;
            }
            return false;
        }
        if let Some(cap) = PYTEST.captures(line) {
            self.count(count_parts(&cap[1]));
            return true;
        }
        if let Some(cap) = CARGO.captures(line) {
            self.count(TestCounts {
                passed: num(&cap[1]),
                failed: num(&cap[2]),
                skipped: num(&cap[3]),
            });
            return true;
        }
        if let Some(cap) = NEXTEST.captures(line) {
            self.count(count_parts(&cap[1]));
            return true;
        }
        if let Some(cap) = MOCHA.captures(line) {
            let n = num(&cap[1]);
            self.count(match &cap[2] {
                "passing" => TestCounts {
                    passed: n,
                    ..TestCounts::default()
                },
                "failing" => TestCounts {
                    failed: n,
                    ..TestCounts::default()
                },
                _ => TestCounts {
                    skipped: n,
                    ..TestCounts::default()
                },
            });
            return true;
        }
        if let Some(cap) = NODE_TEST.captures(line) {
            let n = num(&cap[2]);
            self.count(match &cap[1] {
                "pass" => TestCounts {
                    passed: n,
                    ..TestCounts::default()
                },
                "fail" | "cancelled" => TestCounts {
                    failed: n,
                    ..TestCounts::default()
                },
                _ => TestCounts {
                    skipped: n,
                    ..TestCounts::default()
                },
            });
            return true;
        }
        if let Some(cap) = BUN.captures(line).or_else(|| AVA.captures(line)) {
            let n = num(&cap[1]);
            self.count(match &cap[2] {
                "pass" | "passed" => TestCounts {
                    passed: n,
                    ..TestCounts::default()
                },
                "fail" | "failed" => TestCounts {
                    failed: n,
                    ..TestCounts::default()
                },
                _ => TestCounts {
                    skipped: n,
                    ..TestCounts::default()
                },
            });
            return true;
        }
        if let Some(cap) = DENO.captures(line) {
            self.count(TestCounts {
                passed: num(&cap[1]),
                failed: num(&cap[2]),
                skipped: cap.get(3).map_or(0, |m| num(m.as_str())),
            });
            return true;
        }
        if let Some(cap) = MINITEST.captures(line) {
            let runs = num(&cap[1]);
            let failed = num(&cap[2]) + num(&cap[3]);
            let skipped = num(&cap[4]);
            self.count(TestCounts {
                passed: runs.saturating_sub(failed + skipped),
                failed,
                skipped,
            });
            return true;
        }
        if let Some(cap) = GRADLE.captures(line) {
            let total = num(&cap[1]);
            let failed = num(&cap[2]);
            let skipped = cap.get(3).map_or(0, |m| num(m.as_str()));
            self.count(TestCounts {
                passed: total.saturating_sub(failed + skipped),
                failed,
                skipped,
            });
            return true;
        }
        for pattern in [&*RSPEC, &*JASMINE, &*EXUNIT] {
            if let Some(cap) = pattern.captures(line) {
                let total = num(&cap[1]);
                let failed = num(&cap[2]);
                let skipped = cap.get(3).map_or(0, |m| num(m.as_str()));
                self.count(TestCounts {
                    passed: total.saturating_sub(failed + skipped),
                    failed,
                    skipped,
                });
                return true;
            }
        }
        if let Some(cap) = DOTNET.captures(line) {
            self.count(TestCounts {
                passed: num(&cap[2]),
                failed: num(&cap[1]),
                skipped: num(&cap[3]),
            });
            return true;
        }
        if let Some(cap) = UNITTEST_RAN.captures(line) {
            self.unittest_total = Some(num(&cap[1]));
            return true;
        }
        if let Some(total) = self.unittest_total
            && let Some(cap) = UNITTEST_END.captures(line)
        {
            let (mut failed, mut skipped) = (0, 0);
            if let Some(parts) = cap.get(2) {
                for part in UNITTEST_PARTS.captures_iter(parts.as_str()) {
                    match &part[1] {
                        "failures" | "errors" | "unexpected successes" => failed += num(&part[2]),
                        _ => skipped += num(&part[2]),
                    }
                }
            }
            self.unittest_total = None;
            self.count(TestCounts {
                passed: total.saturating_sub(failed + skipped),
                failed,
                skipped,
            });
            return true;
        }
        false
    }

    /// A coverage line; true when the line was read.
    fn coverage_line(&mut self, line: &str) -> bool {
        // A summary table (Istanbul, Bun, Deno, coverlet): remember which
        // column holds line coverage, read it off the total row.
        if line.contains('|') {
            let cells: Vec<&str> = line
                .split('|')
                .map(str::trim)
                .filter(|c| !c.is_empty())
                .collect();
            if let Some(i) = cells.iter().position(|c| {
                let c = c.to_ascii_lowercase();
                c == "% lines" || c == "lines" || c == "line %" || c == "% line" || c == "line"
            }) {
                self.table_column = Some(i);
                return false;
            }
            if let (Some(i), Some(first)) = (self.table_column, cells.first())
                && (first.eq_ignore_ascii_case("all files") || first.eq_ignore_ascii_case("total"))
                && let Some(value) = cells.get(i).and_then(|c| parse_pct(c))
            {
                self.set_coverage(CoverageKind::Table, value);
                return true;
            }
            return false;
        }
        if let Some(cap) = LINES_SUMMARY.captures(line)
            && let Ok(v) = cap[1].parse()
        {
            self.set_coverage(CoverageKind::Line, v);
            return true;
        }
        if let Some(cap) = GO_TOTAL.captures(line)
            && let Ok(v) = cap[1].parse()
        {
            self.set_coverage(CoverageKind::GoTotal, v);
            return true;
        }
        if TOTAL_ROW.is_match(line) {
            // coverage.py prints one percentage on its TOTAL row; cargo
            // llvm-cov prints regions, functions, lines (and branches):
            // the third is line coverage.
            let values: Vec<f64> = PERCENT
                .captures_iter(line)
                .filter_map(|c| c[1].parse().ok())
                .collect();
            let value = match values.len() {
                0 => None,
                1 | 2 => values.last().copied(),
                _ => values.get(2).copied(),
            };
            if let Some(v) = value {
                self.set_coverage(CoverageKind::Total, v);
                return true;
            }
        }
        if let Some(cap) = GO_PACKAGE.captures(line)
            && let Ok(v) = cap[1].parse()
        {
            self.go_packages.push(v);
            return true;
        }
        if let Some(cap) = TARPAULIN.captures(line)
            && let Ok(v) = cap[1].parse()
        {
            self.set_coverage(CoverageKind::Line, v);
            return true;
        }
        if let Some(cap) = SIMPLECOV.captures(line) {
            let value = cap
                .get(1)
                .or(cap.get(2))
                .and_then(|m| m.as_str().parse().ok());
            if let Some(v) = value {
                self.set_coverage(CoverageKind::Line, v);
                return true;
            }
        }
        // Last resort: "coverage" and exactly one percentage on the line.
        if line.to_ascii_lowercase().contains("coverage") {
            let values: Vec<f64> = PERCENT
                .captures_iter(line)
                .filter_map(|c| c[1].parse().ok())
                .collect();
            if let [v] = values[..] {
                self.set_coverage(CoverageKind::Generic, v);
                return true;
            }
        }
        false
    }

    /// Keep the most trustworthy kind seen; within a kind, the latest.
    fn set_coverage(&mut self, kind: CoverageKind, value: f64) {
        if !(value.is_finite() && (0.0..=100.0).contains(&value)) {
            return;
        }
        match self.coverage {
            Some((seen, _)) if seen > kind => {}
            _ => self.coverage = Some((kind, value)),
        }
    }

    pub fn finish(mut self) -> ParsedOutput {
        if self.go_verbose && self.go_pass + self.go_fail + self.go_skip > 0 {
            self.count(TestCounts {
                passed: self.go_pass,
                failed: self.go_fail,
                skipped: self.go_skip,
            });
        }
        if !self.go_packages.is_empty() {
            let mean = self.go_packages.iter().sum::<f64>() / self.go_packages.len() as f64;
            self.set_coverage(CoverageKind::GoPackage, round1(mean));
        }
        ParsedOutput {
            counts: self.counted.then_some(self.counts),
            coverage_pct: self.coverage.map(|(_, v)| v),
            summary: self.summary,
        }
    }
}

fn parse_pct(cell: &str) -> Option<f64> {
    let value: f64 = cell.trim().trim_end_matches('%').trim().parse().ok()?;
    (value.is_finite() && (0.0..=100.0).contains(&value)).then_some(value)
}

/// Parse a whole output at once (tests, and small outputs).
pub fn parse_output(text: &str) -> ParsedOutput {
    let mut parser = OutputParser::new();
    for line in text.lines() {
        parser.feed_line(line);
    }
    parser.finish()
}

// ───────────────────────────── reports ─────────────────────────────

/// Where coverage tools write their reports by default, most specific
/// first. A report is only read when the run just wrote it.
pub const REPORT_LOCATIONS: &[&str] = &[
    "coverage/coverage-summary.json",
    "coverage/lcov.info",
    "coverage/lcov/lcov.info",
    "lcov.info",
    "coverage/cobertura-coverage.xml",
    "coverage/cobertura.xml",
    "cobertura.xml",
    "coverage.xml",
    "build/reports/jacoco/test/jacocoTestReport.xml",
    "target/site/jacoco/jacoco.xml",
    "coverage.json",
    "coverage.out",
    "cover.out",
    "c.out",
    "coverage.txt",
];

/// The first report under `root` (from [`REPORT_LOCATIONS`]) modified at
/// or after `since` that yields a line coverage: its relative path and the
/// percentage. A stale report — last week's `lcov.info` — is never read.
pub fn fresh_report(root: &Path, since: SystemTime) -> Option<(String, f64)> {
    // File times can be coarser than the clock the run started on.
    let since = since
        .checked_sub(std::time::Duration::from_secs(2))
        .unwrap_or(since);
    REPORT_LOCATIONS.iter().find_map(|rel| {
        let path = root.join(rel);
        let meta = std::fs::metadata(&path).ok()?;
        if !meta.is_file() || meta.len() > MAX_REPORT_BYTES {
            return None;
        }
        if meta.modified().ok()? < since {
            return None;
        }
        read_report(&path).map(|pct| (rel.to_string(), pct))
    })
}

/// Line coverage from a report file, by its name: Istanbul's
/// `coverage-summary.json`, coverage.py's `coverage.json`, lcov, Cobertura
/// XML, JaCoCo XML, or a Go cover profile.
pub fn read_report(path: &Path) -> Option<f64> {
    let name = path.file_name()?.to_str()?;
    let text = std::fs::read_to_string(path).ok()?;
    let pct = if name == "coverage-summary.json" {
        istanbul_summary(&text)
    } else if name == "coverage.json" {
        coverage_py_json(&text)
    } else if name.ends_with(".info") || name.ends_with(".lcov") {
        lcov(&text)
    } else if name.starts_with("jacoco") {
        jacoco(&text)
    } else if name.ends_with(".xml") {
        cobertura(&text)
    } else {
        go_profile(&text)
    }?;
    (pct.is_finite() && (0.0..=100.0).contains(&pct)).then(|| round1(pct))
}

fn istanbul_summary(text: &str) -> Option<f64> {
    let json: serde_json::Value = serde_json::from_str(text).ok()?;
    json.get("total")?.get("lines")?.get("pct")?.as_f64()
}

fn coverage_py_json(text: &str) -> Option<f64> {
    let json: serde_json::Value = serde_json::from_str(text).ok()?;
    let totals = json.get("totals")?;
    let statements = totals.get("num_statements")?.as_f64()?;
    let covered = totals.get("covered_lines")?.as_f64()?;
    (statements > 0.0).then(|| covered / statements * 100.0)
}

fn lcov(text: &str) -> Option<f64> {
    let (mut found, mut hit) = (0u64, 0u64);
    for line in text.lines() {
        if let Some(n) = line.strip_prefix("LF:") {
            found += n.trim().parse::<u64>().unwrap_or(0);
        } else if let Some(n) = line.strip_prefix("LH:") {
            hit += n.trim().parse::<u64>().unwrap_or(0);
        }
    }
    (found > 0).then(|| hit as f64 / found as f64 * 100.0)
}

static COBERTURA_RATE: LazyLock<Regex> =
    LazyLock::new(|| re(r#"<coverage\b[^>]*?\sline-rate="([\d.]+)""#));
static JACOCO_LINE: LazyLock<Regex> =
    LazyLock::new(|| re(r#"<counter\s+type="LINE"\s+missed="(\d+)"\s+covered="(\d+)"\s*/>"#));

fn cobertura(text: &str) -> Option<f64> {
    let rate: f64 = COBERTURA_RATE.captures(text)?[1].parse().ok()?;
    Some(rate * 100.0)
}

/// JaCoCo lists counters per method, class, package and — last — for the
/// whole report.
fn jacoco(text: &str) -> Option<f64> {
    let cap = JACOCO_LINE.captures_iter(text).last()?;
    let missed: f64 = cap[1].parse().ok()?;
    let covered: f64 = cap[2].parse().ok()?;
    (missed + covered > 0.0).then(|| covered / (missed + covered) * 100.0)
}

/// `mode: set` then `file:start.col,end.col statements count` per block.
fn go_profile(text: &str) -> Option<f64> {
    let mut lines = text.lines();
    if !lines.next()?.starts_with("mode:") {
        return None;
    }
    let (mut total, mut covered) = (0u64, 0u64);
    for line in lines {
        let mut fields = line.rsplitn(3, ' ');
        let (Some(count), Some(statements)) = (fields.next(), fields.next()) else {
            continue;
        };
        let (Ok(count), Ok(statements)) = (count.parse::<u64>(), statements.parse::<u64>()) else {
            continue;
        };
        total += statements;
        if count > 0 {
            covered += statements;
        }
    }
    (total > 0).then(|| covered as f64 / total as f64 * 100.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn counts(passed: u64, failed: u64, skipped: u64) -> Option<TestCounts> {
        Some(TestCounts {
            passed,
            failed,
            skipped,
        })
    }

    #[test]
    fn jest_counts_and_its_coverage_table() {
        let out = parse_output(
            "PASS src/a.test.js\nFAIL src/b.test.js\n\
             ----------|---------|----------|---------|---------|-------------------\n\
             File      | % Stmts | % Branch | % Funcs | % Lines | Uncovered Line #s \n\
             ----------|---------|----------|---------|---------|-------------------\n\
             All files |   85.71 |       50 |     100 |   83.33 |                   \n\
              a.js     |   85.71 |       50 |     100 |   83.33 | 4                 \n\
             Test Suites: 1 failed, 1 passed, 2 total\n\
             Tests:       1 failed, 12 passed, 13 total\n\
             Snapshots:   0 total\n",
        );
        assert_eq!(out.counts, counts(12, 1, 0));
        assert_eq!(out.coverage_pct, Some(83.33));
        assert!(
            out.summary.iter().any(|l| l.starts_with("Tests:")),
            "{out:?}"
        );
    }

    #[test]
    fn vitest_counts_through_colour_codes() {
        let out = parse_output(
            " \x1b[2m Test Files \x1b[22m \x1b[1m\x1b[32m2 passed\x1b[39m\x1b[22m\x1b[90m (2)\x1b[39m\n\
             \x1b[2m      Tests \x1b[22m \x1b[1m\x1b[31m1 failed\x1b[39m\x1b[22m\x1b[2m | \x1b[22m\x1b[1m\x1b[32m10 passed\x1b[39m\x1b[22m | 2 skipped\x1b[90m (13)\x1b[39m\n",
        );
        assert_eq!(out.counts, counts(10, 1, 2));
    }

    #[test]
    fn istanbul_text_summary_and_phpunit_lines() {
        let out = parse_output(
            "=============================== Coverage summary ===============================\n\
             Statements   : 85.71% ( 6/7 )\nBranches     : 50% ( 1/2 )\n\
             Functions    : 100% ( 2/2 )\nLines        : 83.33% ( 5/6 )\n",
        );
        assert_eq!(out.coverage_pct, Some(83.33));
        let out = parse_output("  Lines:   71.50% (143/200)\n");
        assert_eq!(out.coverage_pct, Some(71.5));
    }

    #[test]
    fn pytest_with_coverage_and_errors() {
        let out = parse_output(
            "---------- coverage: platform linux, python 3.12.1-final-0 -----------\n\
             Name        Stmts   Miss  Cover\n-------------------------------\n\
             app.py         20      4    80%\ntests.py       10      0   100%\n\
             -------------------------------\nTOTAL          30      4    87%\n\n\
             =========== 2 failed, 11 passed, 1 skipped, 1 error, 3 warnings in 0.52s ===========\n",
        );
        assert_eq!(out.counts, counts(11, 3, 1));
        assert_eq!(out.coverage_pct, Some(87.0));
        // `-q` prints the summary without the frame.
        assert_eq!(
            parse_output("13 passed in 0.12s\n").counts,
            counts(13, 0, 0)
        );
    }

    #[test]
    fn a_suite_that_ran_nothing_is_counted_as_zero() {
        for text in [
            "============================ no tests ran in 0.01s ============================",
            "No tests found, exiting with code 1",
            "No test files found, exiting with code 1",
            "Error: no test specified",
        ] {
            let out = parse_output(text);
            assert_eq!(out.counts, counts(0, 0, 0), "{text}");
            let result = SuiteResult {
                exit_code: Some(1),
                counts: out.counts,
                ..SuiteResult::default()
            };
            assert_eq!(failing_pct(&result), None, "no share to measure: {text}");
        }
    }

    #[test]
    fn cargo_sums_every_test_binary() {
        let out = parse_output(
            "running 3 tests\ntest a ... ok\n\
             test result: ok. 3 passed; 0 failed; 1 ignored; 0 measured; 0 filtered out; finished in 0.00s\n\n\
             running 2 tests\n\
             test result: FAILED. 1 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out\n\n\
             running 0 tests\n\
             test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out\n",
        );
        assert_eq!(out.counts, counts(4, 1, 1));
    }

    #[test]
    fn nextest_summary() {
        let out = parse_output(
            "     Summary [ 152.467s] 2469 tests run: 2468 passed (2 slow), 1 failed, 3 skipped\n",
        );
        assert_eq!(out.counts, counts(2468, 1, 3));
    }

    #[test]
    fn llvm_cov_total_row_reads_the_line_column() {
        let out = parse_output(
            "Filename  Regions  Missed Regions  Cover  Functions  Missed Functions  Executed  Lines  Missed Lines  Cover  Branches  Missed Branches  Cover\n\
             TOTAL          12               2  83.33%          4                 0   100.00%     20             3  85.00%         0                0        -\n",
        );
        assert_eq!(out.coverage_pct, Some(85.0));
    }

    #[test]
    fn mocha_node_bun_ava_and_deno() {
        assert_eq!(
            parse_output("  12 passing (35ms)\n  1 failing\n  2 pending\n").counts,
            counts(12, 1, 2)
        );
        assert_eq!(
            parse_output(
                "# tests 13\n# suites 2\n# pass 12\n# fail 1\n# cancelled 0\n# skipped 0\n"
            )
            .counts,
            counts(12, 1, 0)
        );
        assert_eq!(
            parse_output("ℹ tests 3\nℹ pass 3\nℹ fail 0\n").counts,
            counts(3, 0, 0)
        );
        assert_eq!(
            parse_output(
                " 12 pass\n 1 fail\n 20 expect() calls\nRan 13 tests across 2 files. [40.00ms]\n"
            )
            .counts,
            counts(12, 1, 0)
        );
        assert_eq!(
            parse_output("  3 tests passed\n  1 test failed\n").counts,
            counts(3, 1, 0)
        );
        assert_eq!(
            parse_output("FAILED | 12 passed (3 steps) | 1 failed | 2 ignored (40ms)\n").counts,
            counts(12, 1, 2)
        );
    }

    #[test]
    fn go_counts_only_in_verbose_mode_and_averages_packages() {
        let verbose = parse_output(
            "=== RUN   TestA\n--- PASS: TestA (0.00s)\n=== RUN   TestB\n--- FAIL: TestB (0.00s)\n\
             FAIL\ncoverage: 60.0% of statements\nFAIL\tex/a\t0.002s\n\
             ok  \tex/b\t0.002s\tcoverage: 80.0% of statements\n",
        );
        assert_eq!(verbose.counts, counts(1, 1, 0));
        assert_eq!(verbose.coverage_pct, Some(70.0));
        // Without -v only failures are listed: no count, the exit code decides.
        let quiet = parse_output("--- FAIL: TestB (0.00s)\nFAIL\nFAIL\tex/a\t0.002s\n");
        assert_eq!(quiet.counts, None);
        let total = parse_output("ex/a/a.go:3:\tA\t100.0%\ntotal:\t\t\t(statements)\t75.0%\n");
        assert_eq!(total.coverage_pct, Some(75.0));
    }

    #[test]
    fn ruby_python_php_dotnet_jvm_elixir() {
        assert_eq!(
            parse_output("13 examples, 1 failure, 2 pending\n").counts,
            counts(10, 1, 2)
        );
        assert_eq!(
            parse_output("13 runs, 20 assertions, 1 failures, 1 errors, 1 skips\n").counts,
            counts(10, 2, 1)
        );
        assert_eq!(
            parse_output("Ran 13 tests in 0.002s\n\nFAILED (failures=1, errors=1, skipped=2)\n")
                .counts,
            counts(9, 2, 2)
        );
        assert_eq!(
            parse_output("Ran 4 tests in 0.001s\n\nOK\n").counts,
            counts(4, 0, 0)
        );
        assert_eq!(
            parse_output("OK (13 tests, 20 assertions)\n").counts,
            counts(13, 0, 0)
        );
        assert_eq!(
            parse_output("Tests: 13, Assertions: 20, Errors: 1, Failures: 2, Skipped: 1.\n").counts,
            counts(9, 3, 1)
        );
        assert_eq!(
            parse_output("Failed!  - Failed:     1, Passed:    12, Skipped:     0, Total:    13, Duration: 1 s\n")
                .counts,
            counts(12, 1, 0)
        );
        assert_eq!(
            parse_output("13 tests completed, 1 failed, 2 skipped\n").counts,
            counts(10, 1, 2)
        );
        assert_eq!(
            parse_output("13 tests, 1 failure, 2 excluded\n").counts,
            counts(10, 1, 2)
        );
        assert_eq!(
            parse_output("13 specs, 0 failures\n").counts,
            counts(13, 0, 0)
        );
    }

    #[test]
    fn coverage_summaries_of_other_tools() {
        assert_eq!(
            parse_output("85.00% coverage, 17/20 lines covered\n").coverage_pct,
            Some(85.0)
        );
        assert_eq!(
            parse_output(
                "Coverage report generated for RSpec to /x/coverage. 17 / 20 LOC (85.0%) covered.\n"
            )
            .coverage_pct,
            Some(85.0)
        );
        assert_eq!(
            parse_output("| Module | Line | Branch | Method |\n| app    | 90%  | 50%    | 100%   |\n| Total  | 90%  | 50%    | 100%   |\n")
                .coverage_pct,
            Some(90.0)
        );
        assert_eq!(
            parse_output("File      | % Funcs | % Lines | Uncovered Line #s\nAll files |  100.00 |   85.71 |\n")
                .coverage_pct,
            Some(85.71)
        );
        assert_eq!(
            parse_output("Total coverage: 81.2%\n").coverage_pct,
            Some(81.2)
        );
        // Two percentages on a coverage line say nothing certain.
        assert_eq!(
            parse_output("Jest: \"global\" coverage threshold for lines (80%) not met: 75%\n")
                .coverage_pct,
            None
        );
    }

    #[test]
    fn a_table_total_outranks_a_generic_coverage_line() {
        let out = parse_output(
            "Coverage: 50%\n\
             File      | % Stmts | % Branch | % Funcs | % Lines |\n\
             All files |   90 |   90 |   90 |   88 |\n\
             coverage report written, 12%\n",
        );
        assert_eq!(out.coverage_pct, Some(88.0));
    }

    #[test]
    fn progress_bar_redraws_keep_the_last_segment() {
        let out = parse_output("running…\r  50%\rTests:  3 passed, 3 total\n");
        assert_eq!(out.counts, counts(3, 0, 0));
    }

    #[test]
    fn the_failing_share_prefers_counts_then_the_exit_code() {
        let with_counts = SuiteResult {
            exit_code: Some(1),
            counts: counts(12, 1, 5),
            ..SuiteResult::default()
        };
        assert_eq!(failing_pct(&with_counts), Some(7.7));
        let coverage_gate = SuiteResult {
            // A coverage threshold failed the command; every test passed.
            exit_code: Some(1),
            counts: counts(13, 0, 0),
            ..SuiteResult::default()
        };
        assert_eq!(failing_pct(&coverage_gate), Some(0.0));
        let green = SuiteResult {
            exit_code: Some(0),
            ..SuiteResult::default()
        };
        assert_eq!(failing_pct(&green), Some(0.0));
        let red = SuiteResult {
            exit_code: Some(2),
            ..SuiteResult::default()
        };
        assert_eq!(failing_pct(&red), Some(100.0));
        let killed = SuiteResult::default();
        assert_eq!(failing_pct(&killed), Some(100.0));
    }

    #[test]
    fn metrics_follow_what_was_measured() {
        let both = SuiteResult {
            exit_code: Some(0),
            counts: counts(10, 0, 0),
            coverage_pct: Some(81.04),
            ..SuiteResult::default()
        };
        let m = metrics(&both);
        assert_eq!(m.len(), 2);
        assert_eq!(
            (m[0].id.as_str(), m[0].value, m[0].direction),
            (TESTS_ID, Some(0.0), Direction::Lower)
        );
        assert_eq!(
            (m[1].id.as_str(), m[1].value, m[1].direction),
            (COVERAGE_ID, Some(81.0), Direction::Higher)
        );
        assert!(
            cpd_core::health::validate(&cpd_core::health::HealthConfig {
                metrics: m,
                ..Default::default()
            })
            .is_ok()
        );
        let nothing_ran = SuiteResult {
            exit_code: Some(0),
            counts: counts(0, 0, 0),
            ..SuiteResult::default()
        };
        assert!(metrics(&nothing_ran).is_empty());
    }

    // ── reports ──────────────────────────────────────────────────────

    fn write(dir: &Path, rel: &str, text: &str) {
        let path = dir.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    #[test]
    fn every_report_format_reads_line_coverage() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        write(
            d,
            "coverage/coverage-summary.json",
            r#"{"total":{"lines":{"total":6,"covered":5,"skipped":0,"pct":83.33},"statements":{"pct":85.71}}}"#,
        );
        assert_eq!(
            read_report(&d.join("coverage/coverage-summary.json")),
            Some(83.3)
        );
        write(
            d,
            "lcov.info",
            "TN:\nSF:a.js\nLF:10\nLH:8\nend_of_record\nSF:b.js\nLF:10\nLH:9\nend_of_record\n",
        );
        assert_eq!(read_report(&d.join("lcov.info")), Some(85.0));
        write(
            d,
            "coverage.xml",
            r#"<?xml version="1.0" ?><coverage version="7.4" timestamp="1" lines-valid="30" lines-covered="26" line-rate="0.8667" branch-rate="0"><packages/></coverage>"#,
        );
        assert_eq!(read_report(&d.join("coverage.xml")), Some(86.7));
        write(
            d,
            "target/site/jacoco/jacoco.xml",
            r#"<report name="x"><package name="a"><counter type="LINE" missed="1" covered="1"/></package><counter type="INSTRUCTION" missed="5" covered="15"/><counter type="LINE" missed="3" covered="17"/></report>"#,
        );
        assert_eq!(
            read_report(&d.join("target/site/jacoco/jacoco.xml")),
            Some(85.0)
        );
        write(
            d,
            "coverage.json",
            r#"{"meta":{},"totals":{"covered_lines":17,"num_statements":20,"percent_covered":80.1}}"#,
        );
        assert_eq!(read_report(&d.join("coverage.json")), Some(85.0));
        write(
            d,
            "cover.out",
            "mode: set\nex/a.go:3.14,5.2 2 1\nex/a.go:7.14,9.2 3 0\nex/b.go:1.1,2.2 5 4\n",
        );
        assert_eq!(read_report(&d.join("cover.out")), Some(70.0));
        write(d, "coverage.txt", "not a profile\n");
        assert_eq!(read_report(&d.join("coverage.txt")), None);
    }

    #[test]
    fn only_a_report_the_run_just_wrote_is_read() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "coverage/lcov.info", "LF:4\nLH:3\n");
        let later = SystemTime::now() + std::time::Duration::from_secs(60);
        assert_eq!(fresh_report(dir.path(), later), None, "stale");
        let earlier = SystemTime::now() - std::time::Duration::from_secs(60);
        assert_eq!(
            fresh_report(dir.path(), earlier),
            Some(("coverage/lcov.info".to_string(), 75.0))
        );
    }
}
