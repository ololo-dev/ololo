//! Project session-duration validation.
//!
//! A project's session duration (seconds) must lie within
//! [`SESSION_DURATION_MIN_SECS`]..=[`SESSION_DURATION_MAX_SECS`],
//! bounds inclusive.

/// Minimum allowed session duration in seconds (inclusive).
pub const SESSION_DURATION_MIN_SECS: u64 = 60;

/// Maximum allowed session duration in seconds (inclusive).
pub const SESSION_DURATION_MAX_SECS: u64 = 86_400;

/// Errors produced by [`validate_session_duration`].
#[derive(Debug, thiserror::Error, PartialEq)]
pub enum SessionDurationError {
    #[error("session duration too short (min 60 seconds, got {0})")]
    TooShort(u64),
    #[error("session duration too long (max 86400 seconds, got {0})")]
    TooLong(u64),
}

/// Validate a project's session duration in seconds.
///
/// Rules (applied in order):
/// 1. `secs < 60` → [`SessionDurationError::TooShort`]
/// 2. `secs > 86_400` → [`SessionDurationError::TooLong`]
pub fn validate_session_duration(secs: u64) -> Result<(), SessionDurationError> {
    if secs < SESSION_DURATION_MIN_SECS {
        return Err(SessionDurationError::TooShort(secs));
    }
    if secs > SESSION_DURATION_MAX_SECS {
        return Err(SessionDurationError::TooLong(secs));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rejects_below_minimum() {
        assert_eq!(
            validate_session_duration(59),
            Err(SessionDurationError::TooShort(59))
        );
    }

    #[test]
    fn test_rejects_above_maximum() {
        assert_eq!(
            validate_session_duration(86_401),
            Err(SessionDurationError::TooLong(86_401))
        );
    }

    #[test]
    fn test_accepts_minimum() {
        assert!(validate_session_duration(60).is_ok());
    }

    #[test]
    fn test_accepts_maximum() {
        assert!(validate_session_duration(86_400).is_ok());
    }

    #[test]
    fn test_accepts_mid_value() {
        assert!(validate_session_duration(3_600).is_ok());
    }
}
