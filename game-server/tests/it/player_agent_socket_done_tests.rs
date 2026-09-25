//! A player who has finished every task keeps its socket until the session
//! ends. Its agent reconnects whenever a connection goes silent; a server
//! that answered a finished player's reconnect with the acknowledgement and
//! a closed socket sent that agent into a dial-close loop for the rest of
//! the session.

use std::time::Duration;

use arena_core::entities::{session_scheduler_state, sessions};
use arena_core::session_status::SessionStatus;
use chrono::Utc;
use futures::{SinkExt, StreamExt};
use sea_orm::{ActiveModelTrait, EntityTrait, Set};
use tokio_tungstenite::tungstenite::Message;
use uuid::Uuid;

use super::player_agent_ws_auth_tests::{
    insert_pat, insert_player, insert_user, seed_session, serve, setup_db, test_state,
};

#[tokio::test]
async fn a_finished_player_who_reconnects_waits_for_the_session_end() {
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;
    let state = test_state(setup_db().await);
    let (session_id, _owner) = seed_session(&state, "WSDONE").await;
    let db = state.db.clone();
    // The session is running, and this player already finished its tasks.
    let mut session: sessions::ActiveModel = sessions::Entity::find_by_id(session_id)
        .one(&db)
        .await
        .unwrap()
        .unwrap()
        .into();
    session.status = Set(SessionStatus::Running);
    session.started_at = Set(Some(Utc::now()));
    session.update(&db).await.unwrap();
    let member = insert_user(&db).await;
    let player = insert_player(&db, session_id, member).await;
    insert_pat(&db, member, "ololo_done").await;
    session_scheduler_state::ActiveModel {
        id: Set(Uuid::new_v4()),
        session_id_fk: Set(session_id),
        player_id_fk: Set(player),
        task_id: Set(None),
        state: Set(arena_core::session_completion::SCHEDULER_STATE_COMPLETED.to_string()),
        next_probe_at: Set(None),
        created_at: Set(Utc::now()),
        updated_at: Set(Utc::now()),
    }
    .insert(&db)
    .await
    .unwrap();
    let addr = serve(game_server::build_router(state)).await;

    let mut req = format!("ws://{addr}/ws/player/agent/WSDONE?player_id={player}")
        .into_client_request()
        .unwrap();
    req.headers_mut()
        .insert("X-API-Key", "ololo_done".parse().unwrap());
    let (mut ws, _) = tokio_tungstenite::connect_async(req)
        .await
        .expect("upgrade");

    // The per-player acknowledgement first…
    let ack = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            match ws.next().await {
                Some(Ok(Message::Text(t))) if t.contains("player_tasks_completed") => break t,
                Some(Ok(_)) => continue,
                other => panic!("socket ended before the acknowledgement: {other:?}"),
            }
        }
    })
    .await
    .expect("the acknowledgement arrives");
    assert!(ack.contains("session_complete"), "{ack}");

    // …then the socket stays: it answers a ping and carries on.
    ws.send(Message::Ping(b"alive?".to_vec())).await.unwrap();
    let alive = tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            match ws.next().await {
                Some(Ok(Message::Pong(data))) => break data,
                Some(Ok(_)) => continue,
                other => panic!("closed instead of waiting for the session: {other:?}"),
            }
        }
    })
    .await
    .expect("a pong while the session runs");
    assert_eq!(alive, b"alive?".to_vec());

    // The session ends: now the socket closes.
    let mut session: sessions::ActiveModel = sessions::Entity::find_by_id(session_id)
        .one(&db)
        .await
        .unwrap()
        .unwrap()
        .into();
    session.status = Set(SessionStatus::Cancelled);
    session.update(&db).await.unwrap();
    let closed = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            match ws.next().await {
                None | Some(Ok(Message::Close(_))) | Some(Err(_)) => break,
                Some(Ok(_)) => continue,
            }
        }
    })
    .await;
    assert!(closed.is_ok(), "the socket closes once the session ends");
}
