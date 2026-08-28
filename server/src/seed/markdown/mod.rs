//! Markdown project format loader.
//!
//! A markdown project is a directory containing:
//!
//! ```text
//! {name}/
//!   readme.md        # YAML frontmatter (schema_version + project block) + body = description
//!   tasks/
//!     0.<slug>.md    # YAML frontmatter (task fields + test_template) + body = command_template
//!     ...
//! ```
//!
//! The YAML frontmatter mirrors the `ExportEnvelope` / `ExportTask` shapes so
//! the loaded envelope flows through the same seeding/import validation as the
//! JSON format. The markdown body becomes the human-readable content: the
//! project description for `readme.md`, and the `command_template` (the Arena
//! test DSL) for each task file.

use std::path::Path;

use crate::api::admin_export_import::{
    ExportEnvelope, ExportProject, ExportTask, ExportTaskPoints, JudgeRef,
};
use crate::api::intervals::ExportTaskIntervals;

mod frontmatter;
use frontmatter::*;

#[cfg(test)]
mod tests;

// ---------- split frontmatter / body ----------

/// Split a markdown source into `(yaml_frontmatter, body)`.
///
/// Frontmatter is the text between `---\n` fences at the very start of the
/// file. The body is everything after the closing `---`. Trailing/leading
/// whitespace on the body is trimmed so the `command_template` is exactly what
/// the author wrote between the fences.
pub(crate) fn split_frontmatter<'a>(
    src: &'a str,
    file_label: &str,
) -> Result<(&'a str, &'a str), String> {
    let rest = src
        .strip_prefix("---\n")
        .ok_or_else(|| format!("{file_label}: missing leading '---' frontmatter fence"))?;
    let end = rest
        .find("\n---\n")
        .or_else(|| rest.find("\n---\r\n"))
        .ok_or_else(|| format!("{file_label}: missing closing '---' frontmatter fence"))?;
    let fm = &rest[..end];
    // +4 skips the closing fence + its newline so the body starts clean.
    let body_start = end + 4;
    let body = rest[body_start..].trim();
    // ponytail: simple offset math; safe because the indices come from byte
    // positions found on the same `&str` slice.
    Ok((fm, body))
}

// ---------- public loader ----------

