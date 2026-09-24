//! Personal projects — a user's own work, played as a normal session in
//! their own repository.
//!
//! The user writes a description and, optionally, a navigation map: the
//! tasks the work splits into, in order. Each map entry becomes one
//! open-ended task of the project; an empty map makes the whole description
//! one task. Every task is judged by the panel the user picked and closes
//! when the agent writes the task's done-file, exactly like a challenge
//! task — the only differences are in the words: the brief says the code
//! already exists, and the contract tells the judges to judge the task's
//! own changes, not the codebase they landed in.
//!
//! A personal project is public unless its owner keeps it private (a
//! Premium choice where plans are on): a public one is listed on its
//! owner's profile and its sessions are public history like any other, but
//! the catalog and the landing never list the project itself. A private
//! one, and its sessions, stay with the owner.
//!
//! This module owns the shape of the request as stored
//! (`personal_projects.spec`), the queries every reader uses to ask "is
//! this personal?", and the pure construction of the tasks. The server
//! turns the constructed tasks into rows.

use std::collections::HashSet;

use sea_orm::{ColumnTrait, ConnectionTrait, DbErr, EntityTrait, QueryFilter, QuerySelect};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::entities::{personal_projects, projects, sessions};
use crate::evaluation::{CompletionSpec, CriterionDef, InteractiveLimits};
use crate::task_template::{Backoff, Matchers, Placeholder, TestKind, TestTemplate};

/// Version of the brief and probe wording a spec is built with. Stored in
/// the spec so a rebuild knows which words produced the current tasks.
pub const SPEC_VERSION: u32 = 1;

/// Longest navigation map a project may carry. Every task runs the whole
/// judge panel, so the map is also the judge-run budget of a session.
pub const MAX_TASKS: usize = 10;
pub const MAX_NAME_CHARS: usize = 120;
pub const MAX_DESCRIPTION_CHARS: usize = 8_000;
pub const MAX_TASK_TITLE_CHARS: usize = 200;
pub const MAX_TASK_DESCRIPTION_CHARS: usize = 4_000;
/// How many judges one task may carry.
pub const MAX_JUDGES: usize = 6;

/// Session lengths a personal project may ask for.
pub const MIN_SESSION_SECS: i64 = 30 * 60;
pub const MAX_SESSION_SECS: i64 = 8 * 60 * 60;
pub const DEFAULT_SESSION_SECS: i64 = 2 * 60 * 60;

/// A personal session survives this long with no agent connected — a
/// laptop lid closed over lunch must not cancel an afternoon's work.
pub const IDLE_TIMEOUT_SECS: i32 = 30 * 60;

/// Points one task pays: its judge-panel budget, the bonus for closing it
/// through its done-file, and the health bonus for not making the code
/// worse (see `ololo_health::delta_bonus`).
pub const TASK_POINTS: i32 = 100;
pub const TASK_COMPLETION_BONUS: i32 = 10;
pub const TASK_HEALTH_POINTS: i32 = 20;

/// Section title of every personal task's completion probe.
pub const DONE_PROBE_TITLE: &str = "Definition of done";

/// What the user asked for, as stored on `personal_projects.spec` and
/// returned to the owner for editing and duplicating.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PersonalSpec {
    pub version: u32,
    pub name: String,
    pub description: String,
    /// The navigation map, in play order. Empty = one task for the whole
    /// project.
    #[serde(default)]
    pub tasks: Vec<PersonalTaskSpec>,
    /// The default judge panel, by judge slug: every task that does not
    /// pick its own.
    pub judges: Vec<String>,
    pub session_duration_secs: i64,
}

/// One entry of the navigation map.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PersonalTaskSpec {
    pub title: String,
    #[serde(default)]
    pub description: String,
    /// This task's own judge panel, by slug. `None`: the project's default
    /// panel ([`PersonalSpec::judges`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub judges: Option<Vec<String>>,
}

impl PersonalTaskSpec {
    /// The slugs judging this task: its own panel, else `default`.
    pub fn panel<'a>(&'a self, default: &'a [String]) -> &'a [String] {
        self.judges.as_deref().unwrap_or(default)
    }
}

