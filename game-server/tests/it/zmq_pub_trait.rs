use arena_core::protocol::ZmqEvent;
use game_server::zmq_pub::{EventPublisher, NoopEventPublisher};

#[tokio::test]
async fn noop_publisher_does_not_panic() {
    let publisher = NoopEventPublisher;
    let event = ZmqEvent::SessionTimer {
        join_code: "ABCD1234".to_string(),
        phase: "lobby".to_string(),
        seconds_remaining: 30,
        version: 1,
    };
    publisher.publish(&event).await;
}
