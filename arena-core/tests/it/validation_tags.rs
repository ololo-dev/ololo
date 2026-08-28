use arena_core::validation::tags::*;

#[test]
fn test_valid_tags() {
    let tags: Vec<String> = vec!["rust".to_string(), "async".to_string(), "web".to_string()];
    assert!(validate_tags(&tags).is_ok());
}

#[test]
fn test_too_many_tags() {
    let tags: Vec<String> = (0..21).map(|i| format!("tag{i}")).collect();
    assert_eq!(validate_tags(&tags), Err(TagsError::TooMany));
}

#[test]
fn test_empty_tag() {
    let tags = vec!["  ".to_string()];
    assert_eq!(validate_tags(&tags), Err(TagsError::Empty));
}

#[test]
fn test_tag_too_long() {
    let tags = vec!["a".repeat(129)];
    assert_eq!(validate_tags(&tags), Err(TagsError::TooLong));
}

#[test]
fn test_duplicate_tag() {
    let tags = vec!["foo".to_string(), "foo".to_string()];
    assert_eq!(
        validate_tags(&tags),
        Err(TagsError::Duplicate("foo".to_string()))
    );
}
