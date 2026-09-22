//! From jscpd's 0..100 score to what the game shows and pays.

use serde::{Deserialize, Serialize};

use crate::config::Thresholds;

/// The colour of a score under a [`Thresholds`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Level {
    Green,
    Amber,
    Red,
    /// No score: the tree had nothing jscpd could measure (no code files).
    Unknown,
}

/// Colour of `score`; `None` (and NaN, which jscpd never emits) is
/// [`Level::Unknown`].
pub fn level(score: Option<f64>, thresholds: &Thresholds) -> Level {
    match score {
        Some(s) if s.is_nan() => Level::Unknown,
        Some(s) if s >= thresholds.green_min => Level::Green,
        Some(s) if s >= thresholds.amber_min => Level::Amber,
        Some(_) => Level::Red,
        None => Level::Unknown,
    }
}

/// Whether two scores of one tree agree: within `tolerance` of each other,
/// or both absent (nothing to score on either side). A score against no
/// score is a disagreement — one side saw code the other did not.
pub fn scores_match(a: Option<f64>, b: Option<f64>, tolerance: f64) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(a), Some(b)) => (a - b).abs() <= tolerance,
        _ => false,
    }
}

/// The health bonus of one task.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Bonus {
    /// Points to award; never negative.
    pub points: i32,
    /// The 0..1 factor the score mapped to.
    pub factor: f64,
    pub reason: BonusReason,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BonusReason {
    /// A score was mapped through the thresholds.
    Scored,
    /// The tree had no score (nothing to measure).
    NoScore,
    /// The task pays no health points.
    NoWeight,
}

/// Map a score to a bonus: a linear ramp from 0 at `amber_min` to 1 at
/// `green_min`, clamped — red earns nothing, green earns the full `weight`,
/// amber a proportional share. Rounded half away from zero to whole points.
pub fn bonus(score: Option<f64>, thresholds: &Thresholds, weight: i32) -> Bonus {
    if weight <= 0 {
        return Bonus {
            points: 0,
            factor: 0.0,
            reason: BonusReason::NoWeight,
        };
    }
    let Some(score) = score.filter(|s| s.is_finite()) else {
        return Bonus {
            points: 0,
            factor: 0.0,
            reason: BonusReason::NoScore,
        };
    };
    let span = thresholds.green_min - thresholds.amber_min;
    let factor = if span > 0.0 {
        ((score - thresholds.amber_min) / span).clamp(0.0, 1.0)
    } else {
        // Degenerate thresholds (validation forbids them): a plain step.
        f64::from(u8::from(score >= thresholds.green_min))
    };
    let points = (factor * f64::from(weight)).round() as i32;
    Bonus {
        points: points.max(0),
        factor,
        reason: BonusReason::Scored,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const T: Thresholds = Thresholds {
        green_min: 70.0,
        amber_min: 55.0,
    };

    #[test]
    fn level_boundaries_are_inclusive_at_the_bottom_of_each_band() {
        assert_eq!(level(Some(100.0), &T), Level::Green);
        assert_eq!(level(Some(70.0), &T), Level::Green);
        assert_eq!(level(Some(69.9), &T), Level::Amber);
        assert_eq!(level(Some(55.0), &T), Level::Amber);
        assert_eq!(level(Some(54.9), &T), Level::Red);
        assert_eq!(level(Some(0.0), &T), Level::Red);
        assert_eq!(level(None, &T), Level::Unknown);
        assert_eq!(level(Some(f64::NAN), &T), Level::Unknown);
    }

    #[test]
    fn scores_match_within_tolerance_only() {
        assert!(scores_match(Some(74.3), Some(74.3), 0.0));
        assert!(scores_match(Some(74.3), Some(74.34), 0.05));
        assert!(!scores_match(Some(74.3), Some(74.4), 0.05));
        assert!(scores_match(None, None, 0.05));
        assert!(!scores_match(None, Some(74.3), 0.05));
        assert!(!scores_match(Some(74.3), None, 100.0));
    }

    #[test]
    fn bonus_ramps_linearly_between_amber_and_green() {
        assert_eq!(
            bonus(Some(80.0), &T, 20),
            Bonus {
                points: 20,
                factor: 1.0,
                reason: BonusReason::Scored
            }
        );
        assert_eq!(
            bonus(Some(70.0), &T, 20),
            Bonus {
                points: 20,
                factor: 1.0,
                reason: BonusReason::Scored
            }
        );
        assert_eq!(bonus(Some(55.0), &T, 20).points, 0);
        assert_eq!(bonus(Some(40.0), &T, 20).points, 0);
        let mid = bonus(Some(62.5), &T, 20);
        assert_eq!(mid.points, 10);
        assert!((mid.factor - 0.5).abs() < 1e-9);
        // 58 → factor 0.2 → 4 of 20; 66.25 → 0.75 → 15.
        assert_eq!(bonus(Some(58.0), &T, 20).points, 4);
        assert_eq!(bonus(Some(66.25), &T, 20).points, 15);
        // Rounding is to the nearest whole point (0.5 rounds away from zero).
        assert_eq!(bonus(Some(62.5), &T, 3).points, 2);
    }

    #[test]
    fn bonus_without_score_or_weight_is_zero_with_the_reason() {
        assert_eq!(
            bonus(None, &T, 20),
            Bonus {
                points: 0,
                factor: 0.0,
                reason: BonusReason::NoScore
            }
        );
        assert_eq!(bonus(Some(f64::NAN), &T, 20).reason, BonusReason::NoScore);
        assert_eq!(
            bonus(Some(100.0), &T, 0),
            Bonus {
                points: 0,
                factor: 0.0,
                reason: BonusReason::NoWeight
            }
        );
        assert_eq!(bonus(Some(100.0), &T, -5).reason, BonusReason::NoWeight);
    }
}