/// A judge a task can be given, as the builder needs it: the criteria it
/// scores (they become the task's score sheet) and its share of the task
/// budget.
#[derive(Debug, Clone, PartialEq)]
pub struct PanelJudge {
    pub slug: String,
    pub criteria: Vec<String>,
    pub weight: f64,
}

/// A task ready to be stored.
#[derive(Debug, Clone, PartialEq)]
pub struct BuiltTask {
    /// 0-based, dense.
    pub ordinal: i32,
    pub title: String,
    /// The brief the agent and the judges read.
    pub content: String,
    pub test_template: TestTemplate,
    /// The `tasks.evaluation` contract.
    pub evaluation: serde_json::Value,
    /// The judges of this task, in panel order.
    pub judges: Vec<PanelJudge>,
    /// Workspace-relative path of the file that closes the task.
    pub done_file: String,
}

/// The panel share a judge gets by default: correctness is what the user
/// cares about first, every other judge is an equal voice.
pub fn default_panel_weight(slug: &str) -> f64 {
    if slug == "correctness" { 1.5 } else { 1.0 }
}

/// Whether the project is personal.
pub async fn is_personal_project<C: ConnectionTrait>(
    db: &C,
    project_id: Uuid,
) -> Result<bool, DbErr> {
    Ok(personal_projects::Entity::find_by_id(project_id)
        .one(db)
        .await?
        .is_some())
}

/// Which of `project_ids` are personal.
pub async fn personal_project_ids<C: ConnectionTrait>(
    db: &C,
    project_ids: &[Uuid],
) -> Result<HashSet<Uuid>, DbErr> {
    if project_ids.is_empty() {
        return Ok(HashSet::new());
    }
    let ids: Vec<Uuid> = personal_projects::Entity::find()
        .filter(personal_projects::Column::ProjectIdFk.is_in(project_ids.iter().copied()))
        .select_only()
        .column(personal_projects::Column::ProjectIdFk)
        .into_tuple()
        .all(db)
        .await?;
    Ok(ids.into_iter().collect())
}

/// Whether the session runs a personal project. A missing session is not
/// personal — callers treat it as whatever they did before this existed.
pub async fn is_personal_session<C: ConnectionTrait>(
    db: &C,
    session_id: Uuid,
) -> Result<bool, DbErr> {
    let Some(session) = sessions::Entity::find_by_id(session_id).one(db).await? else {
        return Ok(false);
    };
    is_personal_project(db, session.project_id_fk).await
}

/// The ids of personal projects, as a subquery — for readers that must
/// leave them out (`Column::ProjectIdFk.not_in_subquery(..)`).
pub fn personal_project_ids_query() -> sea_orm::sea_query::SelectStatement {
    use sea_orm::QueryTrait;
    personal_projects::Entity::find()
        .select_only()
        .column(personal_projects::Column::ProjectIdFk)
        .into_query()
}

/// The ids of personal projects their owners keep private, as a subquery.
pub fn private_personal_project_ids_query() -> sea_orm::sea_query::SelectStatement {
    use sea_orm::QueryTrait;
    projects::Entity::find()
        .select_only()
        .column(projects::Column::Id)
        .filter(projects::Column::Public.eq(false))
        .filter(projects::Column::Id.in_subquery(personal_project_ids_query()))
        .into_query()
}

/// The sessions of private personal projects, as a subquery — for readers
/// that must leave them out of what anyone but the owner sees
/// (`Column::SessionIdFk.not_in_subquery`).
pub fn private_personal_session_ids_query() -> sea_orm::sea_query::SelectStatement {
    use sea_orm::QueryTrait;
    sessions::Entity::find()
        .select_only()
        .column(sessions::Column::Id)
        .filter(sessions::Column::ProjectIdFk.in_subquery(private_personal_project_ids_query()))
        .into_query()
}

/// The workspace-relative done-file of task `index` (0-based) of a
/// project with `task_count` tasks.
pub fn done_file(index: usize, task_count: usize) -> String {
    if task_count <= 1 {
        ".ololo/task-done.md".to_string()
    } else {
        format!(".ololo/task-{}-done.md", index + 1)
    }
}

