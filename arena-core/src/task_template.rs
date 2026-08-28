//! Task template types used for storing and processing probe templates.
//!
//! These types are used by HTTP API handlers, the scheduler FSM, and the
//! player-agent WebSocket handler to parse and validate task templates stored
//! as JSON in the database.

use serde::{Deserialize, Serialize};

// ---------- TestKind ----------

/// Discriminator for the kind of probe a `TestTemplate` issues.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TestKind {
    Shell,
    HttpRequest,
    FileExists,
}

// ---------- Placeholder ----------

/// A template placeholder slot supplied by the runner before dispatch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Placeholder {
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub required: bool,
    #[serde(default)]
    pub secret: bool,
}

// ---------- Matchers ----------

fn default_exit_code() -> i32 {
    0
}
fn default_max_duration_ms() -> u64 {
    60_000
}
fn default_param_timeout_ms() -> u64 {
    60_000
}

/// Pass/fail predicates evaluated against a probe's stdout/exit code.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Matchers {
    #[serde(default = "default_exit_code")]
    pub expected_exit_code: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stdout_contains: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stdout_regex: Option<String>,
    #[serde(default = "default_max_duration_ms")]
    pub max_duration_ms: u64,
    #[serde(default = "default_param_timeout_ms")]
    pub param_timeout_ms: u64,
}

impl Default for Matchers {
    fn default() -> Self {
        Self {
            expected_exit_code: default_exit_code(),
            stdout_contains: None,
            stdout_regex: None,
            max_duration_ms: default_max_duration_ms(),
            param_timeout_ms: default_param_timeout_ms(),
        }
    }
}

// ---------- Backoff ----------

fn default_initial_ms() -> u64 {
    5_000
}
fn default_multiplier() -> f64 {
    2.0
}
fn default_max_ms() -> u64 {
    300_000
}
fn default_max_attempts() -> u32 {
    8
}

/// Exponential backoff parameters governing retry pacing.
///
/// `f64` precludes `Eq`; only `PartialEq` is derived.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Backoff {
    #[serde(default = "default_initial_ms")]
    pub initial_ms: u64,
    #[serde(default = "default_multiplier")]
    pub multiplier: f64,
    #[serde(default = "default_max_ms")]
    pub max_ms: u64,
    #[serde(default = "default_max_attempts")]
    pub max_attempts: u32,
}

impl Default for Backoff {
    fn default() -> Self {
        Self {
            initial_ms: default_initial_ms(),
            multiplier: default_multiplier(),
            max_ms: default_max_ms(),
            max_attempts: default_max_attempts(),
        }
    }
}

// ---------- Fixture types ----------

/// Concrete kind of a fixture variable definition.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FixtureKind {
    /// Sample one integer uniformly from [min, max] inclusive.
    NumericRange { min: i64, max: i64 },
    /// Pick one key-value pair uniformly at random from the pool.
    KeyValue {
        pairs: std::collections::HashMap<String, String>,
    },
}

/// A named fixture variable attached to a task's test template.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FixtureDef {
    pub name: String,
    #[serde(flatten)]
    pub kind: FixtureKind,
}

// ---------- TestTemplate ----------

/// Rendered probe template stored in the database and used by the scheduler.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TestTemplate {
    pub kind: TestKind,
    pub command_template: String,
    #[serde(default)]
    pub placeholders: Vec<Placeholder>,
    #[serde(default)]
    pub matchers: Matchers,
    #[serde(default)]
    pub backoff: Backoff,
    /// Variable definitions used to generate concrete probe instances.
    /// An empty list means the command is static (no substitution).
    #[serde(default)]
    pub fixtures: Vec<FixtureDef>,
    /// Template evaluated against sampled fixture values to produce the
    /// expected answer string. `None` means no server-side validation.
    #[serde(default)]
    pub answer_template: Option<String>,
}

/// A single test extracted from structured markdown.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StructuredTest {
    pub command_template: String,
    pub answer_template: String,
    pub fixture_definitions: String,
    /// The `## ` section heading, trimmed. Open-ended tasks reference their
    /// completion probe by this title.
    #[serde(default)]
    pub title: String,
    /// The section's prose (lines outside code fences), single-spaced. This
    /// is the author's own explanation of what the check verifies — the UI
    /// shows it under the probe's title so a failing bubble explains itself.
    #[serde(default)]
    pub description: String,
    /// Raw contents of the optional ` ```yaml probe ` fence. `None` = legacy
    /// participant shell probe. Parse with [`StructuredTest::parsed_probe_config`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub probe_config_yaml: Option<String>,
}

