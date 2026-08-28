use arena_core::validation::judge_results::*;

fn binary() -> RatingScale {
    RatingScale {
        min: 0.0,
        max: 1.0,
        step: 1.0,
    }
}

fn zero_to_ten_half() -> RatingScale {
    RatingScale {
        min: 0.0,
        max: 10.0,
        step: 0.5,
    }
}

fn zero_to_one_tenth() -> RatingScale {
    RatingScale {
        min: 0.0,
        max: 1.0,
        step: 0.1,
    }
}

#[test]
fn valid_binary_zero() {
    assert_eq!(validate_rating_output(0.0, &binary()), Ok(0));
}

#[test]
fn valid_binary_one() {
    assert_eq!(validate_rating_output(1.0, &binary()), Ok(1));
}

#[test]
fn valid_half_step() {
    assert_eq!(validate_rating_output(7.5, &zero_to_ten_half()), Ok(8));
}

#[test]
fn valid_tenth_step_with_float_noise() {
    assert_eq!(validate_rating_output(0.3, &zero_to_one_tenth()), Ok(0));
}

#[test]
fn below_min_out_of_range() {
    assert_eq!(
        validate_rating_output(-0.5, &binary()),
        Err(ValidationError::OutOfRange {
            value: -0.5,
            min: 0.0,
            max: 1.0,
        })
    );
}

#[test]
fn above_max_out_of_range() {
    assert_eq!(
        validate_rating_output(11.0, &zero_to_ten_half()),
        Err(ValidationError::OutOfRange {
            value: 11.0,
            min: 0.0,
            max: 10.0,
        })
    );
}

#[test]
fn not_on_step_rejected() {
    assert_eq!(
        validate_rating_output(0.5, &binary()),
        Err(ValidationError::NotQuantized {
            value: 0.5,
            step: 1.0,
            min: 0.0,
        })
    );
}

#[test]
fn half_step_off_grid_rejected() {
    assert_eq!(
        validate_rating_output(7.3, &zero_to_ten_half()),
        Err(ValidationError::NotQuantized {
            value: 7.3,
            step: 0.5,
            min: 0.0,
        })
    );
}

#[test]
fn magnitude_cap_exceeded() {
    let big = RatingScale {
        min: -2_000_000.0,
        max: 2_000_000.0,
        step: 1.0,
    };
    assert_eq!(
        validate_rating_output(1_500_000.0, &big),
        Err(ValidationError::RatingTooLarge { value: 1_500_000.0 })
    );
}

#[test]
fn magnitude_cap_exceeded_negative() {
    let big = RatingScale {
        min: -2_000_000.0,
        max: 2_000_000.0,
        step: 1.0,
    };
    assert_eq!(
        validate_rating_output(-1_500_000.0, &big),
        Err(ValidationError::RatingTooLarge {
            value: -1_500_000.0,
        })
    );
}

#[test]
fn would_overflow_i32_rejected() {
    let big = RatingScale {
        min: -3_000_000_000.0,
        max: 3_000_000_000.0,
        step: 1.0,
    };
    let result = validate_rating_output(2_500_000_000.0, &big);
    assert!(matches!(
        result,
        Err(ValidationError::RatingTooLarge { .. } | ValidationError::I32Overflow { .. })
    ));
}

#[test]
fn epsilon_tolerance_at_boundary() {
    assert_eq!(validate_rating_output(1.0 + 1e-7, &binary()), Ok(1));
}

#[test]
fn epsilon_violation_at_boundary() {
    assert!(matches!(
        validate_rating_output(1.0 + 1e-4, &binary()),
        Err(ValidationError::OutOfRange { .. })
    ));
}

#[test]
fn empty_feedback_ok() {
    assert_eq!(validate_feedback(""), Ok(()));
}

#[test]
fn feedback_exactly_at_limit_ok() {
    let s = "a".repeat(10_000);
    assert_eq!(validate_feedback(&s), Ok(()));
}

#[test]
fn feedback_one_over_limit_rejected() {
    let s = "a".repeat(10_001);
    assert_eq!(validate_feedback(&s), Err(ValidationError::FeedbackTooLong));
}

#[test]
fn feedback_multibyte_byte_length_used() {
    let s = "é".repeat(5_001);
    assert_eq!(validate_feedback(&s), Err(ValidationError::FeedbackTooLong));
}

#[test]
fn clamp_pulls_out_of_range_to_the_nearest_bound() {
    let penalty = RatingScale {
        min: -20.0,
        max: 0.0,
        step: 1.0,
    };
    // The 6LA7TX case: -50 on [-20, 0] is the cap, not a failure.
    assert_eq!(clamp_rating_to_scale(-50.0, &penalty), -20.0);
    assert_eq!(clamp_rating_to_scale(5.0, &penalty), 0.0);
    // In-range values pass through untouched.
    assert_eq!(clamp_rating_to_scale(-7.0, &penalty), -7.0);
    assert_eq!(clamp_rating_to_scale(-20.0, &penalty), -20.0);
    // A clamped value always survives range validation afterwards.
    assert_eq!(
        validate_rating_output(clamp_rating_to_scale(-50.0, &penalty), &penalty),
        Ok(-20)
    );
}
