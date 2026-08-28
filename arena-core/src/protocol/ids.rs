//! Id newtypes shared across the wire protocol.

use serde::{Deserialize, Serialize};

/// Server-issued task identifier (UUID v4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct TaskId(pub uuid::Uuid);

impl TaskId {
    pub fn as_uuid(&self) -> uuid::Uuid {
        self.0
    }
}

impl From<uuid::Uuid> for TaskId {
    fn from(u: uuid::Uuid) -> Self {
        Self(u)
    }
}

/// Server-issued player identifier (UUID v4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct PlayerId(pub uuid::Uuid);

impl PlayerId {
    pub fn as_uuid(&self) -> uuid::Uuid {
        self.0
    }
}

impl From<uuid::Uuid> for PlayerId {
    fn from(u: uuid::Uuid) -> Self {
        Self(u)
    }
}

/// Server-issued session identifier (UUID v4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct SessionId(pub uuid::Uuid);

impl SessionId {
    pub fn as_uuid(&self) -> uuid::Uuid {
        self.0
    }
}

impl From<uuid::Uuid> for SessionId {
    fn from(u: uuid::Uuid) -> Self {
        Self(u)
    }
}
