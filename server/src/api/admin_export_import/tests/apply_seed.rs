use super::*;

fn envelope_with(slug: &str, name: &str, tasks: Vec<ExportTask>) -> ExportEnvelope {
    let mut project = empty_project();
    project.slug = Some(slug.to_string());
    project.name = name.to_string();
    project.public = true;
    ExportEnvelope {
        schema_version: 1,
        project,
        tasks,
    }
}

async fn call_apply_seed(
    state: &AppState,
    admin_id: Uuid,
    envelope: ExportEnvelope,
) -> ApplySeedResponse {
    let resp = apply_seed(
        AdminUser { id: admin_id },
        axum::extract::State(state.clone()),
        Json(envelope),
    )
    .await
    .expect("apply_seed ok");
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    serde_json::from_slice(&body).unwrap()
}

#[tokio::test]
async fn apply_seed_creates_then_updates_by_slug() {
    let state = fresh_state().await;
    let admin = seed_admin(&state).await;

    // First push: no project with this slug → created, slug/public preserved.
    let mut t0 = task_with(0, sample_template());
    t0.title = "First".to_string();
    let t1 = task_with(1, sample_template());
    let r1 = call_apply_seed(
        &state,
        admin.id,
        envelope_with("pushed-proj", "Pushed", vec![t0, t1]),
    )
    .await;
    assert!(r1.created);
    assert_eq!(r1.tasks_inserted, 2);

    let proj = projects::Entity::find_by_id(r1.project_id)
        .one(&state.db)
        .await
        .unwrap()
        .expect("project exists");
    assert_eq!(proj.slug.as_deref(), Some("pushed-proj"));
    assert_eq!(proj.owner_user_id_fk, admin.id);
    assert!(
        proj.public,
        "apply-seed preserves the envelope's visibility"
    );

    // Second push, same slug: task 0 retitled, task 1 dropped, task 2 added →
    // update in place on the same project id.
    let mut t0b = task_with(0, sample_template());
    t0b.title = "First Renamed".to_string();
    let t2 = task_with(2, sample_template());
    let r2 = call_apply_seed(
        &state,
        admin.id,
        envelope_with("pushed-proj", "Pushed v2", vec![t0b, t2]),
    )
    .await;
    assert!(!r2.created);
    assert_eq!(r2.project_id, r1.project_id, "upsert keyed by slug");
    assert_eq!(r2.tasks_updated, 1);
    assert_eq!(r2.tasks_inserted, 1);
    assert_eq!(r2.tasks_deleted, 1);

    let proj = projects::Entity::find_by_id(r1.project_id)
        .one(&state.db)
        .await
        .unwrap()
        .expect("project exists");
    assert_eq!(proj.name, "Pushed v2");

    let rows = tasks::Entity::find()
        .filter(tasks::Column::ProjectIdFk.eq(r1.project_id))
        .order_by_asc(tasks::Column::Ordinal)
        .all(&state.db)
        .await
        .unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].title, "First Renamed");
    assert_eq!(rows[1].ordinal, 2);
}

#[tokio::test]
async fn apply_seed_rejects_missing_slug() {
    let state = fresh_state().await;
    let admin = seed_admin(&state).await;

    let envelope = ExportEnvelope {
        schema_version: 1,
        project: empty_project(), // slug: None
        tasks: vec![],
    };
    let err = apply_seed(
        AdminUser { id: admin.id },
        axum::extract::State(state.clone()),
        Json(envelope),
    )
    .await
    .expect_err("must reject slugless envelope");
    assert!(matches!(err, ExportImportError::BadRequest(_)));
}

#[tokio::test]
async fn apply_seed_rejects_unknown_judge_slug_on_create() {
    let state = fresh_state().await;
    let admin = seed_admin(&state).await;

    let mut task = task_with(0, sample_template());
    task.judges = vec![crate::api::admin_export_import::JudgeRef::Slug(
        "no-such-judge".to_string(),
    )];
    let err = apply_seed(
        AdminUser { id: admin.id },
        axum::extract::State(state.clone()),
        Json(envelope_with("judged-proj", "Judged", vec![task])),
    )
    .await
    .expect_err("unknown judge slug must fail");
    assert!(matches!(err, ExportImportError::BadRequest(_)));
}

