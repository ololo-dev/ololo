use super::common::is_valid_bool_value;
use super::*;
use async_trait::async_trait;
use axum::http::StatusCode;
use axum::response::IntoResponse;

/// Stub OllamaHttp that returns a canned list of models.
struct StubOllama {
    models: Vec<String>,
    fail: bool,
}

#[async_trait]
impl OllamaHttp for StubOllama {
    async fn list_models(&self) -> Result<Vec<String>, OllamaClientError> {
        if self.fail {
            Err(OllamaClientError::Unreachable)
        } else {
            Ok(self.models.clone())
        }
    }
}

#[test]
fn settings_error_forbidden_is_403() {
    let resp = SettingsError::Forbidden.into_response();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

#[test]
fn settings_error_unknown_key_is_400() {
    let resp = SettingsError::UnknownKey.into_response();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn stub_ollama_returns_models() {
    let stub = StubOllama {
        models: vec!["llama3.2".to_string(), "mistral".to_string()],
        fail: false,
    };
    let models = stub.list_models().await.unwrap();
    assert_eq!(models, vec!["llama3.2", "mistral"]);
}

#[tokio::test]
async fn stub_ollama_fails_gracefully() {
    let stub = StubOllama {
        models: vec![],
        fail: true,
    };
    assert!(stub.list_models().await.is_err());
}

// WP-001: allow_user_project_creation key validation tests (Contract AC-017–AC-019).

#[test]
fn settings_error_invalid_project_creation_value_is_422() {
    let resp = SettingsError::InvalidProjectCreationValue.into_response();
    assert_eq!(resp.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

#[test]
fn validate_project_creation_value_true_is_valid() {
    assert!(is_valid_bool_value("true"));
    assert!(is_valid_bool_value("True"));
    assert!(is_valid_bool_value("TRUE"));
}

#[test]
fn validate_project_creation_value_false_is_valid() {
    assert!(is_valid_bool_value("false"));
    assert!(is_valid_bool_value("False"));
    assert!(is_valid_bool_value("FALSE"));
}

#[test]
fn validate_project_creation_value_invalid_is_rejected() {
    assert!(!is_valid_bool_value("yes"));
    assert!(!is_valid_bool_value("1"));
    assert!(!is_valid_bool_value(""));
    assert!(!is_valid_bool_value("enabled"));
}
