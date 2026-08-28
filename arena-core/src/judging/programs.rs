//! The judge's own programs: two deterministic cut points around the model.
//!
//! Every judge safeguard we have was won by an incident and then written in
//! Rust, where the author of the judge cannot see it, change it, or ship it.
//! The prompt travels separately from the release — so the two halves of a
//! judge drifted apart, and the only lever an author had left was to ask the
//! model more nicely. Session YC4T6K is what that costs: -125 points of
//! penalties on honest work, closed by editing a prompt.
//!
//! So a judge may carry programs, in the same file as its prompt, delivered
//! the same way. There are two places to cut:
//!
//! **Before** — ```js decide``` reads the [`Evidence`] snapshot and returns
//! `skip(reason)` (terminal zero, no model, no sandbox), `score(rating,
//! feedback)` (the verdict itself, no model), or `ask({focus})` (carry on,
//! optionally pointing the model at what the program found).
//!
//! **After** — ```js review``` reads the same snapshot plus the model's
//! `verdict`, and returns `accept()`, `revise(rating, feedback)` or
//! `reject(reason)`. This is where the things a prompt cannot enforce live: a
//! penalty must cite a commit that exists, a rating must sit inside what the
//! task paid. In the YC4T6K post-mortem a verdict cited the task's UUID where
//! a commit sha belonged — three lines here, and no amount of asking.
//!
//! The language is the boa-sandboxed JS that already runs task fixtures and
//! validation, with the same iteration and recursion bounds. Task authors
//! write it today; judge authors now write the same thing.

use boa_engine::{JsValue, NativeFunction};
use boa_engine::{Source, js_string, property::Attribute};

use crate::judging::JudgeError;
use crate::judging::evidence::Evidence;
use crate::probe_engine::js::bounded_js_context;
use crate::probe_engine::js_wrap::wrap_js;

/// What the `decide` program decided, before the model.
#[derive(Debug, Clone, PartialEq)]
pub enum Decision {
    /// Nothing to judge. Writes the terminal zero and stops.
    Skip { reason: String },
    /// The program reached the verdict itself.
    Score { rating: f64, feedback: String },
    /// Carry on to the judge's normal path (LLM or execution).
    Ask { focus: Option<String> },
}

/// What the `review` program made of the model's verdict.
#[derive(Debug, Clone, PartialEq)]
pub enum Review {
    /// The verdict stands.
    Accept,
    /// Replace it. Re-validated against the same scale the model was held to.
    Revise { rating: f64, feedback: String },
    /// Unusable. Fails the run, which re-prompts the model from scratch
    /// within the retry budget rather than persisting something the program
    /// judged wrong.
    Reject { reason: String },
}

/// The programs a judge body carries.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct JudgePrograms {
    /// Runs before the model; see [`Decision`].
    pub decide: Option<String>,
    /// Runs on the model's verdict; see [`Review`].
    pub review: Option<String>,
}

/// Opening fences of the two programs inside the markdown body.
const DECIDE_FENCE: &str = "```js decide";
const REVIEW_FENCE: &str = "```js review";

/// The helpers a program returns through. Defined in JS rather than as native
/// functions because they are pure shape — a native would buy nothing and
/// cost a `Trace` impl.
const PRELUDE: &str = r#"
function skip(reason) {
  return { __decision: "skip", reason: reason == null ? "" : String(reason) };
}
function score(rating, feedback) {
  return {
    __decision: "score",
    rating: Number(rating),
    feedback: feedback == null ? "" : String(feedback)
  };
}
function ask(opts) {
  var out = { __decision: "ask" };
  if (opts && opts.focus != null) out.focus = String(opts.focus);
  return out;
}
function accept() {
  return { __decision: "accept" };
}
function revise(rating, feedback) {
  return {
    __decision: "revise",
    rating: Number(rating),
    feedback: feedback == null ? "" : String(feedback)
  };
}
function reject(reason) {
  return { __decision: "reject", reason: reason == null ? "" : String(reason) };
}
"#;

/// Split a judge body into the prompt the model sees and the programs it
/// carries.
///
/// Both programs are removed from the prompt on purpose: they are
/// instructions for the harness, not for the model, and leaving them in
/// invites the model to re-enact decisions that were never its to make —
/// including one that has not been taken yet when it reads them.
pub fn split_programs(body: &str) -> (String, JudgePrograms) {
    let (body, decide) = split_fenced(body, DECIDE_FENCE);
    let (prompt, review) = split_fenced(&body, REVIEW_FENCE);
    (prompt, JudgePrograms { decide, review })
}