#[tokio::test]
async fn reseed_preserves_task_judge_attachment_ids() {
    // judge_results reference task_judge_id: if a reseed recreates the
    // attachment rows, every verdict a task ever received is orphaned —
    // statuses regress to pending and the recovery sweep re-runs (and
    // re-pays for) the whole panel. Session 4UTUVF hit exactly that.
    let state = fresh_state().await;
    let admin = seed_admin(&state).await;

    let judge_id = Uuid::new_v4();
    arena_core::entities::judges::ActiveModel {
        id: sea_orm::Set(judge_id),
        slug: sea_orm::Set("panel-judge".to_string()),
        name: sea_orm::Set("Panel Judge".to_string()),
        description: sea_orm::Set(String::new()),
        prompt: sea_orm::Set("Evaluate.".to_string()),
        rating_scale: sea_orm::Set(serde_json::json!({"min": 0.0, "max": 10.0, "step": 0.5})),
        kind: sea_orm::Set("llm".to_string()),
        scope: sea_orm::Set("task".to_string()),
        evidence_mode: sea_orm::Set("tools".to_string()),
        evidence_needs: sea_orm::Set(None),
        llm_provider_id_fk: sea_orm::Set(None),
        llm_model: sea_orm::Set(None),
        llm_pool_id_fk: sea_orm::Set(None),
        llm_source_order: sea_orm::Set(
            arena_core::llm::resolve::SOURCE_ORDER_POOL_FIRST.to_string(),
        ),
        criteria: sea_orm::Set(None),
        probes_config: sea_orm::Set(None),
        max_interactive: sea_orm::Set(None),
        avatar_url: sea_orm::Set(None),
        created_at: sea_orm::Set(chrono::Utc::now()),
        updated_at: sea_orm::Set(chrono::Utc::now()),
        ignore_paths: Set(None),
    }
    .insert(&state.db)
    .await
    .expect("judge");

    let mut task = task_with(0, sample_template());
    task.judges = vec![crate::api::admin_export_import::JudgeRef::Slug(
        "panel-judge".to_string(),
    )];
    let r1 = call_apply_seed(
        &state,
        admin.id,
        envelope_with("stable-attach", "V1", vec![task.clone()]),
    )
    .await;
    assert!(r1.created);

    let tj_before = arena_core::entities::task_judges::Entity::find()
        .filter(arena_core::entities::task_judges::Column::JudgeId.eq(judge_id))
        .one(&state.db)
        .await
        .unwrap()
        .expect("attachment exists");

    // Second push of the same definition: the attachment row must survive
    // with the SAME id, not be deleted and recreated.
    let r2 = call_apply_seed(
        &state,
        admin.id,
        envelope_with("stable-attach", "V2", vec![task]),
    )
    .await;
    assert!(!r2.created);

    let tjs_after = arena_core::entities::task_judges::Entity::find()
        .filter(arena_core::entities::task_judges::Column::JudgeId.eq(judge_id))
        .all(&state.db)
        .await
        .unwrap();
    assert_eq!(tjs_after.len(), 1, "no duplicate attachment");
    assert_eq!(
        tjs_after[0].id, tj_before.id,
        "the attachment id must survive a reseed — verdicts reference it"
    );
}

async fn insert_judge(state: &AppState, slug: &str) -> Uuid {
    let judge_id = Uuid::new_v4();
    arena_core::entities::judges::ActiveModel {
        id: sea_orm::Set(judge_id),
        slug: sea_orm::Set(slug.to_string()),
        name: sea_orm::Set(slug.to_string()),
        description: sea_orm::Set(String::new()),
        prompt: sea_orm::Set("Evaluate.".to_string()),
        rating_scale: sea_orm::Set(serde_json::json!({"min": 0.0, "max": 10.0, "step": 0.5})),
        kind: sea_orm::Set("llm".to_string()),
        scope: sea_orm::Set("task".to_string()),
        evidence_mode: sea_orm::Set("tools".to_string()),
        evidence_needs: sea_orm::Set(None),
        llm_provider_id_fk: sea_orm::Set(None),
        llm_model: sea_orm::Set(None),
        llm_pool_id_fk: sea_orm::Set(None),
        llm_source_order: sea_orm::Set(
            arena_core::llm::resolve::SOURCE_ORDER_POOL_FIRST.to_string(),
        ),
        criteria: sea_orm::Set(None),
        probes_config: sea_orm::Set(None),
        max_interactive: sea_orm::Set(None),
        avatar_url: sea_orm::Set(None),
        created_at: sea_orm::Set(chrono::Utc::now()),
        updated_at: sea_orm::Set(chrono::Utc::now()),
        ignore_paths: Set(None),
    }
    .insert(&state.db)
    .await
    .expect("judge");
    judge_id
}

