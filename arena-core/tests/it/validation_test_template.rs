use arena_core::task_template::{Backoff, Matchers, Placeholder, TestKind, TestTemplate};
use arena_core::validation::test_template::*;

fn base_template(kind: TestKind, cmd: &str) -> TestTemplate {
    TestTemplate {
        kind,
        command_template: cmd.to_string(),
        placeholders: vec![],
        matchers: Matchers::default(),
        backoff: Backoff::default(),
        fixtures: vec![],
        answer_template: None,
    }
}

fn ph(name: &str) -> Placeholder {
    Placeholder {
        name: name.to_string(),
        description: "desc".to_string(),
        required: true,
        secret: false,
    }
}

#[test]
fn validate_minimal_shell_template_ok() {
    let t = base_template(TestKind::Shell, "echo hi");
    assert_eq!(validate_template(&t), Ok(()));
}

#[test]
fn validate_with_matching_placeholders_ok() {
    let mut t = base_template(TestKind::Shell, "echo {greeting} {name}");
    t.placeholders = vec![ph("greeting"), ph("name")];
    assert_eq!(validate_template(&t), Ok(()));
}

#[test]
fn validate_unreferenced_placeholder_errors() {
    let mut t = base_template(TestKind::Shell, "echo hi");
    t.placeholders = vec![ph("ghost")];
    assert_eq!(
        validate_template(&t),
        Err(TemplateError::UnreferencedPlaceholder("ghost".to_string()))
    );
}

#[test]
fn validate_undeclared_placeholder_errors() {
    let t = base_template(TestKind::Shell, "echo {x}");
    assert_eq!(
        validate_template(&t),
        Err(TemplateError::UndeclaredPlaceholder("x".to_string()))
    );
}

#[test]
fn validate_duplicate_placeholder_decl_errors() {
    let mut t = base_template(TestKind::Shell, "echo {n}");
    t.placeholders = vec![ph("n"), ph("n")];
    assert_eq!(
        validate_template(&t),
        Err(TemplateError::DuplicatePlaceholder("n".to_string()))
    );
}

#[test]
fn validate_invalid_placeholder_name_errors() {
    let mut t = base_template(TestKind::Shell, "echo {x}");
    t.placeholders = vec![ph("with-dash")];
    assert_eq!(
        validate_template(&t),
        Err(TemplateError::InvalidPlaceholderName(
            "with-dash".to_string()
        ))
    );
}

#[test]
fn validate_empty_command_errors() {
    let t = base_template(TestKind::Shell, "");
    assert_eq!(validate_template(&t), Err(TemplateError::EmptyCommand));
}

#[test]
fn validate_command_too_long_errors() {
    let big = "a".repeat(8193);
    let t = base_template(TestKind::Shell, &big);
    assert_eq!(validate_template(&t), Err(TemplateError::CommandTooLong));
}

#[test]
fn validate_max_duration_out_of_range_errors() {
    let mut t = base_template(TestKind::Shell, "echo hi");
    t.matchers.max_duration_ms = 0;
    assert_eq!(
        validate_template(&t),
        Err(TemplateError::InvalidMaxDuration(0))
    );
    t.matchers.max_duration_ms = 600_001;
    assert_eq!(
        validate_template(&t),
        Err(TemplateError::InvalidMaxDuration(600_001))
    );
}

#[test]
fn validate_param_timeout_out_of_range_errors() {
    let mut t = base_template(TestKind::Shell, "echo hi");
    t.matchers.param_timeout_ms = 0;
    assert_eq!(
        validate_template(&t),
        Err(TemplateError::InvalidParamTimeout(0))
    );
}

#[test]
fn validate_backoff_max_less_than_initial_errors() {
    let mut t = base_template(TestKind::Shell, "echo hi");
    t.backoff.initial_ms = 1000;
    t.backoff.max_ms = 500;
    assert_eq!(validate_template(&t), Err(TemplateError::InvalidBackoffMax));
}

#[test]
fn validate_backoff_initial_zero_errors() {
    let mut t = base_template(TestKind::Shell, "echo hi");
    t.backoff.initial_ms = 0;
    assert_eq!(
        validate_template(&t),
        Err(TemplateError::InvalidBackoffInitial)
    );
}

#[test]
fn validate_backoff_multiplier_out_of_range_errors() {
    let mut t = base_template(TestKind::Shell, "echo hi");
    t.backoff.multiplier = 0.5;
    assert!(matches!(
        validate_template(&t),
        Err(TemplateError::InvalidBackoffMultiplier(_))
    ));
    t.backoff.multiplier = 11.0;
    assert!(matches!(
        validate_template(&t),
        Err(TemplateError::InvalidBackoffMultiplier(_))
    ));
}

#[test]
fn validate_backoff_attempts_out_of_range_errors() {
    let mut t = base_template(TestKind::Shell, "echo hi");
    t.backoff.max_attempts = 0;
    assert_eq!(
        validate_template(&t),
        Err(TemplateError::InvalidBackoffAttempts(0))
    );
    t.backoff.max_attempts = 65;
    assert_eq!(
        validate_template(&t),
        Err(TemplateError::InvalidBackoffAttempts(65))
    );
}

#[test]
fn validate_http_kind_requires_method_prefix() {
    let t = base_template(TestKind::HttpRequest, "echo nope");
    assert_eq!(
        validate_template(&t),
        Err(TemplateError::KindCommandMismatch {
            kind: TestKind::HttpRequest,
            prefix: "<METHOD> ",
        })
    );
}

#[test]
fn validate_http_kind_accepts_get_prefix() {
    let t = base_template(TestKind::HttpRequest, "GET https://x.test/");
    assert_eq!(validate_template(&t), Ok(()));
}

#[test]
fn validate_file_exists_rejects_metachars() {
    let t = base_template(TestKind::FileExists, "/tmp/foo;rm");
    assert_eq!(
        validate_template(&t),
        Err(TemplateError::KindCommandMismatch {
            kind: TestKind::FileExists,
            prefix: "<path>",
        })
    );
}

#[test]
fn validate_file_exists_accepts_simple_path() {
    let t = base_template(TestKind::FileExists, "/tmp/foo");
    assert_eq!(validate_template(&t), Ok(()));
}

#[test]
fn validate_stdout_regex_size_fallback_rejects_oversize() {
    let mut t = base_template(TestKind::Shell, "echo hi");
    t.matchers.stdout_regex = Some("a".repeat(1025));
    assert!(matches!(
        validate_template(&t),
        Err(TemplateError::InvalidRegex(_))
    ));
}

#[test]
fn validate_stdout_regex_size_fallback_accepts_small() {
    let mut t = base_template(TestKind::Shell, "echo hi");
    t.matchers.stdout_regex = Some("ok.*".to_string());
    assert_eq!(validate_template(&t), Ok(()));
}

#[test]
fn extract_referenced_placeholders_handles_escaped_braces() {
    let set = extract_referenced_placeholders("echo {{literal}} {real}");
    assert_eq!(set.len(), 1);
    assert!(set.contains("real"));
}

#[test]
fn extract_referenced_placeholders_ignores_invalid_charset() {
    let set = extract_referenced_placeholders("echo {with-dash}");
    assert!(set.is_empty());
}

#[test]
fn extract_referenced_placeholders_ignores_unclosed_brace() {
    let set = extract_referenced_placeholders("echo {oops");
    assert!(set.is_empty());
}

#[test]
fn extract_referenced_placeholders_dedupes() {
    let set = extract_referenced_placeholders("{a} {a} {b}");
    assert_eq!(set.len(), 2);
}