impl StructuredTest {
    /// Parse and validate the section's probe config, if it declares one.
    ///
    /// YAML is converted to JSON here so callers (seed validation, session
    /// materialization) share one parser and store one shape.
    pub fn parsed_probe_config(&self) -> Result<Option<crate::evaluation::ProbeConfig>, String> {
        let Some(raw) = self.probe_config_yaml.as_deref().map(str::trim) else {
            return Ok(None);
        };
        if raw.is_empty() {
            return Ok(None);
        }
        let yaml: serde_yaml::Value =
            serde_yaml::from_str(raw).map_err(|e| format!("yaml probe fence: {e}"))?;
        let json = serde_json::to_value(yaml).map_err(|e| format!("yaml probe fence: {e}"))?;
        let config = crate::evaluation::ProbeConfig::from_json(&json)?;
        let has_command = !self.command_template.trim().is_empty();
        crate::evaluation::validate_probe_config(&config, has_command)?;
        Ok(Some(config))
    }
}

/// Parse structured markdown tests from a test template document.
///
/// Expects sections headed by `## `, each containing up to four fenced
/// code blocks tagged by language and role:
///
/// - ` ```yaml probe ` — optional extended probe config (mode/executor/
///   schedule/…); its absence marks a legacy participant shell probe
/// - ` ```js fixtures ` — JS expression that evaluates to a fixture object
/// - ` ```sh command `  — shell command template (may contain `{name}` placeholders)
/// - ` ```js validation` — JS expression that validates the result
///
/// A section is emitted as a `StructuredTest` when it contains a non-empty
/// `sh command` block **or** a `yaml probe` fence (analysis/llm/interactive
/// probes have no command). Sections with neither are skipped.
pub fn parse_structured_markdown_tests(input: &str) -> Vec<StructuredTest> {
    let mut out: Vec<StructuredTest> = Vec::new();

    let mut in_section = false;
    let mut in_fence = false;
    let mut fence_lang = String::new();
    let mut fence_role = String::new();
    let mut fence_buf: Vec<&str> = Vec::new();

    let mut current_title = String::new();
    let mut current_prose: Vec<String> = Vec::new();
    let mut current_probe: Option<String> = None;
    let mut current_fixtures: Option<String> = None;
    let mut current_command: Option<String> = None;
    let mut current_validation: Option<String> = None;

    let flush = |out: &mut Vec<StructuredTest>,
                 title: &mut String,
                 prose: &mut Vec<String>,
                 probe: &mut Option<String>,
                 fx: &mut Option<String>,
                 cmd: &mut Option<String>,
                 val: &mut Option<String>| {
        let command = cmd
            .take()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        let probe_config_yaml = probe
            .take()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        if command.is_some() || probe_config_yaml.is_some() {
            let fixture_script = fx.take().unwrap_or_default();
            let fixture_definitions = serde_json::json!({
                "kind": "js",
                "script": fixture_script,
            })
            .to_string();
            let answer_template = val.take().unwrap_or_default().trim().to_string();
            out.push(StructuredTest {
                command_template: command.unwrap_or_default(),
                answer_template,
                fixture_definitions,
                title: std::mem::take(title),
                description: std::mem::take(prose).join(" "),
                probe_config_yaml,
            });
        } else {
            title.clear();
            prose.clear();
            *fx = None;
            *val = None;
        }
    };

    for line in input.lines() {
        let trimmed = line.trim();

        // Section header resets state and flushes any accumulated test.
        if trimmed.starts_with("## ") {
            if in_section {
                flush(
                    &mut out,
                    &mut current_title,
                    &mut current_prose,
                    &mut current_probe,
                    &mut current_fixtures,
                    &mut current_command,
                    &mut current_validation,
                );
            }
            in_section = true;
            in_fence = false;
            fence_lang.clear();
            fence_role.clear();
            fence_buf.clear();
            current_title = trimmed.trim_start_matches("## ").trim().to_string();
            current_prose.clear();
            continue;
        }

        if !in_section {
            continue;
        }

        if trimmed.starts_with("```") {
            if !in_fence {
                // Opening fence — parse language and role from info string.
                in_fence = true;
                let info = trimmed.trim_start_matches('`').trim();
                let mut parts = info.split_whitespace();
                fence_lang = parts.next().unwrap_or_default().to_ascii_lowercase();
                fence_role = parts.next().unwrap_or_default().to_ascii_lowercase();
                fence_buf.clear();
            } else {
                // Closing fence — route content to the right slot.
                let content = fence_buf.join("\n").trim().to_string();
                if fence_lang == "yaml" && fence_role == "probe" {
                    current_probe = Some(content);
                } else if fence_lang == "js" && fence_role == "fixtures" {
                    current_fixtures = Some(content);
                } else if fence_lang == "sh" && fence_role == "command" {
                    current_command = Some(content);
                } else if fence_lang == "js" && fence_role == "validation" {
                    current_validation = Some(content);
                }
                in_fence = false;
                fence_lang.clear();
                fence_role.clear();
                fence_buf.clear();
            }
            continue;
        }

        if in_fence {
            fence_buf.push(line);
        } else if !trimmed.is_empty() {
            // Section prose — the author's explanation of the check.
            current_prose.push(trimmed.to_string());
        }
    }

    // Flush the last section.
    if in_section {
        flush(
            &mut out,
            &mut current_title,
            &mut current_prose,
            &mut current_probe,
            &mut current_fixtures,
            &mut current_command,
            &mut current_validation,
        );
    }

    out
}
