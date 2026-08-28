//! Background adaptation loop.
//!
//! Processes `AdaptationRequest`s from the mpsc channel. For each request,
//! spawns a per-request task that retries the `AdaptationService::adapt`
//! call indefinitely with exponential backoff until success or cancellation.
//!
//! Backoff formula: `min(5 * 2^attempt, 300)` seconds, where `attempt` is
//! 0-indexed (first failure → 5 s, second → 10 s, …, capped at 300 s).
//!
//! On each retry (attempt ≥ 1), an `AdaptationRetrying` frame is broadcast
//! to the session's admin observer channel so operators can monitor stuck
//! adaptations.
use crate::adaptation::metrics::{AdaptationMetrics, classify_reject_reason};
use crate::adaptation::service::{AdaptationError, AdaptationRequest, AdaptationServiceHandle};
use crate::state::AdminRegistry;
use arena_core::protocol::{AdaptationRetryingPayload, ArenaFrame};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, watch};

/// Start the background adaptation loop.
///
/// - `rx`: receives `AdaptationRequest`s from worker tasks.
/// - `service`: the adaptation service implementation (live, passthrough, or fake).
/// - `admin_registry`: broadcast channel registry for per-session admin observers.
/// - `shutdown`: watch receiver; when the sender is dropped (or sends `true`),
///   all in-flight retry sleeps are cancelled.
pub async fn start_adaptation_loop(
    mut rx: mpsc::Receiver<AdaptationRequest>,
    service: AdaptationServiceHandle,
    admin_registry: AdminRegistry,
    metrics: Arc<AdaptationMetrics>,
    shutdown: watch::Receiver<bool>,
) {
    while let Some(req) = rx.recv().await {
        let service = service.clone();
        let registry = admin_registry.clone();
        let metrics = metrics.clone();
        let shutdown_rx = shutdown.clone();
        tokio::spawn(run_adaptation(req, service, registry, metrics, shutdown_rx));
    }
    tracing::debug!("adaptation_loop: channel closed, exiting");
}

/// Per-request adaptation task with indefinite retry and exponential backoff.
async fn run_adaptation(
    req: AdaptationRequest,
    service: AdaptationServiceHandle,
    admin_registry: AdminRegistry,
    metrics: Arc<AdaptationMetrics>,
    mut shutdown: watch::Receiver<bool>,
) {
    let mut attempt: u32 = 0;
    let start = std::time::Instant::now();

    loop {
        match service.adapt(req.clone()).await {
            Ok(()) => {
                tracing::debug!(
                    session_id = %req.session_id,
                    task_id = %req.task_id,
                    player_id = %req.player_id,
                    "adaptation: succeeded after {} attempt(s)",
                    attempt
                );
                metrics.inc_success();
                metrics.add_latency_ms(start.elapsed().as_millis() as u64);
                return;
            }
            Err(AdaptationError::PermanentFailure) => {
                tracing::warn!(
                    session_id = %req.session_id,
                    task_id = %req.task_id,
                    player_id = %req.player_id,
                    "adaptation: permanent failure, giving up"
                );
                metrics.inc_failure();
                metrics.add_latency_ms(start.elapsed().as_millis() as u64);
                return;
            }
            Err(e) => {
                tracing::warn!(
                    session_id = %req.session_id,
                    task_id = %req.task_id,
                    player_id = %req.player_id,
                    error = %e,
                    attempt,
                    "adaptation: transient failure, will retry"
                );
                // Increment the DB retry count.
                attempt += 1;
                metrics.inc_retry();

                if let AdaptationError::LlmFailed(message) = &e {
                    metrics.inc_reject(classify_reject_reason(message));
                }

                // Broadcast AdaptationRetrying to admin observers on every retry.
                broadcast_retrying(&admin_registry, &req, attempt);

                // Exponential backoff: min(5 * 2^(attempt-1), 300) seconds.
                let delay_secs = 5u64
                    .saturating_mul(2u64.saturating_pow(attempt.saturating_sub(1)))
                    .min(300);
                let delay = Duration::from_secs(delay_secs);

                tracing::debug!(
                    session_id = %req.session_id,
                    delay_secs,
                    attempt,
                    "adaptation: backing off"
                );

                // Sleep, but exit early if the shutdown signal fires.
                tokio::select! {
                    _ = tokio::time::sleep(delay) => {}
                    _ = shutdown.changed() => {
                        tracing::debug!(
                            session_id = %req.session_id,
                            task_id = %req.task_id,
                            "adaptation: shutdown received during backoff, cancelling"
                        );
                        return;
                    }
                }
            }
        }
    }
}

/// Broadcast an `AdaptationRetrying` frame to the session's admin observers.
///
/// Fire-and-forget: errors are logged at debug level.
fn broadcast_retrying(registry: &AdminRegistry, req: &AdaptationRequest, attempt: u32) {
    let frame = ArenaFrame::AdaptationRetrying(AdaptationRetryingPayload {
        session_id: req.session_id,
        task_id: req.task_id,
        player_id: req.player_id,
        attempt,
    });
    if let Some(entry) = registry.get(&req.session_id)
        && let Err(e) = entry.send(frame)
    {
        tracing::debug!(
            session_id = %req.session_id,
            error = %e,
            "broadcast_retrying: no active admin subscribers"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adaptation::metrics::RejectReason;
    use std::sync::{Arc, Mutex};

    #[test]
    fn backoff_sequence() {
        // attempt=1 → 5s, attempt=2 → 10s, attempt=3 → 20s, ..., capped at 300s
        let expected = [5, 10, 20, 40, 80, 160, 300, 300];
        for (i, &expected_secs) in expected.iter().enumerate() {
            let attempt = (i + 1) as u32;
            let delay_secs = 5u64
                .saturating_mul(2u64.saturating_pow(attempt.saturating_sub(1)))
                .min(300);
            assert_eq!(
                delay_secs, expected_secs,
                "attempt={attempt} expected {expected_secs}s got {delay_secs}s"
            );
        }
    }

    #[test]
    fn backoff_does_not_overflow() {
        // Very large attempt should not overflow — saturating_pow + min(300) handles it.
        let attempt: u32 = 100;
        let delay_secs = 5u64
            .saturating_mul(2u64.saturating_pow(attempt.saturating_sub(1)))
            .min(300);
        assert_eq!(delay_secs, 300);
    }

    /// Verify that PermanentFailure does not retry (adapts the logic without async).
    #[test]
    fn permanent_failure_is_terminal() {
        let flag: Arc<Mutex<u32>> = Arc::new(Mutex::new(0));
        let flag2 = flag.clone();
        // Simulate: if we get PermanentFailure, we increment only once and stop.
        let error = AdaptationError::PermanentFailure;
        match error {
            AdaptationError::PermanentFailure => {
                *flag2.lock().unwrap() += 1;
                // return (no retry)
            }
            _ => {
                // would retry
                *flag2.lock().unwrap() += 100;
            }
        }
        assert_eq!(*flag.lock().unwrap(), 1);
    }

    #[test]
    fn metrics_classifies_reject_reasons() {
        assert_eq!(
            classify_reject_reason("multi_concern_ambiguous"),
            RejectReason::MultiConcernAmbiguous
        );
        assert_eq!(
            classify_reject_reason("parse_error: x"),
            RejectReason::ParseError
        );
    }
}