/// Split a judge body into the prompt the model sees and the program, if the
/// body carries one.
///
/// The program is removed from the prompt on purpose: it is instructions for
/// the harness, not for the model, and leaving it in invites the model to
/// role-play the decision it was never asked to make.
fn split_fenced(body: &str, fence: &str) -> (String, Option<String>) {
    let Some(start) = body.find(fence) else {
        return (body.to_string(), None);
    };
    let after_fence = start + fence.len();
    // The fence line may carry nothing else; the program starts on the next.
    let Some(body_start) = body[after_fence..].find('\n').map(|i| after_fence + i + 1) else {
        return (body.to_string(), None);
    };
    let Some(end_rel) = body[body_start..].find("\n```") else {
        // An unterminated fence is an authoring error. Reading to end of file
        // would silently swallow the rest of the prompt, so leave the body
        // alone and let the judge run as a plain prompt.
        return (body.to_string(), None);
    };
    let program = body[body_start..body_start + end_rel].to_string();
    let end = body_start + end_rel + "\n```".len();
    let mut prompt = String::with_capacity(body.len());
    prompt.push_str(body[..start].trim_end());
    let rest = body[end..].trim_start_matches('\n');
    if !rest.trim().is_empty() {
        // No leading blank lines when the program opens the file, which is
        // where an author naturally puts it — it runs first.
        if !prompt.is_empty() {
            prompt.push_str("\n\n");
        }
        prompt.push_str(rest);
    }
    if program.trim().is_empty() {
        return (prompt, None);
    }
    (prompt, Some(program))
}

/// Parse a judge program without running it.
///
/// Called where a judge definition is accepted — boot seed, `push-seeds`,
/// admin CRUD — so a typo is rejected at delivery. Otherwise it surfaces
/// once per player per task, as a failed run, long after whoever wrote it
/// stopped looking.
pub fn validate_program(script: &str) -> Result<(), String> {
    let mut ctx = bounded_js_context();
    let source = format!("{PRELUDE}\n{}", wrap_js(script));
    boa_engine::Script::parse(Source::from_bytes(&source), None, &mut ctx)
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// Evaluate `script` against the evidence, plus any extra globals, and
/// return whatever it produced as JSON.
///
/// The snapshot's own field names are registered verbatim (`task`, `tasks`,
/// `memory`, `judge`, `prior`), so what an author reads in the recorded
/// "evidence" of a past run is exactly what their program sees.
fn eval_program(
    script: &str,
    evidence: &Evidence,
    extra: &[(&str, serde_json::Value)],
) -> Result<serde_json::Value, JudgeError> {
    let snapshot = serde_json::to_value(evidence)
        .map_err(|e| JudgeError::DecideFailed(format!("evidence is not serializable: {e}")))?;
    let serde_json::Value::Object(fields) = snapshot else {
        return Err(JudgeError::DecideFailed(
            "evidence did not serialize to an object".to_string(),
        ));
    };

    let mut ctx = bounded_js_context();
    let globals = fields
        .iter()
        .map(|(k, v)| (k.as_str(), v))
        .chain(extra.iter().map(|(k, v)| (*k, v)));
    for (key, value) in globals {
        let js_val = JsValue::from_json(value, &mut ctx)
            .map_err(|e| JudgeError::DecideFailed(format!("cannot expose `{key}`: {e}")))?;
        ctx.register_global_property(js_string!(key), js_val, Attribute::all())
            .map_err(|e| JudgeError::DecideFailed(format!("cannot expose `{key}`: {e}")))?;
    }
    // A program that cannot explain itself is hard to debug from a verdict
    // alone, and `console` is absent from a bare boa context.
    ctx.register_global_builtin_callable(
        js_string!("log"),
        1,
        NativeFunction::from_fn_ptr(noop_log),
    )
    .map_err(|e| JudgeError::DecideFailed(format!("cannot register `log`: {e}")))?;

    let source = format!("{PRELUDE}\n{}", wrap_js(script));
    let value = ctx
        .eval(Source::from_bytes(&source))
        .map_err(|e| JudgeError::DecideFailed(e.to_string()))?;
    value
        .to_json(&mut ctx)
        .map_err(|e| JudgeError::DecideFailed(e.to_string()))?
        .ok_or_else(|| {
            JudgeError::DecideFailed("program returned nothing to decide on".to_string())
        })
}

/// Run the `decide` program: the cut before the model.
pub fn run_decide(script: &str, evidence: &Evidence) -> Result<Decision, JudgeError> {
    decision_from_json(&eval_program(script, evidence, &[])?)
}

/// Run the `review` program: the cut after the model.
///
/// Reads the same snapshot as `decide` plus `verdict` — `{rating, feedback}`,
/// already parsed and already inside the judge's scale. What it is for is
/// what a prompt cannot enforce: that a penalty cites something that exists,
/// that a rating is proportionate to what the task paid.
pub fn run_review(
    script: &str,
    evidence: &Evidence,
    rating: f64,
    feedback: &str,
) -> Result<Review, JudgeError> {
    let verdict = serde_json::json!({ "rating": rating, "feedback": feedback });
    review_from_json(&eval_program(script, evidence, &[("verdict", verdict)])?)
}

/// `log(...)` accepts anything and does nothing. It exists so a program
/// written against a fuller runtime does not explode here; the sandbox has no
/// stdout to write to.
fn noop_log(
    _: &JsValue,
    _: &[JsValue],
    _: &mut boa_engine::Context,
) -> boa_engine::JsResult<JsValue> {
    Ok(JsValue::undefined())
}

fn review_from_json(json: &serde_json::Value) -> Result<Review, JudgeError> {
    let kind = json
        .get("__decision")
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            JudgeError::DecideFailed(
                "review must return accept(), revise(rating, feedback) or reject(reason)"
                    .to_string(),
            )
        })?;
    match kind {
        "accept" => Ok(Review::Accept),
        "revise" => {
            let rating = json
                .get("rating")
                .and_then(|v| v.as_f64())
                .filter(|r| r.is_finite())
                .ok_or_else(|| {
                    JudgeError::DecideFailed("revise() needs a finite rating".to_string())
                })?;
            Ok(Review::Revise {
                rating,
                feedback: json
                    .get("feedback")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string(),
            })
        }
        "reject" => Ok(Review::Reject {
            reason: json
                .get("reason")
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .unwrap_or("the judge's review rejected the verdict")
                .to_string(),
        }),
        // A review that answers with a `decide` verb is an authoring mistake
        // worth naming: the two run at different ends of the same judge.
        other => Err(JudgeError::DecideFailed(format!(
            "unknown review decision `{other}`"
        ))),
    }
}

