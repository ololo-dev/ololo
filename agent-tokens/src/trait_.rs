use crate::types::{AgentId, SessionCounts, SessionStats};

pub trait TokenExtractor: Send + Sync {
    fn id(&self) -> AgentId;
    fn detect(&self) -> bool;
    /// Extract session token counts. If `since` is Some(epoch_ms), only count
    /// tokens from messages/turns with timestamps >= since (per-hour consumption).
    /// If `since` is None, return session-lifetime totals.
    fn extract(&self, since: Option<i64>) -> Vec<SessionCounts>;
    /// Extract behavioural session statistics (message counts, tool usage,
    /// skill loads). Same `since` semantics as `extract`. Default: none —
    /// extractors opt in as their log format exposes the data.
    fn stats(&self, since: Option<i64>) -> Vec<SessionStats> {
        let _ = since;
        vec![]
    }
}