/// Build every task of `spec`. Each is judged by its own panel or the
/// project's default one, looked up in `catalog` (the judges a personal
/// project may use; slugs missing from it are skipped).
///
/// An empty navigation map yields one task: the project itself.
pub fn build_tasks(spec: &PersonalSpec, catalog: &[PanelJudge]) -> Vec<BuiltTask> {
    let map: Vec<PersonalTaskSpec> = if spec.tasks.is_empty() {
        vec![PersonalTaskSpec {
            title: spec.name.clone(),
            description: String::new(),
            judges: None,
        }]
    } else {
        spec.tasks.clone()
    };
    map.iter()
        .enumerate()
        .map(|(index, task)| {
            let done = done_file(index, map.len());
            let judges: Vec<PanelJudge> = task
                .panel(&spec.judges)
                .iter()
                .filter_map(|slug| catalog.iter().find(|j| &j.slug == slug))
                .cloned()
                .collect();
            BuiltTask {
                ordinal: index as i32,
                title: task.title.trim().to_string(),
                content: brief(spec, &map, index, &done),
                test_template: done_probe(&done),
                evaluation: contract(spec, &judges, map.len()),
                judges,
                done_file: done,
            }
        })
        .collect()
}

/// The brief of task `index`. The single-task form is the description
/// itself; a map entry gets its own words first, then the project around
/// it, so the agent reads what to do before it reads why.
fn brief(spec: &PersonalSpec, map: &[PersonalTaskSpec], index: usize, done: &str) -> String {
    let description = spec.description.trim();
    let mut out = String::new();
    if map.len() <= 1 && spec.tasks.is_empty() {
        out.push_str(description);
        out.push_str(
            "\n\nYou are working in an existing codebase: the repository this session runs in. \
             Change what this goal needs and keep everything else working.",
        );
        out.push_str(&format!(
            "\n\nWhen you are done, write {done} with a short description of what you changed \
             and why (at least 10 words)."
        ));
        return out;
    }

    let task = &map[index];
    let own = task.description.trim();
    out.push_str(if own.is_empty() {
        task.title.trim()
    } else {
        own
    });
    out.push_str(&format!(
        "\n\nThis is task {} of {} of the project \"{}\".",
        index + 1,
        map.len(),
        spec.name.trim()
    ));
    if !description.is_empty() {
        out.push_str("\n\n");
        out.push_str(description);
    }
    out.push_str("\n\nNavigation map:\n");
    for (i, entry) in map.iter().enumerate() {
        let title = entry.title.trim();
        let note = match i.cmp(&index) {
            std::cmp::Ordering::Less => " — done before this task",
            std::cmp::Ordering::Equal => " — this task",
            std::cmp::Ordering::Greater => " — a later task, not part of this one",
        };
        out.push_str(&format!("{}. {title}{note}\n", i + 1));
    }
    out.push_str(
        "\nYou are working in an existing codebase: the repository this session runs in. \
         Do this task only, and keep everything else working.",
    );
    out.push_str(&format!(
        "\n\nWhen this task is done, write {done} with a short description of what you \
         changed and why (at least 10 words)."
    ));
    out
}

/// The contract every task of the project shares: done when the done-file
/// probe passes, a work window as long as the session, and a score sheet
/// made of the panel's criteria. Built as the `tasks.evaluation` JSON from
/// typed parts, so it names only the fields every edition's
/// `EvaluationContract` knows.
fn contract(spec: &PersonalSpec, panel: &[PanelJudge], task_count: usize) -> serde_json::Value {
    let mut seen = HashSet::new();
    let criteria: Vec<CriterionDef> = panel
        .iter()
        .flat_map(|j| j.criteria.iter())
        .filter(|key| seen.insert((*key).clone()))
        .map(|key| CriterionDef {
            key: key.clone(),
            title: criterion_title(key),
            weight: 1.0,
        })
        .collect();
    let scope = if task_count > 1 {
        " Later tasks of the navigation map are out of scope; earlier ones were judged on \
         their own."
    } else {
        ""
    };
    serde_json::json!({
        "kind": "open_ended",
        "completion": CompletionSpec {
            probe: DONE_PROBE_TITLE.to_string(),
            deadline_secs: spec.session_duration_secs,
        },
        "constraints": format!(
            "This is the participant's own existing codebase, not a build from scratch. Judge \
             only what this task changed — the work-window diff. Code that predates the session \
             is context: neither credit nor penalize it.{scope}"
        ),
        "criteria": criteria,
        "limits": InteractiveLimits {
            interactive_probes_per_task: 2,
            interactive_probes_per_judge: 1,
        },
    })
}

