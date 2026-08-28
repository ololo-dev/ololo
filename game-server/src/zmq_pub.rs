use arena_core::protocol::ZmqEvent;
use async_trait::async_trait;
use std::sync::Arc;
use tokio::sync::Mutex;
use zeromq::{PubSocket, Socket, SocketSend, ZmqMessage};

#[async_trait]
pub trait EventPublisher: Send + Sync {
    async fn publish(&self, event: &ZmqEvent);
}

pub struct ZmqEventPublisher {
    socket: Arc<Mutex<PubSocket>>,
}

impl ZmqEventPublisher {
    pub async fn bind(addr: &str) -> anyhow::Result<Self> {
        let mut socket = PubSocket::new();
        socket.bind(addr).await?;
        Ok(Self {
            socket: Arc::new(Mutex::new(socket)),
        })
    }
}

#[async_trait]
impl EventPublisher for ZmqEventPublisher {
    async fn publish(&self, event: &ZmqEvent) {
        let payload = match serde_json::to_vec(event) {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!("zmq publish: serialize failed: {e}");
                return;
            }
        };
        let message = ZmqMessage::from(payload);
        let socket: &mut PubSocket = &mut *self.socket.lock().await;
        if let Err(e) = SocketSend::send(socket, message).await {
            tracing::warn!("zmq publish: send failed: {e}");
        }
    }
}

pub struct NoopEventPublisher;

#[async_trait]
impl EventPublisher for NoopEventPublisher {
    async fn publish(&self, _event: &ZmqEvent) {}
}
