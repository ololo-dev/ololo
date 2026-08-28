use std::path::Path;

use uuid::Uuid;

use super::load_markdown_project;

fn write_project(root: &Path, slug: &str) -> std::path::PathBuf {
    let dir = root.join(slug);
    let tasks = dir.join("tasks");
    std::fs::create_dir_all(&tasks).unwrap();

    let readme = concat!(
        "---\n",
        "schema_version: 1\n",
        "project:\n",
        "  name: Markdown Sample\n",
        "  category: Productivity\n",
        "  tags: [web, markdown]\n",
        "  public: true\n",
        "---\n",
        "This is the project description body.\n",
    );
    std::fs::write(dir.join("readme.md"), readme).unwrap();

    let task0 = concat!(
        "---\n",
        "title: First task\n",
        "tags: [setup]\n",
        "points:\n",
        "  value: 10\n",
        "intervals:\n",
        "  deadline_secs: 300\n",
        "test_template:\n",
        "  kind: shell\n",
        "  placeholders:\n",
        "    - name: baseDir\n",
        "      description: auto\n",
        "  matchers:\n",
        "    expected_exit_code: 0\n",
        "    max_duration_ms: 60000\n",
        "    param_timeout_ms: 60000\n",
        "  backoff:\n",
        "    initial_ms: 1000\n",
        "    multiplier: 2\n",
        "    max_ms: 30000\n",
        "    max_attempts: 5\n",
        "---\n",
        "## Verify something\n",
        "\n",
        "```js fixtures\n",
        "({ baseDir: \".\" })\n",
        "```\n",
        "\n",
        "```sh command\n",
        "test -d {baseDir} && echo ok\n",
        "```\n",
        "\n",
        "```js validation\n",
        "result.trim() === 'ok'\n",
        "```\n",
    );
    std::fs::write(tasks.join("0.first-task.md"), task0).unwrap();

    dir
}

#[test]
fn loads_readme_and_task_into_envelope() {
    let root = tempdir();
    let dir = write_project(&root, "md-sample");

    let env = load_markdown_project(&dir).expect("load");
    assert_eq!(env.schema_version, 1);
    assert_eq!(env.project.name, "Markdown Sample");
    // slug derived from the directory name, not frontmatter.
    assert_eq!(env.project.slug.as_deref(), Some("md-sample"));
    assert_eq!(
        env.project.tags,
        vec!["web".to_string(), "markdown".to_string()]
    );
    assert!(env.project.public);
    assert_eq!(
        env.project.description.as_deref(),
        Some("This is the project description body.")
    );
    // No session_duration_secs in frontmatter → one-hour default.
    assert_eq!(env.project.session_duration_secs, 3600);

    assert_eq!(env.tasks.len(), 1);
    let t = &env.tasks[0];
    // ordinal derived from the filename's leading number.
    assert_eq!(t.ordinal, 0);
    assert_eq!(t.title, "First task");
    assert_eq!(t.tags, vec!["setup".to_string()]);
    assert_eq!(t.points.as_ref().and_then(|p| p.value), Some(10));
    assert_eq!(
        t.intervals.as_ref().and_then(|i| i.deadline_secs),
        Some(300)
    );
    assert_eq!(
        t.test_template.kind,
        arena_core::task_template::TestKind::Shell
    );
    assert_eq!(t.test_template.placeholders.len(), 1);
    assert_eq!(t.test_template.placeholders[0].name, "baseDir");
    assert!(
        t.test_template
            .command_template
            .contains("test -d {baseDir} && echo ok"),
        "command_template body should contain the sh command"
    );
}

#[test]
fn readme_session_duration_overrides_default() {
    let root = tempdir();
    let dir = write_project(&root, "md-duration");
    let readme = std::fs::read_to_string(dir.join("readme.md")).unwrap();
    let readme = readme.replace(
        "  public: true\n",
        "  public: true\n  session_duration_secs: 600\n",
    );
    std::fs::write(dir.join("readme.md"), readme).unwrap();

    let env = load_markdown_project(&dir).expect("load");
    assert_eq!(env.project.session_duration_secs, 600);
}

