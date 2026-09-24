//! One score from the tree's dimensions and the test suite's.
//!
//! The server scores a commit's tree once, when it verifies it; the test
//! suite's numbers arrive from the client later, and a checkpoint whose
//! own run did not finish is scored with the last run that did. So the
//! composed score is computed where it is shown and paid — from the
//! dimensions the tree's result already carries plus the suite's external
//! metrics — with jscpd's own formula: a weighted geometric mean of the
//! sub-scores, each floored at 1, rounded to one decimal. Same dimensions
//! in, same number out as `cpd_core::health::compute` with those metrics
//! in its config (a test keeps the two in step).

use cpd_core::health::{Direction, ExternalMetric};
use serde::{Deserialize, Serialize};

use crate::Metrics;

/// One dimension as the mean weighs it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DimensionScore {
    pub id: String,
    pub weight: f64,
    pub score: f64,
}

/// A composed score.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Composed {
    /// `None` when the tree had nothing to score: a test suite alone never
    /// makes a health score.
    pub score: Option<f64>,
    pub grade: Option<char>,
    pub dimensions: Vec<DimensionScore>,
}

impl Composed {
    /// The sub-score of dimension `id`, when it is part of the score.
    pub fn sub_score(&self, id: &str) -> Option<f64> {
        self.dimensions.iter().find(|d| d.id == id).map(|d| d.score)
    }
}

fn round1(value: f64) -> f64 {
    (value * 10.0).round() / 10.0
}

/// The dimensions of a stored `HealthResult` (its JSON), read from jscpd's
/// `health.dimensions`. A result stored without them (cut down for size)
/// falls back to its score as one dimension weighing what its dimensions
/// would have — the composed score then matches jscpd's to within its
/// rounding. `None` when the result has no score: nothing to compose.
pub fn static_dimensions(result: &serde_json::Value) -> Option<Vec<DimensionScore>> {
    let score = result.get("score")?.as_f64()?;
    let dims: Option<Vec<DimensionScore>> = result
        .get("health")
        .and_then(|h| h.get("dimensions"))
        .and_then(|d| d.as_array())
        .map(|dims| {
            dims.iter()
                .filter_map(|d| {
                    Some(DimensionScore {
                        id: d.get("id")?.as_str()?.to_string(),
                        weight: d.get("weight")?.as_f64()?,
                        score: d.get("score")?.as_f64()?,
                    })
                })
                .collect()
        });
    match dims {
        Some(dims) if !dims.is_empty() => Some(dims),
        _ => {
            let metrics: Option<Metrics> = result
                .get("metrics")
                .and_then(|m| serde_json::from_value(m.clone()).ok());
            Some(vec![DimensionScore {
                id: "code".to_string(),
                weight: metrics.as_ref().map_or(1.0, static_weight),
                score,
            }])
        }
    }
}

/// What the built-in dimensions of a tree weigh together: duplication and
/// complexity one each when there is code, dead code the share of the code
/// it could read.
fn static_weight(m: &Metrics) -> f64 {
    let mut weight = 0.0;
    if m.duplication_pct.is_some() {
        weight += 1.0;
    }
    if m.complexity_pct.is_some() {
        weight += 1.0;
    }
    if m.dead_code_pct.is_some() {
        weight += m.dead_code_coverage.map_or(1.0, |c| c / 100.0);
    }
    if weight > 0.0 { weight } else { 1.0 }
}

/// An external metric as jscpd scores it: its `score`, or its `value` on
/// the half-life curve (`higher` metrics by their distance to `max`).
pub fn external(metric: &ExternalMetric) -> DimensionScore {
    let score = metric.score.unwrap_or_else(|| {
        let value = metric.value.unwrap_or_default();
        let distance = match metric.direction {
            Direction::Lower => value,
            Direction::Higher => metric.max.unwrap_or(100.0) - value,
        };
        100.0 * 2f64.powf(-distance.max(0.0) / metric.half_life.unwrap_or(1.0))
    });
    DimensionScore {
        id: metric.id.clone(),
        weight: metric.weight.unwrap_or(1.0),
        score: round1(score),
    }
}

/// jscpd's mean: weighted geometric, every sub-score floored at 1 so one
/// zero does not erase the rest, rounded to one decimal.
pub fn weighted_mean(dims: &[DimensionScore]) -> Option<f64> {
    let weight: f64 = dims
        .iter()
        .filter(|d| d.weight > 0.0)
        .map(|d| d.weight)
        .sum();
    (weight > 0.0).then(|| {
        let log_sum: f64 = dims
            .iter()
            .filter(|d| d.weight > 0.0)
            .map(|d| d.weight * d.score.max(1.0).ln())
            .sum();
        round1((log_sum / weight).exp())
    })
}

/// The tree's dimensions plus `extra`, scored as one.
pub fn compose(static_dims: Option<&[DimensionScore]>, extra: &[ExternalMetric]) -> Composed {
    let Some(static_dims) = static_dims.filter(|d| !d.is_empty()) else {
        return Composed::default();
    };
    let mut dimensions = static_dims.to_vec();
    dimensions.extend(extra.iter().map(external));
    dimensions.retain(|d| d.weight > 0.0);
    let score = weighted_mean(&dimensions);
    Composed {
        score,
        grade: score.map(cpd_core::health::grade),
        dimensions,
    }
}

