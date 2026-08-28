use super::*;

#[tokio::test]
async fn round_trip_export_then_import() {
    let state = fresh_state().await;
    let admin = seed_admin(&state).await;
    let original_id = seed_project(&state, admin.id).await;

    let resp = export_project(
        AdminUser { id: admin.id },
        axum::extract::State(state.clone()),
        Path(original_id),
    )
    .await
    .expect("export ok");
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let envelope: ExportEnvelope = serde_json::from_slice(&body).unwrap();
    assert_eq!(envelope.schema_version, 1);
    assert_eq!(envelope.project.name, "Original");
    assert_eq!(envelope.tasks.len(), 2);

    let resp = import_project(
        AdminUser { id: admin.id },
        axum::extract::State(state.clone()),
        Json(envelope),
    )
    .await
    .expect("import ok");
    assert_eq!(resp.status(), StatusCode::CREATED);
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let import_resp: ImportResponse = serde_json::from_slice(&body).unwrap();
    assert_eq!(import_resp.name, "Original");
    assert_ne!(import_resp.project_id, original_id);

    let new_proj = projects::Entity::find_by_id(import_resp.project_id)
        .one(&state.db)
        .await
        .unwrap()
        .expect("new project exists");
    assert_eq!(new_proj.owner_user_id_fk, admin.id);
    assert!(!new_proj.public);
    assert!(new_proj.archived_at.is_none());
    assert!(new_proj.cover_image_url.is_none());

    let new_tasks = tasks::Entity::find()
        .filter(tasks::Column::ProjectIdFk.eq(import_resp.project_id))
        .order_by_asc(tasks::Column::Ordinal)
        .all(&state.db)
        .await
        .unwrap();
    assert_eq!(new_tasks.len(), 2);
    assert_ne!(new_tasks[0].id, original_id);
    assert_eq!(new_tasks[0].title, "First");
    assert_eq!(new_tasks[1].title, "Second");
}

