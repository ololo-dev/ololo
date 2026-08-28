//! Shared intervals wire types for project and task CRUD.
//!
//! `IntervalsReq` is the input shape (per-field `Option`; `None` means
//! "inherit project default" on create, "clear to NULL" on patch).
//! `IntervalsResp` is the output shape (resolved values).
//! `ExportIntervals` is the project-level export shape (non-null).
//! `ExportTaskIntervals` is the task-level export shape (nullable, preserves NULLs).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[serde(deny_unknown_fields)]
pub struct IntervalsReq {
    #[serde(default)]
    pub deadline_secs: Option<i64>,
    #[serde(default)]
    pub min_interval_secs: Option<i32>,
    #[serde(default)]
    pub interval_increment_secs: Option<i32>,
    #[serde(default)]
    pub max_interval_secs: Option<i32>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct IntervalsResp {
    pub deadline_secs: i64,
    pub min_interval_secs: i32,
    pub interval_increment_secs: i32,
    pub max_interval_secs: i32,
}

pub type ResolvedIntervals = IntervalsResp;

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExportIntervals {
    pub deadline_secs: i64,
    pub min_interval_secs: i32,
    pub interval_increment_secs: i32,
    pub max_interval_secs: i32,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[serde(deny_unknown_fields)]
pub struct ExportTaskIntervals {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deadline_secs: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_interval_secs: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interval_increment_secs: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_interval_secs: Option<i32>,
}

/// Validation error for resolved intervals.
#[derive(Debug, thiserror::Error)]
pub enum IntervalValidationError {
    #[error("deadline_secs must be > 0")]
    DeadlineTooSmall,
    #[error("min_interval_secs must be >= 0")]
    MinNegative,
    #[error("interval_increment_secs must be > 0")]
    IncrementTooSmall,
    #[error("max_interval_secs must be >= 0")]
    MaxNegative,
    #[error("min_interval_secs ({min}) must be <= max_interval_secs ({max})")]
    MinExceedsMax { min: i32, max: i32 },
    #[error("interval_increment_secs ({increment}) must be <= (max - min) = {span}")]
    IncrementExceedsSpan { increment: i32, span: i32 },
}

/// Validate a resolved intervals tuple. Called at write-time where project
/// context is available to produce the resolved tuple before validation.
pub fn validate_resolved_intervals(r: &IntervalsResp) -> Result<(), IntervalValidationError> {
    if r.deadline_secs < 1 {
        return Err(IntervalValidationError::DeadlineTooSmall);
    }
    if r.min_interval_secs < 0 {
        return Err(IntervalValidationError::MinNegative);
    }
    if r.interval_increment_secs < 1 {
        return Err(IntervalValidationError::IncrementTooSmall);
    }
    if r.max_interval_secs < 0 {
        return Err(IntervalValidationError::MaxNegative);
    }
    if r.min_interval_secs > r.max_interval_secs {
        return Err(IntervalValidationError::MinExceedsMax {
            min: r.min_interval_secs,
            max: r.max_interval_secs,
        });
    }
    let span = r.max_interval_secs - r.min_interval_secs;
    if r.interval_increment_secs > span {
        return Err(IntervalValidationError::IncrementExceedsSpan {
            increment: r.interval_increment_secs,
            span,
        });
    }
    Ok(())
}

/// Resolve task interval fields against project defaults, filling `None`
/// from the project's default values. Then apply a defensive clamp that
/// enforces all write-time invariants on the output.
///
/// ponytail: project defaults cached for session lifetime; mid-session
/// PATCH /api/projects does not invalidate running sessions. Acceptable —
/// probe cadence changes mid-run are cosmetic. This clamp bounds the damage
/// if defaults drift to a broken state. Upgrade path: broadcast invalidate
/// on project PATCH if this ever bites.
pub fn resolve_intervals(
    task_deadline: Option<i64>,
    task_min: Option<i32>,
    task_increment: Option<i32>,
    task_max: Option<i32>,
    proj: &ExportIntervals,
) -> ResolvedIntervals {
    let deadline_secs = task_deadline.unwrap_or(proj.deadline_secs).max(1);
    let min_interval_secs = task_min.unwrap_or(proj.min_interval_secs).max(0);
    let interval_increment_secs = task_increment
        .unwrap_or(proj.interval_increment_secs)
        .max(1);
    let max_interval_secs = task_max.unwrap_or(proj.max_interval_secs).max(0);

    // Defensive clamp: enforce all write-time invariants.
    let min_interval_secs = min_interval_secs.min(max_interval_secs);
    let max_interval_secs = max_interval_secs.max(min_interval_secs);
    let span = (max_interval_secs - min_interval_secs).max(0);
    let interval_increment_secs = interval_increment_secs.min(span.max(1));

    IntervalsResp {
        deadline_secs,
        min_interval_secs,
        interval_increment_secs,
        max_interval_secs,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn proj() -> ExportIntervals {
        ExportIntervals {
            deadline_secs: 60,
            min_interval_secs: 5,
            interval_increment_secs: 5,
            max_interval_secs: 60,
        }
    }

    // --- Inheritance tests (4 cases) ---

    #[test]
    fn all_override_task_wins() {
        let r = resolve_intervals(Some(120), Some(10), Some(3), Some(30), &proj());
        assert_eq!(r.deadline_secs, 120);
        assert_eq!(r.min_interval_secs, 10);
        assert_eq!(r.interval_increment_secs, 3);
        assert_eq!(r.max_interval_secs, 30);
    }

    #[test]
    fn all_inherit_project_defaults() {
        let r = resolve_intervals(None, None, None, None, &proj());
        assert_eq!(r.deadline_secs, 60);
        assert_eq!(r.min_interval_secs, 5);
        assert_eq!(r.interval_increment_secs, 5);
        assert_eq!(r.max_interval_secs, 60);
    }

    #[test]
    fn partial_override_mixed() {
        let r = resolve_intervals(Some(600), None, None, None, &proj());
        assert_eq!(r.deadline_secs, 600);
        assert_eq!(r.min_interval_secs, 5); // inherited
        assert_eq!(r.interval_increment_secs, 5); // inherited
        assert_eq!(r.max_interval_secs, 60); // inherited
    }

    #[test]
    fn boundary_deadline_zero_floored() {
        let r = resolve_intervals(Some(0), None, None, None, &proj());
        assert_eq!(r.deadline_secs, 1); // clamped to >= 1
    }

    // --- Validation/clamp tests (4+ cases) ---

    #[test]
    fn validate_rejects_min_exceeds_max() {
        let bad = IntervalsResp {
            deadline_secs: 60,
            min_interval_secs: 50,
            interval_increment_secs: 5,
            max_interval_secs: 10,
        };
        assert!(matches!(
            validate_resolved_intervals(&bad),
            Err(IntervalValidationError::MinExceedsMax { .. })
        ));
    }

    #[test]
    fn validate_rejects_increment_zero() {
        let bad = IntervalsResp {
            deadline_secs: 60,
            min_interval_secs: 5,
            interval_increment_secs: 0,
            max_interval_secs: 60,
        };
        assert!(matches!(
            validate_resolved_intervals(&bad),
            Err(IntervalValidationError::IncrementTooSmall)
        ));
    }

    #[test]
    fn validate_rejects_increment_exceeds_span() {
        let bad = IntervalsResp {
            deadline_secs: 60,
            min_interval_secs: 10,
            interval_increment_secs: 100,
            max_interval_secs: 12,
        };
        assert!(matches!(
            validate_resolved_intervals(&bad),
            Err(IntervalValidationError::IncrementExceedsSpan { .. })
        ));
    }

    #[test]
    fn clamp_resolves_inherited_min_greater_than_set_max() {
        // Task sets max=3, min inherits project default 5 → resolved min=5 > max=3.
        // clamp should pin min = max = 3.
        let r = resolve_intervals(None, None, None, Some(3), &proj());
        assert_eq!(r.min_interval_secs, 3);
        assert_eq!(r.max_interval_secs, 3);
    }

    #[test]
    fn baseline_defaults_pass_validation() {
        let baseline = IntervalsResp {
            deadline_secs: 60,
            min_interval_secs: 5,
            interval_increment_secs: 5,
            max_interval_secs: 60,
        };
        assert!(validate_resolved_intervals(&baseline).is_ok());
    }
}
