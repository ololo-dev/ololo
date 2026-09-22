//! The scan configuration both sides share.
//!
//! Everything here is an input to the score. Two runs are comparable only
//! when they were made with equal configs, so the defaults are the contract
//! and the server sends the client nothing but a timeout.

use std::time::Duration;

use serde::{Deserialize, Serialize};

/// Directory names that never hold a player's own code: regenerable
/// dependency stores, build output and tool caches. The ololo snapshot
/// prunes exactly this list when it commits (so these never reach a
/// snapshot tree), and the scan skips them again for the case where a
/// live checkout is analyzed.
pub const PRUNED_DIRS: &[&str] = &[
    // dependency stores
    "node_modules",
    ".venv",
    "venv",
    // build / framework output
    "target",
    "dist",
    "build",
    "out",
    "obj",
    ".next",
    ".nuxt",
    ".svelte-kit",
    // caches and tool state
    "__pycache__",
    ".cache",
    ".parcel-cache",
    ".turbo",
    ".gradle",
    ".tox",
    ".pytest_cache",
    ".mypy_cache",
    ".ruff_cache",
    ".nyc_output",
    "coverage",
    ".idea",
];

/// Directories the scan never reads even though a snapshot commits them:
/// `.git` (absent from snapshot trees, present in a live checkout) and
/// `.ololo/` — platform plumbing (done-files, artifacts, memory) that every
/// session shares and that would score every player alike.
pub const ALWAYS_IGNORED_DIRS: &[&str] = &[".git", ".ololo"];

/// The ignore globs handed to jscpd: every [`ALWAYS_IGNORED_DIRS`] and
/// [`PRUNED_DIRS`] entry as `<dir>/**`. jscpd adds a `**/` prefix variant
/// of each pattern itself, so the directories match at any depth.
pub fn default_ignore_globs() -> Vec<String> {
    ALWAYS_IGNORED_DIRS
        .iter()
        .chain(PRUNED_DIRS.iter())
        .map(|dir| format!("{dir}/**"))
        .collect()
}

/// Colour thresholds on the 0..100 health score. The defaults follow
/// jscpd's own grade bands — A/B green (≥ 70), C amber (≥ 55), D/E red —
/// which jscpd calibrated so that the median open-source project scores
/// around 75: a healthy, ordinary project is green, not amber.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Thresholds {
    /// Scores from here up are green.
    pub green_min: f64,
    /// Scores from here up to `green_min` are amber; below is red.
    pub amber_min: f64,
}

impl Default for Thresholds {
    fn default() -> Self {
        Self {
            green_min: 70.0,
            amber_min: 55.0,
        }
    }
}

impl Thresholds {
    pub fn validate(&self) -> Result<(), ConfigError> {
        let (g, a) = (self.green_min, self.amber_min);
        let in_range = |v: f64| v.is_finite() && (0.0..=100.0).contains(&v);
        if !in_range(g) || !in_range(a) {
            return Err(ConfigError::Thresholds(format!(
                "thresholds must lie in 0..=100 (green_min {g}, amber_min {a})"
            )));
        }
        if a >= g {
            return Err(ConfigError::Thresholds(format!(
                "amber_min ({a}) must be below green_min ({g})"
            )));
        }
        Ok(())
    }
}

/// Everything a scan needs. `Default` is the shared contract; only
/// `timeout` and `workers` are expected to differ between deployments,
/// and neither influences the score.
#[derive(Debug, Clone)]
pub struct HealthConfig {
    /// Wall-clock budget for one analysis; past it the caller gets
    /// [`crate::HealthError::Timeout`] (the worker thread finishes on its own —
    /// jscpd's pipeline cannot be interrupted).
    pub timeout: Duration,
    /// jscpd ignore globs; see [`default_ignore_globs`].
    pub ignore: Vec<String>,
    /// Files larger than this are skipped by jscpd (a generated bundle or a
    /// data dump is not the player's code) and by the marker survey.
    pub max_file_bytes: u64,
    /// Refuse to scan a tree with more files than this (after the directory
    /// exclusions): the bound on jscpd's memory is the size of its input.
    pub max_files: u64,
    /// Refuse to scan a tree whose files (after the exclusions) exceed this
    /// many bytes in total.
    pub max_total_bytes: u64,
    /// jscpd worker threads; `None` lets rayon decide. Never affects the
    /// result — detection output is sorted.
    pub workers: Option<usize>,
    pub thresholds: Thresholds,
    /// How far a client score may sit from the server's before the
    /// checkpoint is flagged. jscpd rounds to one decimal, so 0.05 means
    /// "equal after rounding".
    pub tolerance: f64,
    /// jscpd's own health tuning (half-lives, weights, complex-file cutoff).
    /// The defaults are jscpd's calibrated ones.
    pub jscpd: cpd_core::health::HealthConfig,
}

