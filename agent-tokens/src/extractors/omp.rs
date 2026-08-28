use super::pi_common;
use crate::paths;
use crate::trait_::TokenExtractor;
use crate::types::{AgentId, SessionCounts, SessionStats};

pub struct Omp;

impl TokenExtractor for Omp {
    fn id(&self) -> AgentId {
        AgentId::Omp
    }

    fn detect(&self) -> bool {
        paths::omp_sessions_dir().exists()
    }

    fn extract(&self, since: Option<i64>) -> Vec<SessionCounts> {
        pi_common::extract_from_root(&paths::omp_sessions_dir(), AgentId::Omp, since)
    }

    fn stats(&self, since: Option<i64>) -> Vec<SessionStats> {
        pi_common::stats_from_root(&paths::omp_sessions_dir(), AgentId::Omp, since)
    }
}
