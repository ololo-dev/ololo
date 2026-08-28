#![allow(dead_code)]

//! `ololo whoami` command and the PAT-expiry display helpers.

use crate::config::Config;
use anyhow::{Result, anyhow};
use serde::Deserialize;
use sha2::Digest;
use std::time::Duration;

use crate::ui;

/// Days since Unix epoch for today (UTC).
fn today_epoch_days() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
        / 86400
}

/// Parse the date portion (first 10 chars) of an ISO-8601 string to days
/// since Unix epoch using the proleptic Gregorian calendar (Howard Hinnant's
/// algorithm). Returns `None` if the string is too short or unparseable.
fn parse_date_epoch_days(s: &str) -> Option<i64> {
    if s.len() < 10 {
        return None;
    }
    let y: i64 = s[0..4].parse().ok()?;
    let m: i64 = s[5..7].parse().ok()?;
    let d: i64 = s[8..10].parse().ok()?;
    let (y, m_off) = if m <= 2 { (y - 1, 9i64) } else { (y, -3i64) };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let m_adj = m + m_off;
    let doy = (153 * m_adj + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some(era * 146097 + doe - 719468)
}

/// Inverse of [`parse_date_epoch_days`]: convert a (non-negative) day count
/// back to an `YYYY-MM-DD` string. Used only to build test fixtures.
fn days_to_iso(days: i64) -> String {
    let z = days.max(0) + 719468;
    let era = z / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}")
}

/// Urgency classification that drives [`display_expiry`]'s coloring.
/// Pure: depends only on the supplied date string and the current day.
enum ExpiryUrgency {
    Unknown,
    Expired(i64),
    Today,
    Soon(i64),
    Later(i64),
}

fn expiry_urgency(expires_at: &str) -> ExpiryUrgency {
    let date = &expires_at[..expires_at.len().min(10)];
    match parse_date_epoch_days(date) {
        None => ExpiryUrgency::Unknown,
        Some(exp_days) => {
            let delta = exp_days - today_epoch_days();
            if delta < 0 {
                ExpiryUrgency::Expired(-delta)
            } else if delta == 0 {
                ExpiryUrgency::Today
            } else if delta <= 7 {
                ExpiryUrgency::Soon(delta)
            } else {
                ExpiryUrgency::Later(delta)
            }
        }
    }
}

/// Print the `expires` field colored by urgency.
fn display_expiry(expires_at: &str) {
    match expiry_urgency(expires_at) {
        ExpiryUrgency::Unknown => ui::field("expires", expires_at),
        ExpiryUrgency::Expired(n) => {
            let s = if n == 1 { "" } else { "s" };
            ui::field_err(
                "expires",
                format!(
                    "{date}  expired {n} day{s} ago",
                    date = &expires_at[..expires_at.len().min(10)],
                    s = s
                ),
            );
        }
        ExpiryUrgency::Today => {
            ui::field_warn(
                "expires",
                format!("{}  expires today", &expires_at[..expires_at.len().min(10)]),
            );
        }
        ExpiryUrgency::Soon(n) => {
            let s = if n == 1 { "" } else { "s" };
            ui::field_warn(
                "expires",
                format!(
                    "{}  in {n} day{s}",
                    &expires_at[..expires_at.len().min(10)],
                    n = n,
                    s = s
                ),
            );
        }
        ExpiryUrgency::Later(n) => {
            let s = if n == 1 { "" } else { "s" };
            ui::field_ok(
                "expires",
                format!(
                    "{}  in {n} day{s}",
                    &expires_at[..expires_at.len().min(10)],
                    n = n,
                    s = s
                ),
            );
        }
    }
}

pub async fn run_whoami(profile: &str) -> Result<()> {
    let cfg =
        Config::load(profile).ok_or_else(|| anyhow!("not logged in; run 'ololo login' first"))?;

    let digest = sha2::Sha256::digest(cfg.token.as_bytes());
    let full_hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
    let fingerprint = &full_hex[..8];

    ui::field("profile", profile);
    ui::field("server", &cfg.server_url);
    ui::field("fingerprint", fingerprint);

    let base = cfg.server_url.trim_end_matches('/');
    let url = format!("{base}/api/users/me/pats");

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap_or_else(|_| reqwest::Client::new());

    match client.get(&url).bearer_auth(&cfg.token).send().await {
        Ok(resp) if resp.status().is_success() => {
            #[derive(Deserialize)]
            struct PatListResponse {
                pats: Vec<PatEntry>,
            }
            #[derive(Deserialize)]
            struct PatEntry {
                fingerprint: String,
                expires_at: Option<String>,
            }

            match resp.json::<PatListResponse>().await {
                Ok(list) => match list.pats.iter().find(|p| p.fingerprint == fingerprint) {
                    Some(pat) => match &pat.expires_at {
                        None => ui::field_ok("expires", "never"),
                        Some(exp) => display_expiry(exp),
                    },
                    None => ui::field_dim("expires", "unknown"),
                },
                Err(_) => ui::field_dim("expires", "unavailable (offline)"),
            }
        }
        _ => ui::field_dim("expires", "unavailable (offline)"),
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn today_epoch_days_is_positive() {
        assert!(today_epoch_days() > 19_000, "Unix epoch was > 1970");
    }

    #[test]
    fn parse_date_epoch_days_known_dates() {
        assert_eq!(parse_date_epoch_days("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(parse_date_epoch_days("2000-01-01"), Some(10957));
        assert_eq!(parse_date_epoch_days("2024-01-01"), Some(19723));
    }

    #[test]
    fn parse_date_epoch_days_roundtrip() {
        let today = today_epoch_days();
        assert_eq!(parse_date_epoch_days(&days_to_iso(today)), Some(today));
    }

    #[test]
    fn parse_date_epoch_days_rejects_short_input() {
        assert_eq!(parse_date_epoch_days(""), None);
        assert_eq!(parse_date_epoch_days("2024-"), None);
        assert_eq!(parse_date_epoch_days("not-a-date"), None);
    }

    #[test]
    fn expiry_urgency_bands() {
        let today = today_epoch_days();
        assert!(matches!(
            expiry_urgency(&days_to_iso(today)),
            ExpiryUrgency::Today
        ));
        assert!(matches!(
            expiry_urgency(&days_to_iso(today - 5)),
            ExpiryUrgency::Expired(5)
        ));
        assert!(matches!(
            expiry_urgency(&days_to_iso(today + 3)),
            ExpiryUrgency::Soon(3)
        ));
        assert!(matches!(
            expiry_urgency(&days_to_iso(today + 100)),
            ExpiryUrgency::Later(100)
        ));
        assert!(matches!(expiry_urgency("garbage"), ExpiryUrgency::Unknown));
    }
}
