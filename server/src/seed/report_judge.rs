//! Every project gets the session report.
//!
//! A report judge is not something a project opts into task by task: it writes
//! the player's debrief, and a player who finished a session deserves one
//! whatever they played. Rather than asking fifty task files to remember an
//! attachment line, the attachment is ensured centrally — at boot for every
//! project, and again whenever a project's tasks are (re)written.
//!
//! It attaches to the project's **first** task, and to exactly one task. The
//! judge is session-scoped, so one attachment is one run per player; attaching
//! it to every task would multiply the settle poll's expectations without
//! adding a word to the report.
//!
//! The first task is not cosmetic. A session-scoped judge only runs over the
//! tasks a player *reached*, and "reached" means every ordinal up to the one
//! they were on when time ran out. Anchored on the last task, the report was
//! written only for players who got all the way there: a session that ended on
//! task 2 of 10 produced no report at all, and the page told the player their
//! project does not run one. The lowest ordinal is in every non-empty reached
//! set, so anchoring there means everyone who played gets a debrief — and the
//! settle poll, which counts the same intersection, waits for it every time.
//!
//! Two consequences fall out of the attachment being a real `task_judges` row,
//! and both are intended: the project's estimated judge reviews include it,
//! and the run is metered against the player's judge-run balance like any
//! other.

use arena_core::entities::{judges, task_judges, tasks};
use sea_orm::{ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, Set};
use uuid::Uuid;

/// Attach every report judge to `project_id`'s first task. Returns how many
/// attachments were made or moved.
///
/// An attachment that already sits on the first task is left alone; one that
/// sits anywhere else is moved there rather than duplicated, which re-anchors
/// every project seeded before this rule without a migration.
///
/// A project with no tasks — a campaign parent — is skipped: it hosts no
/// sessions, so there is nothing to report on.
pub async fn ensure_report_judges(
    db: &DatabaseConnection,
    project_id: Uuid,
) -> Result<usize, sea_orm::DbErr> {
    let reporters: Vec<judges::Model> = judges::Entity::find()
        .filter(judges::Column::Kind.eq(arena_core::judging::JUDGE_KIND_REPORT))
        .all(db)
        .await?;
    if reporters.is_empty() {
        return Ok(0);
    }

    let mut project_tasks = tasks::Entity::find()
        .filter(tasks::Column::ProjectIdFk.eq(project_id))
        .all(db)
        .await?;
    if project_tasks.is_empty() {
        return Ok(0);
    }
    project_tasks.sort_by_key(|t| t.ordinal);
    let first_task = project_tasks.first().expect("non-empty").clone();

    let task_ids: Vec<Uuid> = project_tasks.iter().map(|t| t.id).collect();
    let existing = task_judges::Entity::find()
        .filter(task_judges::Column::TaskId.is_in(task_ids))
        .all(db)
        .await?;

    let now = chrono::Utc::now();
    let mut attached = 0usize;
    for reporter in reporters {
        // Sit last in the task's panel: the debrief is written after the
        // judges it summarises.
        let next_ordinal = existing
            .iter()
            .filter(|tj| tj.task_id == first_task.id)
            .map(|tj| tj.ordinal)
            .max()
            .map(|o| o + 1)
            .unwrap_or(0);

        match existing.iter().find(|tj| tj.judge_id == reporter.id) {
            Some(tj) if tj.task_id == first_task.id => continue,
            // Seeded under the old rule, on a task most players never reach.
            // Move the row rather than adding a second: two attachments would
            // mean two runs, two charges, and two rows the settle poll waits
            // for.
            Some(tj) => {
                let mut am: task_judges::ActiveModel = tj.clone().into();
                am.task_id = Set(first_task.id);
                am.ordinal = Set(next_ordinal);
                am.updated_at = Set(now);
                am.update(db).await?;
                tracing::info!(
                    project_id = %project_id, judge = %reporter.slug,
                    "report judge: re-anchored to the project's first task"
                );
            }
            None => {
                task_judges::ActiveModel {
                    id: Set(Uuid::new_v4()),
                    task_id: Set(first_task.id),
                    judge_id: Set(reporter.id),
                    ordinal: Set(next_ordinal),
                    rating_scale_override: Set(None),
                    weight: Set(None),
                    created_at: Set(now),
                    updated_at: Set(now),
                }
                .insert(db)
                .await?;
            }
        }
        attached += 1;
    }
    Ok(attached)
}

