// ENV_LOCK guards are deliberately held across awaits: they serialize the
// tests that mutate the process-global ARENA_PROJECTS_DIR env var, and the
// guarded sections must span the awaited seeding calls.
#![allow(clippy::await_holding_lock)]

use super::*;
use arena_core::entities::{projects, tasks, users};
use arena_core::task_template::{Backoff, Matchers, TestKind, TestTemplate};
use chrono::Utc;
use migration::{Migrator, MigratorTrait};
use sea_orm::{ActiveModelTrait, Database, PaginatorTrait, Set};
use std::sync::Mutex;

// Serialize tests that mutate ARENA_PROJECTS_DIR (env is process-global).
static ENV_LOCK: Mutex<()> = Mutex::new(());

async fn fresh_db() -> sea_orm::DatabaseConnection {
    let db = Database::connect("sqlite::memory:").await.expect("db");
    Migrator::up(&db, None).await.expect("migrate");
    db
}

async fn seed_admin(db: &sea_orm::DatabaseConnection) -> Uuid {
    let id = Uuid::new_v4();
    users::ActiveModel {
        id: Set(id),
        email: Set("admin@seed.test".to_string()),
        password_hash: Set(None),
        display_name: Set("Admin".to_string()),
        created_at: Set(Utc::now()),
        updated_at: Set(Utc::now()),
        is_admin: Set(true),
        avatar_url: Set(None),
        email_verified: Set(false),
        username: Set(None),
        plan: Set(arena_core::quota::PLAN_PREMIUM.to_string()),
        judge_run_limit: Set(None),
        judge_run_credits: Set(0),
    }
    .insert(db)
    .await
    .expect("insert admin");
    id
}

fn sample_template() -> TestTemplate {
    TestTemplate {
        kind: TestKind::Shell,
        command_template: "echo hi".to_string(),
        placeholders: vec![],
        matchers: Matchers::default(),
        backoff: Backoff::default(),
        fixtures: vec![],
        answer_template: Some("hi".to_string()),
    }
}

fn sample_envelope(slug: &str) -> ExportEnvelope {
    ExportEnvelope {
        schema_version: 1,
        project: crate::api::admin_export_import::ExportProject {
            name: "Seeded".to_string(),
            slug: Some(slug.to_string()),
            description: Some("seeded desc".to_string()),
            category: None,
            tags: vec!["alpha".to_string()],
            cover_image_url: None,
            public: true,
            archived_at: None,
            points: crate::api::admin_export_import::ExportPoints {
                value: 10,
                fail: -5,
                no_response: -10,
                completion_bonus: 10,
            },
            intervals: crate::api::intervals::ExportIntervals {
                deadline_secs: 60,
                min_interval_secs: 5,
                interval_increment_secs: 5,
                max_interval_secs: 60,
            },
            session_duration_secs: 3600,
            memory_schema: None,
            show_tasks: true,
            parts: Vec::new(),
        },
        tasks: vec![crate::api::admin_export_import::ExportTask {
            ordinal: 0,
            title: "t".to_string(),
            content: "c".to_string(),
            test_template: sample_template(),
            tags: vec![],
            points: None,
            intervals: None,
            judges: vec![],
            evaluation: None,
        }],
    }
}

