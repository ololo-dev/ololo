//! Validation helpers for the canonical wire protocol.
//!
//! Per FR-020/FR-021 and the contract `## Substitution` section, payload
//! shape is enforced with `serde(deny_unknown_fields)` (in
//! `crate::protocol`) and *semantic* invariants are enforced here:
//! placeholder declaration vs reference parity, range checks on
//! `matchers`/`backoff`, and lightweight kind/command compatibility.
//!
//! WP-015 scope: `TestTemplate` only. Other protocol structs are leaves
//! whose serde derives already pin them to safe shapes.

pub mod judge_results;
pub mod judges;
pub mod session_duration;
pub mod tags;
pub mod test_template;
pub mod username;

pub use session_duration::{SessionDurationError, validate_session_duration};
pub use tags::{TagsError, validate_tags};
pub use test_template::{TemplateError, extract_referenced_placeholders, validate_template};
pub use username::{UsernameError, validate_username};