#[test]
fn body_wins_over_frontmatter_description() {
    let root = tempdir();
    let dir = write_project(&root, "md-sample");
    // Overwrite readme with a frontmatter description AND a body.
    let readme = concat!(
        "---\n",
        "schema_version: 1\n",
        "project:\n",
        "  name: Markdown Sample\n",
        "  description: frontmatter-desc\n",
        "---\n",
        "body-desc-wins\n",
    );
    std::fs::write(dir.join("readme.md"), readme).unwrap();

    let env = load_markdown_project(&dir).expect("load");
    assert_eq!(env.project.description.as_deref(), Some("body-desc-wins"));
}

#[test]
fn missing_frontmatter_fence_errors() {
    let root = tempdir();
    let dir = root.join("no-fence");
    std::fs::create_dir_all(dir.join("tasks")).unwrap();
    std::fs::write(dir.join("readme.md"), "no frontmatter here").unwrap();
    let err = load_markdown_project(&dir).unwrap_err();
    assert!(err.contains("frontmatter"), "err = {err}");
}

#[test]
fn empty_tasks_dir_errors() {
    let root = tempdir();
    let dir = root.join("empty-tasks");
    std::fs::create_dir_all(dir.join("tasks")).unwrap();
    std::fs::write(
        dir.join("readme.md"),
        "---\nschema_version: 1\nproject:\n  name: x\n---\nbody\n",
    )
    .unwrap();
    let err = load_markdown_project(&dir).unwrap_err();
    assert!(err.contains("no *.md files"), "err = {err}");
}

#[test]
fn tasks_sorted_by_filename() {
    let root = tempdir();
    let dir = root.join("sort-sample");
    let tasks = dir.join("tasks");
    std::fs::create_dir_all(&tasks).unwrap();
    std::fs::write(
        dir.join("readme.md"),
        "---\nschema_version: 1\nproject:\n  name: x\n---\nbody\n",
    )
    .unwrap();
    // Write tasks out of order; ordinals come from the filenames.
    for name in ["2.zeta.md", "0.alpha.md", "1.beta.md"] {
        std::fs::write(
            tasks.join(name),
            concat!(
                "---\ntitle: t\ntest_template:\n  kind: shell\n---\n",
                "```sh command\necho hi\n```\n\n```js validation\ntrue\n```",
            ),
        )
        .unwrap();
    }
    let env = load_markdown_project(&dir).expect("load");
    assert_eq!(
        env.tasks.iter().map(|t| t.ordinal).collect::<Vec<_>>(),
        vec![0, 1, 2]
    );
}

#[test]
fn default_kind_is_shell_when_omitted() {
    let root = tempdir();
    let dir = root.join("default-kind");
    let tasks = dir.join("tasks");
    std::fs::create_dir_all(&tasks).unwrap();
    std::fs::write(
        dir.join("readme.md"),
        "---\nschema_version: 1\nproject:\n  name: x\n---\nbody\n",
    )
    .unwrap();
    // No `kind` key in test_template.
    std::fs::write(
        tasks.join("0.t.md"),
        concat!(
            "---\ntitle: t\ntest_template:\n  placeholders: []\n---\n",
            "```sh command\necho hi\n```\n\n```js validation\ntrue\n```",
        ),
    )
    .unwrap();
    let env = load_markdown_project(&dir).expect("load");
    assert_eq!(
        env.tasks[0].test_template.kind,
        arena_core::task_template::TestKind::Shell
    );
}

#[test]
fn readme_points_become_envelope_project_points() {
    let root = tempdir();
    let dir = root.join("readme-points");
    let tasks = dir.join("tasks");
    std::fs::create_dir_all(&tasks).unwrap();
    std::fs::write(
        dir.join("readme.md"),
        concat!(
            "---\n",
            "schema_version: 1\n",
            "project:\n",
            "  name: x\n",
            "  points:\n",
            "    value: 42\n",
            "    fail: -1\n",
            "    no_response: -2\n",
            "    completion_bonus: 7\n",
            "---\n",
            "body\n",
        ),
    )
    .unwrap();
    std::fs::write(
        tasks.join("0.t.md"),
        concat!(
            "---\ntitle: t\ntest_template:\n  placeholders: []\n---\n",
            "```sh command\necho hi\n```\n\n```js validation\ntrue\n```",
        ),
    )
    .unwrap();
    let env = load_markdown_project(&dir).expect("load");
    assert_eq!(env.project.points.value, 42);
    assert_eq!(env.project.points.fail, -1);
    assert_eq!(env.project.points.no_response, -2);
    assert_eq!(env.project.points.completion_bonus, 7);
}

