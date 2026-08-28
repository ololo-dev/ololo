//! Coverage for `task_template::parse_structured_markdown_tests`.

use arena_core::task_template::parse_structured_markdown_tests;

#[test]
fn extracts_command_not_fixtures() {
    let md = r#"
## Check project structure files exist

Verify that the essential project structure files and directories have been initialized.

```js fixtures
({
  readme: "./README.md",
  packageJson: "./package.json",
  gitignore: "./.gitignore",
  srcDir: "./src"
})
```

```sh command
test -f {readme} && test -f {packageJson} && test -f {gitignore} && test -d {srcDir} && echo "true" || echo "false"
```

```js validation
result === "true"
```
"#;
    let tests = parse_structured_markdown_tests(md);
    assert_eq!(tests.len(), 1);
    assert_eq!(
        tests[0].command_template,
        r#"test -f {readme} && test -f {packageJson} && test -f {gitignore} && test -d {srcDir} && echo "true" || echo "false""#
    );
    assert!(tests[0].fixture_definitions.contains("\"kind\":\"js\""));
    assert!(tests[0].fixture_definitions.contains("readme"));
    assert!(tests[0].answer_template.contains("true"));
    // The section heading and its prose travel with the test — the player
    // chat shows them so a check bubble explains itself.
    assert_eq!(tests[0].title, "Check project structure files exist");
    assert_eq!(
        tests[0].description,
        "Verify that the essential project structure files and directories have been initialized."
    );
}

#[test]
fn description_joins_prose_and_skips_fences() {
    let md = "\n## Declare how to run the project\n\nAGENTS.md must contain a `run:` line.\nAGENTS.md wins when both declare one.\n\n```sh command\necho hi\n```\n\nTrailing prose after the fence also counts.\n";
    let tests = parse_structured_markdown_tests(md);
    assert_eq!(tests.len(), 1);
    assert_eq!(
        tests[0].description,
        "AGENTS.md must contain a `run:` line. AGENTS.md wins when both declare one. Trailing prose after the fence also counts."
    );
}

#[test]
fn multiple_sections() {
    let md = r#"
## Section A

```js fixtures
({ x: 1 })
```

```sh command
echo {x}
```

```js validation
result === "1"
```

## Section B

```sh command
echo hello
```
"#;
    let tests = parse_structured_markdown_tests(md);
    assert_eq!(tests.len(), 2);
    assert_eq!(tests[0].command_template, "echo {x}");
    assert_eq!(tests[1].command_template, "echo hello");
    // Section B has no fixtures — script should be empty string.
    assert!(tests[1].fixture_definitions.contains("\"script\":\"\""));
}

#[test]
fn section_without_command_is_skipped() {
    let md = r#"
## No command here

```js fixtures
({ a: 1 })
```
"#;
    let tests = parse_structured_markdown_tests(md);
    assert!(tests.is_empty());
}

#[test]
fn titles_are_captured_and_legacy_sections_have_no_probe_config() {
    let md = "## Round-trip\n\n```sh command\necho ok\n```\n";
    let tests = parse_structured_markdown_tests(md);
    assert_eq!(tests.len(), 1);
    assert_eq!(tests[0].title, "Round-trip");
    assert!(tests[0].probe_config_yaml.is_none());
    assert_eq!(tests[0].parsed_probe_config().unwrap(), None);
}

#[test]
fn yaml_probe_fence_is_parsed_and_validated() {
    let md = r#"
## Build stays green

```yaml probe
mode: deterministic
executor: participant
report: todo
schedule: { on: [interval], interval_secs: 120 }
points: { pass: 2 }
```

```sh command
cat TODO.md
```
"#;
    let tests = parse_structured_markdown_tests(md);
    assert_eq!(tests.len(), 1);
    let config = tests[0].parsed_probe_config().unwrap().expect("config");
    assert_eq!(
        config.mode,
        arena_core::evaluation::ProbeMode::Deterministic
    );
    assert_eq!(
        config.effective_executor(),
        arena_core::evaluation::ProbeExecutor::Participant
    );
    assert_eq!(config.points.unwrap().pass, 2);
    assert_eq!(
        config.report,
        Some(arena_core::evaluation::ReportKind::Todo)
    );
}

#[test]
fn commandless_section_with_probe_fence_is_emitted() {
    // Analysis/llm/interactive probes have no `sh command` block; the fence
    // alone makes the section a probe.
    let md = r#"
## Duplication stays sane

```yaml probe
mode: analysis
tool: jscpd
schedule: { on: [done] }
```

## Legacy without anything
just prose
"#;
    let tests = parse_structured_markdown_tests(md);
    assert_eq!(tests.len(), 1);
    assert_eq!(tests[0].title, "Duplication stays sane");
    assert_eq!(tests[0].command_template, "");
    let config = tests[0].parsed_probe_config().unwrap().expect("config");
    assert_eq!(config.mode, arena_core::evaluation::ProbeMode::Analysis);
    assert_eq!(config.tool.as_deref(), Some("jscpd"));
}

#[test]
fn bad_probe_fence_is_a_parse_error_not_a_panic() {
    let md = "## Broken\n\n```yaml probe\nmode: teleport\n```\n\n```sh command\necho hi\n```\n";
    let tests = parse_structured_markdown_tests(md);
    assert_eq!(tests.len(), 1);
    let err = tests[0].parsed_probe_config().unwrap_err();
    assert!(err.contains("probe config"), "{err}");
}

#[test]
fn legacy_documents_parse_identically_to_before() {
    // The exact shape older tasks rely on: command/fixtures/validation only.
    // The new fields must be inert — same sections, same command bytes.
    let md = r#"
## Answer a plain number with the number itself

```js fixtures
({ n: 7 })
```

```sh command
R={memory.run}
$R "{n}"
```

```js validation
assertEqual(result.trim(), String(n))
```
"#;
    let tests = parse_structured_markdown_tests(md);
    assert_eq!(tests.len(), 1);
    assert_eq!(tests[0].command_template, "R={memory.run}\n$R \"{n}\"");
    assert!(tests[0].probe_config_yaml.is_none());
    assert!(tests[0].answer_template.contains("assertEqual"));
}
