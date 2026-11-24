//! `ololo health` — the code-health score of a folder, measured the way a
//! session measures it.
//!
//! A session scores the tree after every check and pays the health bonus
//! on that number. This command runs the same analysis on a folder, so a
//! player (or their agent, or a task's own check) can read the score
//! between checks, or of a tree no session is looking at. It is the code's
//! part of the score — duplication, complexity and dead code; a session
//! adds the project's tests and their coverage on top.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};
use ololo_health::{HealthConfig, HealthResult, Level};

fn level_word(level: Level) -> &'static str {
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

/// The report as people read it. The first line is `health: <score> <grade>
/// (<level>)` — stable, so a script can take the number from it.
pub fn render(result: &HealthResult) -> String {
    let m = &result.metrics;
    let head = match result.score {
        Some(score) => {
            let grade = result.grade.map(|g| format!(" {g}")).unwrap_or_default();
            format!("health: {score:.1}{grade} ({})", level_word(result.level))
        }
        None => "health: no score - no code to measure".to_string(),
    };
    let mut parts = vec![
        format!(
            "duplication {} ({} clone pairs)",
            pct(m.duplication_pct),
            m.clones
        ),
        format!("complexity {}", pct(m.complexity_pct)),
    ];
    if m.dead_code_pct.is_some() {
        parts.push(format!(
            "dead code {} ({} unused)",
            pct(m.dead_code_pct),
            m.dead_symbols.unwrap_or(0)
        ));
    }
    format!(
        "{head}\n{}\n{} code files, {} lines. The code only: a session also counts the tests and their coverage.\n",
        parts.join(" · "),
        m.files,
        m.code_lines
    )
}

fn measure(dir: &Path) -> Result<HealthResult> {
    if !dir.is_dir() {
        return Err(anyhow!("{} is not a folder", dir.display()));
    }
    ololo_health::analyze(dir, &HealthConfig::default())
        .map_err(|e| anyhow!("{e}"))
        .with_context(|| format!("measuring the code health of {}", dir.display()))
}

pub async fn run_health(path: Option<PathBuf>, json: bool) -> Result<()> {
    let dir = match path {
        Some(p) => p,
        None => std::env::current_dir().context("reading the working directory")?,
    };
    // The scan is CPU work with its own timeout; keep it off the runtime.
    let result = tokio::task::spawn_blocking(move || measure(&dir))
        .await
        .context("the health scan stopped unexpectedly")??;
    if json {
        println!("{}", serde_json::to_string_pretty(&result)?);
    } else {
        print!("{}", render(&result));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(dir: &Path, name: &str, body: &str) {
        let path = dir.join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, body).unwrap();
    }

    /// A function long enough for the clone detector to see.
    fn block(name: &str) -> String {
        let mut s = format!("export function {name}(items) {{\n  let total = 0;\n");
        for i in 0..14 {
            s.push_str(&format!(
                "  if (items[{i}] && items[{i}].value > {i}) {{ total += items[{i}].value * {i}; }}\n"
            ));
        }
        s.push_str("  return total;\n}\n");
        s
    }

    #[test]
    fn the_first_line_carries_the_score() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "src/a.js", &block("sum"));
        let result = measure(dir.path()).unwrap();
        let text = render(&result);
        let first = text.lines().next().unwrap();
        let score = result.score.expect("code was measured");
        assert!(
            first.starts_with(&format!("health: {score:.1} ")),
            "first line: {first}"
        );
        assert!(text.contains("1 code files"), "{text}");
    }

    #[test]
    fn copy_paste_lowers_the_score_and_what_a_session_skips_is_skipped() {
        let clean = tempfile::tempdir().unwrap();
        write(clean.path(), "src/a.js", &block("sum"));
        let pasted = tempfile::tempdir().unwrap();
        write(pasted.path(), "src/a.js", &block("sum"));
        write(pasted.path(), "src/b.js", &block("sum"));
        write(pasted.path(), "src/c.js", &block("sum"));
        let a = measure(clean.path()).unwrap();
        let b = measure(pasted.path()).unwrap();
        assert!(
            b.score.unwrap() < a.score.unwrap(),
            "{:?} vs {:?}",
            b.score,
            a.score
        );

        // Dependencies and ololo's own folder never count.
        write(clean.path(), "node_modules/x/a.js", &block("sum"));
        write(clean.path(), "node_modules/x/b.js", &block("sum"));
        write(clean.path(), ".ololo/tmp/a.js", &block("sum"));
        let again = measure(clean.path()).unwrap();
        assert_eq!(again.metrics.files, 1);
        assert_eq!(again.score, a.score);
    }

    #[test]
    fn an_empty_folder_has_no_score() {
        let dir = tempfile::tempdir().unwrap();
        let result = measure(dir.path()).unwrap();
        assert!(render(&result).starts_with("health: no score"));
    }

    #[test]
    fn a_missing_folder_is_an_error() {
        let err = measure(Path::new("/no/such/folder/for/ololo")).unwrap_err();
        assert!(err.to_string().contains("is not a folder"));
    }
}
