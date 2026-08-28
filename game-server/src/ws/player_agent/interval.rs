/// Clamp interval bounds so that `min <= max` and `increment <= span`.
/// Pure defensive normalization used when resolving probe cadence.
pub fn clamp_interval_bounds(
    min_interval: i32,
    max_interval: i32,
    increment: i32,
) -> (i32, i32, i32) {
    let mn = min_interval.min(max_interval);
    let mx = max_interval.max(min_interval);
    let span = (mx - mn).max(0);
    let inc = increment.min(span.max(1));
    (mn, mx, inc)
}

/// Advance the inter-probe backoff after a no-response / stale result,
/// keeping the new value within `[min, max]`.
pub fn apply_no_response_backoff(
    current: i32,
    increment: i32,
    min_interval: i32,
    max_interval: i32,
) -> i32 {
    (current + increment).min(max_interval).max(min_interval)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounds_normalized_when_inverted() {
        // min > max must be corrected; increment clamped to span.
        let (mn, mx, inc) = clamp_interval_bounds(10, 5, 3);
        assert_eq!((mn, mx), (5, 10));
        assert_eq!(inc, 3);
    }

    #[test]
    fn increment_clamped_to_span() {
        // span is 4, increment 100 -> clamped to 4 (span.max(1)).
        let (mn, mx, inc) = clamp_interval_bounds(0, 4, 100);
        assert_eq!((mn, mx, inc), (0, 4, 4));
    }

    #[test]
    fn backoff_stays_within_bounds() {
        assert_eq!(apply_no_response_backoff(2, 5, 1, 10), 7);
        assert_eq!(apply_no_response_backoff(9, 5, 1, 10), 10);
        assert_eq!(apply_no_response_backoff(0, 5, 3, 10), 5);
    }
}
