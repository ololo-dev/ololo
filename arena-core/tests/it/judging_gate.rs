//! The task-scoped gate: what a penalty-only judge is allowed to do.
//!
//! Every case here is a session that actually happened. A judge that reverses
//! a reward cannot reverse more than was granted, and cannot reverse anything
//! at all where nothing was granted — the rule the session-scoped path has
//! enforced since it was written, and whose absence here cost honest players
//! real points.

use arena_core::judging::{TaskJudgeGate, gate_task_judge};
use serde_json::json;

fn scale(min: f64, max: f64, step: f64) -> serde_json::Value {
    json!({"min": min, "max": max, "step": step})
}

#[test]
fn a_task_that_paid_nothing_is_not_judged() {
    // Session YC4T6K: a task the session clock cut short, still carrying the
    // anti-cheat judge, judged for "missing" an implementation that was being
    // written when the timer ran out.
    let gate = gate_task_judge(&scale(-50.0, 0.0, 1.0), &None, 0);
    match gate {
        TaskJudgeGate::Skip { reason } => assert!(reason.contains("nothing to withdraw")),
        TaskJudgeGate::Run(s) => panic!("expected a skip, got {s:?}"),
    }
}

#[test]
fn a_task_in_the_red_is_not_judged_either() {
    // Failed probes can leave the tally negative. There is still nothing to
    // take back, and the floor must not be computed from a negative payout.
    assert!(matches!(
        gate_task_judge(&scale(-50.0, 0.0, 1.0), &None, -30),
        TaskJudgeGate::Skip { .. }
    ));
}

#[test]
fn a_penalty_can_never_exceed_what_the_task_paid() {
    // Session OD5FJA: -33 on a rung worth 25, for a truthfully reported
    // solution. The judge's own scale reaches -100; the task's payout is the
    // real ceiling.
    let TaskJudgeGate::Run(s) = gate_task_judge(&scale(-100.0, 0.0, 1.0), &None, 25) else {
        panic!("expected a run");
    };
    assert_eq!(s.min, -25.0);
    assert_eq!(s.max, 0.0);
}

#[test]
fn a_judge_stingier_than_the_payout_keeps_its_own_floor() {
    // The clamp only ever tightens: a -10 judge on a 40-point task still
    // stops at -10.
    let TaskJudgeGate::Run(s) = gate_task_judge(&scale(-10.0, 0.0, 1.0), &None, 40) else {
        panic!("expected a run");
    };
    assert_eq!(s.min, -10.0);
}

#[test]
fn the_floor_stays_a_whole_number_of_steps_from_zero() {
    // 0 is the "nothing wrong here" verdict and must always be on the scale.
    // With step 3 and 25 points paid, -24 is the deepest floor that keeps
    // (0 - min) / step integral — erring towards taking less than was paid.
    let TaskJudgeGate::Run(s) = gate_task_judge(&scale(-99.0, 0.0, 3.0), &None, 25) else {
        panic!("expected a run");
    };
    assert_eq!(s.min, -24.0);
    assert!(((0.0 - s.min) / s.step).fract().abs() < 1e-9);
}

#[test]
fn a_payout_smaller_than_one_step_leaves_nothing_to_take() {
    // Rounding down to a step reaches zero, which is the same as no budget.
    assert!(matches!(
        gate_task_judge(&scale(-100.0, 0.0, 10.0), &None, 4),
        TaskJudgeGate::Skip { .. }
    ));
}

#[test]
fn a_rating_judge_is_left_alone() {
    // Quality judges award points rather than reverse them. Nothing about
    // what the task paid bounds them, and a task that paid nothing can still
    // be rated.
    let TaskJudgeGate::Run(s) = gate_task_judge(&scale(0.0, 10.0, 0.5), &None, 0) else {
        panic!("a rating judge must still run on an unpaid task");
    };
    assert_eq!((s.min, s.max), (0.0, 10.0));
}

#[test]
fn an_admin_override_is_gated_the_same_way() {
    // The override replaces the judge's scale, then the payout bounds it —
    // the same order the session path uses.
    let override_ = Some(scale(-80.0, 0.0, 1.0));
    let TaskJudgeGate::Run(s) = gate_task_judge(&scale(-100.0, 0.0, 1.0), &override_, 12) else {
        panic!("expected a run");
    };
    assert_eq!(s.min, -12.0);
}

#[test]
fn a_broken_scale_is_passed_through_untouched() {
    // A zero step would divide by zero. Seed and API validation both reject
    // it, so this is belt-and-braces: hand it on and let the existing
    // validation reject the verdict rather than compute a nonsense floor.
    let TaskJudgeGate::Run(s) = gate_task_judge(&scale(-50.0, 0.0, 0.0), &None, 10) else {
        panic!("expected a run");
    };
    assert_eq!(s.min, -50.0);
}
