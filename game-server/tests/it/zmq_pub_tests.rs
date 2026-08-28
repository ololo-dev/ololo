//! Coverage for src/zmq_pub.rs: `ZmqEventPublisher::bind` and the success
//! path of `EventPublisher::publish` (serialize + send).

use arena_core::protocol::ZmqEvent;
use game_server::zmq_pub::EventPublisher;
use game_server::zmq_pub::ZmqEventPublisher;

#[tokio::test]
async fn bind_and_publish_does_not_panic() {
    // PUB socket bind; publish serializes the event to JSON and sends. The
    // subscriber side is irrelevant for coverage — we only exercise the
    // bind + serialize + send success path.
    let publisher = ZmqEventPublisher::bind("tcp://127.0.0.1:5999")
        .await
        .expect("zmq bind");

    let event = ZmqEvent::SessionTimer {
        join_code: "ABCD1234".to_string(),
        phase: "lobby".to_string(),
        seconds_remaining: 30,
        version: 1,
    };
    publisher.publish(&event).await;

    let score = ZmqEvent::ScoreChange {
        join_code: "ABCD1234".to_string(),
        player_id: uuid::Uuid::new_v4(),
        delta: 5,
        total: 5,
        version: 2,
    };
    publisher.publish(&score).await;
}
