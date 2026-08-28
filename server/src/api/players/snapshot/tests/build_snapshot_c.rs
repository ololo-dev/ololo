use super::*;

#[tokio::test]
async fn build_snapshot_no_scheduler_shows_all_tasks() {
    let state = fresh_state().await;
    let db = &state.db;

    let user_id = Uuid::new_v4();
    crate::entities::users::ActiveModel {
        id: Set(user_id),
        email: Set("u5@test.local".to_string()),
        password_hash: Set(None),
        display_name: Set("User5".to_string()),
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
    .insert(db)
    .await
    .expect("insert user");

    let project_id = Uuid::new_v4();
    crate::entities::projects::ActiveModel {
        id: Set(project_id),
        name: Set("proj5".to_string()),
        slug: Set(None),
        description: Set("".to_string()),
        category: Set(None),
        tags: Set("[]".to_string()),
        cover_image_url: Set(None),
        owner_user_id_fk: Set(user_id),
        public: Set(false),
        archived_at: Set(None),
        created_at: Set(Utc::now()),
        updated_at: Set(Utc::now()),
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
    .insert(db)
    .await
    .expect("insert project");

    let session_id = Uuid::new_v4();
    crate::entities::sessions::ActiveModel {
        id: Set(session_id),
        name: Set("s5".to_string()),
        created_at: Set(Utc::now()),
        owner_id_fk: Set(Some(user_id)),
        status: Set(SessionStatus::Running),
        join_code: Set("MNO345".to_string()),
        started_at: Set(None),
        finished_at: Set(None),
        paused_at: Set(None),
        paused_duration_secs: Set(None),
        project_id_fk: Set(project_id),
        game_server_id: Set(None),
        cancel_reason: Set(None),
        cancelled_by: Set(None),
    }
    .insert(db)
    .await
    .expect("insert session");

    let player_id = Uuid::new_v4();
    crate::entities::players::ActiveModel {
        id: Set(player_id),
        session_id_fk: Set(session_id),
        user_id_fk: Set(Some(user_id)),
        display_name: Set("p5".to_string()),
        fingerprint: Set(None),
        metadata_json: Set(None),
        joined_at: Set(Utc::now()),
        reconnected_at: Set(None),
        revoked_at: Set(None),
        agent_connected: Set(false),
        agent_last_seen_at: Set(None),
    }
    .insert(db)
    .await
    .expect("insert player");

    let template = crate::task_template::TestTemplate {
        kind: crate::task_template::TestKind::Shell,
        command_template: "echo".to_string(),
        placeholders: vec![],
        matchers: crate::task_template::Matchers::default(),
        backoff: crate::task_template::Backoff::default(),
        fixtures: vec![],
        answer_template: Some("".to_string()),
    };

    let task_a = Uuid::new_v4();
    let task_b = Uuid::new_v4();
    for (idx, tid, title) in [(1, task_a, "A"), (2, task_b, "B")] {
        crate::entities::tasks::ActiveModel {
            id: Set(tid),
            project_id_fk: Set(project_id),
            ordinal: Set(idx),
            title: Set(title.to_string()),
            content: Set("d".to_string()),
            test_template: Set(serde_json::to_value(template.clone()).expect("serialize")),
            created_at: Set(Utc::now()),
            tags: Set("[]".to_string()),
            point_value: Set(10),
            completion_bonus_points: Set(10),
            deadline_secs: Set(Some(30)),
            min_interval_secs: Set(Some(1)),
            interval_increment_secs: Set(Some(1)),
            max_interval_secs: Set(Some(10)),
            fail_points: Set(0),
            no_response_points: Set(0),
            evaluation: Set(None),
        }
        .insert(db)
        .await
        .expect("insert task");
    }

    let snap = build_snapshot(&state, player_id, session_id, "p5", 0).await;
    assert_eq!(
        snap.total_tasks, 2,
        "total_tasks should be unfiltered count"
    );
    assert_eq!(
        snap.tasks.len(),
        2,
        "no scheduler state should show all tasks"
    );
}
