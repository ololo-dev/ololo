//! Validation helpers for judge-produced ratings and feedback.
//!
//! `validate_rating_output` checks a parsed rating against the effective
//! rating scale (range, step quantization, magnitude cap, i32 safety).
//! `validate_feedback` enforces the maximum feedback length.

/// A judge rating scale: `{min, max, step}`.
///
/// Constraints (enforced on write paths by `validation::judges`): `min < max`,
/// `step > 0`, and `(max - min) / step` is integer within 1e-6 epsilon.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RatingScale {
    pub min: f64,
    pub max: f64,
    pub step: f64,
}

/// Errors produced by [`validate_rating_output`] and [`validate_feedback`].
#[derive(Debug, thiserror::Error, PartialEq)]
pub enum ValidationError {
    #[error("rating {value} is outside scale range [{min}, {max}]")]
    OutOfRange { value: f64, min: f64, max: f64 },
    #[error("rating {value} is not on step {step} from min {min}")]
    NotQuantized { value: f64, step: f64, min: f64 },
    #[error("rating {value} exceeds magnitude cap (|v| < 1_000_000)")]
    RatingTooLarge { value: f64 },
    #[error("rating {value} does not fit in i32")]
    I32Overflow { value: f64 },
    #[error("feedback exceeds 10_000 characters")]
    FeedbackTooLong,
}

const EPS: f64 = 1e-6;
const MAGNITUDE_CAP: f64 = 1_000_000.0;
pub const MAX_FEEDBACK_LEN: usize = 10_000;

/// Pull an out-of-range judge rating to the nearest scale bound: -50 on
/// [-20, 0] becomes -20. Applied at every verdict entry point (model,
/// review revision, decide program, session-scope pass) BEFORE
/// [`validate_rating_output`], so magnitude alone can no longer fail — and
/// re-run — a judge whose answer was otherwise sound. The scale is the
/// per-task cap; asking for more than the cap means the cap. NaN is left
/// alone for the validator to reject.
pub fn clamp_rating_to_scale(value: f64, scale: &RatingScale) -> f64 {
    if value.is_nan() {
        return value;
    }
    let clamped = value.clamp(scale.min, scale.max);
    if (clamped - value).abs() > EPS {
        tracing::info!(
            rating = value,
            min = scale.min,
            max = scale.max,
            "judge rating clamped to scale"
        );
    }
    clamped
}

/// Validate a parsed rating against the effective [`RatingScale`] and convert
/// it to a step-quantized `i32`.
///
/// Checks (applied in order):
/// 1. `value` is within `[scale.min, scale.max]` inclusive (±`EPS`).
/// 2. `(value - scale.min)` is divisible by `scale.step` (±`EPS`).
/// 3. `|value| < 1_000_000` (i32-safe magnitude cap).
/// 4. `value.round()` fits in `i32` (defense-in-depth backstop).
///
/// Returns `Ok(quantized_i32)` on success.
pub fn validate_rating_output(value: f64, scale: &RatingScale) -> Result<i32, ValidationError> {
    if value < scale.min - EPS || value > scale.max + EPS {
        return Err(ValidationError::OutOfRange {
            value,
            min: scale.min,
            max: scale.max,
        });
    }

    let offset = value - scale.min;
    let steps = offset / scale.step;
    if (steps - steps.round()).abs() >= EPS {
        return Err(ValidationError::NotQuantized {
            value,
            step: scale.step,
            min: scale.min,
        });
    }

    if value.abs() >= MAGNITUDE_CAP {
        return Err(ValidationError::RatingTooLarge { value });
    }

    let rounded = value.round();
    let as_i64 = rounded as i64;
    let quantized = i32::try_from(as_i64).map_err(|_| ValidationError::I32Overflow { value })?;

    Ok(quantized)
}

/// Validate judge feedback text.
///
/// Empty string is allowed. Strings longer than 10_000 characters are rejected.
pub fn validate_feedback(s: &str) -> Result<(), ValidationError> {
    if s.len() > MAX_FEEDBACK_LEN {
        return Err(ValidationError::FeedbackTooLong);
    }
    Ok(())
}
