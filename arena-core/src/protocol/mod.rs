//! Canonical wire protocol for arena server <-> observer (and dashboard fan-out).
//!
//! Every payload struct enforces `#[serde(deny_unknown_fields)]` so a typo
//! on either end fails fast.
//!
//! The protocol is split into cohesive submodules; all public types are
//! re-exported here for a single import surface (`use arena_core::protocol::*`).

pub mod agent;
pub mod arena_frame;
pub mod ids;
pub mod player;
pub mod session;
pub mod stats;
pub mod zmq;

// Single import surface: `use arena_core::protocol::*;` keeps working.
pub use agent::*;
pub use arena_frame::*;
pub use ids::*;
pub use player::*;
pub use session::*;
pub use stats::*;
pub use zmq::*;

/// Maximum stdout tail size carried in a `TestResult` frame (FR-029).
pub const STDOUT_TAIL_MAX_BYTES: usize = 64 * 1024;

/// One delivered artifact file, as carried by `ArtifactReceived` events.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArtifactFile {
    /// Repo-relative path.
    pub path: String,
    pub size: u64,
}

/// Validate `^[A-Za-z0-9_]{1,64}$` without pulling in a regex crate.
pub fn validate_placeholder_name(name: &str) -> Result<(), &'static str> {
    let bytes = name.as_bytes();
    if bytes.is_empty() || bytes.len() > 64 {
        return Err("invalid placeholder name");
    }
    for &b in bytes {
        let ok = b.is_ascii_alphanumeric() || b == b'_';
        if !ok {
            return Err("invalid placeholder name");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placeholder_accepts_alnum_underscore_within_limit() {
        assert!(validate_placeholder_name("a").is_ok());
        assert!(validate_placeholder_name("Foo_Bar_123").is_ok());
        assert!(validate_placeholder_name(&"a".repeat(64)).is_ok());
    }

    #[test]
    fn placeholder_rejects_empty_too_long_and_illegal_chars() {
        assert!(validate_placeholder_name("").is_err());
        assert!(validate_placeholder_name(&"a".repeat(65)).is_err());
        assert!(validate_placeholder_name("foo-bar").is_err());
        assert!(validate_placeholder_name("foo.bar").is_err());
    }

    #[test]
    fn stdout_tail_constant_is_64k() {
        assert_eq!(STDOUT_TAIL_MAX_BYTES, 64 * 1024);
    }
}