fn write_dir() -> std::path::PathBuf {
    let mut p = std::env::temp_dir();
    p.push(format!("arena-seed-test-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&p).expect("mkdir");
    p
}

#[tokio::test]
async fn inserts_project_and_skips_on_rerun() {
    let _guard = ENV_LOCK.lock().unwrap();
    let db = fresh_db().await;
    let admin = seed_admin(&db).await;
    let dir = write_dir();

    let env1 = sample_envelope("first-project");
    let env2 = ExportEnvelope {
        schema_version: 1,
        project: crate::api::admin_export_import::ExportProject {
            name: "Second".to_string(),
            slug: Some("second-project".to_string()),
            description: None,
            category: None,
            tags: vec![],
            cover_image_url: None,
            public: false,
            archived_at: None,
            points: crate::api::admin_export_import::ExportPoints {
                value: 10,
                fail: -5,
                no_response: -10,
                completion_bonus: 10,
            },
            intervals: crate::api::intervals::ExportIntervals {
                deadline_secs: 60,
                min_interval_secs: 5,
                interval_increment_secs: 5,
                max_interval_secs: 60,
            },
            session_duration_secs: 3600,
            memory_schema: None,
            show_tasks: true,
            parts: Vec::new(),
        },
        tasks: vec![],
    };
    std::fs::write(dir.join("first.json"), serde_json::to_vec(&env1).unwrap()).unwrap();
    std::fs::write(dir.join("second.json"), serde_json::to_vec(&env2).unwrap()).unwrap();

    // safety: serialized by ENV_LOCK
    unsafe { std::env::set_var("ARENA_PROJECTS_DIR", &dir) };
    seed_projects(&db).await;

    let count = projects::Entity::find().count(&db).await.unwrap();
    assert_eq!(count, 2, "both projects should be inserted on first run");

    let first = projects::Entity::find()
        .filter(projects::Column::Slug.eq("first-project"))
        .one(&db)
        .await
        .unwrap()
        .expect("first project exists");
    assert_eq!(first.owner_user_id_fk, admin);
    assert!(first.public);

    let task_count = tasks::Entity::find()
        .filter(tasks::Column::ProjectIdFk.eq(first.id))
        .count(&db)
        .await
        .unwrap();
    assert_eq!(task_count, 1, "first project should have 1 task");

    // Re-run: nothing new inserted (slug skip).
    seed_projects(&db).await;
    let count = projects::Entity::find().count(&db).await.unwrap();
    assert_eq!(count, 2, "re-run should not duplicate");

    // safety: serialized by ENV_LOCK
    unsafe { std::env::remove_var("ARENA_PROJECTS_DIR") };
    drop(_guard);
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn skips_when_no_admin() {
    let _guard = ENV_LOCK.lock().unwrap();
    let db = fresh_db().await;
    let dir = write_dir();
    std::fs::write(
        dir.join("x.json"),
        serde_json::to_vec(&sample_envelope("x")).unwrap(),
    )
    .unwrap();
    // safety: serialized by ENV_LOCK
    unsafe { std::env::set_var("ARENA_PROJECTS_DIR", &dir) };
    seed_projects(&db).await;
    assert_eq!(projects::Entity::find().count(&db).await.unwrap(), 0);
    // safety: serialized by ENV_LOCK
    unsafe { std::env::remove_var("ARENA_PROJECTS_DIR") };
    drop(_guard);
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn missing_dir_is_silent() {
    let _guard = ENV_LOCK.lock().unwrap();
    let db = fresh_db().await;
    seed_admin(&db).await;
    // safety: serialized by ENV_LOCK
    unsafe { std::env::set_var("ARENA_PROJECTS_DIR", "/nonexistent/arena/seed/dir") };
    seed_projects(&db).await;
    assert_eq!(projects::Entity::find().count(&db).await.unwrap(), 0);
    // safety: serialized by ENV_LOCK
    unsafe { std::env::remove_var("ARENA_PROJECTS_DIR") };
    drop(_guard);
}

#[tokio::test]
async fn malformed_file_is_skipped() {
    let _guard = ENV_LOCK.lock().unwrap();
    let db = fresh_db().await;
    seed_admin(&db).await;
    let dir = write_dir();
    std::fs::write(dir.join("bad.json"), b"not json").unwrap();
    // safety: serialized by ENV_LOCK
    unsafe { std::env::set_var("ARENA_PROJECTS_DIR", &dir) };
    seed_projects(&db).await;
    assert_eq!(projects::Entity::find().count(&db).await.unwrap(), 0);
    // safety: serialized by ENV_LOCK
    unsafe { std::env::remove_var("ARENA_PROJECTS_DIR") };
    drop(_guard);
    std::fs::remove_dir_all(&dir).ok();
}

fn write_markdown_project(dir: &std::path::Path, slug: &str) {
    let proj_dir = dir.join(slug);
    let tasks = proj_dir.join("tasks");
    std::fs::create_dir_all(&tasks).unwrap();
    // No `slug` in frontmatter — derived from the directory name.
    let readme = concat!(
        "---\n",
        "schema_version: 1\n",
        "project:\n",
        "  name: MD Seeded\n",
        "  tags: [md]\n",
        "  public: true\n",
        "---\n",
        "Markdown project description body.\n",
    );
    std::fs::write(proj_dir.join("readme.md"), readme).unwrap();

    // No `ordinal` in frontmatter — derived from the filename's leading number.
    let task = concat!(
        "---\n",
        "title: MD task\n",
        "tags: [setup]\n",
        "points:\n",
        "  value: 10\n",
        "deadline_secs: 300\n",
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
    std::fs::write(tasks.join("0.md-task.md"), task).unwrap();
}

#[tokio::test]
async fn seeds_markdown_project_and_skips_on_rerun() {
    let _guard = ENV_LOCK.lock().unwrap();
    let db = fresh_db().await;
    let admin = seed_admin(&db).await;
    let dir = write_dir();
    write_markdown_project(&dir, "md-seeded");

    // safety: serialized by ENV_LOCK
    unsafe { std::env::set_var("ARENA_PROJECTS_DIR", &dir) };
    seed_projects(&db).await;

    let proj = projects::Entity::find()
        .filter(projects::Column::Slug.eq("md-seeded"))
        .one(&db)
        .await
        .unwrap()
        .expect("markdown project should be seeded");
    assert_eq!(proj.owner_user_id_fk, admin);
    assert!(proj.public);
    assert_eq!(proj.description, "Markdown project description body.");

    let task_count = tasks::Entity::find()
        .filter(tasks::Column::ProjectIdFk.eq(proj.id))
        .count(&db)
        .await
        .unwrap();
    assert_eq!(task_count, 1, "markdown project should have 1 task");

    // Re-run: slug skip keeps count stable.
    seed_projects(&db).await;
    assert_eq!(projects::Entity::find().count(&db).await.unwrap(), 1);

    // safety: serialized by ENV_LOCK
    unsafe { std::env::remove_var("ARENA_PROJECTS_DIR") };
    drop(_guard);
    std::fs::remove_dir_all(&dir).ok();
}

/// An open-ended project: evaluation contract in the task frontmatter,
/// weighted judges, a completion probe, and a commandless analysis section.
fn write_open_ended_project(dir: &std::path::Path, slug: &str) {
    let proj_dir = dir.join(slug);
    let tasks = proj_dir.join("tasks");
    std::fs::create_dir_all(&tasks).unwrap();
    let readme = concat!(
        "---\n",
        "schema_version: 1\n",
        "project:\n",
        "  name: Open Ended\n",
        "  tags: [open-ended]\n",
        "  public: true\n",
        "---\n",
        "Build the thing.\n",
    );
    std::fs::write(proj_dir.join("readme.md"), readme).unwrap();

    let task = concat!(
        "---\n",
        "title: Build from the brief\n",
        "tags: [product]\n",
        "points:\n",
        "  value: 200\n",
        "judges:\n",
        "  - slug: code-cleanliness\n",
        "    weight: 2.5\n",
        "  - code-analysis\n",
        "evaluation:\n",
        "  kind: open_ended\n",
        "  completion: { probe: \"TODO complete\", deadline_secs: 5400 }\n",
        "  criteria:\n",
        "    - { key: product, title: Matches the brief, weight: 0.6 }\n",
        "    - { key: cleanliness, weight: 0.4 }\n",
        "---\n",
        "## TODO complete\n",
        "```yaml probe\n",
        "mode: deterministic\n",
        "executor: participant\n",
        "schedule: { on: [interval], interval_secs: 120 }\n",
        "```\n",
        "```sh command\n",
        "test -f TODO.md && ! grep -q '^- \\[ \\]' TODO.md && echo done\n",
        "```\n",
        "```js validation\n",
        "result.trim() === 'done'\n",
        "```\n",
        "\n",
        "## Duplication stays sane\n",
        "```yaml probe\n",
        "mode: analysis\n",
        "tool: jscpd\n",
        "schedule: { on: [done] }\n",
        "```\n",
    );
    std::fs::write(tasks.join("0.build.md"), task).unwrap();
}

fn write_judge_file(dir: &std::path::Path, slug: &str) {
    let judge = format!(
        "---\nname: {slug}\nrating_scale: {{min: 0.0, max: 10.0, step: 0.1}}\n---\nJudge body.\n"
    );
    std::fs::write(dir.join(format!("{slug}.md")), judge).unwrap();
}

#[tokio::test]
async fn seeds_open_ended_project_with_contract_and_weights() {
    let _guard = ENV_LOCK.lock().unwrap();
    let db = fresh_db().await;
    seed_admin(&db).await;
    let dir = write_dir();
    write_open_ended_project(&dir, "open-ended");

    // The task references judges by slug; seed them first, as boot does.
    // The fixture must parse and validate on its own — a load error here
    // gives the reason instead of a silent skip in seed_projects.
    let env = crate::seed::markdown::load_markdown_project(&dir.join("open-ended"))
        .expect("open-ended markdown project parses");
    for task in &env.tasks {
        crate::api::admin_export_import::validate_task_extras(task)
            .expect("open-ended task extras validate");
    }

    let judges_dir = dir.join("judges");
    std::fs::create_dir_all(&judges_dir).unwrap();
    write_judge_file(&judges_dir, "code-cleanliness");
    write_judge_file(&judges_dir, "code-analysis");
    crate::seed::judges::seed_judges_with_dir(&db, &judges_dir)
        .await
        .unwrap();

    // Seed the envelope directly so a failure carries its reason instead of
    // a silent skip inside seed_projects.
    let admin_id = arena_core::entities::users::Entity::find()
        .one(&db)
        .await
        .unwrap()
        .expect("admin exists")
        .id;
    match seed_envelope(&db, &env, std::path::Path::new("open-ended"), admin_id).await {
        SeedOutcome::Inserted(_) => {}
        SeedOutcome::Skipped(why) => panic!("seed skipped: {why}"),
        SeedOutcome::Failed(why) => panic!("seed failed: {why}"),
    }
    drop(_guard);

    let proj = projects::Entity::find()
        .filter(projects::Column::Slug.eq("open-ended"))
        .one(&db)
        .await
        .unwrap()
        .expect("open-ended project seeded");

    let task = tasks::Entity::find()
        .filter(tasks::Column::ProjectIdFk.eq(proj.id))
        .one(&db)
        .await
        .unwrap()
        .expect("task row");
    let contract = arena_core::evaluation::EvaluationContract::from_json(
        task.evaluation.as_ref().expect("evaluation stored"),
    )
    .expect("contract parses back");
    assert_eq!(contract.completion.probe, "TODO complete");
    assert_eq!(contract.criteria.len(), 2);
    assert_eq!(contract.limits.interactive_probes_per_task, 2);

    let mut attached = arena_core::entities::task_judges::Entity::find()
        .filter(arena_core::entities::task_judges::Column::TaskId.eq(task.id))
        .all(&db)
        .await
        .unwrap();
    attached.sort_by_key(|tj| tj.ordinal);
    assert_eq!(attached.len(), 2);
    assert_eq!(attached[0].weight, Some(2.5), "weighted judge keeps weight");
    assert_eq!(attached[1].weight, None, "bare slug has no weight");

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn open_ended_contract_naming_a_missing_section_fails_the_seed() {
    let _guard = ENV_LOCK.lock().unwrap();
    let db = fresh_db().await;
    seed_admin(&db).await;
    let dir = write_dir();
    write_open_ended_project(&dir, "open-ended-bad");
    // Break the contract: completion probe names a section that is not there.
    let task_path = dir.join("open-ended-bad/tasks/0.build.md");
    let src = std::fs::read_to_string(&task_path).unwrap();
    std::fs::write(
        &task_path,
        src.replace("probe: \"TODO complete\"", "probe: \"No such section\""),
    )
    .unwrap();

    let judges_dir = dir.join("judges");
    std::fs::create_dir_all(&judges_dir).unwrap();
    write_judge_file(&judges_dir, "code-cleanliness");
    write_judge_file(&judges_dir, "code-analysis");
    crate::seed::judges::seed_judges_with_dir(&db, &judges_dir)
        .await
        .unwrap();

    // safety: serialized by ENV_LOCK
    unsafe { std::env::set_var("ARENA_PROJECTS_DIR", &dir) };
    seed_projects(&db).await;
    // safety: serialized by ENV_LOCK
    unsafe { std::env::remove_var("ARENA_PROJECTS_DIR") };
    drop(_guard);

    assert!(
        projects::Entity::find()
            .filter(projects::Column::Slug.eq("open-ended-bad"))
            .one(&db)
            .await
            .unwrap()
            .is_none(),
        "a contract naming a missing completion section must fail the seed"
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn real_weather_widget_md_seeds() {
    // Round-trip gate for the single-task Weather Widget project: one
    // open-ended build task carrying the evaluation contract, completed by
    // the .ololo/weather-widget-done.md flag file and scored by the judge panel.
    let Some(real) = fixture_project("weather-widget") else {
        return;
    };
    let env =
        super::markdown::load_markdown_project(&real).expect("real markdown project should load");
    assert_eq!(env.schema_version, 1);
    assert_eq!(env.project.slug.as_deref(), Some("weather-widget"));
    assert_eq!(
        env.tasks.len(),
        3,
        "build task + forecast task + live task expected"
    );

    // Panel-agnostic invariants: the public and private checkouts ship this
    // project with DIFFERENT judge panels (a single reviewer vs. the full
    // panel), so this gate pins consistency, not a roster. Every judge a
    // task names must ship as a file, and every contract criterion (minus
    // the session-scoped workflow pair) must be scored by some judge on the
    // panel — an uncovered criterion silently nulls a slice of the score.
    let judge_criteria: std::collections::HashMap<String, Vec<String>> = {
        let dir = real.parent().and_then(|p| p.parent()).expect("repo root");
        crate::seed::judges::load_judge_defs(&dir.join("judges"))
            .0
            .into_iter()
            .map(|(_, d)| {
                let criteria: Vec<String> = d
                    .criteria
                    .as_deref()
                    .and_then(|c| serde_json::from_str(c).ok())
                    .unwrap_or_default();
                (d.slug, criteria)
            })
            .collect()
    };
    let session_scoped = ["agentic", "skills"];
    for task in &env.tasks {
        for j in &task.judges {
            assert!(
                judge_criteria.contains_key(j.slug()),
                "task {} names judge '{}' which does not ship in judges/",
                task.ordinal,
                j.slug()
            );
        }
        let contract = arena_core::evaluation::EvaluationContract::from_json(
            task.evaluation.as_ref().expect("evaluation contract"),
        )
        .expect("contract parses");
        for c in &contract.criteria {
            if session_scoped.contains(&c.key.as_str()) {
                continue;
            }
            let covered = task.judges.iter().any(|j| {
                judge_criteria
                    .get(j.slug())
                    .is_some_and(|keys| keys.iter().any(|k| k == &c.key))
            });
            assert!(
                covered,
                "task {} criterion '{}' is scored by no attached judge",
                task.ordinal, c.key
            );
        }
    }

    for task in &env.tasks {
        arena_core::validation::validate_template(&task.test_template)
            .unwrap_or_else(|e| panic!("task {} template invalid: {e}", task.ordinal));
        crate::api::admin_export_import::validate_task_extras(task)
            .unwrap_or_else(|e| panic!("task {} extras invalid: {e}", task.ordinal));

        let contract = arena_core::evaluation::EvaluationContract::from_json(
            task.evaluation.as_ref().expect("evaluation contract"),
        )
        .expect("contract parses");
        assert_eq!(contract.completion.probe, "Definition of done");
        assert_eq!(task.judges[0].weight(), Some(1.5));
    }

    // The forecast and live tasks are interactive steps: their contracts
    // bind the judges to screencast verification.
    for ordinal in [1usize, 2] {
        let task = &env.tasks[ordinal];
        let contract = arena_core::evaluation::EvaluationContract::from_json(
            task.evaluation.as_ref().expect("evaluation contract"),
        )
        .expect("contract parses");
        let constraints = contract
            .constraints
            .unwrap_or_else(|| panic!("task {ordinal} has constraints"));
        assert!(
            constraints.contains("screencast"),
            "task {ordinal} demands screencast verification: {constraints}"
        );
    }

    // The live task additionally binds the judges to data-quality review.
    let live = &env.tasks[2];
    let contract = arena_core::evaluation::EvaluationContract::from_json(
        live.evaluation.as_ref().expect("evaluation contract"),
    )
    .expect("contract parses");
    let constraints = contract.constraints.expect("live task has constraints");
    assert!(
        constraints.contains("data quality"),
        "the live contract demands data-quality review: {constraints}"
    );
}

#[test]
fn weather_widget_fixtures_and_validations_evaluate() {
    // Every `js fixtures` block must evaluate, and every validation must
    // pass on a synthesized correct answer and fail on a corrupted one.
    let Some(real) = fixture_project("weather-widget") else {
        return;
    };
    let env = super::markdown::load_markdown_project(&real).expect("load weather-widget");

    let check = |tpl: &str,
                 fx: &serde_json::Map<String, serde_json::Value>,
                 good: &str,
                 bad: &str,
                 label: &str| {
        let ok = arena_core::probe_engine::eval_js_validation_outcome(tpl, fx, good)
            .unwrap_or_else(|e| panic!("{label}: validation eval: {e}"));
        assert!(
            ok.pass,
            "{label}: correct answer must pass (answer: {good})"
        );
        let no = arena_core::probe_engine::eval_js_validation_outcome(tpl, fx, bad)
            .unwrap_or_else(|e| panic!("{label}: validation eval: {e}"));
        assert!(!no.pass, "{label}: corrupted answer must fail");
    };

    for task in &env.tasks {
        let sections = arena_core::task_template::parse_structured_markdown_tests(
            &task.test_template.command_template,
        );
        assert!(!sections.is_empty(), "task {} has sections", task.ordinal);
        for (si, st) in sections.iter().enumerate() {
            let label = format!("task {} section {si} ({})", task.ordinal, st.title);
            let def: serde_json::Value =
                serde_json::from_str(&st.fixture_definitions).expect("fixture def json");
            let script = def["script"].as_str().unwrap_or("");
            if script.is_empty() {
                continue; // analysis/llm sections carry no fixtures
            }
            for _ in 0..12 {
                let fx = arena_core::probe_engine::eval_js_fixtures_with_meta(script)
                    .unwrap_or_else(|e| panic!("{label}: fixtures: {e}"))
                    .fixtures;
                if st.answer_template.trim().is_empty() {
                    continue; // the TODO report probe measures, not grades
                }
                match st.title.as_str() {
                    "Definition of done" => {
                        check(
                            &st.answer_template,
                            &fx,
                            "done-note: present",
                            "not-done: .ololo/weather-widget-done.md is missing - write it when the widget is ready",
                            &label,
                        );
                    }
                    "Declare how to run the project" | "Declare how to test the project" => {
                        check(&st.answer_template, &fx, "npm test", "", &label);
                    }
                    other => panic!("uncovered graded section: {other}"),
                }
            }
        }
    }
}

#[tokio::test]
async fn real_reinvent_the_wheel_ls_md_seeds() {
    // Round-trip gate for the first "Reinvent the Wheel" project: a linear
    // ladder of deterministic tasks (ES-CLI shape) that build up an `ls`
    // clone flag by flag, probed via the session-memory `run:` command
    // against randomly-named fixture trees.
    let Some(real) = fixture_project("reinvent-the-wheel-ls") else {
        return;
    };
    let env =
        super::markdown::load_markdown_project(&real).expect("real markdown project should load");
    assert_eq!(env.schema_version, 1);
    assert_eq!(env.project.slug.as_deref(), Some("reinvent-the-wheel-ls"));
    assert_eq!(
        env.project.category.as_deref(),
        Some("Reinvent the Wheel"),
        "the project carries the new category"
    );
    assert_eq!(env.tasks.len(), 10, "setup + nine flag tasks expected");

    // Session memory: the frontmatter `memory:` block must surface as a valid
    // schema declaring the run command default.
    let mem = env
        .project
        .memory_schema
        .as_ref()
        .expect("memory schema declared in readme frontmatter");
    let defaults = arena_core::memory::parse_memory_schema(mem).expect("valid memory schema");
    assert_eq!(defaults.get("run").map(String::as_str), Some("sh myls.sh"));

    for task in &env.tasks {
        arena_core::validation::validate_template(&task.test_template)
            .unwrap_or_else(|e| panic!("task {} template invalid: {e}", task.ordinal));
        crate::api::admin_export_import::validate_task_extras(task)
            .unwrap_or_else(|e| panic!("task {} extras invalid: {e}", task.ordinal));
        assert!(
            task.evaluation.is_none(),
            "task {} is classic probe-verified, not open-ended",
            task.ordinal
        );
        // Every rung carries the anti-cheat pair: the generic one, and the
        // category's own from-scratch judge (no wrapping the real ls).
        let judge_slugs: Vec<&str> = task.judges.iter().map(|j| j.slug()).collect();
        assert_eq!(
            judge_slugs,
            ["task-anti-cheat", "from-scratch"],
            "task {} judges",
            task.ordinal
        );
    }
}

#[test]
fn reinvent_the_wheel_ls_fixtures_and_validations_evaluate() {
    // Every `js fixtures` block must evaluate, and every validation must
    // pass on a synthesized correct listing and fail on a corrupted one.
    let Some(real) = fixture_project("reinvent-the-wheel-ls") else {
        return;
    };
    let env = super::markdown::load_markdown_project(&real).expect("load reinvent-the-wheel-ls");

    let check = |tpl: &str,
                 fx: &serde_json::Map<String, serde_json::Value>,
                 good: &str,
                 bad: &str,
                 label: &str| {
        let ok = arena_core::probe_engine::eval_js_validation_outcome(tpl, fx, good)
            .unwrap_or_else(|e| panic!("{label}: validation eval: {e}"));
        assert!(
            ok.pass,
            "{label}: correct answer must pass (answer: {good})"
        );
        let no = arena_core::probe_engine::eval_js_validation_outcome(tpl, fx, bad)
            .unwrap_or_else(|e| panic!("{label}: validation eval: {e}"));
        assert!(
            !no.pass,
            "{label}: corrupted answer must fail (answer: {bad})"
        );
    };
    let s = |fx: &serde_json::Map<String, serde_json::Value>, key: &str| -> String {
        fx[key]
            .as_str()
            .unwrap_or_else(|| panic!("fixture {key} is a string"))
            .to_string()
    };

    for task in &env.tasks {
        let sections = arena_core::task_template::parse_structured_markdown_tests(
            &task.test_template.command_template,
        );
        assert!(!sections.is_empty(), "task {} has sections", task.ordinal);
        for (si, st) in sections.iter().enumerate() {
            let label = format!("task {} section {si} ({})", task.ordinal, st.title);
            let def: serde_json::Value =
                serde_json::from_str(&st.fixture_definitions).expect("fixture def json");
            let script = def["script"].as_str().unwrap_or("");
            assert!(!script.is_empty(), "{label}: fixtures present");
            for _ in 0..12 {
                let fx = arena_core::probe_engine::eval_js_fixtures_with_meta(script)
                    .unwrap_or_else(|e| panic!("{label}: fixtures: {e}"))
                    .fixtures;
                let (good, bad) = match st.title.as_str() {
                    // Any language and entry point is allowed, so the run/test
                    // declarations only require a non-empty extracted command.
                    "Declare how to run the project" => ("sh myls.sh".to_string(), String::new()),
                    "Declare how to test the project" => ("sh test.sh".to_string(), String::new()),
                    "Verify myls.sh exists" => ("ok".to_string(), "missing".to_string()),
                    "List one file" => (s(&fx, "f"), String::new()),
                    "Alphabetical, one name per line" => {
                        let (a, k, t, z) = (s(&fx, "a"), s(&fx, "k"), s(&fx, "t"), s(&fx, "z"));
                        (format!("{a}\n{k}\n{t}\n{z}"), format!("{z}\n{a}\n{k}\n{t}"))
                    }
                    "Hidden by default" => {
                        let (v, h) = (s(&fx, "v"), s(&fx, "h"));
                        (v.clone(), format!("{v}\n.{h}"))
                    }
                    "-a shows everything" => {
                        let (v, h) = (s(&fx, "v"), s(&fx, "h"));
                        (format!(".\n..\n.{h}\n{v}"), v)
                    }
                    "A missing path fails politely" => {
                        let ghost = s(&fx, "ghost");
                        (
                            format!("exit=1\ncannot access {ghost}: no such file or directory"),
                            format!("exit=0\n{ghost}"),
                        )
                    }
                    "The long format" => {
                        let (f, d) = (s(&fx, "f"), s(&fx, "d"));
                        let size: i64 = s(&fx, "size").parse().expect("size is an integer string");
                        (
                            format!(
                                "-rw-r--r--  1 u g {size} Aug 14 12:00 {f}\ndrwxr-xr-x  2 u g 64 Aug 14 12:00 {d}"
                            ),
                            format!(
                                "-rw-r--r--  1 u g {} Aug 14 12:00 {f}\ndrwxr-xr-x  2 u g 64 Aug 14 12:00 {d}",
                                size + 1
                            ),
                        )
                    }
                    "Sizes for humans" => {
                        let (big, small) = (s(&fx, "big"), s(&fx, "small"));
                        let k: i64 = s(&fx, "k").parse().expect("k is an integer string");
                        let sz: i64 = s(&fx, "s").parse().expect("s is an integer string");
                        (
                            format!(
                                "-rw-r--r--  1 u g {k}.0K Aug 14 12:00 {big}\n-rw-r--r--  1 u g {sz} Aug 14 12:00 {small}"
                            ),
                            format!(
                                "-rw-r--r--  1 u g {} Aug 14 12:00 {big}\n-rw-r--r--  1 u g {sz} Aug 14 12:00 {small}",
                                k * 1024
                            ),
                        )
                    }
                    "Newest first" => {
                        let (a, m, z) = (s(&fx, "a"), s(&fx, "m"), s(&fx, "z"));
                        (format!("{m}\n{a}\n{z}"), format!("{a}\n{m}\n{z}"))
                    }
                    "Largest first" => {
                        let (a, m, z) = (s(&fx, "a"), s(&fx, "m"), s(&fx, "z"));
                        (format!("{m}\n{z}\n{a}"), format!("{a}\n{m}\n{z}"))
                    }
                    "-r reverses the alphabet" => {
                        let (a, m, z) = (s(&fx, "a"), s(&fx, "m"), s(&fx, "z"));
                        (format!("{z}\n{m}\n{a}"), format!("{a}\n{m}\n{z}"))
                    }
                    "-tr walks time backwards" => {
                        let (a, m, z) = (s(&fx, "a"), s(&fx, "m"), s(&fx, "z"));
                        (format!("{z}\n{a}\n{m}"), format!("{m}\n{a}\n{z}"))
                    }
                    "Every directory gets its section" => {
                        let (f, sub, g) = (s(&fx, "f"), s(&fx, "s"), s(&fx, "g"));
                        (
                            format!("{f}\n{sub}\n\n.ololo/tmp/ls-x/{sub}:\n{g}"),
                            format!("{f}\n{sub}\n{g}"),
                        )
                    }
                    other => panic!("uncovered graded section: {other}"),
                };
                check(&st.answer_template, &fx, &good, &bad, &label);
            }
        }
    }
}

#[tokio::test]
async fn real_scorched_md_seeds() {
    // Round-trip gate for the four-round Scorched build: the core artillery
    // duel, then wind/arsenal/economy, then falling terrain and the computer
    // rival, then online multiplayer with a persistent hall of fame. Each
    // round is an open-ended judged build completed by its own
    // .ololo/scorched-*-done.md flag and scored by the same weighted judge
    // panel; all three are interactive, so their contracts bind the judges to
    // screencast verification.
    let Some(real) = fixture_project("scorched") else {
        return;
    };
    let env =
        super::markdown::load_markdown_project(&real).expect("real markdown project should load");
    assert_eq!(env.schema_version, 1);
    assert_eq!(env.project.slug.as_deref(), Some("scorched"));
    assert_eq!(
        env.tasks.len(),
        4,
        "core + arsenal + polish + multiplayer tasks expected"
    );

    // The workflow judge is session-scoped: one verdict about the whole
    // session, paid out of one task's panel share. Listed on every task it
    // would take a slice of each task's budget and score only one of them, so
    // the seed attaches it exactly once per project.
    assert_eq!(
        env.tasks
            .iter()
            .filter(|t| t.judges.iter().any(|j| j.slug() == "agentic"))
            .count(),
        1,
        "the workflow judge is attached to exactly one task"
    );

    for task in &env.tasks {
        arena_core::validation::validate_template(&task.test_template)
            .unwrap_or_else(|e| panic!("task {} template invalid: {e}", task.ordinal));
        crate::api::admin_export_import::validate_task_extras(task)
            .unwrap_or_else(|e| panic!("task {} extras invalid: {e}", task.ordinal));
    }

    for task in &env.tasks {
        let contract = arena_core::evaluation::EvaluationContract::from_json(
            task.evaluation.as_ref().expect("evaluation contract"),
        )
        .expect("contract parses");
        assert_eq!(contract.completion.probe, "Definition of done");
        let judge_slugs: Vec<&str> = task
            .judges
            .iter()
            .map(|j| j.slug())
            .filter(|s| *s != "agentic")
            .collect();
        assert_eq!(
            judge_slugs,
            [
                "correctness",
                "architecture",
                "data",
                "ux-review",
                "code-quality",
                "test-quality",
                "creativity"
            ]
        );
        assert_eq!(task.judges[0].weight(), Some(1.5));

        // All three rounds are interactive: their contracts demand a screencast.
        let constraints = contract
            .constraints
            .unwrap_or_else(|| panic!("task {} has constraints", task.ordinal));
        assert!(
            constraints.contains("screencast"),
            "task {} demands screencast verification: {constraints}",
            task.ordinal
        );
    }
}

#[tokio::test]
async fn real_tetris_md_seeds() {
    // Round-trip gate for the three-round Tetris build: core game, then
    // scoring/levels/next, then polish. Each round is an open-ended judged
    // build completed by its own .ololo/tetris-*-done.md flag and scored by
    // the same weighted judge panel; all three are interactive, so their
    // contracts bind the judges to screencast verification.
    let Some(real) = fixture_project("tetris") else {
        return;
    };
    let env =
        super::markdown::load_markdown_project(&real).expect("real markdown project should load");
    assert_eq!(env.schema_version, 1);
    assert_eq!(env.project.slug.as_deref(), Some("tetris"));
    assert_eq!(env.tasks.len(), 3, "core + scoring + polish tasks expected");

    // The workflow judge is session-scoped: one verdict about the whole
    // session, paid out of one task's panel share. Listed on every task it
    // would take a slice of each task's budget and score only one of them, so
    // the seed attaches it exactly once per project.
    assert_eq!(
        env.tasks
            .iter()
            .filter(|t| t.judges.iter().any(|j| j.slug() == "agentic"))
            .count(),
        1,
        "the workflow judge is attached to exactly one task"
    );

    for task in &env.tasks {
        arena_core::validation::validate_template(&task.test_template)
            .unwrap_or_else(|e| panic!("task {} template invalid: {e}", task.ordinal));
        crate::api::admin_export_import::validate_task_extras(task)
            .unwrap_or_else(|e| panic!("task {} extras invalid: {e}", task.ordinal));
    }

    // Every round is open-ended: the same contract shape and weighted panel.
    for task in &env.tasks {
        let contract = arena_core::evaluation::EvaluationContract::from_json(
            task.evaluation.as_ref().expect("evaluation contract"),
        )
        .expect("contract parses");
        assert_eq!(contract.completion.probe, "Definition of done");
        // `agentic` is scored by a session-scoped judge — one verdict for the
        // whole session, paid out of one task's panel share — so the seed
        // carries it on a single task per project. The rest of the panel is
        // per task and identical everywhere, which is what this pins.
        let keys: Vec<&str> = contract
            .criteria
            .iter()
            .map(|c| c.key.as_str())
            .filter(|k| *k != "agentic" && *k != "skills")
            .collect();
        assert_eq!(
            keys,
            [
                "product",
                "architecture",
                "data",
                "ux",
                "accessibility",
                "mobile",
                "cleanliness",
                "maintainability",
                "tests",
                "creativity"
            ]
        );
        let judge_slugs: Vec<&str> = task
            .judges
            .iter()
            .map(|j| j.slug())
            .filter(|s| *s != "agentic")
            .collect();
        assert_eq!(
            judge_slugs,
            [
                "correctness",
                "architecture",
                "data",
                "ux-review",
                "code-quality",
                "test-quality",
                "creativity"
            ]
        );
        assert_eq!(task.judges[0].weight(), Some(1.5));

        // All three rounds are interactive: their contracts demand a screencast.
        let constraints = contract
            .constraints
            .unwrap_or_else(|| panic!("task {} has constraints", task.ordinal));
        assert!(
            constraints.contains("screencast"),
            "task {} demands screencast verification: {constraints}",
            task.ordinal
        );
    }
}

#[test]
fn tetris_fixtures_and_validations_evaluate() {
    // Every `js fixtures` block must evaluate, and the completion validation
    // must pass on the done-note marker and fail on a not-done message.
    let Some(real) = fixture_project("tetris") else {
        return;
    };
    let env = super::markdown::load_markdown_project(&real).expect("load tetris");

    for task in &env.tasks {
        let sections = arena_core::task_template::parse_structured_markdown_tests(
            &task.test_template.command_template,
        );
        assert!(!sections.is_empty(), "task {} has sections", task.ordinal);
        for (si, st) in sections.iter().enumerate() {
            let label = format!("task {} section {si} ({})", task.ordinal, st.title);
            let def: serde_json::Value =
                serde_json::from_str(&st.fixture_definitions).expect("fixture def json");
            let script = def["script"].as_str().unwrap_or("");
            if script.is_empty() {
                continue; // analysis/llm sections carry no fixtures
            }
            let fx = arena_core::probe_engine::eval_js_fixtures_with_meta(script)
                .unwrap_or_else(|e| panic!("{label}: fixtures: {e}"))
                .fixtures;
            if st.answer_template.trim().is_empty() {
                continue;
            }
            // The run/test declaration probes grade a non-empty extracted
            // command; every other graded section is the completion check.
            if st.title == "Declare how to run the project"
                || st.title == "Declare how to test the project"
            {
                let ok = arena_core::probe_engine::eval_js_validation_outcome(
                    &st.answer_template,
                    &fx,
                    "npm test",
                )
                .unwrap_or_else(|e| panic!("{label}: validation eval: {e}"));
                assert!(ok.pass, "{label}: a declared command passes");
                let no = arena_core::probe_engine::eval_js_validation_outcome(
                    &st.answer_template,
                    &fx,
                    "",
                )
                .unwrap_or_else(|e| panic!("{label}: validation eval: {e}"));
                assert!(!no.pass, "{label}: a missing declaration fails");
                continue;
            }
            assert_eq!(st.title, "Definition of done", "uncovered graded section");
            let ok = arena_core::probe_engine::eval_js_validation_outcome(
                &st.answer_template,
                &fx,
                "done-note: present",
            )
            .unwrap_or_else(|e| panic!("{label}: validation eval: {e}"));
            assert!(ok.pass, "{label}: done-note marker must pass");
            let no = arena_core::probe_engine::eval_js_validation_outcome(
                &st.answer_template,
                &fx,
                "not-done: flag missing",
            )
            .unwrap_or_else(|e| panic!("{label}: validation eval: {e}"));
            assert!(!no.pass, "{label}: a not-done message must fail");
        }
    }
}

#[tokio::test]
async fn seeds_both_json_and_markdown_in_same_dir() {
    let _guard = ENV_LOCK.lock().unwrap();
    let db = fresh_db().await;
    seed_admin(&db).await;
    let dir = write_dir();

    // JSON file
    std::fs::write(
        dir.join("j.json"),
        serde_json::to_vec(&sample_envelope("json-one")).unwrap(),
    )
    .unwrap();
    // Markdown subdirectory
    write_markdown_project(&dir, "md-one");

    // safety: serialized by ENV_LOCK
    unsafe { std::env::set_var("ARENA_PROJECTS_DIR", &dir) };
    seed_projects(&db).await;

    let count = projects::Entity::find().count(&db).await.unwrap();
    assert_eq!(count, 2, "both json and markdown projects should seed");

    // safety: serialized by ENV_LOCK
    unsafe { std::env::remove_var("ARENA_PROJECTS_DIR") };
    drop(_guard);
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn real_flower_watering_reminder_md_seeds() {
    // Verifies the committed markdown project file loads and validates
    // end-to-end through the real seeding path.
    let Some(real) = fixture_project("flower-watering-reminder") else {
        return;
    };
    let env =
        super::markdown::load_markdown_project(&real).expect("real markdown project should load");
    assert_eq!(env.schema_version, 1);
    // slug is derived from the directory name.
    assert_eq!(
        env.project.slug.as_deref(),
        Some("flower-watering-reminder")
    );
    assert_eq!(env.tasks.len(), 7, "7 task files expected");

    // Full template validation (placeholder parity, matchers, backoff).
    for task in &env.tasks {
        arena_core::validation::validate_template(&task.test_template)
            .unwrap_or_else(|e| panic!("task {} template invalid: {e}", task.ordinal));
    }
}

#[tokio::test]
async fn real_extreme_startup_cli_md_seeds() {
    // Round-trip gate for the CLI edition of Extreme Startup.
    let Some(real) = fixture_project("extreme-startup-cli") else {
        return;
    };
    let env =
        super::markdown::load_markdown_project(&real).expect("real markdown project should load");
    assert_eq!(env.schema_version, 1);
    assert_eq!(env.project.slug.as_deref(), Some("extreme-startup-cli"));
    assert_eq!(env.tasks.len(), 18, "18 task files expected");

    // Session memory: the frontmatter `memory:` block must surface as a valid
    // schema declaring the run command default.
    let mem = env
        .project
        .memory_schema
        .as_ref()
        .expect("memory schema declared in readme frontmatter");
    let defaults = arena_core::memory::parse_memory_schema(mem).expect("valid memory schema");
    assert_eq!(
        defaults.get("run").map(String::as_str),
        Some("sh answer.sh")
    );

    for task in &env.tasks {
        arena_core::validation::validate_template(&task.test_template)
            .unwrap_or_else(|e| panic!("task {} template invalid: {e}", task.ordinal));
    }
}

#[tokio::test]
async fn real_extreme_startup_browser_md_seeds() {
    // Round-trip gate for the browser edition of Extreme Startup.
    let Some(real) = fixture_project("extreme-startup-browser") else {
        return;
    };
    let env =
        super::markdown::load_markdown_project(&real).expect("real markdown project should load");
    assert_eq!(env.schema_version, 1);
    assert_eq!(env.project.slug.as_deref(), Some("extreme-startup-browser"));
    assert_eq!(env.tasks.len(), 18, "18 task files expected");

    for task in &env.tasks {
        arena_core::validation::validate_template(&task.test_template)
            .unwrap_or_else(|e| panic!("task {} template invalid: {e}", task.ordinal));
    }
}

#[tokio::test]
async fn extreme_startup_browser_dom_probe_validates_a_compliant_page() {
    // Regression for the probe that zeroed session US2KZ6 (0/17, -30): the
    // brief demands an EMPTY #answer on a fresh page, but the old capture
    // (`$(get text; echo present)`) turned that empty text into a leading
    // newline, `answer=(\w+)` never matched, and a fully compliant page
    // failed task 0 forever. The rewritten probe reports presence as a word.
    let Some(real) = fixture_project("extreme-startup-browser") else {
        return;
    };
    let env = super::markdown::load_markdown_project(&real).expect("load extreme-startup-browser");
    let task0 = env
        .tasks
        .iter()
        .find(|t| t.ordinal == 0)
        .expect("task 0 present");
    let sections = arena_core::task_template::parse_structured_markdown_tests(
        &task0.test_template.command_template,
    );
    let dom = sections
        .iter()
        .find(|st| st.command_template.contains("answer=$answerEl"))
        .expect("DOM-elements section present");
    // The presence flag must be produced by exit code, never by capturing
    // the element's text — an empty element must still read as present.
    assert!(
        dom.command_template
            .contains("&& echo present || echo missing"),
        "presence is derived from the exit code"
    );
    let fx = arena_core::probe_engine::eval_js_fixtures_with_meta(
        serde_json::from_str::<serde_json::Value>(&dom.fixture_definitions).expect("fixture json")
            ["script"]
            .as_str()
            .expect("js fixture script"),
    )
    .expect("fixtures eval")
    .fixtures;
    // Mirror the grader: an evaluation error (`m && …` yields null when the
    // regex misses — not a boolean) fails closed, exactly as
    // `grade_stdout`'s `unwrap_or_default()` treats it.
    let validate = |stdout: &str| {
        arena_core::probe_engine::eval_js_validation_outcome(&dom.answer_template, &fx, stdout)
            .map(|o| o.pass)
            .unwrap_or(false)
    };
    assert!(
        validate("inputs=1 buttons=1 answer=present"),
        "a compliant page (empty #answer included) passes"
    );
    assert!(
        !validate("inputs=1 buttons=1 answer=missing"),
        "a page without #answer fails"
    );
    assert!(
        !validate("inputs=0 buttons=1 answer=present"),
        "a page without a text input fails"
    );
    assert!(
        !validate("inputs=1 buttons=1 answer=\npresent"),
        "the old broken capture shape stays a failure, not a silent pass"
    );
}

#[tokio::test]
async fn real_hop_hop_md_seeds() {
    // Round-trip gate for the Coding Golf (hop) project.
    let Some(real) = fixture_project("hop-hop") else {
        return;
    };
    let env =
        super::markdown::load_markdown_project(&real).expect("real markdown project should load");
    assert_eq!(env.schema_version, 1);
    assert_eq!(env.project.slug.as_deref(), Some("hop-hop"));
    assert_eq!(env.tasks.len(), 11, "11 task files expected");
    assert_eq!(
        env.project.session_duration_secs, 600,
        "hop-hop is a ten-minute sprint"
    );

    for task in &env.tasks {
        arena_core::validation::validate_template(&task.test_template)
            .unwrap_or_else(|e| panic!("task {} template invalid: {e}", task.ordinal));
    }
}

#[tokio::test]
async fn real_fizzbuzz_md_seeds() {
    // Round-trip gate for the Coding Golf (FizzBuzz) project.
    let Some(real) = fixture_project("fizzbuzz") else {
        return;
    };
    let env =
        super::markdown::load_markdown_project(&real).expect("real markdown project should load");
    assert_eq!(env.schema_version, 1);
    assert_eq!(env.project.slug.as_deref(), Some("fizzbuzz"));
    assert_eq!(env.tasks.len(), 11, "11 task files expected");
    assert_eq!(
        env.project.session_duration_secs, 600,
        "fizzbuzz is a ten-minute sprint"
    );

    for task in &env.tasks {
        arena_core::validation::validate_template(&task.test_template)
            .unwrap_or_else(|e| panic!("task {} template invalid: {e}", task.ordinal));
    }
}

/// End-to-end JS gate for hop-hop: every `js fixtures` block must
/// evaluate in the probe engine, and every `js validation` block must
/// pass when fed the correct answer (computed independently in Rust)
/// and fail on a corrupted one. Fixture sampling is randomized, so run
/// each section repeatedly to cover the category predicates.
#[test]
fn hop_hop_fixtures_and_validations_evaluate() {
    fn hop(s: &str) -> String {
        let t = s.trim();
        // boa serializes fixture numbers as f64 ("42.0"); JS `String(n)`
        // renders "42", so normalize to the integer form.
        let n = t.parse::<f64>().expect("numeric fixture") as i64;
        let t = n.to_string();
        let t = t.as_str();
        let div = n % 3 == 0;
        let has = t.contains('3');
        match (div, has) {
            (true, true) => "hop-hop".to_string(),
            (false, false) => t.to_string(),
            _ => "hop".to_string(),
        }
    }

    let Some(real) = fixture_project("hop-hop") else {
        return;
    };
    let env = super::markdown::load_markdown_project(&real).expect("load hop-hop");

    for task in &env.tasks {
        let tests = arena_core::task_template::parse_structured_markdown_tests(
            &task.test_template.command_template,
        );
        assert!(
            !tests.is_empty(),
            "task {} has structured tests",
            task.ordinal
        );
        for (si, st) in tests.iter().enumerate() {
            let def: serde_json::Value =
                serde_json::from_str(&st.fixture_definitions).expect("fixture def json");
            let script = def["script"].as_str().expect("js fixture script");
            for _ in 0..25 {
                let fx = arena_core::probe_engine::eval_js_fixtures_with_meta(script)
                    .unwrap_or_else(|e| panic!("task {} section {si} fixtures: {e}", task.ordinal))
                    .fixtures;

                // Sections without n/list are the byte-budget checks
                // (validation compares against a `<=` limit; a tiny
                // count must pass, an absurd one must fail) or the
                // task-0 declaration check (any non-empty run command
                // passes).
                let correct = if let Some(n) = fx.get("n") {
                    hop(&n.to_string())
                } else if let Some(list) = fx.get("list").and_then(|v| v.as_str()) {
                    list.split(',').map(hop).collect::<Vec<_>>().join(",")
                } else if !st.answer_template.contains("<=") {
                    let decl = arena_core::probe_engine::eval_js_validation_outcome(
                        &st.answer_template,
                        &fx,
                        "node answer.js",
                    )
                    .unwrap_or_else(|e| {
                        panic!("task {} section {si} decl validation: {e}", task.ordinal)
                    });
                    assert!(
                        decl.pass,
                        "task {} section {si}: declared run command passes",
                        task.ordinal
                    );
                    let empty = arena_core::probe_engine::eval_js_validation_outcome(
                        &st.answer_template,
                        &fx,
                        "  ",
                    )
                    .expect("decl validation evaluates");
                    assert!(
                        !empty.pass,
                        "task {} section {si}: missing declaration must fail",
                        task.ordinal
                    );
                    continue;
                } else {
                    let small = arena_core::probe_engine::eval_js_validation_outcome(
                        &st.answer_template,
                        &fx,
                        "1",
                    )
                    .unwrap_or_else(|e| {
                        panic!("task {} section {si} size validation: {e}", task.ordinal)
                    });
                    assert!(
                        small.pass,
                        "task {} section {si}: 1 byte fits",
                        task.ordinal
                    );
                    let huge = arena_core::probe_engine::eval_js_validation_outcome(
                        &st.answer_template,
                        &fx,
                        "100000",
                    )
                    .expect("size validation evaluates");
                    assert!(
                        !huge.pass,
                        "task {} section {si}: 100000 bytes must not fit",
                        task.ordinal
                    );
                    continue;
                };

                let ok = arena_core::probe_engine::eval_js_validation_outcome(
                    &st.answer_template,
                    &fx,
                    &correct,
                )
                .unwrap_or_else(|e| panic!("task {} section {si} validation: {e}", task.ordinal));
                assert!(
                    ok.pass,
                    "task {} section {si}: correct answer {correct:?} must pass ({fx:?})",
                    task.ordinal
                );

                let bad = arena_core::probe_engine::eval_js_validation_outcome(
                    &st.answer_template,
                    &fx,
                    &format!("{correct}x"),
                )
                .expect("wrong-answer validation evaluates");
                assert!(
                    !bad.pass,
                    "task {} section {si}: corrupted answer must fail ({fx:?})",
                    task.ordinal
                );
            }
        }
    }
}

/// End-to-end JS gate for fizzbuzz: every `js fixtures` block must
/// evaluate in the probe engine, and every `js validation` block must
/// pass when fed the correct answer (computed independently in Rust)
/// and fail on a corrupted one. Fixture sampling is randomized, so run
/// each section repeatedly to cover the category predicates.
#[test]
fn fizzbuzz_fixtures_and_validations_evaluate() {
    fn fizzbuzz(s: &str) -> String {
        let t = s.trim();
        // boa serializes fixture numbers as f64 ("42.0"); JS `String(n)`
        // renders "42", so normalize to the integer form.
        let n = t.parse::<f64>().expect("numeric fixture") as i64;
        let t = n.to_string();
        let f = n % 3 == 0;
        let b = n % 5 == 0;
        match (f, b) {
            (true, true) => "FizzBuzz".to_string(),
            (true, false) => "Fizz".to_string(),
            (false, true) => "Buzz".to_string(),
            (false, false) => t,
        }
    }

    let Some(real) = fixture_project("fizzbuzz") else {
        return;
    };
    let env = super::markdown::load_markdown_project(&real).expect("load fizzbuzz");

    for task in &env.tasks {
        let tests = arena_core::task_template::parse_structured_markdown_tests(
            &task.test_template.command_template,
        );
        assert!(
            !tests.is_empty(),
            "task {} has structured tests",
            task.ordinal
        );
        for (si, st) in tests.iter().enumerate() {
            let def: serde_json::Value =
                serde_json::from_str(&st.fixture_definitions).expect("fixture def json");
            let script = def["script"].as_str().expect("js fixture script");
            for _ in 0..25 {
                let fx = arena_core::probe_engine::eval_js_fixtures_with_meta(script)
                    .unwrap_or_else(|e| panic!("task {} section {si} fixtures: {e}", task.ordinal))
                    .fixtures;

                // Sections without n/list are the byte-budget checks
                // (validation compares against a `<=` limit; a tiny
                // count must pass, an absurd one must fail) or the
                // task-0 declaration check (any non-empty run command
                // passes).
                let correct = if let Some(n) = fx.get("n") {
                    fizzbuzz(&n.to_string())
                } else if let Some(list) = fx.get("list").and_then(|v| v.as_str()) {
                    list.split(',').map(fizzbuzz).collect::<Vec<_>>().join(",")
                } else if !st.answer_template.contains("<=") {
                    let decl = arena_core::probe_engine::eval_js_validation_outcome(
                        &st.answer_template,
                        &fx,
                        "node answer.js",
                    )
                    .unwrap_or_else(|e| {
                        panic!("task {} section {si} decl validation: {e}", task.ordinal)
                    });
                    assert!(
                        decl.pass,
                        "task {} section {si}: declared run command passes",
                        task.ordinal
                    );
                    let empty = arena_core::probe_engine::eval_js_validation_outcome(
                        &st.answer_template,
                        &fx,
                        "  ",
                    )
                    .expect("decl validation evaluates");
                    assert!(
                        !empty.pass,
                        "task {} section {si}: missing declaration must fail",
                        task.ordinal
                    );
                    continue;
                } else {
                    let small = arena_core::probe_engine::eval_js_validation_outcome(
                        &st.answer_template,
                        &fx,
                        "1",
                    )
                    .unwrap_or_else(|e| {
                        panic!("task {} section {si} size validation: {e}", task.ordinal)
                    });
                    assert!(
                        small.pass,
                        "task {} section {si}: 1 byte fits",
                        task.ordinal
                    );
                    let huge = arena_core::probe_engine::eval_js_validation_outcome(
                        &st.answer_template,
                        &fx,
                        "100000",
                    )
                    .expect("size validation evaluates");
                    assert!(
                        !huge.pass,
                        "task {} section {si}: 100000 bytes must not fit",
                        task.ordinal
                    );
                    continue;
                };

                let ok = arena_core::probe_engine::eval_js_validation_outcome(
                    &st.answer_template,
                    &fx,
                    &correct,
                )
                .unwrap_or_else(|e| panic!("task {} section {si} validation: {e}", task.ordinal));
                assert!(
                    ok.pass,
                    "task {} section {si}: correct answer {correct:?} must pass ({fx:?})",
                    task.ordinal
                );

                let bad = arena_core::probe_engine::eval_js_validation_outcome(
                    &st.answer_template,
                    &fx,
                    &format!("{correct}x"),
                )
                .expect("wrong-answer validation evaluates");
                assert!(
                    !bad.pass,
                    "task {} section {si}: corrupted answer must fail ({fx:?})",
                    task.ordinal
                );
            }
        }
    }
}

#[tokio::test]
async fn real_repeat_each_character_md_seeds() {
    // Round-trip gate for the Code Golf (repeat each character) project.
    let Some(real) = fixture_project("repeat-each-character") else {
        return;
    };
    let env =
        super::markdown::load_markdown_project(&real).expect("real markdown project should load");
    assert_eq!(env.schema_version, 1);
    assert_eq!(env.project.slug.as_deref(), Some("repeat-each-character"));
    assert_eq!(env.tasks.len(), 10, "task 0 plus nine golf rungs");
    assert_eq!(
        env.project.session_duration_secs, 600,
        "repeat-each-character is a ten-minute sprint"
    );

    // The run command is session memory: the probes render `{memory.run}`,
    // so the schema must declare that key with the task-0 starting contract
    // as its default (memory is only extracted once a task completes).
    let mem = env
        .project
        .memory_schema
        .as_ref()
        .expect("memory schema declared in readme frontmatter");
    let defaults = arena_core::memory::parse_memory_schema(mem).expect("valid memory schema");
    assert_eq!(
        defaults.get("run").map(String::as_str),
        Some("sh answer.sh")
    );

    for task in &env.tasks {
        arena_core::validation::validate_template(&task.test_template)
            .unwrap_or_else(|e| panic!("task {} template invalid: {e}", task.ordinal));
    }
}

/// End-to-end JS gate for repeat-each-character: every `js fixtures` block
/// must evaluate in the probe engine, and every `js validation` block must
/// pass when fed the correct answer (computed independently in Rust) and
/// fail on a corrupted one. Fixture sampling is randomized, so run each
/// section repeatedly.
#[test]
fn repeat_each_character_fixtures_and_validations_evaluate() {
    fn repeat_each(s: &str, n: i64) -> String {
        s.chars()
            .flat_map(|c| std::iter::repeat_n(c, n as usize))
            .collect()
    }

    let Some(real) = fixture_project("repeat-each-character") else {
        return;
    };
    let env = super::markdown::load_markdown_project(&real).expect("load repeat-each-character");

    for task in &env.tasks {
        let tests = arena_core::task_template::parse_structured_markdown_tests(
            &task.test_template.command_template,
        );
        assert!(
            !tests.is_empty(),
            "task {} has structured tests",
            task.ordinal
        );
        for (si, st) in tests.iter().enumerate() {
            let def: serde_json::Value =
                serde_json::from_str(&st.fixture_definitions).expect("fixture def json");
            let script = def["script"].as_str().expect("js fixture script");
            for _ in 0..25 {
                let fx = arena_core::probe_engine::eval_js_fixtures_with_meta(script)
                    .unwrap_or_else(|e| panic!("task {} section {si} fixtures: {e}", task.ordinal))
                    .fixtures;

                // Sections carrying s/n are the kata probes. The rest are the
                // byte budget (`<=` against a size), the task-0 file check
                // (compares against "ok") and the task-0 declaration check
                // (any non-empty run command passes).
                let correct = match (fx.get("s").and_then(|v| v.as_str()), fx.get("n")) {
                    (Some(s), Some(n)) => {
                        // boa serializes fixture numbers as f64.
                        let n = n.as_f64().expect("numeric count") as i64;
                        assert!((1..=5).contains(&n), "count stays in 1..=5, got {n}");
                        repeat_each(s, n)
                    }
                    _ => {
                        let (good, bad) = if st.answer_template.contains("<=") {
                            ("1", "100000")
                        } else if st.answer_template.contains("\"ok\"") {
                            ("ok", "missing")
                        } else {
                            ("python3 g.py", "  ")
                        };
                        let pass = arena_core::probe_engine::eval_js_validation_outcome(
                            &st.answer_template,
                            &fx,
                            good,
                        )
                        .unwrap_or_else(|e| {
                            panic!("task {} section {si} validation: {e}", task.ordinal)
                        });
                        assert!(
                            pass.pass,
                            "task {} section {si}: {good:?} must pass",
                            task.ordinal
                        );
                        let fail = arena_core::probe_engine::eval_js_validation_outcome(
                            &st.answer_template,
                            &fx,
                            bad,
                        )
                        .expect("validation evaluates");
                        assert!(
                            !fail.pass,
                            "task {} section {si}: {bad:?} must fail",
                            task.ordinal
                        );
                        continue;
                    }
                };

                let ok = arena_core::probe_engine::eval_js_validation_outcome(
                    &st.answer_template,
                    &fx,
                    &correct,
                )
                .unwrap_or_else(|e| panic!("task {} section {si} validation: {e}", task.ordinal));
                assert!(
                    ok.pass,
                    "task {} section {si}: correct answer {correct:?} must pass ({fx:?})",
                    task.ordinal
                );

                let bad = arena_core::probe_engine::eval_js_validation_outcome(
                    &st.answer_template,
                    &fx,
                    &format!("{correct}x"),
                )
                .expect("wrong-answer validation evaluates");
                assert!(
                    !bad.pass,
                    "task {} section {si}: corrupted answer must fail ({fx:?})",
                    task.ordinal
                );
            }
        }
    }
}

/// Every markdown project in this checkout parses, and every task's
/// intervals survive resolution against the project defaults.
///
/// Enumerates whatever ships rather than naming projects: the gate is
/// "nothing on disk is corrupt", not an inventory, so a build carrying a
/// different set of challenges is still covered.
///
/// ponytail: the contract specifies a byte-identical re-export diff, but
/// no markdown writer exists in this crate and building one for a one-time
/// gate is YAGNI. The parse + validate gate catches corruption from the
/// flat→nested rewrite. Add the re-export diff when a markdown exporter
/// is implemented for round-trip parity.
#[test]
fn all_markdown_projects_parse_and_validate_intervals() {
    let Some(root) = fixture_projects_root() else {
        return;
    };

    let mut projects_checked = 0;
    for entry in std::fs::read_dir(&root).expect("read projects dir") {
        let dir = entry.expect("dir entry").path();
        if !dir.join("readme.md").is_file() {
            continue;
        }
        let slug = dir.file_name().unwrap_or_default().to_string_lossy();

        let env = super::markdown::load_markdown_project(&dir)
            .unwrap_or_else(|e| panic!("load {slug}: {e}"));
        projects_checked += 1;

        let proj_int = crate::api::intervals::ExportIntervals {
            deadline_secs: env.project.intervals.deadline_secs,
            min_interval_secs: env.project.intervals.min_interval_secs,
            interval_increment_secs: env.project.intervals.interval_increment_secs,
            max_interval_secs: env.project.intervals.max_interval_secs,
        };

        for task in &env.tasks {
            let ti = task.intervals.clone().unwrap_or_default();
            let resolved = crate::api::intervals::resolve_intervals(
                ti.deadline_secs,
                ti.min_interval_secs,
                ti.interval_increment_secs,
                ti.max_interval_secs,
                &proj_int,
            );
            crate::api::intervals::validate_resolved_intervals(&resolved)
                .unwrap_or_else(|e| panic!("{slug} task {} intervals: {e}", task.ordinal));
        }
    }

    assert!(
        projects_checked > 0,
        "expected at least one markdown project under {}",
        root.display()
    );
}

#[tokio::test]
async fn tops_up_judges_on_existing_project() {
    use arena_core::entities::{judges, task_judges};

    let _guard = ENV_LOCK.lock().unwrap();
    let db = fresh_db().await;
    seed_admin(&db).await;
    let dir = write_dir();

    // First run: seed definition has no judges yet.
    let env = sample_envelope("topup-project");
    std::fs::write(dir.join("p.json"), serde_json::to_vec(&env).unwrap()).unwrap();
    // safety: serialized by ENV_LOCK
    unsafe { std::env::set_var("ARENA_PROJECTS_DIR", &dir) };
    seed_projects(&db).await;
    assert_eq!(
        task_judges::Entity::find().count(&db).await.unwrap(),
        0,
        "no judges attached on first run"
    );

    // The seed definition gains a judge attachment; the judge exists.
    let judge_id = Uuid::new_v4();
    judges::ActiveModel {
        id: Set(judge_id),
        slug: Set("code-cleanliness".to_string()),
        name: Set("Code Cleanliness".to_string()),
        description: Set(String::new()),
        prompt: Set("judge".to_string()),
        rating_scale: Set(serde_json::json!({"min": 0, "max": 5, "point_multiplier": 1})),
        kind: Set("llm".to_string()),
        scope: Set("task".to_string()),
        evidence_mode: Set("tools".to_string()),
        evidence_needs: Set(None),
        created_at: Set(Utc::now()),
        updated_at: Set(Utc::now()),
        llm_provider_id_fk: Set(None),
        llm_model: Set(None),
        llm_pool_id_fk: Set(None),
        llm_source_order: Set(arena_core::llm::resolve::SOURCE_ORDER_POOL_FIRST.to_string()),
        criteria: Set(None),
        max_interactive: Set(None),
        avatar_url: Set(None),
        ignore_paths: Set(None),
        probes_config: Set(None),
    }
    .insert(&db)
    .await
    .expect("insert judge");

    let mut env2 = sample_envelope("topup-project");
    env2.tasks[0].judges = vec![crate::api::admin_export_import::JudgeRef::Slug(
        "code-cleanliness".to_string(),
    )];
    std::fs::write(dir.join("p.json"), serde_json::to_vec(&env2).unwrap()).unwrap();
    seed_projects(&db).await;

    let attached = task_judges::Entity::find().all(&db).await.unwrap();
    assert_eq!(attached.len(), 1, "re-run attaches the seed's judge");
    assert_eq!(attached[0].judge_id, judge_id);

    // Third run: task already has a judge — nothing duplicated.
    seed_projects(&db).await;
    assert_eq!(
        task_judges::Entity::find().count(&db).await.unwrap(),
        1,
        "top-up is idempotent"
    );
    // Still exactly one project (no re-insert).
    assert_eq!(projects::Entity::find().count(&db).await.unwrap(), 1);

    // safety: serialized by ENV_LOCK
    unsafe { std::env::remove_var("ARENA_PROJECTS_DIR") };
    drop(_guard);
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn seeds_duration_and_creates_category() {
    use arena_core::entities::categories;

    let _guard = ENV_LOCK.lock().unwrap();
    let db = fresh_db().await;
    seed_admin(&db).await;
    let dir = write_dir();

    let mut env = sample_envelope("timed-project");
    env.project.session_duration_secs = 1234;
    env.project.category = Some("Brand New Cat".to_string());
    std::fs::write(dir.join("p.json"), serde_json::to_vec(&env).unwrap()).unwrap();

    // safety: serialized by ENV_LOCK
    unsafe { std::env::set_var("ARENA_PROJECTS_DIR", &dir) };
    seed_projects(&db).await;

    let proj = projects::Entity::find()
        .filter(projects::Column::Slug.eq("timed-project"))
        .one(&db)
        .await
        .unwrap()
        .expect("project seeded");
    assert_eq!(
        proj.default_session_duration_secs, 1234,
        "session duration comes from the definition, not a hardcoded default"
    );
    assert_eq!(proj.category.as_deref(), Some("Brand New Cat"));

    let cats = categories::Entity::find().all(&db).await.unwrap();
    assert!(
        cats.iter().any(|c| c.name == "Brand New Cat"),
        "unknown category is created, not dropped"
    );

    // safety: serialized by ENV_LOCK
    unsafe { std::env::remove_var("ARENA_PROJECTS_DIR") };
    drop(_guard);
    std::fs::remove_dir_all(&dir).ok();
}

fn fresh_state(db: sea_orm::DatabaseConnection) -> crate::state::AppState {
    let cfg = crate::AuthConfig {
        jwt_signing_key: b"seed-reseed-tests-secret-key-32b!".to_vec(),
        frontend_origins: vec!["http://localhost:5173".to_string()],
        access_ttl: std::time::Duration::from_secs(900),
        refresh_ttl: std::time::Duration::from_secs(86400),
        max_agents_per_session: 16,
    };
    crate::state::AppState::new(db, cfg)
}

#[tokio::test]
async fn reseed_updates_project_in_place_from_disk() {
    use arena_core::entities::categories;
    use axum::extract::{Path as AxumPath, State};

    let _guard = ENV_LOCK.lock().unwrap();
    let db = fresh_db().await;
    let admin_id = seed_admin(&db).await;
    let dir = write_dir();

    let env = sample_envelope("reseed-me");
    std::fs::write(dir.join("p.json"), serde_json::to_vec(&env).unwrap()).unwrap();
    // safety: serialized by ENV_LOCK
    unsafe { std::env::set_var("ARENA_PROJECTS_DIR", &dir) };
    seed_projects(&db).await;

    let proj = projects::Entity::find()
        .filter(projects::Column::Slug.eq("reseed-me"))
        .one(&db)
        .await
        .unwrap()
        .expect("project seeded");

    // The definition evolves: new name, duration, category, an edited
    // task 0 and a brand-new task 1.
    let mut env2 = sample_envelope("reseed-me");
    env2.project.name = "Renamed".to_string();
    env2.project.session_duration_secs = 999;
    env2.project.category = Some("Reseed Cat".to_string());
    env2.tasks[0].title = "edited title".to_string();
    let mut extra = env2.tasks[0].clone();
    extra.ordinal = 1;
    extra.title = "added task".to_string();
    env2.tasks.push(extra);
    std::fs::write(dir.join("p.json"), serde_json::to_vec(&env2).unwrap()).unwrap();

    let state = fresh_state(db);
    let admin = crate::api::settings::AdminUser { id: admin_id };
    crate::api::admin_export_import::reseed_project(admin, State(state.clone()), AxumPath(proj.id))
        .await
        .expect("reseed succeeds");

    let updated = projects::Entity::find_by_id(proj.id)
        .one(&state.db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(updated.name, "Renamed");
    assert_eq!(updated.default_session_duration_secs, 999);
    assert_eq!(updated.category.as_deref(), Some("Reseed Cat"));
    let cats = categories::Entity::find().all(&state.db).await.unwrap();
    assert!(
        cats.iter().any(|c| c.name == "Reseed Cat"),
        "reseed creates the missing category"
    );

    let mut db_tasks = tasks::Entity::find()
        .filter(tasks::Column::ProjectIdFk.eq(proj.id))
        .all(&state.db)
        .await
        .unwrap();
    db_tasks.sort_by_key(|t| t.ordinal);
    assert_eq!(db_tasks.len(), 2);
    assert_eq!(db_tasks[0].title, "edited title");
    assert_eq!(db_tasks[1].title, "added task");

    // The definition shrinks back to one task — reseed deletes task 1.
    std::fs::write(dir.join("p.json"), serde_json::to_vec(&env).unwrap()).unwrap();
    let admin = crate::api::settings::AdminUser { id: admin_id };
    crate::api::admin_export_import::reseed_project(admin, State(state.clone()), AxumPath(proj.id))
        .await
        .expect("second reseed succeeds");

    let db_tasks = tasks::Entity::find()
        .filter(tasks::Column::ProjectIdFk.eq(proj.id))
        .all(&state.db)
        .await
        .unwrap();
    assert_eq!(db_tasks.len(), 1, "removed task is deleted");
    assert_eq!(db_tasks[0].title, "t");
    let restored = projects::Entity::find_by_id(proj.id)
        .one(&state.db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(restored.name, "Seeded", "project fields follow the source");

    // safety: serialized by ENV_LOCK
    unsafe { std::env::remove_var("ARENA_PROJECTS_DIR") };
    drop(_guard);
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn reseed_fails_without_matching_source() {
    use axum::extract::{Path as AxumPath, State};

    let _guard = ENV_LOCK.lock().unwrap();
    let db = fresh_db().await;
    let admin_id = seed_admin(&db).await;
    let dir = write_dir();

    let env = sample_envelope("was-here");
    std::fs::write(dir.join("p.json"), serde_json::to_vec(&env).unwrap()).unwrap();
    // safety: serialized by ENV_LOCK
    unsafe { std::env::set_var("ARENA_PROJECTS_DIR", &dir) };
    seed_projects(&db).await;
    let proj = projects::Entity::find()
        .filter(projects::Column::Slug.eq("was-here"))
        .one(&db)
        .await
        .unwrap()
        .unwrap();

    // Source disappears from disk.
    std::fs::remove_file(dir.join("p.json")).unwrap();

    let state = fresh_state(db);
    let admin = crate::api::settings::AdminUser { id: admin_id };
    let err =
        crate::api::admin_export_import::reseed_project(admin, State(state), AxumPath(proj.id))
            .await
            .expect_err("reseed without a source must fail");
    assert!(
        matches!(
            err,
            crate::api::admin_export_import::ExportImportError::BadRequest(_)
        ),
        "unexpected error: {err:?}"
    );

    // safety: serialized by ENV_LOCK
    unsafe { std::env::remove_var("ARENA_PROJECTS_DIR") };
    drop(_guard);
    std::fs::remove_dir_all(&dir).ok();
}

// ---------------------------------------------------------------------------
// seed_llm_defaults: legacy ai_provider/ai_model conversion.
// ---------------------------------------------------------------------------

async fn put_setting(db: &sea_orm::DatabaseConnection, key: &str, value: &str) {
    use arena_core::entities::app_settings;
    use sea_orm::sea_query::OnConflict;
    app_settings::Entity::insert(app_settings::ActiveModel {
        key: Set(key.to_string()),
        value: Set(value.to_string()),
    })
    .on_conflict(
        OnConflict::column(app_settings::Column::Key)
            .update_column(app_settings::Column::Value)
            .to_owned(),
    )
    .exec(db)
    .await
    .expect("upsert setting");
}

async fn get_setting(db: &sea_orm::DatabaseConnection, key: &str) -> Option<String> {
    use arena_core::entities::app_settings;
    use sea_orm::EntityTrait;
    app_settings::Entity::find_by_id(key)
        .one(db)
        .await
        .expect("read setting")
        .map(|r| r.value)
}

#[tokio::test]
async fn seed_llm_defaults_migrates_legacy_openrouter_settings() {
    use arena_core::entities::llm_providers;
    use arena_core::llm::resolve::{LLM_DEFAULT_KEY, LlmAssignment, parse_assignment};

    let db = fresh_db().await;
    // Legacy configuration: openrouter provider, model, encrypted key. The
    // ciphertext is opaque to seeding — it must be copied verbatim.
    const CIPHERTEXT: &str = "b64:definitely-opaque-ciphertext==";
    put_setting(&db, "ai_provider", "openrouter").await;
    put_setting(&db, "ai_model", "anthropic/claude-sonnet-4").await;
    put_setting(&db, "openrouter_api_key", CIPHERTEXT).await;

    seed_llm_defaults(&db).await;

    let rows = llm_providers::Entity::find().all(&db).await.expect("rows");
    assert_eq!(rows.len(), 1, "exactly one provider row seeded");
    let row = &rows[0];
    assert_eq!(row.kind, "openrouter");
    assert_eq!(row.name, "OpenRouter");
    assert_eq!(row.catalog_id.as_deref(), Some("openrouter"));
    assert!(row.enabled);
    assert_eq!(row.base_url, None);
    assert_eq!(
        row.api_key_enc.as_deref(),
        Some(CIPHERTEXT),
        "ciphertext must be preserved verbatim (same SettingsEncryption key)"
    );

    let assignment = get_setting(&db, LLM_DEFAULT_KEY)
        .await
        .expect("llm_default written");
    let LlmAssignment::Single(parsed) = parse_assignment(&assignment).expect("valid assignment")
    else {
        panic!("seed must write a single-model assignment, not a pool");
    };
    assert_eq!(parsed.provider_id, row.id);
    assert_eq!(parsed.model, "anthropic/claude-sonnet-4");

    // Rerun: idempotent — no second row, assignment untouched.
    seed_llm_defaults(&db).await;
    let rows = llm_providers::Entity::find().all(&db).await.expect("rows");
    assert_eq!(rows.len(), 1, "rerun must not insert another provider");
    assert_eq!(
        get_setting(&db, LLM_DEFAULT_KEY).await.as_deref(),
        Some(assignment.as_str()),
        "rerun must not rewrite llm_default"
    );
}

#[tokio::test]
async fn seed_llm_defaults_empty_legacy_seeds_ollama() {
    use arena_core::entities::{app_settings, llm_providers};
    use arena_core::llm::resolve::{LLM_DEFAULT_KEY, LlmAssignment, parse_assignment};
    use sea_orm::EntityTrait;

    let db = fresh_db().await;
    // No legacy configuration at all (the squash migration seeds
    // ai_provider/ai_model defaults — remove them to simulate a truly
    // unconfigured install).
    app_settings::Entity::delete_by_id("ai_provider")
        .exec(&db)
        .await
        .expect("delete ai_provider");
    app_settings::Entity::delete_by_id("ai_model")
        .exec(&db)
        .await
        .expect("delete ai_model");

    seed_llm_defaults(&db).await;

    let rows = llm_providers::Entity::find().all(&db).await.expect("rows");
    assert_eq!(rows.len(), 1, "default Ollama row seeded");
    let row = &rows[0];
    assert_eq!(row.kind, "ollama");
    assert_eq!(row.name, "Ollama");
    assert_eq!(row.catalog_id.as_deref(), Some("ollama"));
    assert!(row.enabled);
    assert_eq!(row.base_url, None);
    assert_eq!(row.api_key_enc, None);

    let assignment = get_setting(&db, LLM_DEFAULT_KEY)
        .await
        .expect("llm_default written");
    let LlmAssignment::Single(parsed) = parse_assignment(&assignment).expect("valid assignment")
    else {
        panic!("seed must write a single-model assignment, not a pool");
    };
    assert_eq!(parsed.provider_id, row.id);
    assert_eq!(parsed.model, "llama3.2");
}

/// Every `{memory.KEY}` a task template uses must be declared in its
/// project's `memory:` schema.
///
/// An undeclared key renders as nothing, so the probe silently runs a
/// truncated command instead of failing loudly — exactly how these projects
/// ended up parsing AGENTS.md with `sed` in the shell instead of using the
/// memory they were supposed to use.
#[tokio::test]
async fn real_projects_declare_every_memory_placeholder_they_use() {
    let Some(root) = fixture_projects_root() else {
        return;
    };

    let mut checked = 0;
    for entry in std::fs::read_dir(&root).expect("read projects dir") {
        let dir = entry.expect("dir entry").path();
        if !dir.join("readme.md").is_file() {
            continue;
        }
        let env = match super::markdown::load_markdown_project(&dir) {
            Ok(e) => e,
            Err(e) => panic!("{} failed to load: {e}", dir.display()),
        };
        let declared: std::collections::BTreeMap<String, String> =
            match env.project.memory_schema.as_ref() {
                Some(m) => arena_core::memory::parse_memory_schema(m).unwrap_or_else(|e| {
                    panic!("{} has an invalid memory schema: {e}", dir.display())
                }),
                None => Default::default(),
            };

        for task in &env.tasks {
            let rendered = serde_json::to_string(&task.test_template).expect("template serializes");
            for key in memory_placeholders(&rendered) {
                assert!(
                    declared.contains_key(&key),
                    "{}: task {} uses {{memory.{key}}} but the project declares no such key \
                     (declared: {:?})",
                    dir.display(),
                    task.ordinal,
                    declared.keys().collect::<Vec<_>>(),
                );
                checked += 1;
            }
        }
    }
    assert!(
        checked > 0,
        "expected at least one project to use a memory placeholder"
    );
}

/// Collect `KEY` from every `{memory.KEY}` occurrence.
fn memory_placeholders(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(i) = rest.find("{memory.") {
        rest = &rest[i + "{memory.".len()..];
        if let Some(end) = rest.find('}') {
            out.push(rest[..end].to_string());
            rest = &rest[end..];
        } else {
            break;
        }
    }
    out
}

/// Locate a shipped project directory from wherever the test binary runs.
///
/// `None` when the project is not part of this checkout. The fixture gates
/// below are round-trip checks on the projects that happen to ship here, not
/// a required inventory: a build carrying a different set of challenges must
/// skip the ones it does not have rather than fail. Callers therefore
/// early-return on `None` instead of unwrapping.
fn fixture_project(slug: &str) -> Option<std::path::PathBuf> {
    fixture_projects_root()
        .map(|root| root.join(slug))
        .filter(|p| p.is_dir())
}

/// The shipped `projects/` directory, from wherever the test binary runs.
/// `None` when this checkout ships no seed projects at all.
fn fixture_projects_root() -> Option<std::path::PathBuf> {
    ["../../projects", "../projects", "projects"]
        .into_iter()
        .map(std::path::PathBuf::from)
        .find(|p| p.is_dir())
}

const HANDMADE_POSTGRESQL_PARTS: [&str; 5] = [
    "handmade-postgresql-1-server",
    "handmade-postgresql-2-sql-engine",
    "handmade-postgresql-3-storage",
    "handmade-postgresql-4-indexes",
    "handmade-postgresql-5-distribution",
];

#[test]
fn handmade_postgresql_campaign_md_seeds() {
    // The campaign is six directories that only work as a set: a parent that
    // is a table of contents, and five task-bearing parts it names in play
    // order. A parent that grew tasks, or a part list that drifted from the
    // directories on disk, breaks the ladder at boot rather than at compile
    // time — hence this gate.
    let Some(parent_dir) = fixture_project("handmade-postgresql") else {
        return;
    };
    let parent =
        super::markdown::load_markdown_project(&parent_dir).expect("campaign parent loads");
    assert_eq!(parent.project.slug.as_deref(), Some("handmade-postgresql"));
    assert!(
        parent.tasks.is_empty(),
        "a campaign parent hosts no sessions, so it carries no tasks"
    );
    assert_eq!(
        parent.project.parts, HANDMADE_POSTGRESQL_PARTS,
        "the parent names its five parts in play order"
    );

    for (ordinal, slug) in HANDMADE_POSTGRESQL_PARTS.iter().enumerate() {
        let part_dir = fixture_project(slug)
            .unwrap_or_else(|| panic!("{slug} ships with the campaign parent"));
        let env = super::markdown::load_markdown_project(&part_dir)
            .unwrap_or_else(|e| panic!("{slug} loads: {e}"));
        assert_eq!(env.schema_version, 1, "{slug}");
        assert_eq!(env.project.slug.as_deref(), Some(*slug));
        assert_eq!(
            env.project.category.as_deref(),
            Some("Reinvent the Wheel"),
            "{slug} stays in the category the campaign was designed for"
        );
        assert!(
            env.project.parts.is_empty(),
            "{slug} is a part, not a campaign of its own"
        );
        assert!(
            env.tasks.len() >= 8,
            "{slug} has {} tasks; each part is a full session",
            env.tasks.len()
        );

        // Every part is driven by the same session-memory contract, so a
        // player's `run:` line carries across the whole campaign.
        let mem = env
            .project
            .memory_schema
            .as_ref()
            .unwrap_or_else(|| panic!("{slug} declares a memory schema"));
        let defaults = arena_core::memory::parse_memory_schema(mem).expect("valid memory schema");
        // The campaign drives a server and a client, so both commands plus
        // the test command are what a part must declare.
        for key in ["serve", "sql", "test"] {
            assert!(
                defaults.contains_key(key),
                "{slug} must declare '{key}' in session memory"
            );
        }

        for task in &env.tasks {
            arena_core::validation::validate_template(&task.test_template)
                .unwrap_or_else(|e| panic!("{slug} task {} template invalid: {e}", task.ordinal));
            crate::api::admin_export_import::validate_task_extras(task)
                .unwrap_or_else(|e| panic!("{slug} task {} extras invalid: {e}", task.ordinal));
            let judge_slugs: Vec<&str> = task.judges.iter().map(|j| j.slug()).collect();
            if task.evaluation.is_some() {
                // Each part closes with an open-ended review scored by the
                // quality panel — the campaign's whole point beyond "it works".
                for expected in [
                    "architecture",
                    "performance",
                    "code-quality",
                    "test-quality",
                    "governance",
                ] {
                    assert!(
                        judge_slugs.contains(&expected),
                        "{slug} task {} is the review rung and must be judged on {expected}",
                        task.ordinal
                    );
                }
            } else {
                assert_eq!(
                    judge_slugs,
                    ["task-anti-cheat", "from-scratch"],
                    "{slug} task {} judges",
                    task.ordinal
                );
            }
        }

        // Every part after the first opens by re-checking what the previous
        // part earned, which is what makes a carried-over codebase pay from
        // the first probe instead of starting cold.
        if ordinal > 0 {
            let setup = &env.tasks[0];
            let body = setup.test_template.command_template.to_lowercase();
            assert!(
                body.contains("select"),
                "{slug} task 0 must include a regression probe of the previous part's contract"
            );
        }
    }
}

#[test]
fn handmade_postgresql_fixtures_and_validations_evaluate() {
    // Two invariants per probe: the fixture script runs (many times, since it
    // is randomized), and the validation is not vacuously true — an empty
    // stdout must never pass. A validation that accepts anything hands out
    // points for a database that printed nothing at all.
    if fixture_project("handmade-postgresql").is_none() {
        return;
    }
    for slug in HANDMADE_POSTGRESQL_PARTS {
        let part_dir = fixture_project(slug)
            .unwrap_or_else(|| panic!("{slug} ships with the campaign parent"));
        let env = super::markdown::load_markdown_project(&part_dir)
            .unwrap_or_else(|e| panic!("{slug} loads: {e}"));

        for task in &env.tasks {
            let sections = arena_core::task_template::parse_structured_markdown_tests(
                &task.test_template.command_template,
            );
            assert!(
                !sections.is_empty(),
                "{slug} task {} has probe sections",
                task.ordinal
            );
            for (si, st) in sections.iter().enumerate() {
                let label = format!("{slug} task {} section {si} ({})", task.ordinal, st.title);
                // Analysis probes (jscpd and friends) are declarations, not
                // scripts: no fixtures, no validation to evaluate.
                if st
                    .probe_config_yaml
                    .as_deref()
                    .is_some_and(|y| y.contains("mode: analysis"))
                {
                    continue;
                }
                let def: serde_json::Value =
                    serde_json::from_str(&st.fixture_definitions).expect("fixture def json");
                let script = def["script"].as_str().unwrap_or("");
                assert!(!script.is_empty(), "{label}: fixtures present");

                for _ in 0..12 {
                    let fx = arena_core::probe_engine::eval_js_fixtures_with_meta(script)
                        .unwrap_or_else(|e| panic!("{label}: fixtures: {e}"))
                        .fixtures;
                    // The capture_memory probes accept any non-empty command,
                    // so their "empty fails" check is the same one — an empty
                    // extraction must not pass.
                    let outcome = arena_core::probe_engine::eval_js_validation_outcome(
                        &st.answer_template,
                        &fx,
                        "",
                    )
                    .unwrap_or_else(|e| panic!("{label}: validation eval: {e}"));
                    assert!(
                        !outcome.pass,
                        "{label}: empty output must not pass — the validation accepts anything"
                    );
                }
            }
        }
    }
}

#[tokio::test]
async fn boot_seeding_links_campaign_parts_and_survives_a_missing_one() {
    // The linking pass runs after every project row exists, unconditionally:
    // the insert pass skips slugs already in the DB, so an edited part list
    // would otherwise never reach a long-lived database. A part that does not
    // ship is warned about and skipped — a half-authored campaign must not
    // stop the server from starting.
    let _guard = ENV_LOCK.lock().unwrap();
    let db = fresh_db().await;
    let admin = seed_admin(&db).await;
    let _ = admin;
    let dir = write_dir();

    let part = |slug: &str| {
        let mut env = sample_envelope(slug);
        env.project.public = true;
        env
    };
    let parent = |slug: &str, parts: Vec<&str>| {
        let mut env = sample_envelope(slug);
        env.project.public = true;
        env.project.parts = parts.into_iter().map(str::to_string).collect();
        env.tasks = vec![];
        env
    };

    for (file, env) in [
        ("a.json", part("chapter-one")),
        ("b.json", part("chapter-two")),
        // Names a third chapter that does not ship.
        (
            "z.json",
            parent("saga", vec!["chapter-one", "ghost-chapter", "chapter-two"]),
        ),
    ] {
        std::fs::write(dir.join(file), serde_json::to_vec(&env).unwrap()).unwrap();
    }

    // safety: serialized by ENV_LOCK
    unsafe { std::env::set_var("ARENA_PROJECTS_DIR", &dir) };
    seed_projects(&db).await;

    let saga = projects::Entity::find()
        .filter(projects::Column::Slug.eq("saga"))
        .one(&db)
        .await
        .unwrap()
        .expect("the campaign parent was seeded");

    let mut linked: Vec<(String, i32)> = projects::Entity::find()
        .filter(projects::Column::ParentProjectIdFk.eq(saga.id))
        .all(&db)
        .await
        .unwrap()
        .into_iter()
        .map(|p| (p.slug.unwrap_or_default(), p.part_ordinal.unwrap_or(-1)))
        .collect();
    linked.sort_by_key(|(_, ordinal)| *ordinal);

    assert_eq!(
        linked,
        vec![
            ("chapter-one".to_string(), 0),
            ("chapter-two".to_string(), 1)
        ],
        "the surviving parts keep play order with no gap where the missing one was"
    );

    // Re-running is idempotent — the reconcile clears before it links.
    seed_projects(&db).await;
    let count = projects::Entity::find()
        .filter(projects::Column::ParentProjectIdFk.eq(saga.id))
        .count(&db)
        .await
        .unwrap();
    assert_eq!(count, 2, "a second boot must not duplicate or drop links");

    // safety: serialized by ENV_LOCK
    unsafe { std::env::remove_var("ARENA_PROJECTS_DIR") };
    drop(_guard);
    std::fs::remove_dir_all(&dir).ok();
}

const MONEY_TRACKER_PARTS: [&str; 6] = [
    "money-tracker-1-ledger",
    "money-tracker-2-receipts",
    "money-tracker-3-currencies",
    "money-tracker-4-crypto",
    "money-tracker-5-people",
    "money-tracker-6-dashboard",
];

#[test]
fn money_tracker_campaign_md_seeds() {
    // Seven directories that only work as a set: a parent that is a table of
    // contents, and six product parts it names in play order. Unlike the
    // Handmade PostgreSQL campaign these parts carry no deterministic probes
    // of the product itself — each is one open-ended build closed by a flag
    // file and scored by the judge panel, so the gate here is the evaluation
    // contract rather than the command DSL.
    let Some(parent_dir) = fixture_project("money-tracker") else {
        return;
    };
    let parent =
        super::markdown::load_markdown_project(&parent_dir).expect("campaign parent loads");
    assert_eq!(parent.project.slug.as_deref(), Some("money-tracker"));
    assert!(
        parent.tasks.is_empty(),
        "a campaign parent hosts no sessions, so it carries no tasks"
    );
    assert_eq!(
        parent.project.parts, MONEY_TRACKER_PARTS,
        "the parent names its six parts in play order"
    );

    for slug in MONEY_TRACKER_PARTS {
        let part_dir = fixture_project(slug)
            .unwrap_or_else(|| panic!("{slug} ships with the campaign parent"));
        let env = super::markdown::load_markdown_project(&part_dir)
            .unwrap_or_else(|e| panic!("{slug} loads: {e}"));
        assert_eq!(env.schema_version, 1, "{slug}");
        assert_eq!(env.project.slug.as_deref(), Some(slug));
        assert_eq!(
            env.project.category.as_deref(),
            Some("Product Build"),
            "{slug} stays in the category the campaign was designed for"
        );
        assert!(
            env.project.parts.is_empty(),
            "{slug} is a part, not a campaign of its own"
        );
        // Two tasks per part: a checked setup rung, then the judged build.
        // Part one's setup is the decision the whole campaign inherits (the
        // stack, the documentation, the way of working); every later part's
        // is a smoke test of what it carried in, because a hand-over that
        // arrives broken should cost a minute, not a session.
        assert_eq!(env.tasks.len(), 2, "{slug} task count");
        let setup = &env.tasks[0];
        assert!(
            setup.evaluation.is_none(),
            "{slug}: the setup rung is checked, not judged"
        );
        let setup_body = &setup.test_template.command_template;
        for declared in ["run", "test"] {
            assert!(
                setup_body.contains(&format!("capture_memory: {declared}")),
                "{slug} setup captures '{declared}' into session memory"
            );
        }
        if slug == "money-tracker-1-ledger" {
            assert!(
                setup_body.contains("capture_memory: stack"),
                "the first part declares the stack the campaign will carry"
            );
        } else {
            assert!(
                setup_body.contains("{memory.test}"),
                "{slug} smoke-tests the carried-over code with its own suite"
            );
            assert!(
                setup_body.contains("money-tracker-ledger-done.md"),
                "{slug} checks the earlier parts arrived with the codebase"
            );
            // A green suite is the player's own word. One judge reads the
            // product on this rung too, against the numbers the earlier
            // parts pinned — and without an evaluation contract here it can
            // score from the code and the checks alone, never by sending the
            // participant off to capture something before they have started.
            let judge_slugs: Vec<&str> = setup.judges.iter().map(|j| j.slug()).collect();
            assert_eq!(judge_slugs, ["correctness"], "{slug} setup panel");
            // Whatever it summarises, it must hand the judge at least one of
            // the campaign's pinned figures — an instruction to check that
            // "everything still works" is not something anyone can be wrong
            // about.
            let pinned = ["3223.70", "11.91", "112.80", "1030.75", "42.00"];
            assert!(
                pinned.iter().any(|n| setup.content.contains(n)),
                "{slug} setup names none of the pinned figures, so its judge has no fact to check"
            );
        }

        // The whole campaign is judged, so a part that lost its run/test
        // memory would leave the panel unable to run what it reads.
        let mem = env
            .project
            .memory_schema
            .as_ref()
            .unwrap_or_else(|| panic!("{slug} declares a memory schema"));
        let defaults = arena_core::memory::parse_memory_schema(mem).expect("valid memory schema");
        for key in ["run", "test"] {
            assert!(
                defaults.contains_key(key),
                "{slug} must declare '{key}' in session memory"
            );
        }
        if slug == "money-tracker-1-ledger" {
            assert!(
                defaults.contains_key("stack"),
                "the campaign's first part declares the stack it will carry"
            );
        }

        // The judged build is the last task of the part.
        let task = env.tasks.last().expect("a build task");
        arena_core::validation::validate_template(&task.test_template)
            .unwrap_or_else(|e| panic!("{slug} template invalid: {e}"));
        crate::api::admin_export_import::validate_task_extras(task)
            .unwrap_or_else(|e| panic!("{slug} extras invalid: {e}"));

        let contract = arena_core::evaluation::EvaluationContract::from_json(
            task.evaluation
                .as_ref()
                .unwrap_or_else(|| panic!("{slug} carries an evaluation contract")),
        )
        .unwrap_or_else(|e| panic!("{slug} contract parses: {e}"));
        assert_eq!(contract.completion.probe, "Definition of done", "{slug}");
        // The build is the session: a shorter work window force-evaluates a
        // half-built product while the clock still runs (FBTQYR lost 12
        // minutes to a 900s window copied from a three-task project).
        assert!(
            contract.completion.deadline_secs >= 2400,
            "{slug}: the build's work window must span the whole session"
        );
        // `agentic` is a session appraisal over a workspace the campaign
        // carries from part to part: six verdicts on the same setup would pay
        // the same answer six times. Only the final part scores it, where the
        // whole workflow evolution is visible.
        let last_part = slug == "money-tracker-6-dashboard";
        let mut expected = vec![
            "product",
            "architecture",
            "data",
            "ux",
            "accessibility",
            "mobile",
            "cleanliness",
            "maintainability",
            "tests",
        ];
        if last_part {
            expected.push("agentic");
            expected.push("skills");
        }
        expected.push("creativity");
        let keys: Vec<&str> = contract.criteria.iter().map(|c| c.key.as_str()).collect();
        assert_eq!(keys, expected, "{slug} scores the whole panel's sheet");
        assert_eq!(
            task.judges.iter().any(|j| j.slug() == "agentic"),
            last_part,
            "{slug}: the workflow judge sits on the final part alone"
        );

        // Every part's brief ends with its own flag file, and the completion
        // check must look for exactly that one — a copied section would mark
        // a part done the moment the previous part's flag was found.
        // `money-tracker-3-currencies` → `.ololo/money-tracker-currencies-done.md`
        let flag = format!(
            ".ololo/money-tracker-{}-done.md",
            slug.rsplit('-').next().expect("slug shape")
        );
        assert!(
            task.content.contains(&flag),
            "{slug} brief must name {flag}"
        );
        assert!(
            task.test_template.command_template.contains(&flag),
            "{slug} completion check must look for {flag}"
        );
    }
}