#[test]
fn readme_omitting_points_falls_back_to_hard_baseline() {
    let root = tempdir();
    let dir = root.join("readme-no-points");
    let tasks = dir.join("tasks");
    std::fs::create_dir_all(&tasks).unwrap();
    std::fs::write(
        dir.join("readme.md"),
        "---\nschema_version: 1\nproject:\n  name: x\n---\nbody\n",
    )
    .unwrap();
    std::fs::write(
        tasks.join("0.t.md"),
        concat!(
            "---\ntitle: t\ntest_template:\n  placeholders: []\n---\n",
            "```sh command\necho hi\n```\n\n```js validation\ntrue\n```",
        ),
    )
    .unwrap();
    let env = load_markdown_project(&dir).expect("load");
    // Hard baseline: 10 / -5 / -10 / 10.
    assert_eq!(env.project.points.value, 10);
    assert_eq!(env.project.points.fail, -5);
    assert_eq!(env.project.points.no_response, -10);
    assert_eq!(env.project.points.completion_bonus, 10);
}

#[test]
fn task_points_override_persisted_on_envelope() {
    let root = tempdir();
    let dir = root.join("task-override");
    let tasks = dir.join("tasks");
    std::fs::create_dir_all(&tasks).unwrap();
    std::fs::write(
        dir.join("readme.md"),
        concat!(
            "---\n",
            "schema_version: 1\n",
            "project:\n",
            "  name: x\n",
            "  points:\n",
            "    value: 10\n",
            "    fail: -5\n",
            "    no_response: -10\n",
            "    completion_bonus: 10\n",
            "---\n",
            "body\n",
        ),
    )
    .unwrap();
    // Task overrides only `value`; the other three fields are omitted
    // (inherit project default). The envelope carries them as `None` so
    // the import path can resolve them against the project default.
    std::fs::write(
        tasks.join("0.t.md"),
        concat!(
            "---\n",
            "title: t\n",
            "points:\n",
            "  value: 99\n",
            "test_template:\n",
            "  placeholders: []\n",
            "---\n",
            "```sh command\necho hi\n```\n\n```js validation\ntrue\n```",
        ),
    )
    .unwrap();
    let env = load_markdown_project(&dir).expect("load");
    let pts = env.tasks[0]
        .points
        .as_ref()
        .expect("task points block present");
    assert_eq!(pts.value, Some(99), "task value override carried through");
    assert_eq!(pts.fail, None, "fail omitted → inherits project default");
    assert_eq!(pts.no_response, None);
    assert_eq!(pts.completion_bonus, None);
}

fn tempdir() -> std::path::PathBuf {
    let mut p = std::env::temp_dir();
    p.push(format!("arena-md-project-test-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&p).unwrap();
    p
}

#[test]
fn a_campaign_parent_loads_without_a_tasks_directory() {
    // A parent is a table of contents: it declares its parts and nothing
    // else, so requiring `tasks/*.md` of it (as every other project) would
    // make the shape unauthorable.
    let root = tempdir();
    let dir = root.join("campaign");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("readme.md"),
        concat!(
            "---\n",
            "schema_version: 1\n",
            "project:\n",
            "  name: A Campaign\n",
            "  public: true\n",
            "  parts:\n",
            "    - part-one\n",
            "    - part-two\n",
            "---\n",
            "Play it one part at a time.\n",
        ),
    )
    .unwrap();

    let env = load_markdown_project(&dir).expect("campaign parent loads");
    assert_eq!(env.project.parts, vec!["part-one", "part-two"]);
    assert!(env.tasks.is_empty());
    assert_eq!(env.project.slug.as_deref(), Some("campaign"));
}

#[test]
fn a_campaign_parent_with_tasks_is_rejected() {
    // Tasks on a parent never run — sessions on a parent are refused — so
    // finding both means the author meant one of them.
    let root = tempdir();
    let dir = write_project(&root, "campaign-with-tasks");
    std::fs::write(
        dir.join("readme.md"),
        concat!(
            "---\n",
            "schema_version: 1\n",
            "project:\n",
            "  name: Confused\n",
            "  public: true\n",
            "  parts:\n",
            "    - part-one\n",
            "---\n",
            "body\n",
        ),
    )
    .unwrap();

    let err = load_markdown_project(&dir).expect_err("must reject");
    assert!(err.contains("campaign parent"), "{err}");
}