fn decision_from_json(json: &serde_json::Value) -> Result<Decision, JudgeError> {
    let kind = json
        .get("__decision")
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            JudgeError::DecideFailed(
                "program must return skip(reason), score(rating, feedback) or ask()".to_string(),
            )
        })?;
    match kind {
        "skip" => Ok(Decision::Skip {
            reason: json
                .get("reason")
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .unwrap_or("the judge's program found nothing to judge")
                .to_string(),
        }),
        "score" => {
            let rating = json
                .get("rating")
                .and_then(|v| v.as_f64())
                .filter(|r| r.is_finite())
                .ok_or_else(|| {
                    JudgeError::DecideFailed("score() needs a finite rating".to_string())
                })?;
            Ok(Decision::Score {
                rating,
                feedback: json
                    .get("feedback")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string(),
            })
        }
        "ask" => Ok(Decision::Ask {
            focus: json
                .get("focus")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string),
        }),
        other => Err(JudgeError::DecideFailed(format!(
            "unknown decision `{other}`"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Most cases here exercise one fence; this keeps them readable.
    fn decide_only(body: &str) -> (String, Option<String>) {
        let (prompt, programs) = split_programs(body);
        (prompt, programs.decide)
    }

    #[test]
    fn a_body_without_a_program_is_left_alone() {
        let (prompt, program) = decide_only("You are a judge.\n\nBe fair.");
        assert_eq!(prompt, "You are a judge.\n\nBe fair.");
        assert!(program.is_none());
    }

    #[test]
    fn the_program_is_lifted_out_of_the_prompt() {
        let body = "Before.\n\n```js decide\nreturn ask();\n```\n\nAfter.";
        let (prompt, program) = decide_only(body);
        assert_eq!(program.as_deref(), Some("return ask();"));
        assert!(!prompt.contains("ask()"), "got {prompt:?}");
        assert!(prompt.starts_with("Before."));
        assert!(prompt.ends_with("After."));
    }

    #[test]
    fn an_unterminated_fence_leaves_the_prompt_whole() {
        // Reading to end of file would swallow the rest of the judge's
        // instructions and ship a truncated prompt to the model.
        let body = "Instructions.\n\n```js decide\nreturn ask();\nmore instructions";
        let (prompt, program) = decide_only(body);
        assert!(program.is_none());
        assert_eq!(prompt, body);
    }

    #[test]
    fn a_program_that_decides_nothing_is_an_error() {
        assert!(decision_from_json(&serde_json::json!(true)).is_err());
        assert!(decision_from_json(&serde_json::json!({"__decision": "shrug"})).is_err());
        assert!(decision_from_json(&serde_json::json!({"__decision": "score"})).is_err());
    }
}
