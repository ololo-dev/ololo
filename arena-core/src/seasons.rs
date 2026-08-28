//! Season windows: calendar-month periods used by seasonal boards.
//!
//! Lives apart from the awards module because seasonal splits are not
//! specific to Arena Points — the project Top Players board uses the same
//! windows over plain scoring data.
use chrono::{DateTime, Datelike, TimeZone, Utc};

/// Start of the current season: first day of the calendar month, UTC.
pub fn season_start(now: DateTime<Utc>) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(now.year(), now.month(), 1, 0, 0, 0)
        .single()
        .expect("first of month is a valid UTC timestamp")
}

/// End of the season that begins at `start`: the first instant of the next
/// calendar month, UTC. Seasons are half-open windows `[start, end)`.
pub fn season_end(start: DateTime<Utc>) -> DateTime<Utc> {
    let (year, month) = match start.month() {
        12 => (start.year() + 1, 1),
        m => (start.year(), m + 1),
    };
    Utc.with_ymd_and_hms(year, month, 1, 0, 0, 0)
        .single()
        .expect("first of month is a valid UTC timestamp")
}

/// `YYYY-MM` identifier of the season containing `at` — the stable key the
/// API takes and returns for a past season.
pub fn season_key(at: DateTime<Utc>) -> String {
    format!("{:04}-{:02}", at.year(), at.month())
}

/// Parse a `YYYY-MM` season key into that season's start. `None` when the
/// key is malformed or names a month that does not exist.
pub fn parse_season_key(key: &str) -> Option<DateTime<Utc>> {
    let (year, month) = key.split_once('-')?;
    if year.len() != 4 || month.len() != 2 {
        return None;
    }
    let year: i32 = year.parse().ok()?;
    let month: u32 = month.parse().ok()?;
    Utc.with_ymd_and_hms(year, month, 1, 0, 0, 0).single()
}