/// A readable title for a criterion key the reusable judges score.
fn criterion_title(key: &str) -> String {
    let known = match key {
        "product" => "Does what the task asked",
        "architecture" => "Architecture",
        "data" => "Data layer",
        "ux" => "UI/UX",
        "accessibility" => "Accessibility",
        "mobile" => "Mobile readiness",
        "cleanliness" => "Code cleanliness",
        "maintainability" => "Maintainability",
        "tests" => "Tests",
        "agentic" => "Agentic workflow",
        "skills" => "Skills leverage",
        "creativity" => "Creativity",
        "craft" => "Craft",
        "performance" => "Performance",
        _ => "",
    };
    if !known.is_empty() {
        return known.to_string();
    }
    let mut chars = key.replace(['_', '-'], " ").chars().collect::<Vec<_>>();
    if let Some(first) = chars.first_mut() {
        *first = first.to_ascii_uppercase();
    }
    chars.into_iter().collect()
}

/// The completion probe: a participant-side check, every minute, that the
/// done-file exists and says something.
fn done_probe(done: &str) -> TestTemplate {
    let command_template = format!(
        "## {DONE_PROBE_TITLE}\n\
         \n\
         The task is done when {done} describes what changed and why.\n\
         \n\
         ```yaml probe\n\
         mode: deterministic\n\
         executor: participant\n\
         schedule: {{ on: [interval], interval_secs: 60 }}\n\
         ```\n\
         \n\
         ```js fixtures\n\
         ({{ baseDir: \".\" }})\n\
         ```\n\
         \n\
         ```sh command\n\
         cd {{baseDir}}\n\
         test -f {done} || {{ echo \"not-done: write {done} when this task is finished\"; exit 0; }}\n\
         words=$(wc -w < {done} | tr -d ' ')\n\
         [ \"$words\" -ge 10 ] || {{ echo \"not-done: {done} needs a short description of what you changed (at least 10 words)\"; exit 0; }}\n\
         echo \"done-note: present\"\n\
         ```\n\
         \n\
         ```js validation\n\
         result.includes(\"done-note: present\")\n\
         ```\n"
    );
    TestTemplate {
        kind: TestKind::Shell,
        command_template,
        placeholders: vec![Placeholder {
            name: "baseDir".to_string(),
            description: "Workspace directory the check runs in".to_string(),
            required: false,
            secret: false,
        }],
        matchers: Matchers::default(),
        backoff: Backoff {
            initial_ms: 1_000,
            multiplier: 2.0,
            max_ms: 30_000,
            max_attempts: 5,
        },
        fixtures: Vec::new(),
        answer_template: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::evaluation::EvaluationContract;

    fn panel() -> Vec<PanelJudge> {
        vec![
            PanelJudge {
                slug: "correctness".into(),
                criteria: vec!["product".into()],
                weight: 1.5,
            },
            PanelJudge {
                slug: "code-quality".into(),
                criteria: vec!["cleanliness".into(), "maintainability".into()],
                weight: 1.0,
            },
            PanelJudge {
                slug: "test-quality".into(),
                criteria: vec!["tests".into()],
                weight: 1.0,
            },
        ]
    }

    fn spec(tasks: &[&str]) -> PersonalSpec {
        PersonalSpec {
            version: SPEC_VERSION,
            name: "CSV export".into(),
            description: "Add a CSV export to the reports page.".into(),
            tasks: tasks
                .iter()
                .map(|t| PersonalTaskSpec {
                    title: (*t).into(),
                    description: String::new(),
                    judges: None,
                })
                .collect(),
            judges: vec![
                "correctness".into(),
                "code-quality".into(),
                "test-quality".into(),
            ],
            session_duration_secs: DEFAULT_SESSION_SECS,
        }
    }

    #[test]
    fn an_empty_map_is_one_task_for_the_whole_project() {
        let built = build_tasks(&spec(&[]), &panel());
        assert_eq!(built.len(), 1);
        let task = &built[0];
        assert_eq!(task.ordinal, 0);
        assert_eq!(task.title, "CSV export");
        assert_eq!(task.done_file, ".ololo/task-done.md");
        assert!(
            task.content
                .starts_with("Add a CSV export to the reports page.")
        );
        assert!(task.content.contains("existing codebase"));
        assert!(!task.content.contains("Navigation map"));
        assert!(
            task.content.trim_end().ends_with("(at least 10 words)."),
            "the brief ends with the done-file instruction: {}",
            task.content
        );
    }

    #[test]
    fn every_map_entry_is_a_task_that_knows_its_place() {
        let built = build_tasks(
            &spec(&["Add the endpoint", "Add the button", "Document it"]),
            &panel(),
        );
        assert_eq!(built.len(), 3);
        assert_eq!(
            built.iter().map(|t| t.ordinal).collect::<Vec<_>>(),
            [0, 1, 2]
        );
        let second = &built[1];
        assert_eq!(second.title, "Add the button");
        assert_eq!(second.done_file, ".ololo/task-2-done.md");
        assert!(second.content.starts_with("Add the button"));
        assert!(second.content.contains("task 2 of 3"));
        assert!(
            second
                .content
                .contains("1. Add the endpoint — done before this task")
        );
        assert!(second.content.contains("2. Add the button — this task"));
        assert!(
            second
                .content
                .contains("3. Document it — a later task, not part of this one")
        );
        assert!(second.content.contains(".ololo/task-2-done.md"));
    }

    #[test]
    fn a_task_description_leads_its_brief() {
        let mut s = spec(&["Endpoint"]);
        s.tasks[0].description = "Serve GET /reports.csv with the same filters.".into();
        let built = build_tasks(&s, &panel());
        assert!(
            built[0]
                .content
                .starts_with("Serve GET /reports.csv with the same filters.")
        );
    }

    #[test]
    fn the_contract_validates_against_the_probe_it_names() {
        for tasks in [&[][..], &["One", "Two"][..]] {
            for task in build_tasks(&spec(tasks), &panel()) {
                crate::validation::validate_template(&task.test_template)
                    .expect("template validates");
                let sections = crate::task_template::parse_structured_markdown_tests(
                    &task.test_template.command_template,
                );
                let titles: Vec<String> = sections.iter().map(|s| s.title.clone()).collect();
                assert_eq!(titles, [DONE_PROBE_TITLE]);
                sections[0]
                    .parsed_probe_config()
                    .expect("probe fence parses")
                    .expect("probe fence present");
                let contract =
                    EvaluationContract::from_json(&task.evaluation).expect("contract parses");
                crate::evaluation::validate_evaluation_contract(&contract, &titles)
                    .expect("contract validates");
                assert_eq!(contract.completion.deadline_secs, DEFAULT_SESSION_SECS);
                assert_eq!(
                    contract
                        .criteria
                        .iter()
                        .map(|c| c.key.as_str())
                        .collect::<Vec<_>>(),
                    ["product", "cleanliness", "maintainability", "tests"]
                );
                assert!(
                    contract
                        .constraints
                        .as_deref()
                        .unwrap_or_default()
                        .contains("predates the session")
                );
            }
        }
    }

    #[test]
    fn criteria_shared_by_two_judges_appear_once() {
        let mut p = panel();
        p.push(PanelJudge {
            slug: "build-review".into(),
            criteria: vec!["product".into(), "craft".into()],
            weight: 1.0,
        });
        let mut s = spec(&[]);
        s.judges.push("build-review".into());
        let task = &build_tasks(&s, &p)[0];
        let contract = EvaluationContract::from_json(&task.evaluation).expect("contract parses");
        let keys: Vec<&str> = contract.criteria.iter().map(|c| c.key.as_str()).collect();
        assert_eq!(
            keys,
            [
                "product",
                "cleanliness",
                "maintainability",
                "tests",
                "craft"
            ]
        );
        assert_eq!(contract.criteria[4].title, "Craft");
    }

    #[test]
    fn a_task_with_its_own_judges_is_judged_by_them_and_the_rest_by_the_default() {
        let mut s = spec(&["Endpoint", "Button"]);
        s.tasks[0].judges = Some(vec!["test-quality".into(), "correctness".into()]);
        let built = build_tasks(&s, &panel());

        let own: Vec<&str> = built[0].judges.iter().map(|j| j.slug.as_str()).collect();
        assert_eq!(own, ["test-quality", "correctness"], "the task's order");
        let sheet = EvaluationContract::from_json(&built[0].evaluation).unwrap();
        let keys: Vec<&str> = sheet.criteria.iter().map(|c| c.key.as_str()).collect();
        assert_eq!(keys, ["tests", "product"], "its own panel's criteria only");

        let inherited: Vec<&str> = built[1].judges.iter().map(|j| j.slug.as_str()).collect();
        assert_eq!(inherited, ["correctness", "code-quality", "test-quality"]);
    }

    #[test]
    fn slugs_outside_the_catalog_are_skipped() {
        let mut s = spec(&["Only"]);
        s.tasks[0].judges = Some(vec!["correctness".into(), "retired-judge".into()]);
        let built = build_tasks(&s, &panel());
        assert_eq!(built[0].judges.len(), 1);
    }

    #[test]
    fn unknown_criterion_keys_get_a_readable_title() {
        assert_eq!(criterion_title("security_review"), "Security review");
        assert_eq!(criterion_title("product"), "Does what the task asked");
    }

    /// Run the rendered done-check in a scratch workspace and grade its
    /// stdout the way the server does.
    #[cfg(unix)]
    #[test]
    fn the_done_check_passes_only_on_a_real_note() {
        let task = &build_tasks(&spec(&["One", "Two"]), &panel())[1];
        let section = &crate::task_template::parse_structured_markdown_tests(
            &task.test_template.command_template,
        )[0];
        let scalars = std::collections::HashMap::from([("baseDir".to_string(), ".".to_string())]);
        let command = crate::probe_engine::render_command_shell_aware(
            &section.command_template,
            &scalars,
            &Default::default(),
        )
        .expect("renders");
        let fixtures = serde_json::Map::from_iter([(
            "baseDir".to_string(),
            serde_json::Value::String(".".into()),
        )]);
        let dir = tempfile::tempdir().expect("tempdir");
        let run = || {
            let out = std::process::Command::new("sh")
                .arg("-c")
                .arg(&command)
                .current_dir(dir.path())
                .output()
                .expect("sh runs");
            let stdout = String::from_utf8_lossy(&out.stdout).to_string();
            let pass = crate::probe_engine::eval_js_validation_outcome(
                &section.answer_template,
                &fixtures,
                &stdout,
            )
            .map(|o| o.pass)
            .unwrap_or(false);
            (stdout, pass)
        };

        let (stdout, pass) = run();
        assert!(!pass, "no done-file yet: {stdout}");
        assert!(stdout.contains("not-done: write .ololo/task-2-done.md"));

        std::fs::create_dir_all(dir.path().join(".ololo")).unwrap();
        std::fs::write(dir.path().join(".ololo/task-2-done.md"), "done").unwrap();
        let (stdout, pass) = run();
        assert!(!pass, "a one-word note is not a description: {stdout}");

        std::fs::write(
            dir.path().join(".ololo/task-2-done.md"),
            "Added the export button to the reports toolbar and wired it to the new endpoint.",
        )
        .unwrap();
        let (stdout, pass) = run();
        assert!(pass, "a real note closes the task: {stdout}");
    }

    #[test]
    fn the_spec_round_trips_and_rejects_unknown_fields() {
        let mut s = spec(&["One", "Two"]);
        s.tasks[1].judges = Some(vec!["correctness".into()]);
        let json = serde_json::to_value(&s).unwrap();
        assert!(
            json["tasks"][0].get("judges").is_none(),
            "an inheriting task stores no panel: {json}"
        );
        assert_eq!(
            json["tasks"][1]["judges"],
            serde_json::json!(["correctness"])
        );
        assert_eq!(serde_json::from_value::<PersonalSpec>(json).unwrap(), s);
        // A spec stored before tasks had their own judges still reads.
        let old = serde_json::json!({
            "version": 1, "name": "n", "description": "d",
            "tasks": [{ "title": "t", "description": "" }],
            "judges": ["correctness"], "session_duration_secs": 3600
        });
        let parsed: PersonalSpec = serde_json::from_value(old).unwrap();
        assert_eq!(parsed.tasks[0].judges, None);
        let bad = serde_json::json!({
            "version": 1, "name": "n", "description": "d", "judges": [],
            "session_duration_secs": 3600, "surprise": true
        });
        assert!(serde_json::from_value::<PersonalSpec>(bad).is_err());
    }
}
