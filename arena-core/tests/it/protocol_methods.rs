use uuid::Uuid;

use arena_core::protocol::*;

#[test]
fn newtype_as_uuid_and_from() {
    let u = Uuid::new_v4();
    assert_eq!(PlayerId(u).as_uuid(), u);
    assert_eq!(TaskId(u).as_uuid(), u);
    assert_eq!(SessionId(u).as_uuid(), u);
    let _: PlayerId = u.into();
    let _: TaskId = u.into();
    let _: SessionId = u.into();
}

#[test]
fn zmq_event_join_code_per_variant() {
    let jc = "ABC123";
    assert_eq!(
        ZmqEvent::SessionTimer {
            join_code: jc.to_string(),
            phase: "x".into(),
            seconds_remaining: 1,
            version: 1,
        }
        .join_code(),
        jc
    );
    assert_eq!(
        ZmqEvent::SessionStatus {
            join_code: jc.to_string(),
            status: "x".into(),
            version: 1,
            cancel_reason: None,
            cancelled_by: None,
        }
        .join_code(),
        jc
    );
    assert_eq!(
        ZmqEvent::PlayerJoin {
            join_code: jc.to_string(),
            player_id: Uuid::nil(),
            display_name: "x".into(),
            user_id: None,
            joined_at: "x".into(),
            avatar_url: None,
            fingerprint: None,
            username: None,
            version: 1,
        }
        .join_code(),
        jc
    );
    assert_eq!(
        ZmqEvent::PlayerLeave {
            join_code: jc.to_string(),
            player_id: Uuid::nil(),
            version: 1,
        }
        .join_code(),
        jc
    );
    assert_eq!(
        ZmqEvent::ScoreChange {
            join_code: jc.to_string(),
            player_id: Uuid::nil(),
            delta: 1,
            total: 2,
            version: 1,
        }
        .join_code(),
        jc
    );
}
