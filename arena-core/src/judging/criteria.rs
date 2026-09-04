//! Per-criterion verdicts (judge contract v2, open-ended tasks).
//!
//! A criteria judge answers with a score sheet instead of a bare number:
//! one 0.0–10.0 score (or `null` — "cannot assess", rationale required) per
//! criterion it declared. The sheet is stored verbatim in
//! `judge_results.rating` (already `Json`); the scalar the scoring pipeline
//! needs is derived here — a weight-renormalized mean of the non-null
//! scores, mapped linearly onto the judge's effective point scale, exactly
//! the mapping [`super::execution::aggregate_rating`] applies to
//! pass-fractions. One mapping, two callers, no drift.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::evaluation::{CRITERION_SCORE_MAX, CRITERION_SCORE_MIN};
use crate::validation::judge_results::RatingScale;

/// Everything `run_judge` needs to drive a criteria judge: the keys the
/// judge declared it scores, and the contract's weights for averaging.
#[derive(Debug, Clone)]
pub struct CriteriaContext {
    pub keys: Vec<String>,
    pub weights: BTreeMap<String, f64>,
}

/// One row of a criteria verdict.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CriterionScore {
    pub key: String,
    /// 0.0–10.0, or `null` = "cannot assess" (excluded, weights renormalize).
    pub score: Option<f64>,
    #[serde(default)]
    pub rationale: String,
    /// References into stored data: `probe:<uuid>`, `commit:<sha>`,
    /// `file:<path>:<line>`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evidence: Vec<String>,
}

/// The v2 verdict shape.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CriteriaVerdict {
    pub criteria: Vec<CriterionScore>,
    #[serde(default)]
    pub feedback: String,
}

/// Any verdict a judge model may return: the long-standing scalar shape, or
/// the criteria sheet.
#[derive(Debug, Clone, PartialEq)]
pub enum AnyVerdict {
    Legacy { rating: f64, feedback: String },
    Criteria(CriteriaVerdict),
}

#[derive(Debug, Deserialize)]
struct LegacyJson {
    rating: f64,
    feedback: String,
}

/// Parse either verdict shape out of a model's final text (fences and leaked
/// prose tolerated, same as the legacy parser).
pub fn parse_any_verdict(raw: &str) -> Result<AnyVerdict, String> {
    let candidates = json_object_candidates(raw);
    if candidates.is_empty() {
        return Err("no JSON object in verdict".into());
    }
    let mut last_err = String::new();
    for candidate in candidates {
        if let Ok(v) = serde_json::from_str::<CriteriaVerdict>(candidate)
            && !v.criteria.is_empty()
        {
            return Ok(AnyVerdict::Criteria(v));
        }
        match serde_json::from_str::<LegacyJson>(candidate) {
            Ok(v) => {
                return Ok(AnyVerdict::Legacy {
                    rating: v.rating,
                    feedback: v.feedback,
                });
            }
            Err(e) => last_err = format!("verdict parse: {e}"),
        }
    }
    Err(last_err)
}

/// Where a verdict may sit in a model's final message, most likely first.
///
/// Models end on prose more often than not, and the prose quotes code —
/// `${city || ""}`, a `{ label }` destructuring — so the old "first `{` to
/// last `}`" slice was garbage exactly when the analysis was thorough,
/// while a perfectly good sheet sat in a ```json fence at the bottom
/// (JFV7O5: every nudge round in the session traced to one such brace).
/// Fenced blocks come first, last fence first; then every balanced
/// top-level `{…}` in the text, last first — the verdict is the closing
/// word, and a leaked reasoning block before it is not.
pub fn json_object_candidates(raw: &str) -> Vec<&str> {
    let mut out: Vec<&str> = Vec::new();
    for block in fenced_blocks(raw).into_iter().rev() {
        for span in balanced_objects(block).into_iter().rev() {
            out.push(span);
        }
    }
    for span in balanced_objects(raw).into_iter().rev() {
        if !out
            .iter()
            .any(|c| std::ptr::eq(c.as_ptr(), span.as_ptr()) && c.len() == span.len())
        {
            out.push(span);
        }
    }
    out
}