/// Load a markdown project directory into an `ExportEnvelope`.
///
/// `dir` must contain `readme.md` and a `tasks/` subdirectory with one or more
/// `*.md` task files. Conventions:
///
/// - **slug** — derived from the project directory name (its final path
///   component). Frontmatter `slug` is ignored.
/// - **ordinal** — derived from the leading `<n>` in each task filename
///   (e.g. `0.setup.md` → ordinal `0`). Frontmatter `ordinal` is ignored.
///
/// Task files are sorted by filename so ordinals are stable regardless of
/// filesystem iteration order.
pub fn load_markdown_project(dir: &Path) -> Result<ExportEnvelope, String> {
    let readme_path = dir.join("readme.md");
    let readme_src =
        std::fs::read_to_string(&readme_path).map_err(|e| format!("readme.md: {e}"))?;
    let (fm, body) = split_frontmatter(&readme_src, "readme.md")?;
    let readme: ReadmeFrontmatter =
        serde_yaml::from_str(fm).map_err(|e| format!("readme.md frontmatter: {e}"))?;

    // Slug = directory name. This is the dedup key for seeding.
    let slug = dir
        .file_name()
        .and_then(|s| s.to_str())
        .map(|s| s.to_string())
        .ok_or_else(|| {
            format!(
                "readme.md: cannot derive slug from dir name {}",
                dir.display()
            )
        })?;

    // The readme body is the project description. If the frontmatter also
    // supplied a description, the body wins (it is the authored prose).
    let description = if !body.is_empty() {
        Some(body.to_string())
    } else {
        readme.project.description
    };

    // Resolve the project-level points defaults. If the `points:` block is
    // absent, fall back to the hard baseline. The resulting `ExportPoints`
    // is carried on the envelope and is the source of inheritance for any
    // task that omits a points field.
    let project_points = match &readme.project.points {
        Some(p) => p.resolved(),
        None => baseline_points(),
    };
    let project_intervals = match &readme.project.intervals {
        Some(i) => i.resolved(),
        None => baseline_intervals(),
    };

    // Bridge the optional `memory:` frontmatter block (YAML) to JSON and
    // validate it early so a bad schema fails the whole project load with a
    // pointed message instead of silently degrading at session time.
    let memory_schema: Option<serde_json::Value> = match &readme.project.memory {
        Some(y) => {
            let json: serde_json::Value = serde_yaml::from_value(y.clone())
                .map_err(|e| format!("readme.md frontmatter memory: {e}"))?;
            arena_core::memory::parse_memory_schema(&json)
                .map_err(|e| format!("readme.md frontmatter memory: {e}"))?;
            Some(json)
        }
        None => None,
    };

    let project = ExportProject {
        name: readme.project.name,
        slug: Some(slug),
        description,
        category: readme.project.category,
        tags: readme.project.tags,
        cover_image_url: readme.project.cover_image_url,
        public: readme.project.public,
        archived_at: readme.project.archived_at,
        points: project_points,
        intervals: project_intervals,
        session_duration_secs: readme.project.session_duration_secs.unwrap_or(3600),
        memory_schema,
        show_tasks: readme.project.show_tasks,
        parts: readme.project.parts,
    };

    // A campaign parent is a table of contents, not a playable project: it
    // carries the part list and nothing else, so its `tasks/` directory is
    // absent by design. Tasks there would never run (sessions on a parent are
    // refused), so finding both is an authoring mistake worth naming.
    let is_campaign_parent = !project.parts.is_empty();

    let tasks_dir = dir.join("tasks");
    let mut task_files: Vec<std::path::PathBuf> = match std::fs::read_dir(&tasks_dir) {
        Ok(entries) => entries
            .flatten()
            .filter(|e| e.path().extension().and_then(|s| s.to_str()) == Some("md"))
            .map(|e| e.path())
            .collect(),
        Err(e) if is_campaign_parent && e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(e) => return Err(format!("tasks dir: {e}")),
    };
    task_files.sort();

    if is_campaign_parent {
        if !task_files.is_empty() {
            return Err(format!(
                "campaign parent declares `parts:` and also has task files in {} — a parent hosts no sessions, so its tasks would never run",
                tasks_dir.display()
            ));
        }
        return Ok(ExportEnvelope {
            schema_version: readme.schema_version,
            project,
            tasks: Vec::new(),
        });
    }

    if task_files.is_empty() {
        return Err(format!("tasks: no *.md files in {}", tasks_dir.display()));
    }

    let mut tasks = Vec::with_capacity(task_files.len());
    for tf in &task_files {
        let label = tf.file_name().and_then(|s| s.to_str()).unwrap_or("task");
        let ordinal = parse_ordinal_from_filename(tf).ok_or_else(|| {
            format!("{label}: filename must start with '<n>.' where <n> is the ordinal")
        })?;
        let src = std::fs::read_to_string(tf).map_err(|e| format!("{label}: {e}"))?;
        let (fm, body) = split_frontmatter(&src, label)?;
        let task_fm: TaskFrontmatter =
            serde_yaml::from_str(fm).map_err(|e| format!("{label} frontmatter: {e}"))?;

        // The task body is the command_template. If the frontmatter also set
        // `content`, keep it; otherwise derive a one-line description from the
        // title so downstream consumers that display `content` are not empty.
        let content = if !task_fm.content.is_empty() {
            task_fm.content
        } else {
            task_fm.title.clone()
        };

        // The `fixtures` field is free-form YAML; carry it through as a JSON
        // value so the TestTemplate serde round-trip matches the JSON path.
        // serde_yaml::Value -> serde_json::Value via string round-trip is the
        // cheapest correct bridge without pulling a direct converter crate.
        let fixtures_json: serde_json::Value =
            serde_yaml::from_value(task_fm.test_template.fixtures.clone())
                .map(|v: serde_json::Value| v)
                .unwrap_or(serde_json::Value::Array(vec![]));

        let test_template = arena_core::task_template::TestTemplate {
            kind: parse_kind(&task_fm.test_template.kind, label)?,
            command_template: body.to_string(),
            placeholders: task_fm
                .test_template
                .placeholders
                .into_iter()
                .map(|p| arena_core::task_template::Placeholder {
                    name: p.name,
                    description: p.description,
                    required: p.required,
                    secret: p.secret,
                })
                .collect(),
            matchers: arena_core::task_template::Matchers {
                expected_exit_code: task_fm.test_template.matchers.expected_exit_code,
                stdout_contains: task_fm.test_template.matchers.stdout_contains,
                stdout_regex: task_fm.test_template.matchers.stdout_regex,
                max_duration_ms: task_fm.test_template.matchers.max_duration_ms,
                param_timeout_ms: task_fm.test_template.matchers.param_timeout_ms,
            },
            backoff: arena_core::task_template::Backoff {
                initial_ms: task_fm.test_template.backoff.initial_ms,
                multiplier: task_fm.test_template.backoff.multiplier,
                max_ms: task_fm.test_template.backoff.max_ms,
                max_attempts: task_fm.test_template.backoff.max_attempts,
            },
            fixtures: serde_json::from_value(fixtures_json).unwrap_or_default(),
            answer_template: task_fm.test_template.answer_template,
        };

        // Bridge the optional `evaluation:` block (YAML) to JSON. Contract
        // validation runs downstream (`validate_task_extras`), where the DSL
        // sections are available for the completion-probe check.
        let evaluation: Option<serde_json::Value> = match task_fm.evaluation {
            Some(y) => Some(
                serde_yaml::from_value(y)
                    .map_err(|e| format!("{label} frontmatter evaluation: {e}"))?,
            ),
            None => None,
        };

        tasks.push(ExportTask {
            ordinal,
            title: task_fm.title,
            content,
            test_template,
            tags: task_fm.tags,
            points: task_fm.points.map(|p| ExportTaskPoints {
                value: p.value,
                fail: p.fail,
                no_response: p.no_response,
                completion_bonus: p.completion_bonus,
            }),
            intervals: task_fm.intervals.map(|i| ExportTaskIntervals {
                deadline_secs: i.deadline_secs,
                min_interval_secs: i.min_interval_secs,
                interval_increment_secs: i.interval_increment_secs,
                max_interval_secs: i.max_interval_secs,
            }),
            judges: task_fm
                .judges
                .into_iter()
                .map(|j| match j {
                    JudgeRefFm::Slug(slug) => JudgeRef::Slug(slug),
                    JudgeRefFm::Weighted { slug, weight } => JudgeRef::Weighted { slug, weight },
                })
                .collect(),
            evaluation,
        });
    }

    Ok(ExportEnvelope {
        schema_version: readme.schema_version,
        project,
        tasks,
    })
}

/// Extract the leading ordinal from a task filename like `0.setup.md` → `0`.
fn parse_ordinal_from_filename(path: &Path) -> Option<i32> {
    let name = path.file_name()?.to_str()?;
    let dot = name.find('.')?;
    let n: i32 = name[..dot].parse().ok()?;
    Some(n)
}

fn parse_kind(raw: &str, label: &str) -> Result<arena_core::task_template::TestKind, String> {
    match raw {
        "shell" => Ok(arena_core::task_template::TestKind::Shell),
        "http_request" => Ok(arena_core::task_template::TestKind::HttpRequest),
        "file_exists" => Ok(arena_core::task_template::TestKind::FileExists),
        other => Err(format!(
            "{label}: unknown test_template.kind '{other}' (expected shell|http_request|file_exists)"
        )),
    }
}
