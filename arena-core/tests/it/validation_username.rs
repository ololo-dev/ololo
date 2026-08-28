use arena_core::validation::username::*;

#[test]
fn valid_simple() {
    assert_eq!(validate_username("jane5"), Ok(()));
}

#[test]
fn valid_with_hyphen() {
    assert_eq!(validate_username("jane-doe5"), Ok(()));
}

#[test]
fn valid_with_underscore() {
    assert_eq!(validate_username("jane_doe5"), Ok(()));
}

#[test]
fn valid_exact_min() {
    assert_eq!(validate_username("ab1c"), Ok(()));
}

#[test]
fn valid_exact_max() {
    assert_eq!(validate_username("abcdefghijklmnopqrstuvwxyz1234"), Ok(()));
}

#[test]
fn too_short_3() {
    assert_eq!(validate_username("ab1"), Err(UsernameError::InvalidLength));
}

#[test]
fn too_long_31() {
    assert_eq!(
        validate_username("abcdefghijklmnopqrstuvwxyz12345"),
        Err(UsernameError::InvalidLength)
    );
}

#[test]
fn uppercase_rejected() {
    assert_eq!(
        validate_username("JaneDoe5"),
        Err(UsernameError::InvalidFormat)
    );
}

#[test]
fn starts_with_digit() {
    assert_eq!(
        validate_username("1jane"),
        Err(UsernameError::InvalidFormat)
    );
}

#[test]
fn ends_with_hyphen() {
    assert_eq!(
        validate_username("jane-"),
        Err(UsernameError::InvalidFormat)
    );
}

#[test]
fn ends_with_underscore() {
    assert_eq!(
        validate_username("jane_"),
        Err(UsernameError::InvalidFormat)
    );
}

#[test]
fn reserved_admin() {
    assert_eq!(validate_username("admin"), Err(UsernameError::Reserved));
}

#[test]
fn reserved_ololo() {
    assert_eq!(validate_username("ololo"), Err(UsernameError::Reserved));
}

#[test]
fn reserved_u() {
    assert_eq!(validate_username("uuuu"), Ok(()));
}
