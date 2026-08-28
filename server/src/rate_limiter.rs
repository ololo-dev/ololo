//! Rate limiting primitives for join-code and email-sending endpoints.
//!
//! `RateLimiter` is a DI trait so tests can swap in `NoOpRateLimiter` without
//! any real concurrency overhead. The production implementation is
//! `InMemoryRateLimiter`, which uses a sliding 60-second window capped at
//! `MAX_REQUESTS` per IP. `EmailRateLimiter` enforces 5 requests per 15-minute
//! window for email-sending endpoints.

use std::{
    collections::{HashMap, VecDeque},
    sync::Mutex,
    time::{Duration, Instant},
};

/// Maximum number of requests allowed per IP within the window. Every SSR
/// view of a session page (lobby/spectator/admin/player) spends one lookup,
/// so the cap leaves room for a human bouncing between pages while staying
/// negligible against the join-code space.
const MAX_REQUESTS: usize = 30;
/// Length of the sliding window.
const WINDOW: Duration = Duration::from_secs(60);

/// When a limiter's map exceeds this many IPs, sweep out entries whose window
/// has fully elapsed so it cannot grow without bound.
const EVICT_THRESHOLD: usize = 10_000;

/// Shared sliding-window check+record for the `Mutex<HashMap>` limiters.
///
/// Recovers from a poisoned lock (`into_inner`) rather than propagating the
/// panic — the guarded state is only timestamps, so it is safe to keep using
/// after an unrelated panic; the previous `.expect("poisoned")` bricked the
/// endpoint for the rest of the process instead.
fn sliding_window_check(
    state: &Mutex<HashMap<String, VecDeque<Instant>>>,
    ip: &str,
    now: Instant,
    window: Duration,
    max: usize,
) -> bool {
    let cutoff = now - window;
    let mut map = state.lock().unwrap_or_else(|e| e.into_inner());

    if map.len() > EVICT_THRESHOLD {
        // `back()` is the most recent stamp; if it is stale the whole entry is.
        map.retain(|_, ts| ts.back().is_some_and(|t| *t >= cutoff));
    }

    let timestamps = map.entry(ip.to_owned()).or_default();
    while timestamps.front().is_some_and(|t| *t < cutoff) {
        timestamps.pop_front();
    }
    if timestamps.len() >= max {
        return false;
    }
    timestamps.push_back(now);
    true
}

/// Trait that controls whether a request from a given IP should be allowed.
///
/// Implementations must be `Send + Sync` so they can be stored in Axum's
/// shared `AppState`.
pub trait RateLimiter: Send + Sync {
    /// Returns `true` if the request is allowed, `false` if the rate limit has
    /// been exceeded.  Implementations must both check and record the attempt
    /// atomically (i.e. under the same lock).
    fn check_and_record(&self, ip: &str) -> bool;
}

/// In-memory sliding-window rate limiter.
///
/// State is held in a `Mutex<HashMap>` so multiple Axum worker threads can
/// share it safely.  State is **not** persisted — it is lost on restart, which
/// is an accepted trade-off (FR-JC-014).
pub struct InMemoryRateLimiter {
    state: Mutex<HashMap<String, VecDeque<Instant>>>,
}

impl InMemoryRateLimiter {
    /// Create a new, empty rate limiter.
    pub fn new() -> Self {
        Self {
            state: Mutex::new(HashMap::new()),
        }
    }
}

impl Default for InMemoryRateLimiter {
    fn default() -> Self {
        Self::new()
    }
}

impl RateLimiter for InMemoryRateLimiter {
    fn check_and_record(&self, ip: &str) -> bool {
        sliding_window_check(&self.state, ip, Instant::now(), WINDOW, MAX_REQUESTS)
    }
}

/// Maximum requests allowed per IP within the email rate-limit window.
const EMAIL_MAX_REQUESTS: usize = 5;
/// Length of the email rate-limit sliding window (15 minutes).
const EMAIL_WINDOW: Duration = Duration::from_secs(900);

/// In-memory sliding-window rate limiter for email-sending endpoints.
///
/// Enforces 5 requests per 15-minute window per IP address.
/// Entries are never evicted in v1 — known debt.
/// TODO(v2): evict stale entries to prevent unbounded memory growth.
pub struct EmailRateLimiter {
    state: Mutex<HashMap<String, VecDeque<Instant>>>,
}

impl EmailRateLimiter {
    pub fn new() -> Self {
        Self {
            state: Mutex::new(HashMap::new()),
        }
    }
}

impl Default for EmailRateLimiter {
    fn default() -> Self {
        Self::new()
    }
}

impl RateLimiter for EmailRateLimiter {
    fn check_and_record(&self, ip: &str) -> bool {
        sliding_window_check(
            &self.state,
            ip,
            Instant::now(),
            EMAIL_WINDOW,
            EMAIL_MAX_REQUESTS,
        )
    }
}

/// Sliding-window rate limiter with a configurable cap and window.
///
/// Used for the credential endpoints (`/auth/login`, `/auth/register`) and
/// the token endpoints (`/auth/refresh`, `/auth/logout`), which previously
/// had no rate limiting at all. Same mechanics as `InMemoryRateLimiter`,
/// with the cap/window supplied at construction.
pub struct SlidingWindowLimiter {
    state: Mutex<HashMap<String, VecDeque<Instant>>>,
    window: Duration,
    max: usize,
}

