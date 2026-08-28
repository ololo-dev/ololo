use std::path::Path;

/// Read all lines from a file. Handles \r\n line endings.
pub fn read_lines(path: &Path) -> Vec<String> {
    let Ok(content) = std::fs::read_to_string(path) else {
        return vec![];
    };
    content
        .lines()
        .map(|l| l.trim_end_matches('\r'))
        .map(String::from)
        .collect()
}

/// Parse a timestamp from a serde_json::Value into epoch-ms.
/// Handles ISO-8601 strings with Z or offset suffixes, and numeric (ms) values.
pub fn parse_ts_ms(value: &serde_json::Value) -> Option<i64> {
    if let Some(s) = value.as_str() {
        parse_iso8601_to_ms(s)
    } else {
        value.as_i64()
    }
}

fn parse_iso8601_to_ms(s: &str) -> Option<i64> {
    // ponytail: minimal ISO-8601 parser; handles YYYY-MM-DDTHH:MM:SS(.fff)?(Z|+HH:MM)
    // For anything more exotic, upgrade to a proper chrono/jiff dep.
    let s = s.trim();
    if s.is_empty() {
        return None;
    }

    // Try to parse as a number first (some tools store ms directly)
    if let Ok(n) = s.parse::<i64>() {
        return Some(n);
    }

    // Must have at least YYYY-MM-DDTHH:MM:SS
    if s.len() < 19 {
        return None;
    }

    let year: i64 = s.get(0..4)?.parse().ok()?;
    let month: i64 = s.get(5..7)?.parse().ok()?;
    let day: i64 = s.get(8..10)?.parse().ok()?;
    let hour: i64 = s.get(11..13)?.parse().ok()?;
    let minute: i64 = s.get(14..16)?.parse().ok()?;
    let second: i64 = s.get(17..19)?.parse().ok()?;

    // Parse fractional seconds if present
    let mut ms: i64 = 0;
    let mut rest = &s[19..];

    if rest.starts_with('.') {
        let frac_start = 20;
        let frac_end = s[frac_start..]
            .find(|c: char| !c.is_ascii_digit())
            .map(|p| frac_start + p)
            .unwrap_or(s.len());
        let frac = &s[frac_start..frac_end];
        if !frac.is_empty() {
            // Pad/truncate to 3 digits for ms
            let padded = format!("{:0<3}", &frac[..frac.len().min(3)]);
            ms = padded.parse().unwrap_or(0);
        }
        rest = &s[frac_end..];
    }

    // Parse timezone
    let tz_offset_secs: i64 = if rest.is_empty() || rest.starts_with('Z') || rest.starts_with('z') {
        0
    } else if rest.starts_with('+') || rest.starts_with('-') {
        let sign: i64 = if rest.starts_with('-') { -1 } else { 1 };
        let tz = &rest[1..];
        if tz.len() >= 5 && tz.contains(':') {
            let tz_h: i64 = tz.get(0..2)?.parse().ok()?;
            let tz_m: i64 = tz.get(3..5)?.parse().ok()?;
            sign * (tz_h * 3600 + tz_m * 60)
        } else {
            0
        }
    } else {
        0
    };

    let epoch_days = days_from_civil(year, month, day);
    let epoch_secs = epoch_days * 86400 + hour * 3600 + minute * 60 + second - tz_offset_secs;
    Some(epoch_secs * 1000 + ms)
}

/// Howard Hinnant's algorithm: days since Unix epoch from civil date.
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_iso_z() {
        let ms = parse_ts_ms(&serde_json::Value::String("2026-06-19T12:00:00Z".into()));
        assert!(ms.is_some());
        assert_eq!(ms.unwrap() % 1000, 0);
    }

    #[test]
    fn parse_iso_with_offset() {
        let ms = parse_ts_ms(&serde_json::Value::String(
            "2026-06-19T12:00:00+02:00".into(),
        ));
        assert!(ms.is_some());
        // +02:00 means 2 hours earlier in UTC
    }

    #[test]
    fn parse_iso_with_ms() {
        let ms = parse_ts_ms(&serde_json::Value::String(
            "2026-06-19T12:00:00.123Z".into(),
        ));
        assert_eq!(
            ms,
            parse_ts_ms(&serde_json::Value::String("2026-06-19T12:00:00Z".into(),))
                .map(|v| v + 123)
        );
    }

    #[test]
    fn parse_numeric() {
        let ms = parse_ts_ms(&serde_json::Value::Number(1718808000000i64.into()));
        assert_eq!(ms, Some(1718808000000));
    }

    #[test]
    fn parse_empty_returns_none() {
        assert!(parse_ts_ms(&serde_json::Value::String(String::new())).is_none());
    }
}