#[tokio::test]
async fn import_inherits_project_default_for_omitted_task_points() {
    let state = fresh_state().await;
    let admin = seed_admin(&state).await;

    let envelope = ExportEnvelope {
        schema_version: 1,
        project: ExportProject {
            name: "Inherit".to_string(),
            slug: None,
            description: None,
            category: None,
            tags: vec![],
            cover_image_url: None,
            public: false,
            archived_at: None,
            points: ExportPoints {
                value: 42,
                fail: -1,
                no_response: -2,
                completion_bonus: 7,
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
        },
        tasks: vec![ExportTask {
            ordinal: 0,
            title: "t".to_string(),
            content: "c".to_string(),
            test_template: sample_template(),
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
        }],
    };

    let resp = import_project(
        AdminUser { id: admin.id },
        axum::extract::State(state.clone()),
        Json(envelope),
    )
    .await
    .expect("import ok");
    assert_eq!(resp.status(), StatusCode::CREATED);
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let import_resp: ImportResponse = serde_json::from_slice(&body).unwrap();

    let row = tasks::Entity::find()
        .filter(tasks::Column::ProjectIdFk.eq(import_resp.project_id))
        .one(&state.db)
        .await
        .unwrap()
        .expect("task row exists");
    assert_eq!(row.point_value, 42, "value inherits project default");
    assert_eq!(row.fail_points, -1, "fail inherits project default");
    assert_eq!(
        row.no_response_points, -2,
        "no_response inherits project default"
    );
    assert_eq!(
        row.completion_bonus_points, 7,
        "completion_bonus inherits project default"
    );
}

#[tokio::test]
async fn import_task_override_beats_project_default() {
    let state = fresh_state().await;
    let admin = seed_admin(&state).await;

    let envelope = ExportEnvelope {
        schema_version: 1,
        project: ExportProject {
            name: "Override".to_string(),
            slug: None,
            description: None,
            category: None,
            tags: vec![],
            cover_image_url: None,
            public: false,
            archived_at: None,
            points: ExportPoints {
                value: 42,
                fail: -1,
                no_response: -2,
                completion_bonus: 7,
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
        },
        tasks: vec![ExportTask {
            ordinal: 0,
            title: "t".to_string(),
            content: "c".to_string(),
            test_template: sample_template(),
            tags: vec![],
            points: Some(ExportTaskPoints {
                value: Some(99),
                fail: None,
                no_response: None,
                completion_bonus: None,
            }),
            intervals: Some(ExportTaskIntervals {
                deadline_secs: Some(60),
                min_interval_secs: Some(30),
                interval_increment_secs: Some(10),
                max_interval_secs: Some(300),
            }),
            judges: vec![],
            evaluation: None,
        }],
    };

    let resp = import_project(
        AdminUser { id: admin.id },
        axum::extract::State(state.clone()),
        Json(envelope),
    )
    .await
    .expect("import ok");
    assert_eq!(resp.status(), StatusCode::CREATED);
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let import_resp: ImportResponse = serde_json::from_slice(&body).unwrap();

    let row = tasks::Entity::find()
        .filter(tasks::Column::ProjectIdFk.eq(import_resp.project_id))
        .one(&state.db)
        .await
        .unwrap()
        .expect("task row exists");
    assert_eq!(row.point_value, 99, "task override wins");
    assert_eq!(row.fail_points, -1, "fail still inherits project default");
    assert_eq!(row.no_response_points, -2);
    assert_eq!(row.completion_bonus_points, 7);
}

#[tokio::test]
async fn import_rolls_back_on_bad_ordinal() {
    let state = fresh_state().await;
    let admin = seed_admin(&state).await;

    let mut envelope = ExportEnvelope {
        schema_version: 1,
        project: ExportProject {
            name: "Bad".to_string(),
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
        },
        tasks: vec![ExportTask {
            ordinal: -1,
            title: "t".to_string(),
            content: "c".to_string(),
            test_template: sample_template(),
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
        }],
    };

    let err = import_project(
        AdminUser { id: admin.id },
        axum::extract::State(state.clone()),
        Json(envelope.clone()),
    )
    .await
    .expect_err("should reject");
    match err {
        ExportImportError::BadRequest(_) => {}
        other => panic!("expected BadRequest, got {other:?}"),
    }

    let count = projects::Entity::find()
        .filter(projects::Column::OwnerUserIdFk.eq(admin.id))
        .count(&state.db)
        .await
        .unwrap();
    assert_eq!(count, 0, "no project should have been inserted");

    envelope.tasks[0].ordinal = 0;
    let resp = import_project(
        AdminUser { id: admin.id },
        axum::extract::State(state.clone()),
        Json(envelope),
    )
    .await
    .expect("import ok");
    assert_eq!(resp.status(), StatusCode::CREATED);
}

#[tokio::test]
async fn import_rejects_unsupported_schema_version() {
    let state = fresh_state().await;
    let admin = seed_admin(&state).await;

    let envelope = ExportEnvelope {
        schema_version: 2,
        project: ExportProject {
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
        },
        tasks: vec![],
    };

    let err = import_project(
        AdminUser { id: admin.id },
        axum::extract::State(state.clone()),
        Json(envelope),
    )
    .await
    .expect_err("should reject");
    match err {
        ExportImportError::UnsupportedSchema => {}
        other => panic!("expected UnsupportedSchema, got {other:?}"),
    }
    let resp = err.into_response();
    assert_eq!(resp.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn import_rejects_duplicate_ordinal() {
    let state = fresh_state().await;
    let admin = seed_admin(&state).await;
    let tpl = sample_template();

    let envelope = ExportEnvelope {
        schema_version: 1,
        project: empty_project(),
        tasks: vec![task_with(0, tpl.clone()), task_with(0, tpl)],
    };

    let err = import_project(
        AdminUser { id: admin.id },
        axum::extract::State(state.clone()),
        Json(envelope),
    )
    .await
    .expect_err("should reject duplicate ordinal");
    match err {
        ExportImportError::BadRequest(ref msg) => {
            assert!(msg.contains("duplicate ordinal"), "msg was: {msg}");
        }
        other => panic!("expected BadRequest, got {other:?}"),
    }
    let resp = err.into_response();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

    let count = projects::Entity::find()
        .filter(projects::Column::OwnerUserIdFk.eq(admin.id))
        .count(&state.db)
        .await
        .unwrap();
    assert_eq!(count, 0, "no project should have been inserted");
}

#[tokio::test]
async fn export_import_round_trips_session_duration() {
    let state = fresh_state().await;
    let admin = seed_admin(&state).await;
    let original_id = seed_project(&state, admin.id).await;

    // Give the source project a non-default duration.
    let row = projects::Entity::find_by_id(original_id)
        .one(&state.db)
        .await
        .unwrap()
        .expect("seeded project");
    let mut am: projects::ActiveModel = row.into();
    am.default_session_duration_secs = Set(7200);
    am.update(&state.db).await.expect("update duration");

    let resp = export_project(
        AdminUser { id: admin.id },
        axum::extract::State(state.clone()),
        Path(original_id),
    )
    .await
    .expect("export ok");
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let envelope: ExportEnvelope = serde_json::from_slice(&body).unwrap();
    assert_eq!(envelope.project.session_duration_secs, 7200);

    let resp = import_project(
        AdminUser { id: admin.id },
        axum::extract::State(state.clone()),
        Json(envelope),
    )
    .await
    .expect("import ok");
    assert_eq!(resp.status(), StatusCode::CREATED);
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let import_resp: ImportResponse = serde_json::from_slice(&body).unwrap();

    let new_proj = projects::Entity::find_by_id(import_resp.project_id)
        .one(&state.db)
        .await
        .unwrap()
        .expect("new project exists");
    assert_eq!(new_proj.default_session_duration_secs, 7200);
}

#[test]
fn export_project_session_duration_defaults_when_absent() {
    // Export files written before the field existed must import as 3600.
    let mut v = serde_json::to_value(empty_project()).unwrap();
    v.as_object_mut()
        .unwrap()
        .remove("session_duration_secs")
        .expect("field present in current serialization");
    let p: ExportProject = serde_json::from_value(v).unwrap();
    assert_eq!(p.session_duration_secs, 3600);
}

#[tokio::test]
async fn import_rejects_malformed_test_template() {
    let state = fresh_state().await;
    let admin = seed_admin(&state).await;

    let mut bad_tpl = sample_template();
    bad_tpl.command_template = "".to_string();

    let envelope = ExportEnvelope {
        schema_version: 1,
        project: empty_project(),
        tasks: vec![task_with(0, bad_tpl)],
    };

    let err = import_project(
        AdminUser { id: admin.id },
        axum::extract::State(state.clone()),
        Json(envelope),
    )
    .await
    .expect_err("should reject malformed template");
    match err {
        ExportImportError::BadRequest(_) => {}
        other => panic!("expected BadRequest, got {other:?}"),
    }
    let resp = err.into_response();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

    let count = projects::Entity::find()
        .filter(projects::Column::OwnerUserIdFk.eq(admin.id))
        .count(&state.db)
        .await
        .unwrap();
    assert_eq!(count, 0, "no project should have been inserted");
}
