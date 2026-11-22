//! Code health as the F3 menu hands it to the agent: the newest score, what
//! it is made of, and what would raise it.
//!
//! The CLI scores the tree after every check (`health_run`) and runs the
//! project's tests (`suite_run`); both tell the TUI what they found. The
//! number shown here is composed the way the server composes it — the
//! tree's dimensions plus the tests' — so the agent reads the score the
//! session pays on.

use ololo_health::compose::{Composed, compose, static_dimensions};
use ololo_health::suite::{COVERAGE_ID, SuiteResult, TESTS_ID, metrics};
use ololo_health::{HealthResult, Level, Thresholds};

/// The newest code-health analysis of the workspace.
#[derive(Debug, Clone)]
pub struct HealthNote {
    /// The check whose commit was scored (0 when the server did not number it).
    pub probe_seq: u32,
    pub result: HealthResult,
}

/// The newest run of the project's tests.
#[derive(Debug, Clone)]
pub struct TestsNote {
    pub probe_seq: u32,
    pub command: String,
    pub result: SuiteResult,
    /// Workspace-relative path of the run's log, when it was written.
    pub log: Option<String>,
}

/// The score with the tests folded in, and its colour.
pub struct HealthView {
    pub composed: Composed,
    pub level: Level,
}

pub fn view(health: &HealthNote, tests: Option<&TestsNote>) -> HealthView {
    let dims = serde_json::to_value(&health.result)
        .ok()
        .and_then(|v| static_dimensions(&v));
    let extra = tests.map(|t| metrics(&t.result)).unwrap_or_default();
    let composed = compose(dims.as_deref(), &extra);
    let level = ololo_health::level(composed.score, &Thresholds::default());
    HealthView { composed, level }
}

pub fn level_word(level: Level) -> &'static str {
    match level {
        Level::Green => "green",
        Level::Amber => "amber",
        Level::Red => "red",
        Level::Unknown => "no score",
    }
}

fn pct(value: Option<f64>) -> String {
    value.map_or_else(|| "n/a".to_string(), |v| format!("{v:.1}%"))
}

fn tests_counts(result: &SuiteResult) -> String {
    match result.counts {
        Some(c) if c.passed + c.failed > 0 => {
            let mut s = format!("{} passed", c.passed);
            if c.failed > 0 {
                s.push_str(&format!(", {} failed", c.failed));
            }
            if c.skipped > 0 {
                s.push_str(&format!(", {} skipped", c.skipped));
            }
            s
        }
        Some(_) => "no tests ran".to_string(),
        None => match result.exit_code {
            Some(0) => "passed (exit 0)".to_string(),
            Some(code) => format!("failed (exit {code})"),
            None => "killed".to_string(),
        },
    }
}

/// `62.5 C · amber` — the menu row's headline.
pub fn headline(view: &HealthView) -> String {
    match view.composed.score {
        Some(score) => {
            let grade = view
                .composed
                .grade
                .map(|g| format!(" {g}"))
                .unwrap_or_default();
            format!("{score:.1}{grade} · {}", level_word(view.level))
        }
        None => "no score yet — no code to measure".to_string(),
    }
}

/// One line of the numbers behind the score, for the menu row.
pub fn hint(health: &HealthNote, tests: Option<&TestsNote>) -> String {
    let m = &health.result.metrics;
    let mut parts = vec![
        format!("duplication {}", pct(m.duplication_pct)),
        format!("complexity {}", pct(m.complexity_pct)),
    ];
    if m.dead_code_pct.is_some() {
        parts.push(format!("dead code {}", pct(m.dead_code_pct)));
    }
    if let Some(t) = tests {
        parts.push(format!("tests {}", tests_counts(&t.result)));
        if let Some(c) = t.result.coverage_pct {
            parts.push(format!("coverage {c:.0}%"));
        }
    }
    format!("{} — with what would raise it", parts.join(" · "))
}

