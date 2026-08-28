//! apply-seed linking of campaign parents to their parts.
//!
//! The reconcile is unlink-all-then-link, which is what makes a reordered or
//! shortened `parts:` list safe: writing new ordinals over live rows trips the
//! `(parent, ordinal)` unique index halfway through a swap.

use super::*;
use arena_core::entities::projects;
use sea_orm::{ColumnTrait, QueryFilter, QueryOrder};

fn envelope(slug: &str, tasks: Vec<ExportTask>, parts: Vec<&str>) -> ExportEnvelope {
    let mut project = empty_project();
    project.slug = Some(slug.to_string());
    project.name = slug.to_string();
    project.public = true;
    project.parts = parts.into_iter().map(str::to_string).collect();
    ExportEnvelope {
        schema_version: 1,
        project,
        tasks,
    }
}

async fn push(state: &AppState, admin_id: Uuid, env: ExportEnvelope) -> ApplySeedResponse {
    let resp = apply_seed(
        AdminUser { id: admin_id },
        axum::extract::State(state.clone()),
        Json(env),
    )
    .await
    .expect("apply_seed ok");
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    serde_json::from_slice(&body).unwrap()
}

async fn push_err(state: &AppState, admin_id: Uuid, env: ExportEnvelope) -> String {
    let err = apply_seed(
        AdminUser { id: admin_id },
        axum::extract::State(state.clone()),
        Json(env),
    )
    .await
    .expect_err("apply_seed must reject");
    err.to_string()
}

/// Slugs of `parent`'s parts, in play order.
async fn parts_of(state: &AppState, parent: Uuid) -> Vec<String> {
    projects::Entity::find()
        .filter(projects::Column::ParentProjectIdFk.eq(parent))
        .order_by_asc(projects::Column::PartOrdinal)
        .all(&state.db)
        .await
        .expect("query")
        .into_iter()
        .filter_map(|p| p.slug)
        .collect()
}

async fn seed_two_parts(state: &AppState, admin: Uuid) {
    push(
        state,
        admin,
        envelope("part-a", vec![task_with(0, sample_template())], vec![]),
    )
    .await;
    push(
        state,
        admin,
        envelope("part-b", vec![task_with(0, sample_template())], vec![]),
    )
    .await;
}

#[tokio::test]
async fn a_parent_envelope_links_its_parts_in_order() {
    let state = fresh_state().await;
    let admin = seed_admin(&state).await;
    seed_two_parts(&state, admin.id).await;

    let parent = push(
        &state,
        admin.id,
        envelope("campaign", vec![], vec!["part-b", "part-a"]),
    )
    .await;

    assert_eq!(
        parts_of(&state, parent.project_id).await,
        vec!["part-b".to_string(), "part-a".to_string()],
        "play order is the order the parent declares, not alphabetical"
    );
}

#[tokio::test]
async fn re_pushing_reorders_and_drops_parts() {
    let state = fresh_state().await;
    let admin = seed_admin(&state).await;
    seed_two_parts(&state, admin.id).await;
    push(
        &state,
        admin.id,
        envelope("part-c", vec![task_with(0, sample_template())], vec![]),
    )
    .await;

    let parent = push(
        &state,
        admin.id,
        envelope("campaign", vec![], vec!["part-a", "part-b", "part-c"]),
    )
    .await;
    // Swap the first two and drop the third — the shape that used to die on
    // the unique index if ordinals were written over live rows.
    push(
        &state,
        admin.id,
        envelope("campaign", vec![], vec!["part-b", "part-a"]),
    )
    .await;

    assert_eq!(
        parts_of(&state, parent.project_id).await,
        vec!["part-b".to_string(), "part-a".to_string()]
    );
    let dropped = projects::Entity::find()
        .filter(projects::Column::Slug.eq("part-c"))
        .one(&state.db)
        .await
        .expect("query")
        .expect("part-c still exists");
    assert!(
        dropped.parent_project_id_fk.is_none() && dropped.part_ordinal.is_none(),
        "a part removed from the list becomes a standalone project again"
    );
}

#[tokio::test]
async fn an_unknown_part_slug_is_rejected() {
    let state = fresh_state().await;
    let admin = seed_admin(&state).await;

    let msg = push_err(&state, admin.id, envelope("campaign", vec![], vec!["nope"])).await;
    assert!(
        msg.contains("nope"),
        "the admin pushing the seed must hear which slug is wrong: {msg}"
    );
}

#[tokio::test]
async fn a_campaign_parent_may_not_carry_tasks() {
    let state = fresh_state().await;
    let admin = seed_admin(&state).await;
    seed_two_parts(&state, admin.id).await;

    let msg = push_err(
        &state,
        admin.id,
        envelope(
            "campaign",
            vec![task_with(0, sample_template())],
            vec!["part-a"],
        ),
    )
    .await;
    assert!(
        msg.contains("no tasks"),
        "a parent hosts no sessions, so its tasks would never run: {msg}"
    );
}

#[tokio::test]
async fn campaigns_do_not_nest_and_a_part_belongs_to_one_campaign() {
    let state = fresh_state().await;
    let admin = seed_admin(&state).await;
    seed_two_parts(&state, admin.id).await;
    push(
        &state,
        admin.id,
        envelope("campaign", vec![], vec!["part-a"]),
    )
    .await;

    // A second campaign cannot claim part-a...
    let msg = push_err(
        &state,
        admin.id,
        envelope("other-campaign", vec![], vec!["part-a"]),
    )
    .await;
    assert!(msg.contains("part-a"), "{msg}");

    // ...and a campaign cannot become someone else's part.
    let msg = push_err(
        &state,
        admin.id,
        envelope("outer", vec![], vec!["campaign"]),
    )
    .await;
    assert!(msg.contains("nest"), "{msg}");
}

#[tokio::test]
async fn exporting_a_parent_round_trips_its_part_list() {
    let state = fresh_state().await;
    let admin = seed_admin(&state).await;
    seed_two_parts(&state, admin.id).await;
    let parent = push(
        &state,
        admin.id,
        envelope("campaign", vec![], vec!["part-a", "part-b"]),
    )
    .await;

    let resp = export_project(
        AdminUser { id: admin.id },
        axum::extract::State(state.clone()),
        axum::extract::Path(parent.project_id),
    )
    .await
    .expect("export ok");
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let exported: ExportEnvelope = serde_json::from_slice(&body).expect("parse export");

    assert_eq!(exported.project.parts, vec!["part-a", "part-b"]);
    assert!(
        exported.tasks.is_empty(),
        "a campaign parent exports as a table of contents"
    );
}
