use super::pi_common;
use crate::paths;
use crate::trait_::TokenExtractor;
use crate::types::{AgentId, SessionCounts, SessionStats};

pub struct Pi;

impl TokenExtractor for Pi {
    fn id(&self) -> AgentId {
        AgentId::Pi
    }

    fn detect(&self) -> bool {
        paths::pi_sessions_dir().exists()
    }

    fn extract(&self, since: Option<i64>) -> Vec<SessionCounts> {
        pi_common::extract_from_root(&paths::pi_sessions_dir(), AgentId::Pi, since)
    }

    fn stats(&self, since: Option<i64>) -> Vec<SessionStats> {
        pi_common::stats_from_root(&paths::pi_sessions_dir(), AgentId::Pi, since)
    }
}
