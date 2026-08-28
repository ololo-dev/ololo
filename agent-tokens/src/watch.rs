use crate::extractors::EXTRACTORS;
use crate::types::{SessionCounts, SessionStats};
use futures_util::Stream;
use std::time::Duration;

/// One-shot scan: run all extractors whose detect() returns true.
/// If `since` is Some(epoch_ms), only count tokens from activity within
/// that time window (per-hour consumption, not session lifetime totals).
/// If `since` is None, return session-lifetime totals.
pub fn snapshot(since: Option<i64>) -> Vec<SessionCounts> {
    let mut all = Vec::new();
    for ext in EXTRACTORS {
        if ext.detect() {
            all.extend(ext.extract(since));
        }
    }
    all
}

/// One-shot scan of behavioural session statistics (message counts, tool
/// usage, skill loads) across all detected agents. Same `since` semantics
/// as snapshot().
pub fn stats_snapshot(since: Option<i64>) -> Vec<SessionStats> {
    let mut all = Vec::new();
    for ext in EXTRACTORS {
        if ext.detect() {
            all.extend(ext.stats(since));
        }
    }
    all
}

/// Watch loop: emits a full Vec<SessionCounts> every `interval`.
/// `since` has the same semantics as snapshot().
/// Full re-scan per tick; upgrade to incremental (byte-offset seek+parse)
/// when session count exceeds ~100.
pub fn watch(interval: Duration, since: Option<i64>) -> impl Stream<Item = Vec<SessionCounts>> {
    futures_util::stream::unfold((), move |()| async move {
        tokio::time::sleep(interval).await;
        Some((snapshot(since), ()))
    })
}
