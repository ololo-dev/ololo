use super::*;

// Tags Vec<String> JSON round-trip (FR-014, NFR-002)
#[test]
fn test_tags_json_round_trip() {
    let tags: Vec<String> = vec!["rust".to_string(), "web".to_string(), "async".to_string()];
    let json = serde_json::to_string(&tags).unwrap();
    let deserialized: Vec<String> = serde_json::from_str(&json).unwrap();
    assert_eq!(tags, deserialized);
}

// FR-011 category validation
#[test]
fn test_validate_category_passes_when_in_list() {
    let allowed = Some(vec!["Workshop".to_string(), "Hackathon".to_string()]);
    assert!(validate_category("Workshop", &allowed).is_ok());
}

#[test]
fn test_validate_category_fails_when_not_in_list() {
    let allowed = Some(vec!["Workshop".to_string(), "Hackathon".to_string()]);
    assert!(validate_category("Unknown", &allowed).is_err());
}

#[test]
fn test_validate_category_permissive_when_key_absent() {
    // FR-011: key absent from app_settings → any string accepted
    assert!(validate_category("anything-at-all", &None).is_ok());
}

// FR-003 tags validation
#[test]
fn test_validate_tags_passes_valid() {
    let tags: Vec<String> = vec!["rust".to_string(), "async".to_string()];
    assert!(validate_tags(&tags).is_ok());
}

#[test]
fn test_validate_tags_fails_more_than_twenty() {
    let tags: Vec<String> = (0..21).map(|i| format!("tag{i}")).collect();
    assert!(validate_tags(&tags).is_err());
}

#[test]
fn test_validate_tags_fails_tag_too_long() {
    let tags = vec!["a".repeat(129)];
    assert!(validate_tags(&tags).is_err());
}

#[test]
fn test_validate_tags_fails_duplicates() {
    let tags = vec!["rust".to_string(), "rust".to_string()];
    assert!(validate_tags(&tags).is_err());
}

// FR-013 cover URL validation
#[test]
fn test_validate_cover_image_url_passes() {
    assert!(validate_cover_image_url("https://ik.imagekit.io/demo/img.png").is_ok());
}

#[test]
fn test_validate_cover_image_url_fails_http() {
    assert!(validate_cover_image_url("http://example.com/img.png").is_err());
}

#[test]
fn test_validate_cover_image_url_fails_too_long() {
    let url = format!("https://{}", "a".repeat(2048));
    assert!(validate_cover_image_url(&url).is_err());
}