impl Default for HealthConfig {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(30),
            ignore: default_ignore_globs(),
            max_file_bytes: 512 * 1024,
            max_files: 5_000,
            max_total_bytes: 64 * 1024 * 1024,
            workers: None,
            thresholds: Thresholds::default(),
            tolerance: 0.05,
            jscpd: cpd_core::health::HealthConfig::default(),
        }
    }
}

impl HealthConfig {
    pub fn validate(&self) -> Result<(), ConfigError> {
        self.thresholds.validate()?;
        if !(self.tolerance.is_finite() && self.tolerance >= 0.0) {
            return Err(ConfigError::Tolerance(self.tolerance));
        }
        if self.timeout.is_zero() {
            return Err(ConfigError::Timeout);
        }
        cpd_core::health::validate(&self.jscpd).map_err(ConfigError::Jscpd)
    }
}

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum ConfigError {
    #[error("{0}")]
    Thresholds(String),
    #[error("tolerance must be a non-negative number, got {0}")]
    Tolerance(f64),
    #[error("timeout must be positive")]
    Timeout,
    #[error("jscpd health config: {0}")]
    Jscpd(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_follow_jscpd_grades() {
        let t = Thresholds::default();
        assert_eq!((t.green_min, t.amber_min), (70.0, 55.0));
        assert!(t.validate().is_ok());
        assert!(HealthConfig::default().validate().is_ok());
    }

    #[test]
    fn thresholds_reject_inverted_and_out_of_range_values() {
        assert!(
            Thresholds {
                green_min: 55.0,
                amber_min: 70.0
            }
            .validate()
            .is_err()
        );
        assert!(
            Thresholds {
                green_min: 70.0,
                amber_min: 70.0
            }
            .validate()
            .is_err()
        );
        assert!(
            Thresholds {
                green_min: 101.0,
                amber_min: 55.0
            }
            .validate()
            .is_err()
        );
        assert!(
            Thresholds {
                green_min: 70.0,
                amber_min: -1.0
            }
            .validate()
            .is_err()
        );
        assert!(
            Thresholds {
                green_min: f64::NAN,
                amber_min: 55.0
            }
            .validate()
            .is_err()
        );
        assert!(
            Thresholds {
                green_min: 100.0,
                amber_min: 0.0
            }
            .validate()
            .is_ok()
        );
    }

    #[test]
    fn config_rejects_bad_tolerance_and_zero_timeout() {
        let mut cfg = HealthConfig {
            tolerance: -0.1,
            ..Default::default()
        };
        assert_eq!(cfg.validate(), Err(ConfigError::Tolerance(-0.1)));
        cfg.tolerance = 0.0;
        cfg.timeout = Duration::ZERO;
        assert_eq!(cfg.validate(), Err(ConfigError::Timeout));
    }

    #[test]
    fn ignore_globs_cover_plumbing_and_dependency_stores() {
        let globs = default_ignore_globs();
        for expected in [
            ".git/**",
            ".ololo/**",
            "node_modules/**",
            "target/**",
            "coverage/**",
        ] {
            assert!(globs.iter().any(|g| g == expected), "missing {expected}");
        }
        assert_eq!(globs.len(), ALWAYS_IGNORED_DIRS.len() + PRUNED_DIRS.len());
    }
}
