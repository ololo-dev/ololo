//! From a personal-project request to rows: validation, the judge panel,
//! the name and slug, and the task definitions the shared seed/import
//! inserter stores.

use std::collections::HashSet;

use arena_core::entities::{judges, projects};
use arena_core::personal::{
    self, BuiltTask, PanelJudge, PersonalSpec, PersonalTaskSpec, SPEC_VERSION,
};
use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter};
use uuid::Uuid;

use super::{PersonalProjectError, PersonalProjectReq};
use crate::api::admin_export_import::{ExportTask, ExportTaskPoints, JudgeRef};

/// The judges a personal project may put on its panel, in the order the
/// form offers them. Each reads the task's own changes and scores them on
/// its criteria; judges built for one challenge (a CLI's DX, a harness, a
/// from-scratch rule) or for the whole session are left out.
pub(super) const OFFERED_JUDGES: &[&str] = &[
    "correctness",
    "code-quality",
    "test-quality",
    "architecture",
    "data",
    "performance",
    "ux-review",
    "build-review",
];

/// The panel a request without `judges` gets — whichever of these exist.
const DEFAULT_PANEL: &[&str] = &["correctness", "code-quality", "test-quality"];

/// A judge that can sit on a personal panel.
pub(super) fn is_eligible(judge: &judges::Model) -> bool {
    OFFERED_JUDGES.contains(&judge.slug.as_str())
        && judge.kind == arena_core::judging::JUDGE_KIND_LLM
        && judge.scope == arena_core::judging::JUDGE_SCOPE_TASK
        && !criteria_of(judge).is_empty()
        && judge
            .rating_scale
            .get("min")
            .and_then(serde_json::Value::as_f64)
            .is_some_and(|min| min >= 0.0)
}

/// The criteria keys a judge scores (`judges.criteria` JSON array).
pub(super) fn criteria_of(judge: &judges::Model) -> Vec<String> {
    judge
        .criteria
        .as_deref()
        .and_then(|raw| serde_json::from_str::<Vec<String>>(raw).ok())
        .unwrap_or_default()
}

/// Every eligible judge, in [`OFFERED_JUDGES`] order.
pub(super) async fn eligible_judges(
    db: &DatabaseConnection,
) -> Result<Vec<judges::Model>, sea_orm::DbErr> {
    let mut rows: Vec<judges::Model> = judges::Entity::find()
        .filter(judges::Column::Slug.is_in(OFFERED_JUDGES.iter().copied()))
        .all(db)
        .await?
        .into_iter()
        .filter(is_eligible)
        .collect();
    rows.sort_by_key(|j| {
        OFFERED_JUDGES
            .iter()
            .position(|s| *s == j.slug)
            .unwrap_or(usize::MAX)
    });
    Ok(rows)
}

/// The default panel among `eligible`: the three reviewers of a change
/// that exist. Whether the change does what was asked is the question a
/// panel must answer first, so an instance without `correctness` (the open
/// platform ships `build-review` in its place) leads with `build-review`;
/// with none of them, the first judge offered.
pub(super) fn default_panel(eligible: &[judges::Model]) -> Vec<String> {
    let has = |slug: &str| eligible.iter().any(|j| j.slug == slug);
    let mut panel: Vec<String> = DEFAULT_PANEL
        .iter()
        .filter(|slug| has(slug))
        .map(|slug| (*slug).to_string())
        .collect();
    if !has("correctness") && has("build-review") {
        panel.insert(0, "build-review".to_string());
    }
    if panel.is_empty() {
        eligible.iter().take(1).map(|j| j.slug.clone()).collect()
    } else {
        panel
    }
}

fn invalid(field: &'static str, detail: impl Into<String>) -> PersonalProjectError {
    PersonalProjectError::Invalid {
        field,
        detail: detail.into(),
    }
}

fn bounded(
    field: &'static str,
    raw: &str,
    min: usize,
    max: usize,
) -> Result<String, PersonalProjectError> {
    let trimmed = raw.trim();
    let len = trimmed.chars().count();
    if len < min {
        return Err(invalid(field, "must not be empty"));
    }
    if len > max {
        return Err(invalid(field, format!("must be at most {max} characters")));
    }
    Ok(trimmed.to_string())
}

