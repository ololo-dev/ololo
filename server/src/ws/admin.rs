//! Admin WebSocket handler.
//!
//! Mounted at `GET /ws/admin`. Accepts `?session_id=<id>&token=<jwt>`.
//!
//! Auth flow (all checks performed pre-upgrade):
//!   1. `session_id` missing or empty → 400.
//!   2. Session not found in DB → 404.
//!   3. `token` missing or JWT verification fails → 403.
//!   4. `users.is_admin` is false for the token subject → 403.
//!   5. WS upgrade proceeds.
//!
//! On connect: immediately pushes `AdminAdaptedTasksSnapshot` for the session.
//! Thereafter: forwards frames from the `AdminRegistry` broadcast channel.
//! On disconnect: drops receiver; evicts the registry entry if no subscribers remain.

use crate::auth::jwt::verify_access_token;
use crate::state::AppState;
use arena_core::entities::{players, probes, sessions, tasks, tests, users};
use arena_core::protocol::{AdaptedTaskStatus, AdminAdaptedTaskView, AdminTaskEntry, ArenaFrame};
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder};
use serde::Deserialize;
use std::collections::HashMap;
use tokio::sync::broadcast;
use uuid::Uuid;

/// Query parameters for the admin WS handshake.
#[derive(Deserialize)]
pub struct AdminWsParams {
    pub session_id: Option<Uuid>,
    pub token: Option<String>,
}

