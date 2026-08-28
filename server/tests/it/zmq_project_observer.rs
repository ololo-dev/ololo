//! Regression: a game-server-driven session status change must reach the
//! project page's WS observers.
//!
//! `/projects/[slug]` keeps its session list — and the "Live now" strip above
//! the tabs — live from `ProjectSessionUpdate` frames. Only the REST handlers
//! used to emit those, so a session ending on its own timer (the normal game
//! over) arrives purely as a ZMQ `SessionStatus` and left the project page
//! showing it as running until a reload.

use arena_core::protocol::ZmqEvent;
use arena_core::session_status::SessionStatus;
use migration::{Migrator, MigratorTrait};
use sea_orm::{ActiveModelTrait, Set};
use server::entities::{players, projects, sessions, users};
use server::protocol::ArenaFrame;
use server::{AppState, AuthConfig};
use std::time::Duration;
use tokio::sync::broadcast;
use uuid::Uuid;

const JOIN_CODE: &str = "OVER01";

async fn test_state() -> AppState {
    let db = sea_orm::Database::connect("sqlite::memory:")
        .await
        .expect("sqlite connect");
    Migrator::up(&db, None).await.expect("migrate up");
    let cfg = AuthConfig {
        jwt_signing_key: b"integration-test-secret-32-bytes-or-more-xxxxxxx".to_vec(),
        frontend_origins: vec!["http://localhost:5173".to_string()],
        access_ttl: Duration::from_secs(900),
        refresh_ttl: Duration::from_secs(30 * 86_400),
        max_agents_per_session: 16,
    };
    AppState::new(db, cfg)
}

