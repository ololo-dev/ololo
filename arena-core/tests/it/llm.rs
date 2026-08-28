use std::path::PathBuf;

use arena_core::llm::{LlmError, ModelConfig, PROVIDER_REGISTRY, ProviderKind, lookup_provider};

#[test]
fn lookup_provider_finds_known() {
    assert!(lookup_provider("ollama").is_some());
    assert!(lookup_provider("openrouter").is_some());
    let opencode = lookup_provider("opencode").expect("opencode registered");
    assert_eq!(opencode.kind, ProviderKind::OpenAiCompatible);
    assert_eq!(opencode.default_model, "");
}

#[test]
fn lookup_provider_rejects_unknown() {
    assert!(lookup_provider("does-not-exist").is_none());
    assert!(lookup_provider("").is_none());
}

#[test]
fn registry_ids_are_unique() {
    let mut seen = std::collections::HashSet::new();
    for spec in PROVIDER_REGISTRY {
        assert!(seen.insert(spec.id), "duplicate provider id: {}", spec.id);
    }
}

#[test]
fn build_judge_unknown_provider_errors() {
    let cfg = ModelConfig {
        provider_name: None,
        provider: "nope".to_string(),
        model: "m".to_string(),
        base_url: None,
        api_key: None,
    };
    assert!(matches!(
        cfg.build_judge(PathBuf::new(), None, None),
        Err(LlmError::UnknownProvider(_))
    ));
}

#[test]
fn build_judge_openrouter_requires_key() {
    let cfg = ModelConfig {
        provider_name: None,
        provider: "openrouter".to_string(),
        model: "m".to_string(),
        base_url: None,
        api_key: None,
    };
    assert!(matches!(
        cfg.build_judge(PathBuf::new(), None, None),
        Err(LlmError::MissingApiKey(_))
    ));
}

#[test]
fn build_judge_openai_compatible_requires_key() {
    let cfg = ModelConfig {
        provider_name: None,
        provider: "opencode".to_string(),
        model: "m".to_string(),
        base_url: None,
        api_key: None,
    };
    assert!(matches!(
        cfg.build_judge(PathBuf::new(), None, None),
        Err(LlmError::MissingApiKey(_))
    ));

    // With a key, opencode falls back to its default base URL and builds.
    let cfg2 = ModelConfig {
        provider_name: None,
        provider: "opencode".to_string(),
        model: "m".to_string(),
        base_url: None,
        api_key: Some("key".to_string()),
    };
    assert!(cfg2.build_judge(PathBuf::new(), None, None).is_ok());
}

#[test]
fn build_judge_ollama_succeeds_without_key() {
    let cfg = ModelConfig {
        provider_name: None,
        provider: "ollama".to_string(),
        model: "llama3.2".to_string(),
        base_url: None,
        api_key: None,
    };
    // No network call; client construction only.
    assert!(cfg.build_judge(PathBuf::new(), None, None).is_ok());
}

#[test]
fn build_judge_openrouter_with_key_and_base_succeeds() {
    let cfg = ModelConfig {
        provider_name: None,
        provider: "openrouter".to_string(),
        model: "m".to_string(),
        base_url: Some("https://api.openrouter.ai/api/v1".to_string()),
        api_key: Some("sk-test".to_string()),
    };
    assert!(cfg.build_judge(PathBuf::new(), None, None).is_ok());
}