#[tokio::test]
async fn reseed_survives_swapping_the_judge_panel() {
    // (task_id, ordinal) is unique on task_judges. Replacing a panel puts a
    // NEW judge on an ordinal a surviving row still holds — the one-pass
    // reconcile inserted straight onto it and died on the unique index
    // (weather-widget's panel swap 500'd every apply-seed on Postgres).
    let state = fresh_state().await;
    let admin = seed_admin(&state).await;

    let judge_a = insert_judge(&state, "judge-a").await;
    insert_judge(&state, "judge-b").await;
    let judge_c = insert_judge(&state, "judge-c").await;

    let jref = |s: &str| crate::api::admin_export_import::JudgeRef::Slug(s.to_string());

    let mut task = task_with(0, sample_template());
    task.judges = vec![jref("judge-a"), jref("judge-b")];
    let r1 = call_apply_seed(
        &state,
        admin.id,
        envelope_with("panel-swap", "V1", vec![task]),
    )
    .await;
    assert!(r1.created);

    let a_before = arena_core::entities::task_judges::Entity::find()
        .filter(arena_core::entities::task_judges::Column::JudgeId.eq(judge_a))
        .one(&state.db)
        .await
        .unwrap()
        .expect("judge-a attached");

    // judge-c takes ordinal 0 (judge-a's old slot), judge-a moves to 1,
    // judge-b detaches.
    let mut task = task_with(0, sample_template());
    task.judges = vec![jref("judge-c"), jref("judge-a")];
    let r2 = call_apply_seed(
        &state,
        admin.id,
        envelope_with("panel-swap", "V2", vec![task]),
    )
    .await;
    assert!(!r2.created);

    let tjs = arena_core::entities::task_judges::Entity::find()
        .filter(arena_core::entities::task_judges::Column::TaskId.eq(a_before.task_id))
        .all(&state.db)
        .await
        .unwrap();
    assert_eq!(tjs.len(), 2, "judge-b detached, judge-c attached");
    let by_judge = |id: Uuid| tjs.iter().find(|r| r.judge_id == id).expect("attached");
    assert_eq!(by_judge(judge_c).ordinal, 0);
    assert_eq!(by_judge(judge_a).ordinal, 1);
    assert_eq!(
        by_judge(judge_a).id,
        a_before.id,
        "surviving attachment keeps its id — verdicts reference it"
    );
}

#[tokio::test]
async fn apply_seed_preserves_operator_set_cover_image() {
    let state = fresh_state().await;
    let admin = seed_admin(&state).await;

    // Create via apply-seed; seed envelopes never carry a cover image.
    let r1 = call_apply_seed(
        &state,
        admin.id,
        envelope_with("cover-proj", "Cover", vec![task_with(0, sample_template())]),
    )
    .await;
    let pid = r1.project_id;

    // The operator sets a cover image through the UI (simulated as a direct update).
    projects::ActiveModel {
        id: Set(pid),
        cover_image_url: Set(Some("https://img.example/cover.png".to_string())),
        ..Default::default()
    }
    .update(&state.db)
    .await
    .expect("set cover image");

    // Re-read the seed (still no cover) — the operator's image must survive.
    call_apply_seed(
        &state,
        admin.id,
        envelope_with(
            "cover-proj",
            "Cover v2",
            vec![task_with(0, sample_template())],
        ),
    )
    .await;
    let proj = projects::Entity::find_by_id(pid)
        .one(&state.db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        proj.cover_image_url.as_deref(),
        Some("https://img.example/cover.png"),
        "re-read must not clobber an operator-set cover image"
    );

    // A definition that DOES carry a cover image still overwrites it.
    let mut env = envelope_with(
        "cover-proj",
        "Cover v3",
        vec![task_with(0, sample_template())],
    );
    env.project.cover_image_url = Some("https://img.example/new.png".to_string());
    call_apply_seed(&state, admin.id, env).await;
    let proj = projects::Entity::find_by_id(pid)
        .one(&state.db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        proj.cover_image_url.as_deref(),
        Some("https://img.example/new.png"),
        "an explicit cover image in the definition overwrites"
    );
}