/// user → project → running session(JOIN_CODE). Returns (project_id, session_id).
async fn seed(state: &AppState) -> (Uuid, Uuid) {
    let now = chrono::Utc::now();
    let user_id = Uuid::new_v4();
    users::ActiveModel {
        id: Set(user_id),
        email: Set(format!("u-{user_id}@example.com")),
        password_hash: Set(None),
        display_name: Set("Owner".to_string()),
        created_at: Set(now),
        updated_at: Set(now),
        is_admin: Set(false),
        avatar_url: Set(None),
        email_verified: Set(false),
        username: Set(None),
        plan: Set(arena_core::quota::PLAN_PREMIUM.to_string()),
        judge_run_limit: Set(None),
        judge_run_credits: Set(0),
    }
    .insert(&state.db)
    .await
    .expect("user");

    let project_id = Uuid::new_v4();
    projects::ActiveModel {
        id: Set(project_id),
        name: Set("P".to_string()),
        slug: Set(None),
        description: Set(String::new()),
        category: Set(None),
        tags: Set(String::new()),
        cover_image_url: Set(None),
        owner_user_id_fk: Set(user_id),
        public: Set(true),
        archived_at: Set(None),
        created_at: Set(now),
        updated_at: Set(now),
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
    .insert(&state.db)
    .await
    .expect("project");

    let session_id = Uuid::new_v4();
    sessions::ActiveModel {
        id: Set(session_id),
        name: Set("Friday Night Showdown".to_string()),
        created_at: Set(now),
        owner_id_fk: Set(None),
        status: Set(SessionStatus::Running),
        join_code: Set(JOIN_CODE.to_string()),
        game_server_id: Set(None),
        cancel_reason: Set(None),
        cancelled_by: Set(None),
        started_at: Set(Some(now)),
        finished_at: Set(None),
        paused_at: Set(None),
        paused_duration_secs: Set(None),
        project_id_fk: Set(project_id),
    }
    .insert(&state.db)
    .await
    .expect("session");

    (project_id, session_id)
}

/// Subscribe the way `ws/project.rs` does on observer connect.
fn observe(state: &AppState, project_id: Uuid) -> broadcast::Receiver<ArenaFrame> {
    state
        .project_registry
        .entry(project_id)
        .or_insert_with(|| broadcast::channel::<ArenaFrame>(64).0)
        .value()
        .subscribe()
}

#[tokio::test]
async fn game_over_reaches_project_observers_without_a_dashboard_open() {
    let state = test_state().await;
    let (project_id, session_id) = seed(&state).await;
    let mut rx = observe(&state, project_id);

    // Deliberately no `session_registry` entry: nobody has the session
    // dashboard open, which is the common case for a project-page visitor.
    server::zmq_sub::route_event(
        &state,
        &ZmqEvent::SessionStatus {
            join_code: JOIN_CODE.to_string(),
            status: SessionStatus::Finished.to_string(),
            version: 7,
            cancel_reason: None,
            cancelled_by: None,
        },
    )
    .await
    .expect("route_event");

    match rx.try_recv().expect("project observers get a frame") {
        ArenaFrame::ProjectSessionUpdate {
            session_id: got_id,
            status,
            project_id: got_project,
            name,
            ..
        } => {
            assert_eq!(got_id, session_id);
            assert_eq!(got_project, project_id);
            assert_eq!(
                status,
                SessionStatus::Finished,
                "strip must stop showing it"
            );
            assert_eq!(name, "Friday Night Showdown");
        }
        other => panic!("unexpected frame: {other:?}"),
    }
}

#[tokio::test]
async fn cancellation_carries_its_reason_to_project_observers() {
    let state = test_state().await;
    let (project_id, _) = seed(&state).await;
    let mut rx = observe(&state, project_id);

    server::zmq_sub::route_event(
        &state,
        &ZmqEvent::SessionStatus {
            join_code: JOIN_CODE.to_string(),
            status: SessionStatus::Cancelled.to_string(),
            version: 8,
            cancel_reason: Some("idle_timeout".to_string()),
            cancelled_by: None,
        },
    )
    .await
    .expect("route_event");

    match rx.try_recv().expect("project observers get a frame") {
        ArenaFrame::ProjectSessionUpdate {
            status,
            cancel_reason,
            ..
        } => {
            assert_eq!(status, SessionStatus::Cancelled);
            assert_eq!(cancel_reason.as_deref(), Some("idle_timeout"));
        }
        other => panic!("unexpected frame: {other:?}"),
    }
}

#[tokio::test]
async fn a_player_joining_refreshes_the_count_on_the_live_card() {
    let state = test_state().await;
    let (project_id, session_id) = seed(&state).await;
    let mut rx = observe(&state, project_id);

    // Two players in the room, one of them already revoked — the card should
    // report only the live one.
    for (name, revoked) in [("A", false), ("B", true)] {
        players::ActiveModel {
            id: Set(Uuid::new_v4()),
            session_id_fk: Set(session_id),
            user_id_fk: Set(None),
            display_name: Set(name.to_string()),
            fingerprint: Set(None),
            metadata_json: Set(None),
            joined_at: Set(chrono::Utc::now()),
            reconnected_at: Set(None),
            revoked_at: Set(revoked.then(chrono::Utc::now)),
            agent_connected: Set(false),
            agent_last_seen_at: Set(None),
        }
        .insert(&state.db)
        .await
        .expect("player");
    }

    server::zmq_sub::route_event(
        &state,
        &ZmqEvent::PlayerJoin {
            join_code: JOIN_CODE.to_string(),
            player_id: Uuid::new_v4(),
            display_name: "A".to_string(),
            user_id: None,
            joined_at: chrono::Utc::now().to_rfc3339(),
            avatar_url: None,
            fingerprint: None,
            username: None,
            version: 1,
        },
    )
    .await
    .expect("route_event");

    match rx.try_recv().expect("project observers get a frame") {
        ArenaFrame::ProjectSessionUpdate {
            player_count,
            status,
            ..
        } => {
            assert_eq!(player_count, 1, "revoked players must not be counted");
            assert_eq!(status, SessionStatus::Running, "status carried through");
        }
        other => panic!("unexpected frame: {other:?}"),
    }
}

#[tokio::test]
async fn countdown_ticks_reach_project_observers() {
    let state = test_state().await;
    let (project_id, session_id) = seed(&state).await;
    let mut rx = observe(&state, project_id);

    server::zmq_sub::route_event(
        &state,
        &ZmqEvent::SessionTimer {
            join_code: JOIN_CODE.to_string(),
            phase: SessionStatus::Running.to_string(),
            seconds_remaining: 42,
            version: 3,
        },
    )
    .await
    .expect("route_event");

    match rx.try_recv().expect("project observers get a tick") {
        ArenaFrame::RunningCountdown {
            session_id: got,
            seconds_remaining,
            ..
        } => {
            assert_eq!(got, session_id);
            assert_eq!(seconds_remaining, 42, "the card's \"Ends in\" clock");
        }
        other => panic!("unexpected frame: {other:?}"),
    }
}

#[tokio::test]
async fn an_unparseable_status_is_dropped_rather_than_routed() {
    let state = test_state().await;
    let (project_id, _) = seed(&state).await;
    let mut rx = observe(&state, project_id);

    server::zmq_sub::route_event(
        &state,
        &ZmqEvent::SessionStatus {
            join_code: JOIN_CODE.to_string(),
            status: "not-a-status".to_string(),
            version: 9,
            cancel_reason: None,
            cancelled_by: None,
        },
    )
    .await
    .expect("route_event must not fail on bad input");

    assert!(rx.try_recv().is_err(), "no frame for an unparseable status");
}
