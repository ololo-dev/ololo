//! Session dashboard WebSocket handler.
//!
//! Mounted at `GET /ws/s/:join_code`. Authenticates via two distinct paths
//! that also determine the client kind and which frames are forwarded:
//!
//!   * `X-API-Key: <pat>` header — CLI clients only. PAT lookup.
//!     Receives: session-lifecycle frames + member events (no leaderboard churn).
//!
//!   * `?token=<jwt>` query parameter — browser clients only. JWT verification.
//!     Browsers cannot set custom headers on a WS upgrade, so the query param
//!     is the standard workaround. PAT tokens via query param are rejected.
//!     Receives: all dashboard frames.
//!
//! Auth flow (all checks pre-upgrade):
//!   1. `X-API-Key` header present → PAT lookup → ClientKind::Cli. Failure → 401.
//!   2. `?token=<jwt>` present → JWT verify → ClientKind::Browser. Failure → 401.
//!      PAT prefix (`ololo_`) via query param is always rejected → 401.
//!   3. Neither present → 401.
//!   4. `join_code` present in `state.session_registry`. Failure → 404.
//!   5. WS upgrade proceeds. Upsert `session_members` row; send `MemberList`
//!      snapshot; broadcast `MemberJoined` if this is a new participant.

use crate::api::sessions::load_session_activity;
use crate::auth::jwt::verify_access_token;
use crate::auth::pat;
use crate::state::{AppState, SessionCacheInner};
use arena_core::entities::players;
use arena_core::entities::users;
use arena_core::protocol::{ArenaFrame, MemberInfo, PlayerSummary, SessionSnapshotPayload};
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, Query, State};
use axum::http::HeaderMap;
use axum::response::{IntoResponse, Response};
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, Set};
use serde::Deserialize;
use std::sync::{Arc, RwLock};
use uuid::Uuid;

/// Query-string parameters accepted on the WS handshake.
#[derive(Deserialize)]
pub struct TokenQuery {
    pub token: Option<String>,
}

/// Which kind of client connected — determines which frames are forwarded.
#[derive(Debug, Clone, Copy)]
enum ClientKind {
    /// CLI tool (`ololo start`). Authenticated via `X-API-Key` header.
    Cli,
    /// Browser dashboard. Authenticated via `?token=<jwt>` query param.
    Browser,
}

fn should_register_as_participant(kind: ClientKind) -> bool {
    matches!(kind, ClientKind::Cli)
}

/// Returns `true` when `frame` should be forwarded to a client of `kind`.
///
/// Browser clients receive all dashboard-oriented frames.
/// CLI clients receive only session-lifecycle and member-event frames
/// (leaderboard updates and agent progress are browser-only).
fn should_forward(frame: &ArenaFrame, kind: ClientKind) -> bool {
    match kind {
        ClientKind::Browser => !matches!(
            frame,
            ArenaFrame::TestPush { .. } | ArenaFrame::TestResult { .. }
        ),
        ClientKind::Cli => matches!(
            frame,
            ArenaFrame::LobbyCountdown { .. }
                | ArenaFrame::SessionStarted { .. }
                | ArenaFrame::SessionComplete { .. }
                | ArenaFrame::MemberJoined { .. }
                | ArenaFrame::Heartbeat
        ),
    }
}

/// Reasons we reject a handshake before completing the WS upgrade.
#[derive(Debug)]
enum HandshakeReject {
    Unauthorized,
    NotFound,
    DatabaseError,
}

crate::api::error::impl_api_error!(HandshakeReject {
    Self::Unauthorized => (UNAUTHORIZED, "unauthorized"),
    Self::NotFound => (NOT_FOUND, "not_found"),
    Self::DatabaseError => (INTERNAL_SERVER_ERROR, "database_error"),
});

/// Extract the value of the `X-API-Key` header, if present.
fn extract_api_key(headers: &HeaderMap) -> Option<String> {
    headers
        .get("x-api-key")
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned)
}

