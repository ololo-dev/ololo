//! Operator settings of the code-health checkpoints (`app_settings` rows).
//!
//! The game server reads them to decide whether to ask agents for health
//! reports and how to verify and pay; the web server reads the thresholds
//! to colour the chart. The scan configuration itself is not a setting —
//! it is `ololo_health::HealthConfig::default()` on every side, so the
//! numbers stay comparable.

use std::time::Duration;

use ololo_health::Thresholds;
use sea_orm::{ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter};

use crate::entities::app_settings;

/// Master switch. Health is OFF unless this is explicitly `"true"`: no
/// per-probe commits, no reports, no verification, no bonus — the game
/// behaves exactly as before the feature existed.
pub const HEALTH_ENABLED_KEY: &str = "health_enabled";
/// Wall-clock budget of one analysis, client and server alike, in seconds.
pub const HEALTH_TIMEOUT_SECS_KEY: &str = "health_timeout_secs";
/// Score from which a checkpoint is green.
pub const HEALTH_GREEN_MIN_KEY: &str = "health_green_min";
/// Score from which a checkpoint is amber (below: red).
pub const HEALTH_AMBER_MIN_KEY: &str = "health_amber_min";
/// How far the client's score may sit from the server's before the
/// checkpoint is flagged `score_mismatch`.
pub const HEALTH_MISMATCH_TOLERANCE_KEY: &str = "health_mismatch_tolerance";

pub const SETTING_KEYS: &[&str] = &[
    HEALTH_ENABLED_KEY,
    HEALTH_TIMEOUT_SECS_KEY,
    HEALTH_GREEN_MIN_KEY,
    HEALTH_AMBER_MIN_KEY,
    HEALTH_MISMATCH_TOLERANCE_KEY,
];

pub const DEFAULT_TIMEOUT_SECS: u32 = 30;
pub const MIN_TIMEOUT_SECS: u32 = 5;
pub const MAX_TIMEOUT_SECS: u32 = 300;

/// The settings as read at runtime, defaults filled in.
#[derive(Debug, Clone, PartialEq)]
pub struct HealthSettings {
    pub enabled: bool,
    pub timeout: Duration,
    pub thresholds: Thresholds,
    pub tolerance: f64,
}

impl Default for HealthSettings {
    fn default() -> Self {
        let scan = ololo_health::HealthConfig::default();
        Self {
            enabled: false,
            timeout: Duration::from_secs(u64::from(DEFAULT_TIMEOUT_SECS)),
            thresholds: scan.thresholds,
            tolerance: scan.tolerance,
        }
    }
}

impl HealthSettings {
    /// The scan configuration for this deployment: the shared defaults with
    /// the operator's budget and thresholds laid over.
    pub fn scan_config(&self) -> ololo_health::HealthConfig {
        ololo_health::HealthConfig {
            timeout: self.timeout,
            thresholds: self.thresholds,
            tolerance: self.tolerance,
            ..ololo_health::HealthConfig::default()
        }
    }

