//! Tag-list validation shared across project creation and update.
//!
//! FR-003: max 20 tags, each trimmed non-empty ≤ 128 chars, no duplicates.

use std::collections::HashSet;

/// Errors produced by [`validate_tags`].
#[derive(Debug, thiserror::Error, PartialEq)]
pub enum TagsError {
    #[error("too many tags (max 20)")]
    TooMany,
    #[error("empty tag not allowed")]
    Empty,
    #[error("tag too long (max 128 chars)")]
    TooLong,
    #[error("duplicate tag: {0}")]
    Duplicate(String),
}

/// Validate a tag list (FR-003).
///
/// Rules (applied in order):
/// 1. `tags.len() > 20` → [`TagsError::TooMany`]
/// 2. For each tag after trimming whitespace:
///    - `trimmed.is_empty()` → [`TagsError::Empty`]
///    - `trimmed.len() > 128` → [`TagsError::TooLong`]
/// 3. Case-sensitive duplicate check on trimmed values → [`TagsError::Duplicate`]
pub fn validate_tags(tags: &[String]) -> Result<(), TagsError> {
    if tags.len() > 20 {
        return Err(TagsError::TooMany);
    }
    let mut seen = HashSet::new();
    for tag in tags {
        let trimmed = tag.trim();
        if trimmed.is_empty() {
            return Err(TagsError::Empty);
        }
        if trimmed.len() > 128 {
            return Err(TagsError::TooLong);
        }
        if !seen.insert(trimmed.to_string()) {
            return Err(TagsError::Duplicate(trimmed.to_string()));
        }
    }
    Ok(())
}