/// Build the `SessionSnapshot` payload sent to a client on WS connect.
///
/// Shared by the authenticated dashboard handler and the public observer
/// handler. Reads the cache under short-lived locks only — the async DB
/// calls (score history, activity, completion statuses) never run while the
/// lock is held.
///
/// This is a single-session detail path, so the derived per-player
/// completion status is computed here (one batched call per snapshot) and
/// joined into `participants` by player id. Participants without a status
/// entry (revoked players, dashboard-only connections) keep `None` — the
/// field is skip-serialized, so such entries stay wire-identical. List
/// endpoints never compute the aggregate.
pub(crate) async fn build_session_snapshot(
    db: &sea_orm::DatabaseConnection,
    session_cache: &Arc<RwLock<SessionCacheInner>>,
) -> SessionSnapshotPayload {
    let (session_id, started_at) = {
        let cache = session_cache.read().unwrap_or_else(|e| e.into_inner());
        (cache.session_id, cache.started_at)
    };
    let score_history = arena_core::scoring::build_score_history(db, session_id, started_at)
        .await
        .ok()
        .flatten();
    let activity = load_session_activity(db, session_id).await;
    // Best-effort like score_history: a failure leaves every status None
    // rather than blocking the snapshot.
    let status_by_player: std::collections::HashMap<
        Uuid,
        arena_core::protocol::PlayerCompletionStatus,
    > = arena_core::session_completion::session_player_statuses(db, session_id)
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|s| (s.player_id, s.status))
        .collect();
    let cache = session_cache.read().unwrap_or_else(|e| e.into_inner());
    SessionSnapshotPayload {
        session_id: cache.session_id,
        phase: cache.phase,
        version: cache.version,
        participants: cache
            .participants
            .iter()
            .map(|m| MemberInfo {
                completion_status: m
                    .player_id
                    .and_then(|pid| status_by_player.get(&pid).copied()),
                ..m.clone()
            })
            .collect(),
        leaderboard: cache.leaderboard.clone(),
        started_at: cache.started_at,
        timeline: None,
        activity: Some(activity),
        score_history,
    }
}

pub async fn ws_session_handler(
    State(state): State<AppState>,
    Path(join_code): Path<String>,
    Query(query): Query<TokenQuery>,
    headers: HeaderMap,
    ws: Result<WebSocketUpgrade, axum::extract::ws::rejection::WebSocketUpgradeRejection>,
) -> Response {
    // (1) Resolve AccessClaims and ClientKind.
    let (claims, client_kind) = if let Some(api_key) = extract_api_key(&headers) {
        // X-API-Key header → CLI client. PAT lookup only.
        match pat::lookup_pat(&state.db, &api_key).await {
            Ok(c) => (c, ClientKind::Cli),
            Err(_) => return HandshakeReject::Unauthorized.into_response(),
        }
    } else if let Some(token) = query.token {
        // ?token= query param → browser client. JWT only; PAT rejected.
        if token.starts_with("ololo_") {
            return HandshakeReject::Unauthorized.into_response();
        }
        match verify_access_token(&state.jwt_decoding_key, &token) {
            Ok(c) => (c, ClientKind::Browser),
            Err(_) => return HandshakeReject::Unauthorized.into_response(),
        }
    } else {
        return HandshakeReject::Unauthorized.into_response();
    };

    // (2) Resolve user_id from claims.
    let user_id: Uuid = match claims.user_id() {
        Ok(u) => u,
        Err(_) => return HandshakeReject::Unauthorized.into_response(),
    };

    // (3) Look up join_code in session_registry — clone tx and cache.
    let join_code_upper = join_code.to_uppercase();
    let (broadcast_tx, broadcast_rx, session_cache) =
        match state.session_registry.get(&join_code_upper) {
            Some(entry) => (
                entry.tx.clone(),
                entry.tx.subscribe(),
                Arc::clone(&entry.cache),
            ),
            None => return HandshakeReject::NotFound.into_response(),
        };

    // (4) Resolve session_id from DB for the join_code.
    use arena_core::entities::sessions;
    let session_row = match sessions::Entity::find()
        .filter(sessions::Column::JoinCode.eq(join_code_upper.clone()))
        .one(&state.db)
        .await
    {
        Ok(Some(r)) => r,
        Ok(None) => return HandshakeReject::NotFound.into_response(),
        Err(_) => return HandshakeReject::DatabaseError.into_response(),
    };
    let session_id = session_row.id;

    // Auth passed. Now require a real WS upgrade.
    let ws = match ws {
        Ok(ws) => ws,
        Err(rej) => return rej.into_response(),
    };

    let db = state.db.clone();
    ws.on_upgrade(move |socket| {
        handle_session_socket(
            socket,
            session_id,
            user_id,
            client_kind,
            broadcast_tx,
            broadcast_rx,
            session_cache,
            db,
        )
    })
}