/// Validate a request into the spec that will be stored.
pub(super) fn validate(
    req: &PersonalProjectReq,
    eligible: &[judges::Model],
) -> Result<PersonalSpec, PersonalProjectError> {
    let description = bounded(
        "description",
        &req.description,
        1,
        personal::MAX_DESCRIPTION_CHARS,
    )?;
    let name = match req.name.as_deref().map(str::trim).filter(|n| !n.is_empty()) {
        Some(name) => bounded("name", name, 1, personal::MAX_NAME_CHARS)?,
        None => derive_name(&description),
    };

    if req.tasks.len() > personal::MAX_TASKS {
        return Err(invalid(
            "tasks",
            format!(
                "a navigation map holds at most {} tasks",
                personal::MAX_TASKS
            ),
        ));
    }
    let tasks = req
        .tasks
        .iter()
        .enumerate()
        .map(|(i, t)| {
            Ok(PersonalTaskSpec {
                title: bounded("tasks", &t.title, 1, personal::MAX_TASK_TITLE_CHARS)?,
                description: bounded(
                    "tasks",
                    t.description.as_deref().unwrap_or_default(),
                    0,
                    personal::MAX_TASK_DESCRIPTION_CHARS,
                )?,
                judges: t
                    .judges
                    .as_deref()
                    .map(|slugs| panel_of(slugs, eligible))
                    .transpose()
                    .map_err(|detail| invalid("tasks", format!("task {}: {detail}", i + 1)))?,
            })
        })
        .collect::<Result<Vec<_>, PersonalProjectError>>()?;

    let judges = match &req.judges {
        None => default_panel(eligible),
        Some(slugs) => panel_of(slugs, eligible).map_err(|detail| invalid("judges", detail))?,
    };
    if judges.is_empty() {
        return Err(invalid("judges", "pick at least one judge"));
    }

    let session_duration_secs = req
        .session_duration_secs
        .unwrap_or(personal::DEFAULT_SESSION_SECS);
    if !(personal::MIN_SESSION_SECS..=personal::MAX_SESSION_SECS).contains(&session_duration_secs) {
        return Err(invalid(
            "session_duration_secs",
            format!(
                "must be between {} and {} minutes",
                personal::MIN_SESSION_SECS / 60,
                personal::MAX_SESSION_SECS / 60
            ),
        ));
    }

    Ok(PersonalSpec {
        version: SPEC_VERSION,
        name,
        description,
        tasks,
        judges,
        session_duration_secs,
    })
}

/// A panel as asked for: every slug a judge a personal project may use, no
/// duplicates (the first mention keeps its place), 1 to
/// [`personal::MAX_JUDGES`] of them. The error is the detail for the field.
fn panel_of(slugs: &[String], eligible: &[judges::Model]) -> Result<Vec<String>, String> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for slug in slugs {
        let slug = slug.trim();
        if !eligible.iter().any(|j| j.slug == slug) {
            return Err(format!(
                "{slug:?} is not a judge a personal project can use"
            ));
        }
        if seen.insert(slug.to_string()) {
            out.push(slug.to_string());
        }
    }
    if out.is_empty() {
        return Err("pick at least one judge".to_string());
    }
    if out.len() > personal::MAX_JUDGES {
        return Err(format!("at most {} judges per task", personal::MAX_JUDGES));
    }
    Ok(out)
}

/// A name for a project the user did not name: the first line of the
/// description, stripped of list and heading marks, cut at a word.
pub(super) fn derive_name(description: &str) -> String {
    const LIMIT: usize = 60;
    let line = description
        .lines()
        .map(|l| {
            l.trim()
                .trim_start_matches(['#', '-', '*', '>', ' '])
                .trim()
        })
        .find(|l| !l.is_empty())
        .unwrap_or_default();
    if line.chars().count() <= LIMIT {
        return if line.is_empty() {
            "My project".to_string()
        } else {
            line.to_string()
        };
    }
    let cut: String = line.chars().take(LIMIT).collect();
    let at_word = cut
        .rfind(char::is_whitespace)
        .filter(|&i| i > LIMIT / 2)
        .map_or(cut.as_str(), |i| &cut[..i]);
    format!("{}…", at_word.trim_end_matches([',', '.', ';', ':', ' ']))
}

