//! `TestTemplate` semantic validation per FR-020, FR-021, FR-029.
//!
//! Wave-3 deviation: the `regex` crate is not in the server dependency
//! tree and was deliberately not added (see Floor rule 1). We therefore
//! do *not* compile `matchers.stdout_regex` here. Instead a size-only
//! sanity check (non-empty, ≤ 1024 bytes) is applied; full pattern
//! validation happens on the observer side where the regex engine is
//! already linked. Documented in the WP-015 report.

use std::collections::BTreeSet;

use crate::protocol::validate_placeholder_name;
use crate::task_template::{Backoff, Matchers, TestKind, TestTemplate};

/// Maximum bytes in a `command_template`. Mirrors the soft cap in
/// `STDOUT_TAIL_MAX_BYTES`-adjacent reasoning: 8 KiB is comfortably
/// larger than any reasonable shell one-liner while bounding payloads.
pub const COMMAND_TEMPLATE_MAX_BYTES: usize = 8192;

/// Maximum bytes in a `stdout_regex` source string under the no-regex
/// fallback (see module docs).
pub const STDOUT_REGEX_MAX_BYTES: usize = 1024;

const MAX_DURATION_MS_HI: u64 = 600_000;
const MAX_PARAM_TIMEOUT_MS_HI: u64 = 600_000;
const BACKOFF_MULTIPLIER_LO: f64 = 1.0;
const BACKOFF_MULTIPLIER_HI: f64 = 10.0;
const BACKOFF_ATTEMPTS_HI: u32 = 64;

const HTTP_METHOD_PREFIXES: &[&str] = &[
    "GET ", "POST ", "PUT ", "DELETE ", "PATCH ", "HEAD ", "OPTIONS ",
];

/// Shell metacharacters disallowed inside a `FileExists` command.
const FILE_EXISTS_METACHARS: &[u8] = b";|&<>$`";

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum TemplateError {
    #[error("placeholder name '{0}' is invalid (must match ^[A-Za-z0-9_]{{1,64}}$)")]
    InvalidPlaceholderName(String),
    #[error("placeholder '{0}' declared but not referenced in command_template")]
    UnreferencedPlaceholder(String),
    #[error("placeholder '{0}' referenced in command_template but not declared")]
    UndeclaredPlaceholder(String),
    #[error("duplicate placeholder declaration: {0}")]
    DuplicatePlaceholder(String),
    #[error("command_template is empty")]
    EmptyCommand,
    #[error("command_template exceeds 8192 bytes")]
    CommandTooLong,
    #[error("matchers.max_duration_ms must be in 1..=600_000 (got {0})")]
    InvalidMaxDuration(u64),
    #[error("matchers.param_timeout_ms must be in 1..=600_000 (got {0})")]
    InvalidParamTimeout(u64),
    #[error("backoff.initial_ms must be > 0")]
    InvalidBackoffInitial,
    #[error("backoff.max_ms must be >= initial_ms")]
    InvalidBackoffMax,
    #[error("backoff.multiplier must be in 1.0..=10.0 (got {0})")]
    InvalidBackoffMultiplier(f64),
    #[error("backoff.max_attempts must be in 1..=64 (got {0})")]
    InvalidBackoffAttempts(u32),
    #[error("stdout_regex is invalid: {0}")]
    InvalidRegex(String),
    #[error("test kind requires command starting with prefix '{prefix}' for kind {kind:?}")]
    KindCommandMismatch {
        kind: TestKind,
        prefix: &'static str,
    },
}

/// Validate every static invariant declared by FR-020 / FR-021 for the
/// given `TestTemplate`. Returns the first error encountered; callers
/// should expose the message verbatim — it is suitable for HTTP 422.
pub fn validate_template(t: &TestTemplate) -> Result<(), TemplateError> {
    validate_command_size(&t.command_template)?;
    let declared = validate_placeholder_decls(&t.placeholders)?;
    let referenced = extract_referenced_placeholders(&t.command_template);
    validate_placeholder_parity(&declared, &referenced)?;
    validate_matchers(&t.matchers)?;
    validate_backoff(&t.backoff)?;
    validate_kind_command(t.kind, &t.command_template)?;
    Ok(())
}

fn validate_command_size(cmd: &str) -> Result<(), TemplateError> {
    if cmd.is_empty() {
        return Err(TemplateError::EmptyCommand);
    }
    if cmd.len() > COMMAND_TEMPLATE_MAX_BYTES {
        return Err(TemplateError::CommandTooLong);
    }
    Ok(())
}

fn validate_placeholder_decls(
    placeholders: &[crate::task_template::Placeholder],
) -> Result<BTreeSet<String>, TemplateError> {
    let mut declared: BTreeSet<String> = BTreeSet::new();
    for p in placeholders {
        if validate_placeholder_name(&p.name).is_err() {
            return Err(TemplateError::InvalidPlaceholderName(p.name.clone()));
        }
        if !declared.insert(p.name.clone()) {
            return Err(TemplateError::DuplicatePlaceholder(p.name.clone()));
        }
    }
    Ok(declared)
}

