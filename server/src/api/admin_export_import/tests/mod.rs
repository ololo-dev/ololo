use crate::AuthConfig;
use arena_core::entities::users;
use arena_core::task_template::{Backoff, Matchers, TestKind};
use chrono::Utc;
use migration::{Migrator, MigratorTrait};
use sea_orm::{ActiveModelTrait, Database, PaginatorTrait, Set};

use super::*;

async fn fresh_state() -> AppState {
    let db = Database::connect("sqlite::memory:")
        .await
        .expect("db connect");
    Migrator::up(&db, None).await.expect("migrate");
    let cfg = AuthConfig {
        jwt_signing_key: b"export-import-tests-secret-key-32!".to_vec(),
        frontend_origins: vec!["http://localhost:5173".to_string()],
        access_ttl: std::time::Duration::from_secs(900),
        refresh_ttl: std::time::Duration::from_secs(86400),
        max_agents_per_session: 16,
    };
    AppState::new(db, cfg)
}

async fn seed_admin(state: &AppState) -> AdminUser {
    let user_id = Uuid::new_v4();
    users::ActiveModel {
        id: Set(user_id),
        email: Set("admin@test.local".to_string()),
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
    .insert(&state.db)
    .await
    .expect("insert admin");
    AdminUser { id: user_id }
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

async fn seed_project(state: &AppState, owner_id: Uuid) -> Uuid {
    let project_id = Uuid::new_v4();
    let now = Utc::now();
    projects::ActiveModel {
        id: Set(project_id),
        name: Set("Original".to_string()),
        slug: Set(None),
        description: Set("desc".to_string()),
        category: Set(None),
        tags: Set(serde_json::to_string(&["alpha".to_string()]).unwrap()),
        cover_image_url: Set(None),
        owner_user_id_fk: Set(owner_id),
        public: Set(true),
        archived_at: Set(None),
        created_at: Set(now),
        updated_at: Set(now),
        default_value_points: Set(10),
        default_fail_points: Set(-5),
        default_no_response_points: Set(-10),
        default_completion_bonus_points: Set(10),
        default_deadline_secs: Set(60),
        default_session_duration_secs: Set(3600),
        idle_timeout_secs: Set(300),
        default_min_interval_secs: Set(5),
        default_interval_increment_secs: Set(5),
        default_max_interval_secs: Set(60),
        memory_schema: Set(None),
        show_tasks: Set(true),
        parent_project_id_fk: Set(None),
        part_ordinal: Set(None),
    }
    .insert(&state.db)
    .await
    .expect("insert project");

    let tpl = sample_template();
    let tpl_json = serde_json::to_value(&tpl).unwrap();
    for (ordinal, title) in [(1, "First"), (2, "Second")] {
        tasks::ActiveModel {
            id: Set(Uuid::new_v4()),
            project_id_fk: Set(project_id),
            ordinal: Set(ordinal),
            title: Set(title.to_string()),
            content: Set(format!("content-{title}")),
            test_template: Set(tpl_json.clone()),
            tags: Set(serde_json::to_string(&["t".to_string()]).unwrap()),
            created_at: Set(now),
            point_value: Set(10),
            deadline_secs: Set(Some(60)),
            min_interval_secs: Set(Some(30)),
            interval_increment_secs: Set(Some(10)),
            max_interval_secs: Set(Some(300)),
            fail_points: Set(-5),
            no_response_points: Set(-10),
            completion_bonus_points: Set(10),
            evaluation: Set(None),
        }
        .insert(&state.db)
        .await
        .expect("insert task");
    }
    project_id
}

fn empty_project() -> ExportProject {
    ExportProject {
        name: "X".to_string(),
        slug: None,
        description: None,
        category: None,
        tags: vec![],
        cover_image_url: None,
        public: false,
        archived_at: None,
        points: ExportPoints {
            value: 10,
            fail: -5,
            no_response: -10,
            completion_bonus: 10,
        },
        intervals: ExportIntervals {
            deadline_secs: 60,
            min_interval_secs: 5,
            interval_increment_secs: 5,
            max_interval_secs: 60,
        },
        session_duration_secs: 3600,
        memory_schema: None,
        show_tasks: true,
        parts: Vec::new(),
    }
}

fn task_with(ordinal: i32, template: TestTemplate) -> ExportTask {
    ExportTask {
        ordinal,
        title: "t".to_string(),
        content: "c".to_string(),
        test_template: template,
        tags: vec![],
        points: None,
        intervals: Some(ExportTaskIntervals {
            deadline_secs: Some(60),
            min_interval_secs: Some(30),
            interval_increment_secs: Some(10),
            max_interval_secs: Some(300),
        }),
        judges: vec![],
        evaluation: None,
    }
}

mod apply_seed;
mod campaign_parts;
mod export_import;
