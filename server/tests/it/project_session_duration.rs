//! `projects.default_session_duration_secs` column semantics.
//!
//! The column is NOT NULL with a schema default of 3600, so rows inserted
//! without it (i.e. rows that predate the migration) must read back as 3600.

use chrono::Utc;
use migration::{Migrator, MigratorTrait};
use sea_orm::{ActiveModelTrait, Database, Set};
use uuid::Uuid;

use arena_core::entities::{projects, users};

#[tokio::test]
async fn project_session_duration_defaults_to_3600_when_unset() {
    let db = Database::connect("sqlite::memory:").await.expect("connect");
    Migrator::up(&db, None).await.expect("migrate up");

    let owner = Uuid::new_v4();
    users::ActiveModel {
        id: Set(owner),
        email: Set(format!("u{owner}@example.com")),
        password_hash: Set(None),
        display_name: Set("tester".to_string()),
        created_at: Set(Utc::now()),
        updated_at: Set(Utc::now()),
        is_admin: Set(false),
        avatar_url: Set(None),
        email_verified: Set(false),
        username: Set(None),
        plan: Set(arena_core::quota::PLAN_PREMIUM.to_string()),
        judge_run_limit: Set(None),
        judge_run_credits: Set(0),
    }
    .insert(&db)
    .await
    .expect("insert user");

    // The new column is deliberately left NotSet so the INSERT omits it and
    // the schema default applies — the same path pre-migration rows take.
    let project = projects::ActiveModel {
        id: Set(Uuid::new_v4()),
        name: Set("proj".to_string()),
        slug: Set(None),
        description: Set(String::new()),
        category: Set(None),
        tags: Set(String::new()),
        cover_image_url: Set(None),
        owner_user_id_fk: Set(owner),
        public: Set(true),
        archived_at: Set(None),
        created_at: Set(Utc::now()),
        updated_at: Set(Utc::now()),
        default_value_points: Set(10),
        default_fail_points: Set(-5),
        default_no_response_points: Set(-10),
        default_completion_bonus_points: Set(10),
        default_deadline_secs: Set(60),
        default_min_interval_secs: Set(5),
        default_interval_increment_secs: Set(5),
        default_max_interval_secs: Set(60),
        ..Default::default()
    }
    .insert(&db)
    .await
    .expect("insert project");

    assert_eq!(project.default_session_duration_secs, 3600);
}
