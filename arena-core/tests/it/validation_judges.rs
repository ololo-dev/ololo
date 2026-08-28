use arena_core::validation::judges::validate_rating_scale;
use serde_json::json;

#[test]
fn binary_scale_valid() {
    assert!(validate_rating_scale(&json!({"min": 0.0, "max": 1.0, "step": 1.0})).is_ok());
}

#[test]
fn zero_to_ten_half_step_valid() {
    assert!(validate_rating_scale(&json!({"min": 0.0, "max": 10.0, "step": 0.5})).is_ok());
}

#[test]
fn zero_to_one_tenth_step_valid() {
    assert!(validate_rating_scale(&json!({"min": 0.0, "max": 1.0, "step": 0.1})).is_ok());
}

#[test]
fn non_divisible_step_invalid() {
    assert!(validate_rating_scale(&json!({"min": 0.0, "max": 1.0, "step": 0.3})).is_err());
}

#[test]
fn min_greater_than_max_invalid() {
    assert!(validate_rating_scale(&json!({"min": 5.0, "max": 0.0, "step": 1.0})).is_err());
}

#[test]
fn step_zero_invalid() {
    assert!(validate_rating_scale(&json!({"min": 0.0, "max": 10.0, "step": 0.0})).is_err());
}

#[test]
fn missing_step_invalid() {
    assert!(validate_rating_scale(&json!({"min": 0.0, "max": 10.0})).is_err());
}

#[test]
fn non_numeric_min_invalid() {
    assert!(validate_rating_scale(&json!({"min": "a", "max": 1.0, "step": 1.0})).is_err());
}

#[test]
fn not_an_object_invalid() {
    assert!(validate_rating_scale(&json!([1, 2, 3])).is_err());
    assert!(validate_rating_scale(&json!("nope")).is_err());
    assert!(validate_rating_scale(&json!(42)).is_err());
    assert!(validate_rating_scale(&json!(null)).is_err());
}

#[test]
fn negative_step_invalid() {
    assert!(validate_rating_scale(&json!({"min": 0.0, "max": 10.0, "step": -1.0})).is_err());
}

#[test]
fn equal_min_max_invalid() {
    assert!(validate_rating_scale(&json!({"min": 5.0, "max": 5.0, "step": 1.0})).is_err());
}

#[test]
fn integer_values_valid() {
    assert!(validate_rating_scale(&json!({"min": 0, "max": 5, "step": 1})).is_ok());
}
