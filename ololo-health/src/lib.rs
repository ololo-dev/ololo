//! Code-health analysis shared by the ololo client and the servers.
//!
//! One crate owns everything that has to agree between the two sides for a
//! health score to mean the same thing: the jscpd version, the scan
//! configuration (ignore rules, size caps), the normalization of jscpd's
//! result into ololo's score and colour level, the thresholds and the
//! bonus mapping. The client scores the tree it just committed; the server
//! re-scores the same commit and its number is the one that counts. Same
//! crate version + same tree ⇒ byte-identical [`HealthResult::health`].
//!
//! jscpd is a library here (`cpd-core` + `cpd-finder`), never a binary: the
//! pipeline in [`analyze`] is the in-process twin of `jscpd --health`.
//!
//! What the scan reads is decided entirely by this crate — the tree's own
//! `.jscpd.json` / `package.json#jscpd` is deliberately **not** honoured
//! (a player could otherwise switch detection off), only reported through
//! [`Metrics::jscpd_config_present`]. `.gitignore` is applied when the tree
//! is committed, not when it is scanned, so a scan never depends on whether
//! the directory happens to sit inside a git repository.

mod analyze;
pub mod compose;
pub mod config;
pub mod score;
pub mod suite;

pub use analyze::{HealthError, HealthResult, Metrics, analyze, analyze_async};
pub use config::{
    ALWAYS_IGNORED_DIRS, ConfigError, HealthConfig, PRUNED_DIRS, Thresholds, default_ignore_globs,
};
pub use score::{Bonus, BonusReason, DELTA_SPAN, Level, bonus, delta_bonus, level, scores_match};

/// Version of `cpd-core` this crate links — the health formula lives there.
/// Reported in every result so the server can tell a client built against
/// another jscpd from a genuine score disagreement. Hand-maintained next to
/// the `=` pins in Cargo.toml; `pinned_versions_match_cargo_lock` fails the
/// tests when they drift (jscpd exposes no version constant yet).
pub const JSCPD_CORE_VERSION: &str = "0.1.17";
/// Version of `cpd-finder` this crate links (walking, tokenizing, detection).
pub const JSCPD_FINDER_VERSION: &str = "0.1.17";
/// jscpd's dead-code analyzer (`basta`), the third health dimension.
pub const BASTA_VERSION: &str = "0.3.0";
/// Version of `cpd-tokenizer`, whose format table decides which files a
/// scan reads.
pub const JSCPD_TOKENIZER_VERSION: &str = "0.1.17";

/// Schema of [`HealthResult`] as ololo serializes it — not jscpd's own
/// output, which is unversioned upstream. Bump when a field changes meaning.
pub const HEALTH_SCHEMA: u32 = 2;

#[cfg(test)]
mod tests {
    use super::*;

    /// The pinned crate versions in Cargo.toml are what the workspace
    /// actually resolves — and what the constants above claim.
    #[test]
    fn pinned_versions_match_cargo_lock() {
        let lock = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../Cargo.lock"))
            .expect("workspace Cargo.lock");
        for (name, expected) in [
            ("cpd-core", JSCPD_CORE_VERSION),
            ("cpd-finder", JSCPD_FINDER_VERSION),
            ("basta", BASTA_VERSION),
            ("cpd-tokenizer", JSCPD_TOKENIZER_VERSION),
        ] {
            let resolved = locked_version(&lock, name)
                .unwrap_or_else(|| panic!("{name} is not in Cargo.lock"));
            assert_eq!(
                resolved, expected,
                "{name}: constant drifted from the Cargo.lock pin"
            );
        }
    }

    fn locked_version(lock: &str, name: &str) -> Option<String> {
        let mut lines = lock.lines();
        while let Some(line) = lines.next() {
            if line.trim() == format!("name = \"{name}\"") {
                let version = lines.next()?.trim().strip_prefix("version = \"")?;
                return version.strip_suffix('"').map(str::to_string);
            }
        }
        None
    }
}