/// Two composed scores made comparable: each recomputed over only the
/// dimensions both have. "Two scores are comparable only when they are
/// built from the same dimensions" — a start measured before the tests
/// ran against an end measured with them would otherwise compare apples
/// with apples-and-tests.
pub fn comparable(a: &Composed, b: &Composed) -> (Option<f64>, Option<f64>) {
    let common = |x: &Composed, y: &Composed| -> Vec<DimensionScore> {
        x.dimensions
            .iter()
            .filter(|d| y.dimensions.iter().any(|o| o.id == d.id))
            .cloned()
            .collect()
    };
    let (ca, cb) = (common(a, b), common(b, a));
    if ca.is_empty() {
        return (a.score, b.score);
    }
    (weighted_mean(&ca), weighted_mean(&cb))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::suite::{SuiteResult, TestCounts, metrics};
    use crate::{HealthConfig, analyze};

    const CODE: &str = r#"export function summarize(items, options) {
  let total = 0;
  let count = 0;
  for (const item of items) {
    if (item.price > 0 && item.quantity > 0) {
      total += item.price * item.quantity;
      count += 1;
    } else if (options.strict) {
      throw new Error("invalid item: " + item.id);
    }
  }
  const average = count > 0 ? total / count : 0;
  return { total, count, average, currency: options.currency || "USD" };
}
"#;

    fn suite(passed: u64, failed: u64, coverage: Option<f64>) -> SuiteResult {
        SuiteResult {
            exit_code: Some(i32::from(failed > 0)),
            counts: Some(TestCounts {
                passed,
                failed,
                skipped: 0,
            }),
            coverage_pct: coverage,
            ..SuiteResult::default()
        }
    }

    /// The composition is jscpd's own score with the suite's metrics in
    /// its config — for the same tree, the same number.
    #[test]
    fn composing_matches_jscpd_scoring_the_metrics_itself() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.js"), CODE).unwrap();
        std::fs::write(dir.path().join("b.js"), CODE).unwrap();
        std::fs::write(dir.path().join("c.py"), "def f(x):\n    return x + 1\n").unwrap();
        let plain = analyze(dir.path(), &HealthConfig::default()).unwrap();
        let json = serde_json::to_value(&plain).unwrap();
        let dims = static_dimensions(&json).unwrap();
        for run in [
            suite(12, 1, Some(81.0)),
            suite(40, 0, None),
            suite(0, 5, Some(3.0)),
            suite(3, 0, Some(100.0)),
        ] {
            let extra = metrics(&run);
            let composed = compose(Some(&dims), &extra);
            let mut cfg = HealthConfig::default();
            cfg.jscpd.metrics = extra.clone();
            let direct = analyze(dir.path(), &cfg).unwrap();
            assert_eq!(composed.score, direct.score, "{run:?}");
            assert_eq!(composed.grade, direct.grade, "{run:?}");
        }
    }

    #[test]
    fn nothing_extra_changes_nothing() {
        let dims = vec![
            DimensionScore {
                id: "duplication".into(),
                weight: 1.0,
                score: 75.0,
            },
            DimensionScore {
                id: "complexity".into(),
                weight: 1.0,
                score: 76.0,
            },
        ];
        let composed = compose(Some(&dims), &[]);
        assert_eq!(composed.score, weighted_mean(&dims));
        assert_eq!(composed.dimensions, dims);
    }

    #[test]
    fn a_suite_alone_makes_no_score() {
        let extra = metrics(&suite(10, 0, Some(90.0)));
        assert_eq!(compose(None, &extra), Composed::default());
        assert_eq!(compose(Some(&[]), &extra).score, None);
    }

    #[test]
    fn passing_tests_lift_and_failing_ones_sink() {
        let dims = vec![DimensionScore {
            id: "duplication".into(),
            weight: 1.0,
            score: 78.0,
        }];
        let base = compose(Some(&dims), &[]).score.unwrap();
        let green = compose(Some(&dims), &metrics(&suite(20, 0, None)));
        assert!(green.score.unwrap() > base);
        assert_eq!(green.sub_score("tests"), Some(100.0));
        let one_in_ten = compose(Some(&dims), &metrics(&suite(9, 1, None)));
        assert_eq!(one_in_ten.sub_score("tests"), Some(50.0));
        let red = compose(Some(&dims), &metrics(&suite(0, 3, None)));
        assert!(red.score.unwrap() < one_in_ten.score.unwrap());
        // jscpd's documented example: 81% coverage scores 72 (71.9).
        let covered = compose(Some(&dims), &metrics(&suite(20, 0, Some(81.0))));
        assert_eq!(covered.sub_score("coverage"), Some(71.9));
    }

    #[test]
    fn a_cut_down_result_composes_from_its_score() {
        let json = serde_json::json!({
            "score": 80.0,
            "metrics": {
                "files": 3, "code_lines": 90, "duplication_pct": 0.0, "clones": 0,
                "complexity_pct": 0.0, "dead_code_pct": 2.0, "dead_code_coverage": 50.0,
                "ignore_markers": 0, "jscpd_config_present": false
            },
            "truncated": true
        });
        let dims = static_dimensions(&json).unwrap();
        assert_eq!(dims.len(), 1);
        assert!((dims[0].weight - 2.5).abs() < 1e-9);
        assert_eq!(compose(Some(&dims), &[]).score, Some(80.0));
        assert_eq!(static_dimensions(&serde_json::json!({"score": null})), None);
    }

    #[test]
    fn comparable_scores_share_their_dimensions() {
        let tree = vec![DimensionScore {
            id: "duplication".into(),
            weight: 1.0,
            score: 70.0,
        }];
        let start = compose(Some(&tree), &[]);
        let end = compose(Some(&tree), &metrics(&suite(0, 4, None)));
        // The end's failing tests have no counterpart at the start.
        assert_eq!(comparable(&start, &end), (Some(70.0), Some(70.0)));
        let start_tested = compose(Some(&tree), &metrics(&suite(4, 0, None)));
        let (a, b) = comparable(&start_tested, &end);
        assert!(a.unwrap() > b.unwrap());
    }
}
