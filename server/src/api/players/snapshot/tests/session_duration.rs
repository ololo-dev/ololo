use super::*;

#[tokio::test]
async fn build_snapshot_session_ends_at_uses_project_duration() {
    let state = fresh_state().await;
    let db = &state.db;

    let user_id = Uuid::new_v4();
    crate::entities::users::ActiveModel {
        id: Set(user_id),
        email: Set("u6@test.local".to_string()),
        password_hash: Set(None),
        display_name: Set("User6".to_string()),
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
        name: Set("proj6".to_string()),
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
        default_session_duration_secs: Set(120),
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

    let started_at = Utc::now();
    let session_id = Uuid::new_v4();
    crate::entities::sessions::ActiveModel {
        id: Set(session_id),
        name: Set("s6".to_string()),
        created_at: Set(Utc::now()),
        owner_id_fk: Set(Some(user_id)),
        status: Set(SessionStatus::Running),
        join_code: Set("PQR678".to_string()),
        started_at: Set(Some(started_at)),
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
        display_name: Set("p6".to_string()),
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

    let snap = build_snapshot(&state, player_id, session_id, "p6", 0).await;
    let ends_at = snap.session_ends_at.expect("session_ends_at");
    assert_eq!(
        ends_at,
        started_at + chrono::Duration::seconds(120),
        "session_ends_at must reflect the project's default_session_duration_secs (120), \
         not a process-wide default"
    );
}