#[allow(clippy::too_many_arguments)]
async fn handle_session_socket(
    socket: WebSocket,
    session_id: Uuid,
    user_id: Uuid,
    client_kind: ClientKind,
    broadcast_tx: tokio::sync::broadcast::Sender<ArenaFrame>,
    mut broadcast_rx: tokio::sync::broadcast::Receiver<ArenaFrame>,
    session_cache: Arc<RwLock<SessionCacheInner>>,
    db: sea_orm::DatabaseConnection,
) {
    // Fetch the connecting user's display name and username for participant notifications.
    let user_row = users::Entity::find_by_id(user_id)
        .one(&db)
        .await
        .ok()
        .flatten();
    let display_name = user_row
        .as_ref()
        .map(|u| u.display_name.clone())
        .unwrap_or_else(|| "Unknown".to_string());
    let avatar_url = user_row.as_ref().and_then(|u| u.avatar_url.clone());
    let username = user_row.and_then(|u| u.username.clone());

    let user_id_str = user_id.to_string();
    let joined_at = chrono::Utc::now();

    if should_register_as_participant(client_kind) {
        let existing = players::Entity::find()
            .filter(players::Column::SessionIdFk.eq(session_id))
            .filter(players::Column::UserIdFk.eq(user_id))
            .filter(players::Column::RevokedAt.is_null())
            .one(&db)
            .await;
        match existing {
            Ok(None) => {
                let _ = players::ActiveModel {
                    id: Set(Uuid::new_v4()),
                    session_id_fk: Set(session_id),
                    user_id_fk: Set(Some(user_id)),
                    display_name: Set(display_name.clone()),
                    fingerprint: Set(None),
                    metadata_json: Set(None),
                    joined_at: Set(joined_at),
                    reconnected_at: Set(None),
                    revoked_at: Set(None),
                    agent_connected: Set(false),
                    agent_last_seen_at: Set(None),
                }
                .insert(&db)
                .await;
            }
            Ok(Some(_)) => {}
            Err(e) => {
                tracing::warn!(error = %e, session_id = %session_id, user_id = %user_id, "ws_session: participant upsert lookup failed");
            }
        }
    }

    let is_new_member = if should_register_as_participant(client_kind) {
        let cache = session_cache.read().unwrap_or_else(|e| e.into_inner());
        !cache.participants.iter().any(|p| p.user_id == user_id_str)
    } else {
        false
    };

    let (mut sink, mut stream) = futures::StreamExt::split(socket);

    // Send SessionSnapshot to this client on connect.
    let snapshot = build_session_snapshot(&db, &session_cache).await;
    if let Ok(json) = serde_json::to_string(&ArenaFrame::SessionSnapshot(snapshot))
        && futures::SinkExt::send(&mut sink, Message::Text(json))
            .await
            .is_err()
    {
        return;
    }

    // For browser clients, send the user's own player summaries for this session.
    if matches!(client_kind, ClientKind::Browser) {
        let own_players = players::Entity::find()
            .filter(players::Column::SessionIdFk.eq(session_id))
            .filter(players::Column::UserIdFk.eq(user_id))
            .filter(players::Column::RevokedAt.is_null())
            .all(&db)
            .await
            .unwrap_or_default();
        let summaries: Vec<PlayerSummary> = own_players
            .into_iter()
            .map(|p| PlayerSummary {
                player_id: p.id,
                user_id: p.user_id_fk,
                display_name: p.display_name,
                fingerprint: p.fingerprint,
                joined_at: p.joined_at,
                reconnected_at: p.reconnected_at,
                revoked_at: p.revoked_at,
            })
            .collect();
        if let Ok(json) =
            serde_json::to_string(&ArenaFrame::UserPlayersSnapshot { players: summaries })
            && futures::SinkExt::send(&mut sink, Message::Text(json))
                .await
                .is_err()
        {
            return;
        }
    }

    // Append to participants cache and broadcast MemberJoined for new members.
    if is_new_member {
        // Resolve this user's player row so the participant entry carries the
        // player id the leaderboard is keyed by (None when they haven't
        // joined as a player, e.g. a dashboard-only CLI connection).
        let member_player_id = players::Entity::find()
            .filter(players::Column::SessionIdFk.eq(session_id))
            .filter(players::Column::UserIdFk.eq(user_id))
            .filter(players::Column::RevokedAt.is_null())
            .one(&db)
            .await
            .ok()
            .flatten()
            .map(|p| p.id);
        let member_version = {
            let mut cache = session_cache.write().unwrap_or_else(|e| e.into_inner());
            cache.version = cache.version.saturating_add(1);
            cache.participants.push(MemberInfo {
                user_id: user_id_str.clone(),
                player_id: member_player_id,
                display_name: display_name.clone(),
                joined_at: joined_at.to_rfc3339(),
                avatar_url: avatar_url.clone(),
                fingerprint: None,
                username: username.clone(),
                agent_display_name: None,
                completion_status: None,
            });
            cache.version
        };
        let _ = broadcast_tx.send(ArenaFrame::MemberJoined {
            user_id: user_id_str.clone(),
            player_id: member_player_id,
            display_name: display_name.clone(),
            joined_at: joined_at.to_rfc3339(),
            avatar_url: avatar_url.clone(),
            fingerprint: None,
            username: username.clone(),
            agent_display_name: None,
            version: member_version,
        });
    }

    // Forward broadcast frames, filtered by client kind. The receive half must
    // be polled too: it drives the protocol (auto-Pong to client Pings, so
    // proxies don't reap the idle dashboard) and surfaces Close, without which
    // the task and its broadcast::Receiver slot leaked until the next frame.
    loop {
        tokio::select! {
            inbound = futures::StreamExt::next(&mut stream) => {
                match inbound {
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Ok(_)) => {}
                    Some(Err(_)) => break,
                }
            }
            frame = broadcast_rx.recv() => {
                match frame {
                    Ok(frame) => {
                        if !should_forward(&frame, client_kind) {
                            continue;
                        }
                        let json = match serde_json::to_string(&frame) {
                            Ok(s) => s,
                            Err(e) => {
                                tracing::warn!(error = %e, "ws_session: serialize failed");
                                continue;
                            }
                        };
                        if futures::SinkExt::send(&mut sink, Message::Text(json))
                            .await
                            .is_err()
                        {
                            break;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        tracing::debug!(session_id = %session_id, skipped = n, "ws_session: lagged");
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                        break;
                    }
                }
            }
        }
    }

    let _ = futures::SinkExt::close(&mut sink).await;

    // On disconnect, remove from participants cache and notify peers.
    if is_new_member {
        let disconnect_version = {
            let mut cache = session_cache.write().unwrap_or_else(|e| e.into_inner());
            cache.version = cache.version.saturating_add(1);
            cache.participants.retain(|p| p.user_id != user_id_str);
            cache.version
        };
        let _ = broadcast_tx.send(ArenaFrame::PlayerDisconnected {
            player_id: user_id,
            version: disconnect_version,
        });
    }

    tracing::debug!(
        session_id = %session_id,
        user_id = %user_id,
        kind = ?client_kind,
        "ws_session: disconnected"
    );
}