pub async fn ws_admin_handler(
    State(state): State<AppState>,
    Query(params): Query<AdminWsParams>,
    ws: Result<WebSocketUpgrade, axum::extract::ws::rejection::WebSocketUpgradeRejection>,
) -> Response {
    // --- pre-upgrade: session_id required ---
    let session_id = match params.session_id {
        Some(id) => id,
        _ => return StatusCode::BAD_REQUEST.into_response(),
    };

    // --- pre-upgrade: session must exist ---
    match sessions::Entity::find_by_id(session_id)
        .one(&state.db)
        .await
    {
        Ok(Some(_)) => {}
        Ok(None) => return StatusCode::NOT_FOUND.into_response(),
        Err(e) => {
            tracing::error!(error = %e, "ws_admin_handler: DB error looking up session");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    }

    // --- pre-upgrade: JWT required and must belong to an admin ---
    let token = match params.token {
        Some(ref t) if !t.is_empty() => t.clone(),
        _ => return StatusCode::FORBIDDEN.into_response(),
    };

    let claims = match verify_access_token(&state.jwt_decoding_key, &token) {
        Ok(c) => c,
        Err(_) => return StatusCode::FORBIDDEN.into_response(),
    };

    let user_id = match claims.user_id() {
        Ok(id) => id,
        Err(_) => return StatusCode::FORBIDDEN.into_response(),
    };

    let user = match users::Entity::find_by_id(user_id).one(&state.db).await {
        Ok(Some(u)) => u,
        Ok(None) => return StatusCode::FORBIDDEN.into_response(),
        Err(e) => {
            tracing::error!(error = %e, "ws_admin_handler: DB error looking up user");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    if !user.is_admin {
        return StatusCode::FORBIDDEN.into_response();
    }

    // --- pre-upgrade checks passed; proceed with WS upgrade ---
    let ws_upgrade = match ws {
        Ok(u) => u,
        Err(_) => return StatusCode::BAD_REQUEST.into_response(),
    };

    let admin_registry = state.admin_registry.clone();

    // Create or reuse broadcast channel for this session.
    let rx: broadcast::Receiver<ArenaFrame> = {
        let entry = admin_registry
            .entry(session_id)
            .or_insert_with(|| broadcast::channel::<ArenaFrame>(64).0);
        entry.subscribe()
    };

    ws_upgrade.on_upgrade(move |socket| handle_admin_socket(socket, rx, session_id, state))
}

async fn handle_admin_socket(
    mut socket: WebSocket,
    mut rx: broadcast::Receiver<ArenaFrame>,
    session_id: Uuid,
    state: AppState,
) {
    // Send initial snapshot.
    match build_snapshot(&state, session_id).await {
        Ok(snapshot) => {
            if let Ok(json) = serde_json::to_string(&snapshot)
                && socket.send(Message::Text(json)).await.is_err()
            {
                return;
            }
        }
        Err(e) => {
            tracing::error!(error = %e, session_id = %session_id, "ws_admin_handler: failed to build snapshot");
            return;
        }
    }

    // Forward loop.
    loop {
        match rx.recv().await {
            Ok(frame) => {
                let Ok(json) = serde_json::to_string(&frame) else {
                    continue;
                };
                if socket.send(Message::Text(json)).await.is_err() {
                    break;
                }
            }
            Err(broadcast::error::RecvError::Lagged(_)) => {
                // Skip lagged messages and continue.
                continue;
            }
            Err(broadcast::error::RecvError::Closed) => {
                break;
            }
        }
    }

    // Cleanup: drop rx (already dropped when this scope exits); evict registry
    // entry if no other subscribers remain.
    drop(rx);
    if let Some(entry) = state.admin_registry.get(&session_id)
        && entry.receiver_count() == 0
    {
        drop(entry);
        state.admin_registry.remove(&session_id);
    }
}

async fn build_snapshot(state: &AppState, session_id: Uuid) -> Result<ArenaFrame, sea_orm::DbErr> {
    let session = sessions::Entity::find_by_id(session_id)
        .one(&state.db)
        .await?
        .ok_or_else(|| sea_orm::DbErr::RecordNotFound("session missing".to_string()))?;

    let session_players = players::Entity::find()
        .filter(players::Column::SessionIdFk.eq(session_id))
        .all(&state.db)
        .await?;

    let project_tasks = tasks::Entity::find()
        .filter(tasks::Column::ProjectIdFk.eq(session.project_id_fk))
        .order_by_asc(tasks::Column::Ordinal)
        .all(&state.db)
        .await?;

    let adapted_test_rows = tests::Entity::find()
        .filter(tests::Column::SessionId.eq(session_id))
        .all(&state.db)
        .await?;

    let probe_rows = probes::Entity::find()
        .filter(probes::Column::SessionId.eq(session_id))
        .all(&state.db)
        .await?;

    let mut test_ids_by_task: HashMap<Uuid, Vec<Uuid>> = HashMap::new();
    let mut snippet_by_task: HashMap<Uuid, String> = HashMap::new();
    for row in adapted_test_rows {
        test_ids_by_task
            .entry(row.task_id)
            .or_default()
            .push(row.id);
        snippet_by_task
            .entry(row.task_id)
            .or_insert_with(|| row.command_template.clone());
    }

    let mut task_id_by_test_id: HashMap<Uuid, Uuid> = HashMap::new();
    for (task_id, ids) in &test_ids_by_task {
        for id in ids {
            task_id_by_test_id.insert(*id, *task_id);
        }
    }

    let player_views: Vec<AdminAdaptedTaskView> = session_players
        .into_iter()
        .map(|player| {
            let player_probe_rows: Vec<_> = probe_rows
                .iter()
                .filter(|probe| probe.player_id == player.id)
                .cloned()
                .collect();

            let mut attempts_by_task: HashMap<Uuid, u32> = HashMap::new();
            let mut latest_outcome_by_task: HashMap<Uuid, Option<String>> = HashMap::new();
            let mut latest_dispatched_by_task: HashMap<Uuid, chrono::DateTime<chrono::Utc>> =
                HashMap::new();
            for probe in player_probe_rows {
                let Some(task_id) = task_id_by_test_id.get(&probe.test_id).copied() else {
                    continue;
                };
                *attempts_by_task.entry(task_id).or_insert(0) += 1;
                let should_replace = latest_dispatched_by_task
                    .get(&task_id)
                    .map(|ts| probe.dispatched_at > *ts)
                    .unwrap_or(true);
                if should_replace {
                    latest_dispatched_by_task.insert(task_id, probe.dispatched_at);
                    latest_outcome_by_task.insert(task_id, probe.outcome.clone());
                }
            }

            let tasks: Vec<AdminTaskEntry> = project_tasks
                .iter()
                .map(|task| {
                    let status = match latest_outcome_by_task
                        .get(&task.id)
                        .and_then(|v| v.as_ref())
                    {
                        Some(outcome) if outcome == "error" || outcome == "no_response" => {
                            AdaptedTaskStatus::Failed
                        }
                        Some(_) => AdaptedTaskStatus::Ready,
                        None if test_ids_by_task.contains_key(&task.id) => AdaptedTaskStatus::Ready,
                        None => AdaptedTaskStatus::Pending,
                    };
                    AdminTaskEntry {
                        task_id: task.id,
                        task_order: task.ordinal,
                        title: task.title.clone(),
                        adapted_content: snippet_by_task.get(&task.id).cloned().unwrap_or_default(),
                        status,
                        adaptation_attempts: attempts_by_task.get(&task.id).copied().unwrap_or(0),
                    }
                })
                .collect();

            AdminAdaptedTaskView {
                player_id: player.id,
                player_display_name: player.display_name.clone(),
                tasks,
            }
        })
        .collect();

    Ok(ArenaFrame::AdminAdaptedTasksSnapshot {
        session_id,
        players: player_views,
    })
}
