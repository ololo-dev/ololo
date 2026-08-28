use super::*;

#[test]
fn player_error_not_found_status() {
    let response = PlayerError::NotFound.into_response();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[test]
fn player_error_forbidden_status() {
    let response = PlayerError::Forbidden.into_response();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[test]
fn player_error_internal_status() {
    let response = PlayerError::Internal("db_error".to_string()).into_response();
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
}
