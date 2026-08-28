//! Tests for `auth::password` re-exposed at integration level.
use argon2::Argon2;
use server::auth::password::{hash_password, verify_password};

#[test]
fn hash_then_verify_succeeds_integration() {
    let argon2 = Argon2::default();
    let h = hash_password(&argon2, "hunter2").unwrap();
    assert!(verify_password(&argon2, &h, "hunter2").unwrap());
}

#[test]
fn verify_with_wrong_password_fails_integration() {
    let argon2 = Argon2::default();
    let h = hash_password(&argon2, "hunter2").unwrap();
    assert!(!verify_password(&argon2, &h, "hunter3").unwrap());
}

#[test]
fn hash_strings_differ_for_same_password() {
    // Salt must be randomized.
    let argon2 = Argon2::default();
    let a = hash_password(&argon2, "same").unwrap();
    let b = hash_password(&argon2, "same").unwrap();
    assert_ne!(a, b);
}