impl SlidingWindowLimiter {
    pub fn new(max: usize, window: Duration) -> Self {
        Self {
            state: Mutex::new(HashMap::new()),
            window,
            max,
        }
    }
}

impl RateLimiter for SlidingWindowLimiter {
    fn check_and_record(&self, ip: &str) -> bool {
        sliding_window_check(&self.state, ip, Instant::now(), self.window, self.max)
    }
}

/// No-op rate limiter that always allows requests.
///
/// Intended for use in tests where real rate-limiting behaviour is not under
/// test and would only add flakiness.
pub struct NoOpRateLimiter;

impl RateLimiter for NoOpRateLimiter {
    fn check_and_record(&self, _ip: &str) -> bool {
        true
    }
}

/// Per-IP rate limiter for the public profile endpoints.
///
/// Uses a `DashMap` for lock-striped concurrency (one shard per hashed IP
/// range) rather than a global `Mutex`, giving better throughput under load.
/// The sliding window is 1 second; the cap defaults to
/// `ARENA_PROFILE_RATE_LIMIT_RPS` (default 20 RPS).
pub struct ProfileRateLimiter {
    state: dashmap::DashMap<std::net::IpAddr, std::collections::VecDeque<Instant>>,
    window: Duration,
    max_rps: usize,
}

impl ProfileRateLimiter {
    /// Create a new limiter that allows at most `max_rps` requests per second per IP.
    pub fn new(max_rps: usize) -> Self {
        Self {
            state: dashmap::DashMap::new(),
            window: Duration::from_secs(1),
            max_rps,
        }
    }

    /// Returns `true` if the request from `ip` is within the limit.
    ///
    /// Check and record are performed atomically under the DashMap shard lock.
    pub fn check_and_record(&self, ip: std::net::IpAddr) -> bool {
        let now = Instant::now();
        let cutoff = now - self.window;
        let mut timestamps = self.state.entry(ip).or_default();
        while timestamps.front().is_some_and(|t| *t < cutoff) {
            timestamps.pop_front();
        }
        if timestamps.len() >= self.max_rps {
            return false;
        }
        timestamps.push_back(now);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allows_up_to_max_requests() {
        let rl = InMemoryRateLimiter::new();
        for _ in 0..MAX_REQUESTS {
            assert!(rl.check_and_record("1.2.3.4"));
        }
    }

    #[test]
    fn blocks_on_eleventh_request() {
        let rl = InMemoryRateLimiter::new();
        for _ in 0..MAX_REQUESTS {
            rl.check_and_record("1.2.3.4");
        }
        assert!(!rl.check_and_record("1.2.3.4"));
    }

    #[test]
    fn different_ips_are_independent() {
        let rl = InMemoryRateLimiter::new();
        for _ in 0..MAX_REQUESTS {
            rl.check_and_record("1.1.1.1");
        }
        // A different IP must still be allowed.
        assert!(rl.check_and_record("2.2.2.2"));
    }

    #[test]
    fn survives_a_poisoned_lock() {
        // CONC-M1: a panic while holding the lock must not brick the endpoint.
        use std::sync::Arc;
        let rl = Arc::new(InMemoryRateLimiter::new());
        let rl2 = rl.clone();
        let _ = std::thread::spawn(move || {
            let _guard = rl2.state.lock().unwrap();
            panic!("poison the mutex");
        })
        .join();
        // The mutex is now poisoned; check_and_record must still work.
        assert!(rl.check_and_record("1.2.3.4"));
    }

    #[test]
    fn noop_always_allows() {
        let rl = NoOpRateLimiter;
        for _ in 0..100 {
            assert!(rl.check_and_record("any"));
        }
    }

    #[test]
    fn email_allows_up_to_five() {
        let rl = EmailRateLimiter::new();
        for _ in 0..EMAIL_MAX_REQUESTS {
            assert!(rl.check_and_record("1.2.3.4"));
        }
    }

    #[test]
    fn email_blocks_sixth_request() {
        let rl = EmailRateLimiter::new();
        for _ in 0..EMAIL_MAX_REQUESTS {
            rl.check_and_record("1.2.3.4");
        }
        assert!(!rl.check_and_record("1.2.3.4"));
    }

    #[test]
    fn sliding_window_allows_up_to_max() {
        let rl = SlidingWindowLimiter::new(3, Duration::from_secs(60));
        for _ in 0..3 {
            assert!(rl.check_and_record("1.2.3.4"));
        }
        assert!(!rl.check_and_record("1.2.3.4"));
    }

    #[test]
    fn sliding_window_ips_are_independent() {
        let rl = SlidingWindowLimiter::new(1, Duration::from_secs(60));
        assert!(rl.check_and_record("1.1.1.1"));
        assert!(!rl.check_and_record("1.1.1.1"));
        assert!(rl.check_and_record("2.2.2.2"));
    }

    #[test]
    fn email_different_ips_are_independent() {
        let rl = EmailRateLimiter::new();
        for _ in 0..EMAIL_MAX_REQUESTS {
            rl.check_and_record("1.1.1.1");
        }
        assert!(rl.check_and_record("2.2.2.2"));
    }
}
