//! Analysis tool adapters — `mode: analysis` probes.
//!
//! One adapter = one tool. Adding a tool touches this file's registry and
//! nothing else: the orchestrator (probe ticker, done-probes, judge-declared
//! probes) dispatches by name through [`find_adapter`] and treats every
//! failure the same way — outcome `unavailable`, reason recorded, zero
//! points. The golf-verify sandbox incident is the governing precedent: a
//! missing or broken tool is our problem, never the player's.
//!
//! Adapters run inside the same sandbox as deterministic probes, against the
//! same materialized snapshot; their structured metrics land in
//! `probes.result_json` and — serialized — in `probes.output`, so a section
//! may attach a `js validation` threshold over them
//! (`JSON.parse(result).duplicated_pct < 5`).

use std::path::Path;
use std::time::Duration;

use arena_core::sandbox::{self, SandboxBackend};
use async_trait::async_trait;

/// What one analysis run measured.
#[derive(Debug)]
pub struct AnalysisResult {
    /// Structured metrics for `result_json` and threshold validation.
    pub metrics: serde_json::Value,
    /// One human-readable line for logs/feedback.
    pub summary: String,
}

/// One analysis tool.
#[async_trait]
pub trait ToolAdapter: Send + Sync {
    fn name(&self) -> &'static str;
    /// Prove the tool can run at all (outside the sandbox — presence check).
    /// `Err(reason)` ⇒ the probe records `unavailable`.
    async fn self_check(&self) -> Result<(), String>;
    /// Run against the materialized snapshot. `Err(reason)` ⇒ `unavailable`.
    async fn run(
        &self,
        workdir: &Path,
        backend: SandboxBackend,
        deadline: Duration,
    ) -> Result<AnalysisResult, String>;
}

/// The registry. One line per tool.
pub fn find_adapter(name: &str) -> Option<&'static dyn ToolAdapter> {
    static JSCPD: JscpdAdapter = JscpdAdapter;
    static OXLINT: OxlintAdapter = OxlintAdapter;
    match name.trim() {
        "jscpd" => Some(&JSCPD),
        "oxlint" => Some(&OXLINT),
        _ => None,
    }
}

async fn binary_exists(bin: &'static str) -> Result<(), String> {
    tokio::task::spawn_blocking(move || {
        which::which(bin)
            .map(|_| ())
            .map_err(|_| format!("`{bin}` is not installed on the game server"))
    })
    .await
    .map_err(|e| format!("self-check join: {e}"))?
}

/// Exit 126/127 inside the sandbox = the tool is not reachable *there*
/// (missing robind, missing interpreter) — unavailable, not a measurement.
fn sandbox_missing(exit_code: i32, stdout: &str) -> bool {
    stdout.is_empty() && matches!(exit_code, 126 | 127)
}

// ---------- jscpd: copy-paste duplication ----------

struct JscpdAdapter;

#[async_trait]
impl ToolAdapter for JscpdAdapter {
    fn name(&self) -> &'static str {
        "jscpd"
    }

    async fn self_check(&self) -> Result<(), String> {
        binary_exists("jscpd").await
    }

    async fn run(
        &self,
        workdir: &Path,
        backend: SandboxBackend,
        deadline: Duration,
    ) -> Result<AnalysisResult, String> {
        // JSON report into the (writable) scratch tree, read back after the
        // sandboxed run. `--silent` keeps stdout clean of clone dumps.
        let command = "jscpd --silent --reporters json --output .ololo-analysis . >/dev/null 2>&1; \
                       cat .ololo-analysis/jscpd-report.json 2>/dev/null";
        let out = sandbox::run(backend, command, workdir, deadline)
            .await
            .map_err(|e| format!("jscpd run: {e}"))?;
        if sandbox_missing(out.exit_code, &out.stdout) {
            return Err("jscpd unavailable inside the sandbox".to_string());
        }
        if out.timed_out {
            return Err("jscpd timed out".to_string());
        }
        let report: serde_json::Value = serde_json::from_str(out.stdout.trim())
            .map_err(|e| format!("jscpd report unparseable: {e}"))?;
        let total = &report["statistics"]["total"];
        let metrics = serde_json::json!({
            "tool": "jscpd",
            "duplicated_pct": total["percentage"],
            "duplicated_lines": total["duplicatedLines"],
            "clones": total["clones"],
            "total_lines": total["lines"],
        });
        let summary = format!(
            "jscpd: {}% duplicated ({} clones)",
            total["percentage"], total["clones"]
        );
        Ok(AnalysisResult { metrics, summary })
    }
}