/// The bodies of every ``` fence in `raw`, in order, language tag dropped.
fn fenced_blocks(raw: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut rest = raw;
    while let Some(open) = rest.find("```") {
        let after_open = &rest[open + 3..];
        // The language tag is whatever follows the fence on its own line.
        let body_start = match after_open.find('\n') {
            Some(nl) if after_open[..nl].trim().chars().all(|c| c.is_alphanumeric()) => nl + 1,
            _ => 0,
        };
        let body = &after_open[body_start..];
        match body.find("```") {
            Some(close) => {
                out.push(&body[..close]);
                rest = &body[close + 3..];
            }
            None => {
                out.push(body);
                break;
            }
        }
    }
    out
}

/// Every balanced top-level `{…}` in `text`, in order. String-aware, so a
/// brace inside a quoted rationale neither opens nor closes an object.
fn balanced_objects(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut depth = 0usize;
    let mut start = None;
    let mut in_string = false;
    let mut escaped = false;
    for (i, ch) in text.char_indices() {
        if in_string {
            match ch {
                '\\' if !escaped => escaped = true,
                '"' if !escaped => in_string = false,
                _ => escaped = false,
            }
            continue;
        }
        match ch {
            '"' if depth > 0 => in_string = true,
            '{' => {
                if depth == 0 {
                    start = Some(i);
                }
                depth += 1;
            }
            '}' if depth > 0 => {
                depth -= 1;
                if depth == 0
                    && let Some(s) = start.take()
                {
                    out.push(&text[s..=i]);
                }
            }
            _ => {}
        }
    }
    out
}

/// Validate a criteria sheet against the keys this judge declared and the
/// fixed 0–10 scale. A `null` score demands a rationale (YC4T6K rule: no
/// silent shrugs); unknown keys are rejected by name.
pub fn validate_criteria_verdict(
    verdict: &CriteriaVerdict,
    allowed_keys: &[String],
) -> Result<(), String> {
    let mut seen = std::collections::BTreeSet::new();
    for c in &verdict.criteria {
        let key = c.key.trim();
        if !allowed_keys.iter().any(|k| k.trim() == key) {
            return Err(format!(
                "criterion '{key}' was not declared by this judge (declared: {})",
                allowed_keys.join(", ")
            ));
        }
        if !seen.insert(key.to_string()) {
            return Err(format!("criterion '{key}' scored twice"));
        }
        match c.score {
            Some(s) => {
                if !s.is_finite() || !(CRITERION_SCORE_MIN..=CRITERION_SCORE_MAX).contains(&s) {
                    return Err(format!("criterion '{key}' score out of range: {s}"));
                }
            }
            None => {
                if c.rationale.trim().is_empty() {
                    return Err(format!(
                        "criterion '{key}' is null without a rationale — \"cannot assess\" must say why"
                    ));
                }
            }
        }
    }
    for key in allowed_keys {
        if !verdict.criteria.iter().any(|c| c.key.trim() == key.trim()) {
            return Err(format!("declared criterion '{key}' was not scored"));
        }
    }
    Ok(())
}

/// The 0–10 overall of a sheet: weight-renormalized mean of the non-null
/// scores. Weights come from the task's evaluation contract; a criterion the
/// contract does not weight gets weight 1. `None` = every score was null
/// ("cannot assess anything") — the caller records a skip, not a zero.
pub fn criteria_overall(
    verdict: &CriteriaVerdict,
    contract_weights: &BTreeMap<String, f64>,
) -> Option<f64> {
    let mut weighted_sum = 0.0;
    let mut weight_total = 0.0;
    for c in &verdict.criteria {
        let Some(score) = c.score else { continue };
        let w = contract_weights
            .get(c.key.trim())
            .copied()
            .filter(|w| w.is_finite() && *w > 0.0)
            .unwrap_or(1.0);
        weighted_sum += score * w;
        weight_total += w;
    }
    (weight_total > 0.0).then(|| weighted_sum / weight_total)
}