/// A URL- and CLI-safe slug for `name` (see `projects::common::validate_slug`).
pub(super) fn slugify(name: &str) -> String {
    const LIMIT: usize = 48;
    let mut out = String::new();
    for c in name.chars().flat_map(char::to_lowercase) {
        if c.is_ascii_alphanumeric() {
            out.push(c);
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
        if out.len() >= LIMIT {
            break;
        }
    }
    let slug = out.trim_matches('-').to_string();
    if slug.is_empty() {
        "project".to_string()
    } else {
        slug
    }
}

/// A slug no project uses yet. Personal slugs avoid every existing slug,
/// not only the owner's: `ololo start <slug>` resolves the caller's own
/// project first, and a personal project named like a catalog one would
/// otherwise hide it from its owner.
pub(super) async fn unique_slug(
    db: &DatabaseConnection,
    name: &str,
) -> Result<String, sea_orm::DbErr> {
    let base = slugify(name);
    for n in 1..=50 {
        let candidate = if n == 1 {
            base.clone()
        } else {
            format!("{base}-{n}")
        };
        let taken = projects::Entity::find()
            .filter(projects::Column::Slug.eq(candidate.as_str()))
            .one(db)
            .await?
            .is_some();
        if !taken {
            return Ok(candidate);
        }
    }
    let suffix = Uuid::new_v4().simple().to_string();
    Ok(format!("{base}-{}", &suffix[..8]))
}

/// Every judge a personal task may be given, as the task builder needs it.
pub(super) fn catalog(eligible: &[judges::Model]) -> Vec<PanelJudge> {
    eligible
        .iter()
        .map(|j| PanelJudge {
            slug: j.slug.clone(),
            criteria: criteria_of(j),
            weight: personal::default_panel_weight(&j.slug),
        })
        .collect()
}

/// A built task in the shape the shared inserter
/// (`admin_export_import::insert_task_with_judges`) stores.
pub(super) fn export_task(task: BuiltTask) -> ExportTask {
    ExportTask {
        ordinal: task.ordinal,
        title: task.title,
        content: task.content,
        test_template: task.test_template,
        tags: vec!["personal".to_string()],
        points: Some(ExportTaskPoints {
            value: Some(personal::TASK_POINTS),
            fail: None,
            no_response: None,
            completion_bonus: Some(personal::TASK_COMPLETION_BONUS),
            health: Some(personal::TASK_HEALTH_POINTS),
        }),
        intervals: None,
        judges: task
            .judges
            .iter()
            .map(|j| JudgeRef::Weighted {
                slug: j.slug.clone(),
                weight: Some(j.weight),
            })
            .collect(),
        evaluation: Some(task.evaluation),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_is_the_first_line_cut_at_a_word() {
        assert_eq!(derive_name("# Add CSV export\nmore"), "Add CSV export");
        assert_eq!(
            derive_name("\n\n- fix the login flow"),
            "fix the login flow"
        );
        assert_eq!(derive_name("   "), "My project");
        let long = derive_name(
            "Rework the billing reconciliation so that failed webhooks retry with backoff and alert",
        );
        assert!(long.ends_with('…'), "{long}");
        assert!(long.chars().count() <= 61, "{long}");
        assert!(!long.contains("backoff"), "{long}");
    }

    fn judge(slug: &str) -> judges::Model {
        judges::Model {
            id: Uuid::new_v4(),
            slug: slug.to_string(),
            name: slug.to_string(),
            description: String::new(),
            prompt: String::new(),
            rating_scale: serde_json::json!({"min": 0, "max": 10, "step": 0.1}),
            kind: arena_core::judging::JUDGE_KIND_LLM.to_string(),
            scope: arena_core::judging::JUDGE_SCOPE_TASK.to_string(),
            evidence_mode: "tools".to_string(),
            evidence_needs: None,
            llm_provider_id_fk: None,
            llm_model: None,
            llm_pool_id_fk: None,
            llm_source_order: "pool_first".to_string(),
            criteria: Some(r#"["x"]"#.to_string()),
            ignore_paths: None,
            probes_config: None,
            max_interactive: None,
            avatar_url: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        }
    }

    #[test]
    fn the_default_panel_answers_does_it_work_first() {
        let full: Vec<_> = [
            "correctness",
            "code-quality",
            "test-quality",
            "build-review",
        ]
        .map(judge)
        .into();
        assert_eq!(
            default_panel(&full),
            ["correctness", "code-quality", "test-quality"]
        );
        // The open platform: build-review stands in for correctness.
        let open: Vec<_> = ["code-quality", "test-quality", "build-review"]
            .map(judge)
            .into();
        assert_eq!(
            default_panel(&open),
            ["build-review", "code-quality", "test-quality"]
        );
        let lone = vec![judge("architecture")];
        assert_eq!(default_panel(&lone), ["architecture"]);
    }

    #[test]
    fn slugs_are_what_validate_slug_accepts() {
        for (name, want) in [
            ("Add CSV export", "add-csv-export"),
            ("  Fix: the -- login flow!  ", "fix-the-login-flow"),
            ("Ünïcode ✓ only", "n-code-only"),
            ("✓✓✓", "project"),
        ] {
            let slug = slugify(name);
            assert_eq!(slug, want, "{name:?}");
            crate::api::projects::validate_slug(&slug).expect("valid slug");
        }
        let long = slugify(&"word ".repeat(40));
        assert!(long.len() <= 48, "{long}");
        crate::api::projects::validate_slug(&long).expect("valid slug");
    }
}