/// What a dimension measures, in the numbers the agent can act on.
fn dimension_line(id: &str, health: &HealthNote, tests: Option<&TestsNote>) -> String {
    let m = &health.result.metrics;
    match id {
        "duplication" => format!(
            "duplication — {} of the code lines are copy-paste ({} clone pairs)",
            pct(m.duplication_pct),
            m.clones
        ),
        "complexity" => format!(
            "complexity — {} of the code lines sit in complex files",
            pct(m.complexity_pct)
        ),
        "dead-code" => format!(
            "dead code — {} of the code lines are unused ({} unused files, exports, symbols or imports)",
            pct(m.dead_code_pct),
            m.dead_symbols.unwrap_or(0)
        ),
        TESTS_ID => format!(
            "tests — {}",
            tests.map_or_else(|| "n/a".to_string(), |t| tests_counts(&t.result))
        ),
        COVERAGE_ID => format!(
            "coverage — {} of the lines are covered by the tests",
            pct(tests.and_then(|t| t.result.coverage_pct))
        ),
        other => other.to_string(),
    }
}

/// What to do about the weakest dimension.
fn advice(id: &str) -> &'static str {
    match id {
        "duplication" => {
            "remove the copy-paste: move the repeated code into shared functions or modules"
        }
        "complexity" => "split the long, branchy functions and the biggest files into smaller ones",
        "dead-code" => "delete the unused files, exports, functions and imports",
        TESTS_ID => "make the failing tests pass",
        COVERAGE_ID => "cover the untested code with tests",
        _ => "improve this dimension",
    }
}

/// The health report as text for the agent: the level, what it means, the
/// dimensions behind it and where the points are.
pub fn paste_text(health: &HealthNote, tests: Option<&TestsNote>) -> String {
    let view = view(health, tests);
    let t = Thresholds::default();
    let after = if health.probe_seq > 0 {
        format!(" — measured after check #{}", health.probe_seq)
    } else {
        String::new()
    };
    let mut out = format!("Code health{after}\n");
    match view.composed.score {
        Some(score) => {
            let grade = view
                .composed
                .grade
                .map(|g| format!(", grade {g}"))
                .unwrap_or_default();
            out.push_str(&format!(
                "Level: {} — score {score:.1} of 100{grade}.\n",
                level_word(view.level)
            ));
        }
        None => out.push_str("Level: no score yet — the workspace has no code to measure.\n"),
    }
    out.push_str(&format!(
        "What it is: ololo scores the code in this workspace after every check — one number \
         from duplication, complexity, dead code and the project's tests. Green starts at {:.0}, \
         amber at {:.0}, below is red. Healthier code earns a bigger health bonus on every task; \
         red earns none.\n",
        t.green_min, t.amber_min
    ));
    if !view.composed.dimensions.is_empty() {
        out.push_str("What it is made of (each scored 0-100; the score is their weighted mean):\n");
        for d in &view.composed.dimensions {
            out.push_str(&format!(
                "- {}: {:.1}\n",
                dimension_line(&d.id, health, tests),
                d.score
            ));
        }
    }
    let m = &health.result.metrics;
    out.push_str(&format!(
        "Size: {} code files, {} lines.\n",
        m.files, m.code_lines
    ));
    match tests {
        Some(t) => {
            out.push_str(&format!("Tests ran with: {}", t.command));
            if let Some(log) = &t.log {
                out.push_str(&format!(" (log: {log})"));
            }
            out.push('\n');
        }
        None => out.push_str(
            "Tests: not measured yet — say how to run them in AGENTS.md or README.md and health \
             counts them too.\n",
        ),
    }
    if let Some(weakest) = view
        .composed
        .dimensions
        .iter()
        .min_by(|a, b| a.score.total_cmp(&b.score))
        .filter(|d| d.score < 100.0)
    {
        out.push_str(&format!(
            "What would raise it most: {} (the weakest part, {:.1}).\n",
            advice(&weakest.id),
            weakest.score
        ));
    }
    out
}
