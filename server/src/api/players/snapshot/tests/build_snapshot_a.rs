use super::*;

#[tokio::test]
async fn build_snapshot_uses_project_tasks_when_adapted_tasks_absent() {
    let state = fresh_state().await;
    let db = &state.db;

    let user_id = Uuid::new_v4();
    crate::entities::users::ActiveModel {
        id: Set(user_id),
        email: Set("u@test.local".to_string()),
        password_hash: Set(None),
        display_name: Set("User".to_string()),
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
        name: Set("proj".to_string()),
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
        name: Set("s".to_string()),
        created_at: Set(Utc::now()),
        owner_id_fk: Set(Some(user_id)),
        status: Set(SessionStatus::Running),
        join_code: Set("ABC123".to_string()),
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
        display_name: Set("p".to_string()),
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

    let task_id = Uuid::new_v4();
    let template = crate::task_template::TestTemplate {
        kind: crate::task_template::TestKind::Shell,
        command_template: "echo hi".to_string(),
        placeholders: vec![],
        matchers: crate::task_template::Matchers::default(),
        backoff: crate::task_template::Backoff::default(),
        fixtures: vec![],
        answer_template: Some("".to_string()),
    };
    crate::entities::tasks::ActiveModel {
        id: Set(task_id),
        project_id_fk: Set(project_id),
        ordinal: Set(1),
        title: Set("t1".to_string()),
        content: Set("d".to_string()),
        test_template: Set(serde_json::to_value(template).expect("serialize")),
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

    let snap = build_snapshot(&state, player_id, session_id, "p", 0).await;
    assert_eq!(snap.tasks.len(), 1);
    assert_eq!(snap.tasks[0].task_id, task_id);
    assert_eq!(snap.tasks[0].title, "t1");
}

#[tokio::test]
async fn build_snapshot_includes_test_probe_and_result_details() {
    let state = fresh_state().await;
    let db = &state.db;

    let user_id = Uuid::new_v4();
    crate::entities::users::ActiveModel {
        id: Set(user_id),
        email: Set("u2@test.local".to_string()),
        password_hash: Set(None),
        display_name: Set("User2".to_string()),
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
        name: Set("proj2".to_string()),
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
        name: Set("s2".to_string()),
        created_at: Set(Utc::now()),
        owner_id_fk: Set(Some(user_id)),
        status: Set(SessionStatus::Running),
        join_code: Set("DEF456".to_string()),
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
        display_name: Set("p2".to_string()),
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

    let task_id = Uuid::new_v4();
    let template = crate::task_template::TestTemplate {
        kind: crate::task_template::TestKind::Shell,
        command_template: "echo true".to_string(),
        placeholders: vec![],
        matchers: crate::task_template::Matchers::default(),
        backoff: crate::task_template::Backoff::default(),
        fixtures: vec![],
        answer_template: Some("true".to_string()),
    };
    crate::entities::tasks::ActiveModel {
        id: Set(task_id),
        project_id_fk: Set(project_id),
        ordinal: Set(1),
        title: Set("task".to_string()),
        content: Set("d".to_string()),
        test_template: Set(serde_json::to_value(template).expect("serialize")),
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

    let test_id = Uuid::new_v4();
    crate::entities::tests::ActiveModel {
        id: Set(test_id),
        command_template: Set("echo true".to_string()),
        answer_template: Set("true".to_string()),
        fixture_definitions: Set("[]".to_string()),
        created_at: Set(Utc::now()),
        session_id: Set(session_id),
        task_id: Set(task_id),
        ordinal: Set(0),
        prompt: Set("".to_string()),
        description: Set(None),
        initiator: Set("system".to_string()),
        probe_config: Set(None),
        registered_by_judge_id: Set(None),
    }
    .insert(db)
    .await
    .expect("insert adapted test");

    crate::entities::probes::ActiveModel {
        id: Set(Uuid::new_v4()),
        test_id: Set(test_id),
        player_id: Set(player_id),
        session_id: Set(session_id),
        attempt: Set(1),
        rendered_command: Set("echo true".to_string()),
        fixture_values: Set("{}".to_string()),
        expected_answer: Set(Some("true".to_string())),
        outcome: Set(Some("error".to_string())),
        dispatched_at: Set(Utc::now()),
        deadline_at: Set(Utc::now()),
        resolved_at: Set(Some(Utc::now())),
        output: Set(Some("false".to_string())),
        exit_code: Set(Some(1)),
        duration_ms: Set(Some(12)),
        point_delta: Set(Some(0)),
        updated_at: Set(Some(Utc::now())),
        resolved_answer: Set(None),
        secret_meta: Set(None),
        artifact_path: Set(None),
        result_json: Set(None),
    }
    .insert(db)
    .await
    .expect("insert probe");

    let snap = build_snapshot(&state, player_id, session_id, "p2", 0).await;
    assert_eq!(snap.tasks.len(), 1);
    assert_eq!(snap.probes.len(), 1);
    assert_eq!(snap.probes[0].point_delta, 0);
    let result = snap.probes[0].result.as_ref().expect("probe result");
    assert_eq!(result.status, "failed");
    assert_eq!(result.expected.as_deref(), Some("true"));
    assert_eq!(result.actual.as_deref(), Some("false"));
}