/// Map a 0–10 overall linearly onto a judge's effective scale, quantized to
/// its step — the same shape `aggregate_rating` gives a pass-fraction.
pub fn map_overall_to_scale(overall: f64, scale: &RatingScale) -> f64 {
    let fraction = (overall / CRITERION_SCORE_MAX).clamp(0.0, 1.0);
    let raw = scale.min + fraction * (scale.max - scale.min);
    if scale.step > 0.0 {
        let steps = ((raw - scale.min) / scale.step).round();
        (scale.min + steps * scale.step).clamp(scale.min, scale.max)
    } else {
        raw.clamp(scale.min, scale.max)
    }
}

/// A panel judge's share of an open-ended task's point budget: its weight
/// over the panel's total, applied to `point_value`. The resulting scale is
/// what the 0–10 overall maps onto. Floors at one step so a tiny share still
/// distinguishes "good" from "nothing".
pub fn panel_share_scale(point_value: i32, weight: f64, weight_total: f64) -> RatingScale {
    let share = if weight_total > 0.0 && weight.is_finite() && weight > 0.0 {
        (point_value.max(0) as f64) * (weight / weight_total)
    } else {
        0.0
    };
    RatingScale {
        min: 0.0,
        max: share.round().max(1.0),
        step: 1.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sheet(entries: &[(&str, Option<f64>, &str)]) -> CriteriaVerdict {
        CriteriaVerdict {
            criteria: entries
                .iter()
                .map(|(k, s, r)| CriterionScore {
                    key: k.to_string(),
                    score: *s,
                    rationale: r.to_string(),
                    evidence: vec![],
                })
                .collect(),
            feedback: "fb".to_string(),
        }
    }

    #[test]
    fn parses_both_shapes_with_fences() {
        let legacy =
            parse_any_verdict("```json\n{\"rating\": 7.5, \"feedback\": \"ok\"}\n```").unwrap();
        assert_eq!(
            legacy,
            AnyVerdict::Legacy {
                rating: 7.5,
                feedback: "ok".to_string()
            }
        );

        let v2 = parse_any_verdict(
            "Verdict:\n{\"criteria\": [{\"key\": \"product\", \"score\": 6.0, \
             \"rationale\": \"solid\", \"evidence\": [\"probe:x\"]}], \"feedback\": \"ok\"}",
        )
        .unwrap();
        match v2 {
            AnyVerdict::Criteria(v) => {
                assert_eq!(v.criteria[0].score, Some(6.0));
                assert_eq!(v.criteria[0].evidence, vec!["probe:x"]);
            }
            other => panic!("wrong shape: {other:?}"),
        }
    }

    /// The tail of a real glm-5.3-flash final (JFV7O5, Architecture on
    /// task 0): a page of analysis quoting template literals, then the
    /// sheet in a fence. The first `{` in the text belongs to `${city}`.
    const PROSE_THEN_SHEET: &str = "Also app.js interpolates user-provided `city` into HTML \
        via template strings without escaping (`value=\"${city || \"\"}\"` and `unknown \
        city: \"${city}\"`) — that's an XSS concern.\n\nScore: 8.5.\n\n```json\n\
        {\"criteria\": [{\"key\": \"architecture\", \"score\": 8.5, \"rationale\": \
        \"Clean layering {data} vs {view}\", \"evidence\": [\"file:weather.js:3\"]}], \
        \"feedback\": \"tidy\"}\n```";

    #[test]
    fn a_sheet_fenced_after_prose_with_braces_is_found() {
        match parse_any_verdict(PROSE_THEN_SHEET).unwrap() {
            AnyVerdict::Criteria(v) => {
                assert_eq!(v.criteria[0].key, "architecture");
                assert_eq!(v.criteria[0].score, Some(8.5));
                assert_eq!(v.feedback, "tidy");
            }
            other => panic!("wrong shape: {other:?}"),
        }
    }

    #[test]
    fn without_a_fence_the_last_balanced_object_wins() {
        let raw = "Reasoning: the map { a: 1 } is fine.\nDraft {\"rating\": 3.0, \
                   \"feedback\": \"draft\"}\nFinal answer:\n{\"rating\": 7.0, \
                   \"feedback\": \"final {with} braces\"}";
        assert_eq!(
            parse_any_verdict(raw).unwrap(),
            AnyVerdict::Legacy {
                rating: 7.0,
                feedback: "final {with} braces".to_string()
            }
        );
    }

    #[test]
    fn a_fenced_sheet_beats_a_later_stray_object() {
        let raw = "```json\n{\"rating\": 6.0, \"feedback\": \"fenced\"}\n```\n\
                   P.S. the config {\"debug\": true} is unrelated.";
        assert_eq!(
            parse_any_verdict(raw).unwrap(),
            AnyVerdict::Legacy {
                rating: 6.0,
                feedback: "fenced".to_string()
            }
        );
    }

    #[test]
    fn prose_alone_is_not_a_verdict() {
        assert!(parse_any_verdict("I think the code is great").is_err());
        assert!(parse_any_verdict("see { and } scattered } {").is_err());
    }

    #[test]
    fn validation_enforces_declared_keys_and_null_rationales() {
        let keys = vec!["product".to_string(), "tests".to_string()];

        validate_criteria_verdict(
            &sheet(&[
                ("product", Some(6.0), ""),
                ("tests", None, "no tests visible"),
            ]),
            &keys,
        )
        .unwrap();

        // Undeclared key, missing key, silent null, out-of-range: rejected.
        assert!(
            validate_criteria_verdict(&sheet(&[("vibes", Some(1.0), "")]), &keys)
                .unwrap_err()
                .contains("not declared")
        );
        assert!(
            validate_criteria_verdict(&sheet(&[("product", Some(1.0), "")]), &keys)
                .unwrap_err()
                .contains("tests")
        );
        assert!(
            validate_criteria_verdict(
                &sheet(&[("product", None, ""), ("tests", Some(1.0), "")]),
                &keys
            )
            .unwrap_err()
            .contains("rationale")
        );
        assert!(
            validate_criteria_verdict(
                &sheet(&[("product", Some(11.0), ""), ("tests", Some(1.0), "")]),
                &keys
            )
            .unwrap_err()
            .contains("out of range")
        );
    }

    #[test]
    fn overall_renormalizes_over_nulls_and_maps_onto_scales() {
        let mut weights = BTreeMap::new();
        weights.insert("product".to_string(), 0.6);
        weights.insert("tests".to_string(), 0.4);

        // Both scored: plain weighted mean.
        let v = sheet(&[("product", Some(10.0), ""), ("tests", Some(5.0), "")]);
        let overall = criteria_overall(&v, &weights).unwrap();
        assert!((overall - 8.0).abs() < 1e-9);

        // One null: its weight leaves the denominator.
        let v = sheet(&[("product", Some(10.0), ""), ("tests", None, "why")]);
        assert!((criteria_overall(&v, &weights).unwrap() - 10.0).abs() < 1e-9);

        // All null: nothing to score.
        let v = sheet(&[("product", None, "a"), ("tests", None, "b")]);
        assert!(criteria_overall(&v, &weights).is_none());

        // Mapping: 8.0/10 onto a 0..50 step-1 scale = 40.
        let scale = RatingScale {
            min: 0.0,
            max: 50.0,
            step: 1.0,
        };
        assert_eq!(map_overall_to_scale(8.0, &scale), 40.0);
        assert_eq!(map_overall_to_scale(0.0, &scale), 0.0);
        assert_eq!(map_overall_to_scale(10.0, &scale), 50.0);
    }

    #[test]
    fn panel_shares_split_the_budget_by_weight() {
        // 200 points, weights 1.0 / 0.5 / 1.5 → shares 67 / 33 / 100.
        let total = 3.0;
        assert_eq!(panel_share_scale(200, 1.0, total).max, 67.0);
        assert_eq!(panel_share_scale(200, 0.5, total).max, 33.0);
        assert_eq!(panel_share_scale(200, 1.5, total).max, 100.0);
        // Degenerate inputs floor at one step.
        assert_eq!(panel_share_scale(0, 1.0, 1.0).max, 1.0);
        assert_eq!(panel_share_scale(200, 0.0, 0.0).max, 1.0);
    }
}
