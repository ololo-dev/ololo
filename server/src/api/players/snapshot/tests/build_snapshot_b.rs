use super::*;

#[tokio::test]
async fn build_snapshot_marks_only_one_current_task() {
    let state = fresh_state().await;
    let db = &state.db;

    let user_id = Uuid::new_v4();
    crate::entities::users::ActiveModel {
        id: Set(user_id),
        email: Set("u3@test.local".to_string()),
        password_hash: Set(None),
        display_name: Set("User3".to_string()),
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
        name: Set("proj3".to_string()),
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
        name: Set("s3".to_string()),
        created_at: Set(Utc::now()),
        owner_id_fk: Set(Some(user_id)),
        status: Set(SessionStatus::Running),
        join_code: Set("GHI789".to_string()),
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
        display_name: Set("p3".to_string()),
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

    crate::entities::session_scheduler_state::ActiveModel {
        id: Set(Uuid::new_v4()),
        session_id_fk: Set(session_id),
        player_id_fk: Set(player_id),
        task_id: Set(Some(task_b)),
        state: Set("awaiting_result".to_string()),
        next_probe_at: Set(None),
        created_at: Set(Utc::now()),
        updated_at: Set(Utc::now()),
    }
    .insert(db)
    .await
    .expect("insert sched state");

    let snap = build_snapshot(&state, player_id, session_id, "p3", 0).await;
    let current_count = snap
        .tasks
        .iter()
        .filter(|task| task.scheduler_state.is_some())
        .count();
    assert_eq!(current_count, 1);
    let current_task = snap
        .tasks
        .iter()
        .find(|task| task.scheduler_state.is_some())
        .expect("one current task");
    assert_eq!(current_task.task_id, task_b);
}

#[tokio::test]
async fn build_snapshot_filters_tasks_by_current_ordinal() {
    let state = fresh_state().await;
    let db = &state.db;

    let user_id = Uuid::new_v4();
    crate::entities::users::ActiveModel {
        id: Set(user_id),
        email: Set("u4@test.local".to_string()),
        password_hash: Set(None),
        display_name: Set("User4".to_string()),
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
        name: Set("proj4".to_string()),
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
        name: Set("s4".to_string()),
        created_at: Set(Utc::now()),
        owner_id_fk: Set(Some(user_id)),
        status: Set(SessionStatus::Running),
        join_code: Set("JKL012".to_string()),
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
        display_name: Set("p4".to_string()),
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
    let task_c = Uuid::new_v4();
    for (idx, tid, title) in [(1, task_a, "A"), (2, task_b, "B"), (3, task_c, "C")] {
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

    crate::entities::session_scheduler_state::ActiveModel {
        id: Set(Uuid::new_v4()),
        session_id_fk: Set(session_id),
        player_id_fk: Set(player_id),
        task_id: Set(Some(task_b)),
        state: Set("awaiting_result".to_string()),
        next_probe_at: Set(None),
        created_at: Set(Utc::now()),
        updated_at: Set(Utc::now()),
    }
    .insert(db)
    .await
    .expect("insert sched state");

    let snap = build_snapshot(&state, player_id, session_id, "p4", 0).await;
    assert_eq!(
        snap.total_tasks, 3,
        "total_tasks should be unfiltered count"
    );
    assert_eq!(
        snap.tasks.len(),
        2,
        "tasks should be filtered to ordinal <= current_ordinal"
    );
    let task_ids: Vec<Uuid> = snap.tasks.iter().map(|t| t.task_id).collect();
    assert!(
        task_ids.contains(&task_a),
        "ordinal 1 task should be included"
    );
    assert!(
        task_ids.contains(&task_b),
        "ordinal 2 task (current) should be included"
    );
    assert!(
        !task_ids.contains(&task_c),
        "ordinal 3 task should be filtered out"
    );
}
