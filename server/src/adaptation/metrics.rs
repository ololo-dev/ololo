//! Bounded-cardinality adaptation metrics.

use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RejectReason {
    EmptyCommand,
    InvalidMarkdownFence,
    MultiConcernAmbiguous,
    ParseError,
    Timeout,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AdaptationMetricsSnapshot {
    pub retries_total: u64,
    pub split_generated_total: u64,
    pub reject_empty_command_total: u64,
    pub reject_invalid_markdown_fence_total: u64,
    pub reject_multi_concern_ambiguous_total: u64,
    pub reject_parse_error_total: u64,
    pub reject_timeout_total: u64,
    pub reject_other_total: u64,
    pub success_total: u64,
    pub failure_total: u64,
    pub latency_ms_total: u64,
}

#[derive(Debug)]
pub struct AdaptationMetrics {
    retries_total: AtomicU64,
    split_generated_total: AtomicU64,
    reject_empty_command_total: AtomicU64,
    reject_invalid_markdown_fence_total: AtomicU64,
    reject_multi_concern_ambiguous_total: AtomicU64,
    reject_parse_error_total: AtomicU64,
    reject_timeout_total: AtomicU64,
    reject_other_total: AtomicU64,
    success_total: AtomicU64,
    failure_total: AtomicU64,
    latency_ms_total: AtomicU64,
}

impl Default for AdaptationMetrics {
    fn default() -> Self {
        Self::new()
    }
}

impl AdaptationMetrics {
    pub const fn new() -> Self {
        Self {
            retries_total: AtomicU64::new(0),
            split_generated_total: AtomicU64::new(0),
            reject_empty_command_total: AtomicU64::new(0),
            reject_invalid_markdown_fence_total: AtomicU64::new(0),
            reject_multi_concern_ambiguous_total: AtomicU64::new(0),
            reject_parse_error_total: AtomicU64::new(0),
            reject_timeout_total: AtomicU64::new(0),
            reject_other_total: AtomicU64::new(0),
            success_total: AtomicU64::new(0),
            failure_total: AtomicU64::new(0),
            latency_ms_total: AtomicU64::new(0),
        }
    }

    pub fn inc_retry(&self) {
        self.retries_total.fetch_add(1, Ordering::Relaxed);
    }

    pub fn inc_split_generated_by(&self, count: u64) {
        self.split_generated_total
            .fetch_add(count, Ordering::Relaxed);
    }

    pub fn inc_success(&self) {
        self.success_total.fetch_add(1, Ordering::Relaxed);
    }

    pub fn inc_failure(&self) {
        self.failure_total.fetch_add(1, Ordering::Relaxed);
    }

    pub fn add_latency_ms(&self, ms: u64) {
        self.latency_ms_total.fetch_add(ms, Ordering::Relaxed);
    }

    pub fn inc_reject(&self, reason: RejectReason) {
        match reason {
            RejectReason::EmptyCommand => {
                self.reject_empty_command_total
                    .fetch_add(1, Ordering::Relaxed);
            }
            RejectReason::InvalidMarkdownFence => {
                self.reject_invalid_markdown_fence_total
                    .fetch_add(1, Ordering::Relaxed);
            }
            RejectReason::MultiConcernAmbiguous => {
                self.reject_multi_concern_ambiguous_total
                    .fetch_add(1, Ordering::Relaxed);
            }
            RejectReason::ParseError => {
                self.reject_parse_error_total
                    .fetch_add(1, Ordering::Relaxed);
            }
            RejectReason::Timeout => {
                self.reject_timeout_total.fetch_add(1, Ordering::Relaxed);
            }
            RejectReason::Other => {
                self.reject_other_total.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    pub fn snapshot(&self) -> AdaptationMetricsSnapshot {
        AdaptationMetricsSnapshot {
            retries_total: self.retries_total.load(Ordering::Relaxed),
            split_generated_total: self.split_generated_total.load(Ordering::Relaxed),
            reject_empty_command_total: self.reject_empty_command_total.load(Ordering::Relaxed),
            reject_invalid_markdown_fence_total: self
                .reject_invalid_markdown_fence_total
                .load(Ordering::Relaxed),
            reject_multi_concern_ambiguous_total: self
                .reject_multi_concern_ambiguous_total
                .load(Ordering::Relaxed),
            reject_parse_error_total: self.reject_parse_error_total.load(Ordering::Relaxed),
            reject_timeout_total: self.reject_timeout_total.load(Ordering::Relaxed),
            reject_other_total: self.reject_other_total.load(Ordering::Relaxed),
            success_total: self.success_total.load(Ordering::Relaxed),
            failure_total: self.failure_total.load(Ordering::Relaxed),
            latency_ms_total: self.latency_ms_total.load(Ordering::Relaxed),
        }
    }
}

pub fn classify_reject_reason(message: &str) -> RejectReason {
    let lower = message.to_ascii_lowercase();
    if lower.contains("empty_command") {
        return RejectReason::EmptyCommand;
    }
    if lower.contains("invalid_markdown_fence") {
        return RejectReason::InvalidMarkdownFence;
    }
    if lower.contains("multi_concern_ambiguous") {
        return RejectReason::MultiConcernAmbiguous;
    }
    if lower.contains("parse_error") {
        return RejectReason::ParseError;
    }
    if lower.contains("timeout") {
        return RejectReason::Timeout;
    }
    RejectReason::Other
}

#[cfg(test)]
mod tests {
    use super::{AdaptationMetrics, RejectReason, classify_reject_reason};

    #[test]
    fn classify_reject_reason_is_bounded() {
        assert_eq!(
            classify_reject_reason("empty_command"),
            RejectReason::EmptyCommand
        );
        assert_eq!(
            classify_reject_reason("invalid_markdown_fence"),
            RejectReason::InvalidMarkdownFence
        );
        assert_eq!(
            classify_reject_reason("multi_concern_ambiguous"),
            RejectReason::MultiConcernAmbiguous
        );
        assert_eq!(
            classify_reject_reason("parse_error: x"),
            RejectReason::ParseError
        );
        assert_eq!(classify_reject_reason("llm_timeout"), RejectReason::Timeout);
        assert_eq!(
            classify_reject_reason("something_else"),
            RejectReason::Other
        );
    }

    #[test]
    fn metrics_snapshot_accumulates() {
        let metrics = AdaptationMetrics::new();
        metrics.inc_retry();
        metrics.inc_split_generated_by(2);
        metrics.inc_reject(RejectReason::MultiConcernAmbiguous);
        metrics.inc_success();
        metrics.inc_failure();
        metrics.add_latency_ms(12);

        let snap = metrics.snapshot();
        assert_eq!(snap.retries_total, 1);
        assert_eq!(snap.split_generated_total, 2);
        assert_eq!(snap.reject_multi_concern_ambiguous_total, 1);
        assert_eq!(snap.success_total, 1);
        assert_eq!(snap.failure_total, 1);
        assert_eq!(snap.latency_ms_total, 12);
    }
}