#[cfg(test)]
mod tests {
    use super::{ClientKind, build_session_snapshot, should_register_as_participant};
    use crate::state::SessionCacheInner;
    use arena_core::protocol::{MemberInfo, PlayerCompletionStatus};
    use arena_core::session_completion::SCHEDULER_STATE_COMPLETED;
    use arena_core::session_status::SessionStatus;
    use chrono::Utc;
    use migration::{Migrator, MigratorTrait};
    use sea_orm::{ActiveModelTrait, Database, DatabaseConnection, Set};
    use std::sync::{Arc, RwLock};
    use uuid::Uuid;

    #[test]
    fn browser_clients_are_not_participants() {
        assert!(should_register_as_participant(ClientKind::Cli));
        assert!(!should_register_as_participant(ClientKind::Browser));
    }

    fn member(user_id: Uuid, player_id: Option<Uuid>, name: &str) -> MemberInfo {
        MemberInfo {
            user_id: user_id.to_string(),
            player_id,
            display_name: name.to_string(),
            joined_at: Utc::now().to_rfc3339(),
            avatar_url: None,
            fingerprint: None,
            username: None,
            agent_display_name: None,
            completion_status: None,
        }
    }

    async fn insert_player(
        db: &DatabaseConnection,
        session_id: Uuid,
        user_id: Uuid,
        name: &str,
        revoked: bool,
    ) -> Uuid {
        let player_id = Uuid::new_v4();
        crate::entities::players::ActiveModel {
            id: Set(player_id),
            session_id_fk: Set(session_id),
            user_id_fk: Set(Some(user_id)),
            display_name: Set(name.to_string()),
            fingerprint: Set(None),
            metadata_json: Set(None),
            joined_at: Set(Utc::now()),
            reconnected_at: Set(None),
            revoked_at: Set(if revoked { Some(Utc::now()) } else { None }),
            agent_connected: Set(false),
            agent_last_seen_at: Set(None),
        }
        .insert(db)
        .await
        .expect("insert player");
        player_id
    }

