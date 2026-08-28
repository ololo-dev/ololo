use arena_core::protocol::ZmqEvent;
use game_server::zmq_pub::{EventPublisher, NoopEventPublisher, ZmqEventPublisher};
use server::zmq_sub::{EventSubscriber, InMemoryEventSubscriber, ZmqEventSubscriber};

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

#[tokio::test]
async fn in_memory_subscriber_roundtrip() {
    let (tx, rx) = tokio::sync::mpsc::channel::<ZmqEvent>(16);
    let mut subscriber = InMemoryEventSubscriber::new(rx);

    let event = ZmqEvent::ScoreChange {
        join_code: "XYZ9999".to_string(),
        player_id: uuid::Uuid::new_v4(),
        delta: 100,
        total: 500,
        version: 7,
    };

    tx.send(event.clone()).await.unwrap();
    let received = subscriber.recv().await.unwrap();
    assert_eq!(received.join_code(), "XYZ9999");
}

#[tokio::test]
async fn zmq_pub_sub_roundtrip() {
    let port = 15432u16 + std::process::id() as u16 % 1000;
    let addr = format!("tcp://127.0.0.1:{}", port);

    let publisher = ZmqEventPublisher::bind(&addr).await.unwrap();
    let mut subscriber = ZmqEventSubscriber::connect(&addr, "").await.unwrap();

    tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;

    let event = ZmqEvent::SessionTimer {
        join_code: "RT123456".to_string(),
        phase: "running".to_string(),
        seconds_remaining: 300,
        version: 99,
    };

    publisher.publish(&event).await;

    let received = tokio::time::timeout(tokio::time::Duration::from_secs(5), subscriber.recv())
        .await
        .expect("timeout waiting for ZMQ message")
        .expect("subscriber returned None");

    assert_eq!(received.join_code(), "RT123456");
    if let ZmqEvent::SessionTimer {
        phase,
        seconds_remaining,
        version,
        ..
    } = received
    {
        assert_eq!(phase, "running");
        assert_eq!(seconds_remaining, 300);
        assert_eq!(version, 99);
    } else {
        panic!("Expected SessionTimer variant, got {:?}", received);
    }
}
