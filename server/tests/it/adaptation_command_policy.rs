use server::adaptation::command_policy::{normalize_shell_command, split_when_safe};

#[test]
fn splits_agents_and_readme_into_two_atomic_commands() {
    let split = split_when_safe("test -f ./AGENTS.md && test -f ./README.md").expect("split");
    assert_eq!(split.len(), 2);
    assert_eq!(split[0], "test -f ./AGENTS.md");
    assert_eq!(split[1], "test -f ./README.md");
}

#[test]
fn strips_markdown_wrapper_to_shell_command() {
    let normalized = normalize_shell_command("```sh\ncat ./README.md\n```").expect("normalized");
    assert_eq!(normalized, "cat ./README.md");
}

#[test]
fn rejects_ambiguous_piped_multi_concern_command() {
    let err = split_when_safe("cat ./AGENTS.md | grep -q policy || cat ./README.md")
        .expect_err("ambiguous command must fail");
    assert_eq!(err.as_code(), "multi_concern_ambiguous");
}

#[test]
fn rejects_invalid_markdown_fence() {
    let err = normalize_shell_command("```sh\ncat ./README.md")
        .expect_err("missing closing fence must fail");
    assert_eq!(err.as_code(), "invalid_markdown_fence");
}