/// Ensure the report judge on every project. Logged and swallowed per project:
/// a failure here must never stop the server from starting.
pub async fn ensure_report_judges_everywhere(db: &DatabaseConnection) {
    let project_ids: Vec<Uuid> = match arena_core::entities::projects::Entity::find().all(db).await
    {
        Ok(rows) => rows.into_iter().map(|p| p.id).collect(),
        Err(e) => {
            tracing::error!(error = %e, "report judge: project lookup failed");
            return;
        }
    };

    let mut attached = 0usize;
    for project_id in project_ids {
        match ensure_report_judges(db, project_id).await {
            Ok(n) => attached += n,
            Err(e) => {
                tracing::error!(project_id = %project_id, error = %e, "report judge: attach failed")
            }
        }
    }
    if attached > 0 {
        tracing::info!(
            attached,
            "report judge: attached to projects that lacked one"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use migration::{Migrator, MigratorTrait};
    use sea_orm::Database;

    async fn db() -> DatabaseConnection {
        let db = Database::connect("sqlite::memory:").await.expect("connect");
        Migrator::up(&db, None).await.expect("migrate");
        db
    }

    async fn seed_judge(db: &DatabaseConnection, slug: &str, kind: &str) -> Uuid {
        let id = Uuid::new_v4();
        let now = chrono::Utc::now();
        judges::ActiveModel {
            id: Set(id),
            slug: Set(slug.to_string()),
            name: Set(slug.to_string()),
            description: Set(String::new()),
            prompt: Set("p".into()),
            rating_scale: Set(serde_json::json!({"min": 0, "max": 1, "step": 1})),
            kind: Set(kind.to_string()),
            scope: Set(if kind == arena_core::judging::JUDGE_KIND_REPORT {
                "session".to_string()
            } else {
                "task".to_string()
            }),
            evidence_mode: Set("tools".into()),
            evidence_needs: Set(None),
            criteria: Set(None),
            max_interactive: Set(None),
            avatar_url: Set(None),
            ignore_paths: Set(None),
            llm_provider_id_fk: Set(None),
            llm_model: Set(None),
            llm_pool_id_fk: Set(None),
            llm_source_order: Set("pool_first".into()),
            probes_config: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
        }
        .insert(db)
        .await
        .expect("insert judge");
        id
    }

    async fn seed_project_with_tasks(db: &DatabaseConnection, ordinals: &[i32]) -> Uuid {
        let owner = Uuid::new_v4();
        let now = chrono::Utc::now();
        arena_core::entities::users::ActiveModel {
            id: Set(owner),
            email: Set(format!("{owner}@x.test")),
            password_hash: Set(None),
            display_name: Set("o".into()),
            created_at: Set(now),
            updated_at: Set(now),
            is_admin: Set(false),
            avatar_url: Set(None),
            email_verified: Set(true),
            username: Set(None),
            plan: Set("free".into()),
            judge_run_limit: Set(None),
            judge_run_credits: Set(0),
        }
        .insert(db)
        .await
        .expect("insert user");

        let project_id = Uuid::new_v4();
        arena_core::entities::projects::ActiveModel {
            id: Set(project_id),
            name: Set("p".into()),
            slug: Set(Some(format!("p{}", &project_id.to_string()[..8]))),
            description: Set(String::new()),
            category: Set(None),
            tags: Set("[]".into()),
            cover_image_url: Set(None),
            owner_user_id_fk: Set(owner),
            public: Set(true),
            archived_at: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
            default_value_points: Set(10),
            default_fail_points: Set(-5),
            default_no_response_points: Set(-10),
            default_completion_bonus_points: Set(10),
            default_deadline_secs: Set(60),
            default_session_duration_secs: Set(900),
            idle_timeout_secs: Set(300),
            default_min_interval_secs: Set(5),
            default_interval_increment_secs: Set(5),
            default_max_interval_secs: Set(60),
            memory_schema: Set(None),
            show_tasks: Set(true),
            parent_project_id_fk: Set(None),
            part_ordinal: Set(None),
        }
        .insert(db)
        .await
        .expect("insert project");

        for ordinal in ordinals {
            tasks::ActiveModel {
                id: Set(Uuid::new_v4()),
                project_id_fk: Set(project_id),
                ordinal: Set(*ordinal),
                title: Set(format!("t{ordinal}")),
                content: Set(String::new()),
                test_template: Set(serde_json::json!({"kind":"shell","command_template":"echo"})),
                created_at: Set(now),
                tags: Set("[]".into()),
                point_value: Set(10),
                deadline_secs: Set(None),
                min_interval_secs: Set(None),
                interval_increment_secs: Set(None),
                max_interval_secs: Set(None),
                fail_points: Set(-5),
                no_response_points: Set(-10),
                completion_bonus_points: Set(10),
                evaluation: Set(None),
            }
            .insert(db)
            .await
            .expect("insert task");
        }
        project_id
    }

    async fn attachments(db: &DatabaseConnection, project_id: Uuid) -> Vec<(i32, Uuid)> {
        let task_rows = tasks::Entity::find()
            .filter(tasks::Column::ProjectIdFk.eq(project_id))
            .all(db)
            .await
            .unwrap();
        let by_id: std::collections::HashMap<Uuid, i32> =
            task_rows.iter().map(|t| (t.id, t.ordinal)).collect();
        let mut out: Vec<(i32, Uuid)> = task_judges::Entity::find()
            .filter(task_judges::Column::TaskId.is_in(task_rows.iter().map(|t| t.id)))
            .all(db)
            .await
            .unwrap()
            .into_iter()
            .map(|tj| (by_id[&tj.task_id], tj.judge_id))
            .collect();
        out.sort();
        out
    }

    #[tokio::test]
    async fn the_report_lands_on_the_first_task() {
        // Everyone who played reached ordinal 0; only the finishers reached
        // the last one. The report is for everyone.
        let db = db().await;
        let reporter = seed_judge(&db, "general", arena_core::judging::JUDGE_KIND_REPORT).await;
        let project = seed_project_with_tasks(&db, &[0, 1, 2]).await;

        assert_eq!(ensure_report_judges(&db, project).await.unwrap(), 1);
        assert_eq!(attachments(&db, project).await, vec![(0, reporter)]);
    }

    #[tokio::test]
    async fn an_attachment_seeded_on_the_last_task_moves_to_the_first() {
        // Projects seeded under the old rule produced no report for a player
        // who ran out of time early. Re-anchoring is how they are repaired —
        // moved, never duplicated, or the player would be charged twice and
        // the settle poll would wait for a row nobody writes.
        let db = db().await;
        let reporter = seed_judge(&db, "general", arena_core::judging::JUDGE_KIND_REPORT).await;
        let project = seed_project_with_tasks(&db, &[0, 1, 2]).await;
        let last = tasks::Entity::find()
            .filter(tasks::Column::ProjectIdFk.eq(project))
            .all(&db)
            .await
            .unwrap()
            .into_iter()
            .max_by_key(|t| t.ordinal)
            .unwrap();
        let now = chrono::Utc::now();
        task_judges::ActiveModel {
            id: Set(Uuid::new_v4()),
            task_id: Set(last.id),
            judge_id: Set(reporter),
            ordinal: Set(0),
            rating_scale_override: Set(None),
            weight: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
        }
        .insert(&db)
        .await
        .expect("seed the old attachment");

        assert_eq!(ensure_report_judges(&db, project).await.unwrap(), 1);
        assert_eq!(attachments(&db, project).await, vec![(0, reporter)]);
        // And the second sweep leaves it where it is.
        assert_eq!(ensure_report_judges(&db, project).await.unwrap(), 0);
        assert_eq!(attachments(&db, project).await.len(), 1);
    }

    #[tokio::test]
    async fn running_twice_does_not_attach_twice() {
        // Boot sweeps and seed pushes both call this; a second attachment
        // would double what the settle poll waits for and what the player is
        // billed.
        let db = db().await;
        seed_judge(&db, "general", arena_core::judging::JUDGE_KIND_REPORT).await;
        let project = seed_project_with_tasks(&db, &[0, 1]).await;

        assert_eq!(ensure_report_judges(&db, project).await.unwrap(), 1);
        assert_eq!(ensure_report_judges(&db, project).await.unwrap(), 0);
        assert_eq!(attachments(&db, project).await.len(), 1);
    }

    #[tokio::test]
    async fn a_project_with_no_tasks_is_left_alone() {
        // A campaign parent hosts no sessions, so there is nothing to report.
        let db = db().await;
        seed_judge(&db, "general", arena_core::judging::JUDGE_KIND_REPORT).await;
        let project = seed_project_with_tasks(&db, &[]).await;

        assert_eq!(ensure_report_judges(&db, project).await.unwrap(), 0);
        assert!(attachments(&db, project).await.is_empty());
    }

    #[tokio::test]
    async fn scoring_judges_are_not_attached_by_this_sweep() {
        let db = db().await;
        seed_judge(&db, "architecture", arena_core::judging::JUDGE_KIND_LLM).await;
        let project = seed_project_with_tasks(&db, &[0]).await;

        assert_eq!(ensure_report_judges(&db, project).await.unwrap(), 0);
        assert!(attachments(&db, project).await.is_empty());
    }

    #[tokio::test]
    async fn every_project_gets_one() {
        let db = db().await;
        seed_judge(&db, "general", arena_core::judging::JUDGE_KIND_REPORT).await;
        let a = seed_project_with_tasks(&db, &[0]).await;
        let b = seed_project_with_tasks(&db, &[0, 1]).await;

        ensure_report_judges_everywhere(&db).await;
        assert_eq!(attachments(&db, a).await.len(), 1);
        assert_eq!(attachments(&db, b).await.len(), 1);
    }
}