    async fn insert_scheduler_state(
        db: &DatabaseConnection,
        session_id: Uuid,
        player_id: Uuid,
        task_id: Option<Uuid>,
        state: &str,
    ) {
        crate::entities::session_scheduler_state::ActiveModel {
            id: Set(Uuid::new_v4()),
            session_id_fk: Set(session_id),
            player_id_fk: Set(player_id),
            task_id: Set(task_id),
            state: Set(state.to_string()),
            next_probe_at: Set(None),
            created_at: Set(Utc::now()),
            updated_at: Set(Utc::now()),
        }
        .insert(db)
        .await
        .expect("insert scheduler state");
    }

    /// The WS snapshot (detail path shared by dashboard and observer
    /// handlers) must carry the derived judge-aware completion status per
    /// participant: tasks-done + pending judge → awaiting_judges, mid-tasks
    /// → in_progress, revoked (no status entry) → None.
    #[tokio::test]
    async fn session_snapshot_carries_completion_status_per_participant() {
        let db = Database::connect("sqlite::memory:")
            .await
            .expect("db connect");
        Migrator::up(&db, None).await.expect("migrate");

        let user_id = Uuid::new_v4();
        crate::entities::users::ActiveModel {
            id: Set(user_id),
            email: Set("ws-cs@test.local".to_string()),
            password_hash: Set(None),
            display_name: Set("User".to_string()),
            created_at: Set(Utc::now()),
            updated_at: Set(Utc::now()),
            is_admin: Set(false),
            avatar_url: Set(None),
            email_verified: Set(false),
            username: Set(None),
            plan: Set(arena_core::quota::PLAN_FREE.to_string()),
            judge_run_limit: Set(None),
            judge_run_credits: Set(0),
        }
        .insert(&db)
        .await
        .expect("insert user");

        let project_id = Uuid::new_v4();
        crate::entities::projects::ActiveModel {
            id: Set(project_id),
            name: Set("proj-ws-cs".to_string()),
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
        .insert(&db)
        .await
        .expect("insert project");

        let session_id = Uuid::new_v4();
        crate::entities::sessions::ActiveModel {
            id: Set(session_id),
            name: Set("s-ws-cs".to_string()),
            created_at: Set(Utc::now()),
            owner_id_fk: Set(Some(user_id)),
            status: Set(SessionStatus::Running),
            join_code: Set("WSCSCD".to_string()),
            started_at: Set(Some(Utc::now())),
            finished_at: Set(None),
            paused_at: Set(None),
            paused_duration_secs: Set(None),
            project_id_fk: Set(project_id),
            game_server_id: Set(None),
            cancel_reason: Set(None),
            cancelled_by: Set(None),
        }
        .insert(&db)
        .await
        .expect("insert session");

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
        .insert(&db)
        .await
        .expect("insert task");

        // One judge attached to the task; no judge_results row → run pending.
        let judge_id = Uuid::new_v4();
        crate::entities::judges::ActiveModel {
            id: Set(judge_id),
            slug: Set("test-coverage".to_string()),
            name: Set("Test Coverage".to_string()),
            description: Set("".to_string()),
            prompt: Set("judge it".to_string()),
            rating_scale: Set(serde_json::json!({"min": 1, "max": 10})),
            kind: Set("llm".to_string()),
            scope: Set("task".to_string()),
            evidence_mode: Set("tools".to_string()),
            evidence_needs: Set(None),
            created_at: Set(Utc::now()),
            updated_at: Set(Utc::now()),
            llm_provider_id_fk: Set(None),
            llm_model: Set(None),
            llm_pool_id_fk: Set(None),
            llm_source_order: Set(arena_core::llm::resolve::SOURCE_ORDER_POOL_FIRST.to_string()),
            criteria: Set(None),
            max_interactive: Set(None),
            avatar_url: Set(None),
            probes_config: Set(None),
            ignore_paths: Set(None),
        }
        .insert(&db)
        .await
        .expect("insert judge");
        crate::entities::task_judges::ActiveModel {
            id: Set(Uuid::new_v4()),
            task_id: Set(task_id),
            judge_id: Set(judge_id),
            ordinal: Set(0),
            rating_scale_override: Set(None),
            created_at: Set(Utc::now()),
            updated_at: Set(Utc::now()),
            weight: Set(None),
        }
        .insert(&db)
        .await
        .expect("insert task_judge");

        // Player A: tasks done, judge pending. Player B: mid-task.
        // Player C: revoked — excluded from the status computation.
        let player_a = insert_player(&db, session_id, user_id, "a", false).await;
        let player_b = insert_player(&db, session_id, user_id, "b", false).await;
        let player_c = insert_player(&db, session_id, user_id, "c", true).await;
        insert_scheduler_state(&db, session_id, player_a, None, SCHEDULER_STATE_COMPLETED).await;
        insert_scheduler_state(&db, session_id, player_b, Some(task_id), "active").await;

        let cache = Arc::new(RwLock::new(SessionCacheInner {
            session_id,
            phase: SessionStatus::Running,
            version: 1,
            participants: vec![
                member(user_id, Some(player_a), "a"),
                member(user_id, Some(player_b), "b"),
                member(user_id, Some(player_c), "c"),
            ],
            leaderboard: vec![],
            started_at: None,
        }));

        let snapshot = build_session_snapshot(&db, &cache).await;

        let status_of = |pid: Uuid| {
            snapshot
                .participants
                .iter()
                .find(|m| m.player_id == Some(pid))
                .expect("participant present")
                .completion_status
        };
        assert_eq!(
            status_of(player_a),
            Some(PlayerCompletionStatus::AwaitingJudges),
            "tasks-done player with a pending judge run must be awaiting_judges"
        );
        assert_eq!(
            status_of(player_b),
            Some(PlayerCompletionStatus::InProgress),
            "player with tasks remaining must be in_progress"
        );
        assert_eq!(
            status_of(player_c),
            None,
            "revoked player has no status entry and must stay None"
        );
    }
}