    /// Build from raw `key → value` rows; anything absent, unparseable or
    /// out of range falls back to the default. Thresholds that do not
    /// validate as a pair (amber not below green) fall back together, so a
    /// half-edited pair never colours the chart inside out.
    pub fn from_values<'a>(mut get: impl FnMut(&str) -> Option<&'a str>) -> Self {
        let defaults = HealthSettings::default();
        let enabled = get(HEALTH_ENABLED_KEY).is_some_and(|v| v.trim() == "true");
        let timeout = get(HEALTH_TIMEOUT_SECS_KEY)
            .and_then(|v| v.trim().parse::<u32>().ok())
            .filter(|n| (MIN_TIMEOUT_SECS..=MAX_TIMEOUT_SECS).contains(n))
            .map(|n| Duration::from_secs(u64::from(n)))
            .unwrap_or(defaults.timeout);
        let score = |key: &str, get: &mut dyn FnMut(&str) -> Option<&'a str>| {
            get(key)
                .and_then(|v| v.trim().parse::<f64>().ok())
                .filter(|n| n.is_finite() && (0.0..=100.0).contains(n))
        };
        let green = score(HEALTH_GREEN_MIN_KEY, &mut get);
        let amber = score(HEALTH_AMBER_MIN_KEY, &mut get);
        let thresholds = Thresholds {
            green_min: green.unwrap_or(defaults.thresholds.green_min),
            amber_min: amber.unwrap_or(defaults.thresholds.amber_min),
        };
        let thresholds = if thresholds.validate().is_ok() {
            thresholds
        } else {
            defaults.thresholds
        };
        let tolerance = get(HEALTH_MISMATCH_TOLERANCE_KEY)
            .and_then(|v| v.trim().parse::<f64>().ok())
            .filter(|n| n.is_finite() && (0.0..=100.0).contains(n))
            .unwrap_or(defaults.tolerance);
        HealthSettings {
            enabled,
            timeout,
            thresholds,
            tolerance,
        }
    }

    /// Read every health key in one query.
    pub async fn load<C: ConnectionTrait>(db: &C) -> Result<Self, sea_orm::DbErr> {
        let rows = app_settings::Entity::find()
            .filter(app_settings::Column::Key.is_in(SETTING_KEYS.iter().copied()))
            .all(db)
            .await?;
        Ok(Self::from_values(|key| {
            rows.iter().find(|r| r.key == key).map(|r| r.value.as_str())
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn settings(pairs: &[(&str, &str)]) -> HealthSettings {
        let map: HashMap<&str, &str> = pairs.iter().copied().collect();
        HealthSettings::from_values(|k| map.get(k).copied())
    }

    #[test]
    fn absent_rows_are_the_defaults_with_health_off() {
        let s = settings(&[]);
        assert_eq!(s, HealthSettings::default());
        assert!(!s.enabled);
        assert_eq!(s.timeout, Duration::from_secs(30));
        assert_eq!(s.thresholds, Thresholds::default());
        assert_eq!(s.tolerance, 0.05);
    }

    #[test]
    fn values_are_read_and_bounded() {
        let s = settings(&[
            (HEALTH_ENABLED_KEY, "true"),
            (HEALTH_TIMEOUT_SECS_KEY, " 45 "),
            (HEALTH_GREEN_MIN_KEY, "80"),
            (HEALTH_AMBER_MIN_KEY, "50.5"),
            (HEALTH_MISMATCH_TOLERANCE_KEY, "0.2"),
        ]);
        assert!(s.enabled);
        assert_eq!(s.timeout, Duration::from_secs(45));
        assert_eq!(
            (s.thresholds.green_min, s.thresholds.amber_min),
            (80.0, 50.5)
        );
        assert_eq!(s.tolerance, 0.2);
        assert_eq!(s.scan_config().timeout, Duration::from_secs(45));
        assert_eq!(s.scan_config().thresholds, s.thresholds);

        // Out of range or unparseable → default for that key only.
        let s = settings(&[
            (HEALTH_ENABLED_KEY, "yes"),
            (HEALTH_TIMEOUT_SECS_KEY, "1"),
            (HEALTH_MISMATCH_TOLERANCE_KEY, "-1"),
        ]);
        assert!(!s.enabled);
        assert_eq!(s.timeout, Duration::from_secs(30));
        assert_eq!(s.tolerance, 0.05);
    }

    #[test]
    fn an_inverted_threshold_pair_falls_back_together() {
        let s = settings(&[(HEALTH_GREEN_MIN_KEY, "40"), (HEALTH_AMBER_MIN_KEY, "60")]);
        assert_eq!(s.thresholds, Thresholds::default());
        // One valid key on its own still applies.
        let s = settings(&[(HEALTH_GREEN_MIN_KEY, "90")]);
        assert_eq!(
            (s.thresholds.green_min, s.thresholds.amber_min),
            (90.0, 55.0)
        );
    }
}
