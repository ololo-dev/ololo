//! A session's code-health history: every participant's checkpoints and
//! the task ranges of their git history, for the chart. One loader serves
//! the WebSocket snapshot (both dashboard handlers), the player snapshot
//! and `GET /api/sessions/:id/health`, so they can never disagree.

use std::collections::{BTreeMap, HashMap};

use arena_core::entities::{health_checkpoints, sessions, tasks};
use arena_core::health::{checkpoint_view, elapsed_secs, first_parent_log};
use arena_core::health_settings::HealthSettings;
use arena_core::protocol::{PlayerHealthPayload, PlayerId, SessionHealthPayload, TaskRangeView};
use arena_core::snapshot_message::task_ranges;
use axum::Json;
use axum::extract::{Path, State};
use axum::response::{IntoResponse, Response};
use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, QueryOrder};
use uuid::Uuid;

use crate::AppState;
use crate::api::sessions::common::{SessionError, authorize_session_view};
use crate::auth::jwt::AccessClaims;

/// Commits of a player's first-parent log read for the task ranges.
const LOG_LIMIT: usize = 4_000;

/// The session's health history, or `None` when the session has no
/// checkpoints and health tracking is off — the payload is then omitted
/// from snapshots so pages predating the feature look exactly as before.
/// `only_player` narrows the payload to one participant (the player page).
pub async fn load_session_health(
    db: &DatabaseConnection,
    session_id: Uuid,
    only_player: Option<Uuid>,
) -> Option<SessionHealthPayload> {
    let settings = HealthSettings::load(db).await.unwrap_or_default();
    let mut query = health_checkpoints::Entity::find()
        .filter(health_checkpoints::Column::SessionIdFk.eq(session_id));
    if let Some(player) = only_player {
        query = query.filter(health_checkpoints::Column::PlayerIdFk.eq(player));
    }
    let rows = query
        .order_by_asc(health_checkpoints::Column::CreatedAt)
        .all(db)
        .await
        .unwrap_or_default();
    if rows.is_empty() && !settings.enabled {
        return None;
    }
    let session = sessions::Entity::find_by_id(session_id)
        .one(db)
        .await
        .ok()
        .flatten()?;
    let titles: HashMap<Uuid, (String, i32)> = tasks::Entity::find()
        .filter(tasks::Column::ProjectIdFk.eq(session.project_id_fk))
        .all(db)
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|t| (t.id, (t.title, t.ordinal)))
        .collect();

    let mut players: BTreeMap<PlayerId, PlayerHealthPayload> = BTreeMap::new();
    for row in &rows {
        let title = row
            .task_id_fk
            .and_then(|t| titles.get(&t))
            .map(|(title, _)| title.clone());
        players
            .entry(PlayerId(row.player_id_fk))
            .or_default()
            .checkpoints
            .push(checkpoint_view(
                row,
                title,
                session.started_at,
                &settings.thresholds,
            ));
    }
    // Task ranges come from each player's history; a player with no
    // checkpoints yet still gets ranges once they have pushed.
    let player_ids: Vec<Uuid> = match only_player {
        Some(p) => vec![p],
        None => {
            let mut ids: Vec<Uuid> = players.keys().map(|p| p.0).collect();
            ids.sort();
            ids
        }
    };
    if let Some(base) = arena_core::git_store::repos_base_dir() {
        for player_id in player_ids {
            let repo_dir = arena_core::git_store::player_repo_path(&base, session_id, player_id);
            if !repo_dir.join("HEAD").exists() {
                continue;
            }
            let Ok(log) = first_parent_log(&repo_dir, LOG_LIMIT).await else {
                continue;
            };
            let ranges: Vec<TaskRangeView> = task_ranges(&log)
                .ranges
                .into_iter()
                .map(|r| {
                    let (title, ordinal) = titles
                        .get(&r.task_id)
                        .map(|(t, o)| (Some(t.clone()), Some(*o)))
                        .unwrap_or((r.title.clone(), None));
                    TaskRangeView {
                        task_id: r.task_id,
                        title,
                        ordinal,
                        start_commit: r.start_sha,
                        end_commit: r.end_sha,
                        start_at: r.start_at,
                        end_at: r.end_at,
                        start_t: r
                            .start_at
                            .and_then(|at| elapsed_secs(session.started_at, at)),
                        end_t: r.end_at.and_then(|at| elapsed_secs(session.started_at, at)),
                    }
                })
                .collect();
            if ranges.is_empty() && !players.contains_key(&PlayerId(player_id)) {
                continue;
            }
            players.entry(PlayerId(player_id)).or_default().task_ranges = ranges;
        }
    }
    Some(SessionHealthPayload {
        thresholds: settings.thresholds,
        players,
    })
}

/// `GET /api/sessions/:id/health` — the same payload the snapshot carries,
/// under the same visibility rule as the rest of the session.
pub async fn get_health(
    State(state): State<AppState>,
    claims: Option<AccessClaims>,
    Path(id): Path<Uuid>,
) -> Result<Response, SessionError> {
    let session = sessions::Entity::find_by_id(id)
        .one(&state.db)
        .await?
        .ok_or(SessionError::NotFound)?;
    authorize_session_view(&state.db, &session, claims.as_ref()).await?;
    let payload = load_session_health(&state.db, id, None)
        .await
        .unwrap_or_else(|| SessionHealthPayload {
            thresholds: HealthSettings::default().thresholds,
            players: BTreeMap::new(),
        });
    Ok(Json(payload).into_response())
}
