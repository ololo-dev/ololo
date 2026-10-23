//! A probe's deadline is the command's run time, not the player's reading
//! time. In session V2LZPE every Extreme Startup (cli) probe had a 2-second
//! deadline, and the permission prompt the agent shows before running a
//! command closed with it: nobody could read it, every probe scored
//! `no_response`. An agent that says it is asking gets the permission window
//! on top of the deadline.

use std::time::Duration;

use arena_core::entities::{sessions, tasks};
use arena_core::protocol::PROBE_PERMISSION_WAIT_SECS;
use arena_core::session_status::SessionStatus;
use chrono::Utc;
use futures::{SinkExt, StreamExt};
use sea_orm::{ActiveModelTrait, EntityTrait, Set};
use tokio_tungstenite::tungstenite::Message;
use uuid::Uuid;

use super::player_agent_ws_auth_tests::{
    insert_pat, insert_player, insert_user, seed_session, serve, setup_db, test_state,
};

const ONE_CHECK_MD: &str = r#"## Answer a city question

```js fixtures
({ q: "city" })
```

```sh command
echo city
```

```js validation
result.trim() === "city"
```
"#;

/// The next text frame of `type`, skipping countdowns and the like.
async fn next_of_type<S>(ws: &mut S, kind: &str) -> serde_json::Value
where
    S: StreamExt<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin,
{
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            match ws.next().await {
                Some(Ok(Message::Text(t))) => {
                    let v: serde_json::Value = serde_json::from_str(&t).unwrap();
                    if v["type"] == kind {
                        break v;
                    }
                }
                Some(Ok(_)) => continue,
                other => panic!("socket ended before a {kind} frame: {other:?}"),
            }
        }
    })
    .await
    .unwrap_or_else(|_| panic!("no {kind} frame"))
}

#[tokio::test]
async fn a_player_reading_the_permission_prompt_is_not_a_missed_deadline() {
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;
    let state = test_state(setup_db().await);
    let (session_id, _owner) = seed_session(&state, "PERMIT").await;
    let db = state.db.clone();
    let session = sessions::Entity::find_by_id(session_id)
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    let project_id = session.project_id_fk;
    let mut running: sessions::ActiveModel = session.into();
    running.status = Set(SessionStatus::Running);
    running.started_at = Set(Some(Utc::now()));
    running.update(&db).await.unwrap();

    // One task, one check, a 2-second deadline — the Extreme Startup shape.
    tasks::ActiveModel {
        id: Set(Uuid::new_v4()),
        project_id_fk: Set(project_id),
        ordinal: Set(0),
        title: Set("Answer".to_string()),
        content: Set("answer the question".to_string()),
        test_template: Set(
            serde_json::json!({ "kind": "shell", "command_template": ONE_CHECK_MD }),
        ),
        created_at: Set(Utc::now()),
        tags: Set("[]".to_string()),
        point_value: Set(10),
        deadline_secs: Set(Some(2)),
        min_interval_secs: Set(None),
        interval_increment_secs: Set(None),
        max_interval_secs: Set(None),
        fail_points: Set(-5),
        no_response_points: Set(-10),
        health_points: sea_orm::ActiveValue::NotSet,
        completion_bonus_points: Set(10),
        evaluation: Set(None),
    }
    .insert(&db)
    .await
    .unwrap();

    let member = insert_user(&db).await;
    let player = insert_player(&db, session_id, member).await;
    insert_pat(&db, member, "ololo_permit").await;
    let addr = serve(game_server::build_router(state)).await;
    let mut req = format!("ws://{addr}/ws/player/agent/PERMIT?player_id={player}")
        .into_client_request()
        .unwrap();
    req.headers_mut()
        .insert("X-API-Key", "ololo_permit".parse().unwrap());
    let (mut ws, _) = tokio_tungstenite::connect_async(req)
        .await
        .expect("upgrade");

    let push = next_of_type(&mut ws, "test_push").await;
    assert_eq!(push["deadline_secs"], 2);
    assert_eq!(push["permission_wait_secs"], PROBE_PERMISSION_WAIT_SECS);
    let probe_id = push["probe_id"].as_str().unwrap().to_string();

    // The agent is asking; the player takes longer than the deadline to read.
    ws.send(Message::Text(
        serde_json::json!({ "type": "probe_permission_pending", "probe_id": probe_id }).to_string(),
    ))
    .await
    .unwrap();
    tokio::time::sleep(Duration::from_secs(3)).await;
    ws.send(Message::Text(
        serde_json::json!({
            "type": "test_result",
            "probe_id": probe_id,
            "stdout": "city",
            "exit_code": 0,
            "duration_ms": 5,
        })
        .to_string(),
    ))
    .await
    .unwrap();

    let graded = next_of_type(&mut ws, "probe_graded").await;
    assert_eq!(graded["probe_id"], probe_id);
    assert_eq!(graded["outcome"], "pass", "{graded}");
}