// ---------- oxlint: lint diagnostics ----------

struct OxlintAdapter;

#[async_trait]
impl ToolAdapter for OxlintAdapter {
    fn name(&self) -> &'static str {
        "oxlint"
    }

    async fn self_check(&self) -> Result<(), String> {
        binary_exists("oxlint").await
    }

    async fn run(
        &self,
        workdir: &Path,
        backend: SandboxBackend,
        deadline: Duration,
    ) -> Result<AnalysisResult, String> {
        // Diagnostics are the measurement; oxlint exits non-zero when it
        // finds any, so the exit code is not an error signal here.
        let command = "oxlint --format json . 2>/dev/null";
        let out = sandbox::run(backend, command, workdir, deadline)
            .await
            .map_err(|e| format!("oxlint run: {e}"))?;
        if sandbox_missing(out.exit_code, &out.stdout) {
            return Err("oxlint unavailable inside the sandbox".to_string());
        }
        if out.timed_out {
            return Err("oxlint timed out".to_string());
        }
        let report: serde_json::Value =
            serde_json::from_str(out.stdout.trim()).unwrap_or(serde_json::json!({}));
        let diagnostics = report["diagnostics"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        let errors = diagnostics
            .iter()
            .filter(|d| d["severity"] == "error")
            .count();
        let warnings = diagnostics.len() - errors;
        let metrics = serde_json::json!({
            "tool": "oxlint",
            "errors": errors,
            "warnings": warnings,
            "diagnostics_total": diagnostics.len(),
        });
        let summary = format!("oxlint: {errors} errors, {warnings} warnings");
        Ok(AnalysisResult { metrics, summary })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fake adapter proves the trait shape: registry lookup by name and
    /// unavailable-on-missing semantics live in the orchestrator, not here.
    struct FakeAdapter;

    #[async_trait]
    impl ToolAdapter for FakeAdapter {
        fn name(&self) -> &'static str {
            "fake"
        }
        async fn self_check(&self) -> Result<(), String> {
            Ok(())
        }
        async fn run(
            &self,
            _workdir: &Path,
            _backend: SandboxBackend,
            _deadline: Duration,
        ) -> Result<AnalysisResult, String> {
            Ok(AnalysisResult {
                metrics: serde_json::json!({"tool": "fake", "score": 7}),
                summary: "fake: 7".to_string(),
            })
        }
    }

    #[tokio::test]
    async fn fake_adapter_round_trips() {
        let a = FakeAdapter;
        a.self_check().await.unwrap();
        let r = a
            .run(
                Path::new("/tmp"),
                SandboxBackend::Unsandboxed,
                Duration::from_secs(1),
            )
            .await
            .unwrap();
        assert_eq!(r.metrics["score"], 7);
    }

    #[test]
    fn registry_knows_its_tools_and_nothing_else() {
        assert!(find_adapter("jscpd").is_some());
        assert!(find_adapter("oxlint").is_some());
        assert!(find_adapter(" jscpd ").is_some(), "names are trimmed");
        assert!(find_adapter("teleport").is_none());
    }

    #[tokio::test]
    async fn missing_binary_fails_self_check() {
        // A tool that certainly does not exist on any CI machine.
        let err = binary_exists("definitely-not-a-real-tool-xyz")
            .await
            .unwrap_err();
        assert!(err.contains("not installed"), "{err}");
    }
}