fn validate_placeholder_parity(
    declared: &BTreeSet<String>,
    referenced: &BTreeSet<String>,
) -> Result<(), TemplateError> {
    if let Some(missing) = declared.difference(referenced).next() {
        return Err(TemplateError::UnreferencedPlaceholder(missing.clone()));
    }
    if let Some(missing) = referenced.difference(declared).next() {
        return Err(TemplateError::UndeclaredPlaceholder(missing.clone()));
    }
    Ok(())
}

fn validate_matchers(m: &Matchers) -> Result<(), TemplateError> {
    if m.max_duration_ms == 0 || m.max_duration_ms > MAX_DURATION_MS_HI {
        return Err(TemplateError::InvalidMaxDuration(m.max_duration_ms));
    }
    if m.param_timeout_ms == 0 || m.param_timeout_ms > MAX_PARAM_TIMEOUT_MS_HI {
        return Err(TemplateError::InvalidParamTimeout(m.param_timeout_ms));
    }
    if let Some(rx) = &m.stdout_regex {
        // Size-only fallback: see module-level deviation note.
        if rx.is_empty() {
            return Err(TemplateError::InvalidRegex("empty pattern".to_string()));
        }
        if rx.len() > STDOUT_REGEX_MAX_BYTES {
            return Err(TemplateError::InvalidRegex(format!(
                "pattern exceeds {STDOUT_REGEX_MAX_BYTES} bytes"
            )));
        }
    }
    Ok(())
}

fn validate_backoff(b: &Backoff) -> Result<(), TemplateError> {
    if b.initial_ms == 0 {
        return Err(TemplateError::InvalidBackoffInitial);
    }
    if b.max_ms < b.initial_ms {
        return Err(TemplateError::InvalidBackoffMax);
    }
    if !(BACKOFF_MULTIPLIER_LO..=BACKOFF_MULTIPLIER_HI).contains(&b.multiplier)
        || !b.multiplier.is_finite()
    {
        return Err(TemplateError::InvalidBackoffMultiplier(b.multiplier));
    }
    if b.max_attempts == 0 || b.max_attempts > BACKOFF_ATTEMPTS_HI {
        return Err(TemplateError::InvalidBackoffAttempts(b.max_attempts));
    }
    Ok(())
}

fn validate_kind_command(kind: TestKind, cmd: &str) -> Result<(), TemplateError> {
    match kind {
        TestKind::Shell => Ok(()),
        TestKind::HttpRequest => {
            if HTTP_METHOD_PREFIXES.iter().any(|p| cmd.starts_with(p)) {
                Ok(())
            } else {
                Err(TemplateError::KindCommandMismatch {
                    kind,
                    prefix: "<METHOD> ",
                })
            }
        }
        TestKind::FileExists => {
            let bytes = cmd.as_bytes();
            let has_ws = bytes.iter().any(|b| b.is_ascii_whitespace());
            let has_meta = bytes.iter().any(|b| FILE_EXISTS_METACHARS.contains(b));
            if has_ws || has_meta {
                Err(TemplateError::KindCommandMismatch {
                    kind,
                    prefix: "<path>",
                })
            } else {
                Ok(())
            }
        }
    }
}

/// Extract the set of `{name}` placeholder references from `command`.
///
/// Rules:
/// - `{NAME}` where `NAME` matches `^[A-Za-z0-9_]{1,64}$` adds `NAME`.
/// - `{{` is the literal-`{` escape and is consumed as two bytes; the
///   following byte is *not* re-scanned as the start of a placeholder.
/// - Unmatched / empty / over-long / invalid-charset braces are
///   silently ignored at this layer; downstream parity check surfaces
///   them as `UndeclaredPlaceholder` mismatches if the user intended
///   one.
pub fn extract_referenced_placeholders(command: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let bytes = command.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b == b'{' {
            // Escaped `{{` -> consume both, do not start a placeholder.
            if i + 1 < bytes.len() && bytes[i + 1] == b'{' {
                i += 2;
                continue;
            }
            // Find matching `}` within 64 bytes (placeholder name cap).
            let start = i + 1;
            let mut end = None;
            let mut j = start;
            let limit = (start + 64).min(bytes.len());
            while j < limit {
                if bytes[j] == b'}' {
                    end = Some(j);
                    break;
                }
                j += 1;
            }
            if let Some(e) = end {
                let name = &bytes[start..e];
                if !name.is_empty() && name.iter().all(|c| c.is_ascii_alphanumeric() || *c == b'_')
                {
                    // Safe: pre-validated as ASCII subset.
                    if let Ok(s) = std::str::from_utf8(name) {
                        out.insert(s.to_string());
                    }
                }
                i = e + 1;
                continue;
            }
            // No closing brace within the cap — drop the `{` and move on.
            i += 1;
        } else {
            i += 1;
        }
    }
    out
}
