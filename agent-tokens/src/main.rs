use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "agent-tokens",
    version,
    about = "Debug CLI for agent-tokens crate"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// One-shot snapshot of agent token usage
    Snapshot {
        /// Output as JSON
        #[arg(long)]
        json: bool,
        /// Filter sessions started after this date (YYYY-MM-DD)
        #[arg(long)]
        start_date: Option<String>,
        /// Only show sessions from the last N period (e.g. 1h, 30m, 2d)
        #[arg(long)]
        period: Option<String>,
    },
    /// Session statistics: message counts, tool usage, skill loads
    Stats {
        /// Output as JSON
        #[arg(long)]
        json: bool,
        /// Filter activity after this date (YYYY-MM-DD)
        #[arg(long)]
        start_date: Option<String>,
        /// Only count activity from the last N period (e.g. 1h, 30m, 2d)
        #[arg(long)]
        period: Option<String>,
        /// Only show sessions for this agent (e.g. claude, opencode, pi)
        #[arg(long)]
        agent: Option<String>,
    },
    /// Watch agent token usage in real-time
    Watch {
        /// Poll interval in seconds
        #[arg(long, default_value_t = 5)]
        interval: u64,
        /// Output as JSON (one array per tick)
        #[arg(long)]
        json: bool,
        /// Filter sessions started after this date (YYYY-MM-DD)
        #[arg(long)]
        start_date: Option<String>,
        /// Only show sessions from the last N period (e.g. 1h, 30m, 2d)
        #[arg(long)]
        period: Option<String>,
    },
}

fn parse_start_date(s: &str) -> Option<i64> {
    // Parse YYYY-MM-DD to epoch-ms at 00:00:00 UTC
    let s = s.trim();
    if s.len() < 10 {
        return None;
    }
    let y: i64 = s.get(0..4)?.parse().ok()?;
    let m: i64 = s.get(5..7)?.parse().ok()?;
    let d: i64 = s.get(8..10)?.parse().ok()?;
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146097 + doe - 719468;
    Some(days * 86400 * 1000)
}

/// Parse a period string like "1h", "30m", "2d", "15s" into epoch-ms cutoff (now - period).
fn parse_period(s: &str) -> Option<i64> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    let last = s.chars().last()?;
    let num: i64 = s[..s.len() - 1].parse().ok()?;
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_millis() as i64;
    let period_ms = match last {
        's' => num * 1000,
        'm' => num * 60 * 1000,
        'h' => num * 60 * 60 * 1000,
        'd' => num * 24 * 60 * 60 * 1000,
        _ => return None,
    };
    Some(now_ms - period_ms)
}

/// Resolve the `since` cutoff from --start-date and/or --period.
/// --period takes precedence over --start-date if both are set.
fn resolve_since(start_date: &Option<String>, period: &Option<String>) -> Option<i64> {
    if let Some(p) = period
        && let Some(ms) = parse_period(p)
    {
        return Some(ms);
    }
    if let Some(s) = start_date {
        return parse_start_date(s);
    }
    None
}

fn format_table(sessions: &[agent_tokens::SessionCounts], hourly: bool) {
    if sessions.is_empty() {
        println!("No agent sessions detected.");
        return;
    }
    println!(
        "{:<10} {:<20} {:>8} {:>8} {:>8} {:>8} {:>8} {:<36} {:<20}",
        "AGENT", "MODEL", "INPUT", "OUTPUT", "CACHE_R", "CACHE_W", "COST", "SESSION_ID", "STARTED"
    );
    for s in sessions {
        let model = s.model.as_deref().unwrap_or("—");
        let cost = s.cost.map(|c| format!("{c:.4}")).unwrap_or("—".into());
        let started = s.started_at_ms.map(format_ms_to_date).unwrap_or("—".into());
        println!(
            "{:<10} {:<20} {:>8} {:>8} {:>8} {:>8} {:>8} {:<36} {:<20}",
            s.agent.as_kebab(),
            model,
            s.counts.input,
            s.counts.output,
            s.counts.cache_read,
            s.counts.cache_write,
            cost,
            s.session_id,
            started,
        );
    }
    println!();
    if hourly {
        format_hourly_summary(sessions);
    } else {
        format_summary(sessions);
    }
}

/// Extract (provider, model_id) from the model string.
/// OpenCode stores model as JSON: {"id":"glm-5.2","providerID":"ollama-cloud","variant":"default"}
/// Claude stores model as a plain string: "gpt-4-test"
fn parse_model_info(model: &Option<String>) -> (String, String) {
    match model {
        Some(m) => {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(m) {
                let id = v
                    .get("id")
                    .and_then(|i| i.as_str())
                    .unwrap_or(m)
                    .to_string();
                let provider = v
                    .get("providerID")
                    .and_then(|p| p.as_str())
                    .unwrap_or("—")
                    .to_string();
                (provider, id)
            } else {
                ("—".to_string(), m.clone())
            }
        }
        None => ("—".to_string(), "—".to_string()),
    }
}

fn format_summary(sessions: &[agent_tokens::SessionCounts]) {
    use std::collections::BTreeMap;

    // Aggregate by (agent, provider, model) -> totals
    #[derive(Default)]
    struct Agg {
        sessions: usize,
        input: u64,
        output: u64,
        cache_read: u64,
        cache_write: u64,
        cost: f64,
    }

    let mut groups: BTreeMap<(String, String, String), Agg> = BTreeMap::new();

    for s in sessions {
        let (provider, model_id) = parse_model_info(&s.model);
        let key = (s.agent.as_kebab().to_string(), provider, model_id);
        let agg = groups.entry(key).or_default();
        agg.sessions += 1;
        agg.input += s.counts.input;
        agg.output += s.counts.output;
        agg.cache_read += s.counts.cache_read;
        agg.cache_write += s.counts.cache_write;
        agg.cost += s.cost.unwrap_or(0.0);
    }

    println!(
        "── Summary (by agent + provider + model) ──────────────────────────────────────────────"
    );
    println!(
        "{:<10} {:<16} {:<24} {:>6} {:>10} {:>10} {:>10} {:>10} {:>10}",
        "AGENT", "PROVIDER", "MODEL", "SESS", "INPUT", "OUTPUT", "CACHE_R", "CACHE_W", "COST"
    );

    let mut total_sessions = 0usize;
    let mut total_input = 0u64;
    let mut total_output = 0u64;
    let mut total_cache_r = 0u64;
    let mut total_cache_w = 0u64;
    let mut total_cost = 0.0f64;

    for ((agent, provider, model_id), agg) in &groups {
        println!(
            "{:<10} {:<16} {:<24} {:>6} {:>10} {:>10} {:>10} {:>10} {:>10.4}",
            agent,
            provider,
            model_id,
            agg.sessions,
            agg.input,
            agg.output,
            agg.cache_read,
            agg.cache_write,
            agg.cost,
        );
        total_sessions += agg.sessions;
        total_input += agg.input;
        total_output += agg.output;
        total_cache_r += agg.cache_read;
        total_cache_w += agg.cache_write;
        total_cost += agg.cost;
    }

    println!(
        "{:<10} {:<16} {:<24} {:>6} {:>10} {:>10} {:>10} {:>10} {:>10.4}",
        "TOTAL",
        "",
        "",
        total_sessions,
        total_input,
        total_output,
        total_cache_r,
        total_cache_w,
        total_cost,
    );
}

fn format_hourly_summary(sessions: &[agent_tokens::SessionCounts]) {
    use std::collections::BTreeMap;

    // Aggregate by (hour_bucket, agent, provider, model) -> totals
    #[derive(Default)]
    struct Agg {
        sessions: usize,
        input: u64,
        output: u64,
        cache_read: u64,
        cache_write: u64,
        cost: f64,
    }

    let mut groups: BTreeMap<(i64, String, String, String), Agg> = BTreeMap::new();

    for s in sessions {
        let Some(ts) = s.started_at_ms else { continue };
        let hour_bucket = ts / (3600 * 1000); // truncate to hour
        let (provider, model_id) = parse_model_info(&s.model);
        let key = (
            hour_bucket,
            s.agent.as_kebab().to_string(),
            provider,
            model_id,
        );
        let agg = groups.entry(key).or_default();
        agg.sessions += 1;
        agg.input += s.counts.input;
        agg.output += s.counts.output;
        agg.cache_read += s.counts.cache_read;
        agg.cache_write += s.counts.cache_write;
        agg.cost += s.cost.unwrap_or(0.0);
    }

    println!(
        "── Hourly Summary (by hour + agent + provider + model) ────────────────────────────────────"
    );
    println!(
        "{:<16} {:<10} {:<16} {:<24} {:>6} {:>10} {:>10} {:>10} {:>10} {:>10}",
        "HOUR",
        "AGENT",
        "PROVIDER",
        "MODEL",
        "SESS",
        "INPUT",
        "OUTPUT",
        "CACHE_R",
        "CACHE_W",
        "COST"
    );

    let mut total_sessions = 0usize;
    let mut total_input = 0u64;
    let mut total_output = 0u64;
    let mut total_cache_r = 0u64;
    let mut total_cache_w = 0u64;
    let mut total_cost = 0.0f64;

    for ((hour_bucket, agent, provider, model_id), agg) in &groups {
        let hour_str = format_hour_bucket(*hour_bucket);
        println!(
            "{:<16} {:<10} {:<16} {:<24} {:>6} {:>10} {:>10} {:>10} {:>10} {:>10.4}",
            hour_str,
            agent,
            provider,
            model_id,
            agg.sessions,
            agg.input,
            agg.output,
            agg.cache_read,
            agg.cache_write,
            agg.cost,
        );
        total_sessions += agg.sessions;
        total_input += agg.input;
        total_output += agg.output;
        total_cache_r += agg.cache_read;
        total_cache_w += agg.cache_write;
        total_cost += agg.cost;
    }

    println!(
        "{:<16} {:<10} {:<16} {:<24} {:>6} {:>10} {:>10} {:>10} {:>10} {:>10.4}",
        "TOTAL",
        "",
        "",
        "",
        total_sessions,
        total_input,
        total_output,
        total_cache_r,
        total_cache_w,
        total_cost,
    );
}

fn format_stats_table(stats: &[agent_tokens::SessionStats]) {
    use std::collections::BTreeMap;

    if stats.is_empty() {
        println!("No agent sessions detected.");
        return;
    }

    println!(
        "{:<10} {:<36} {:>6} {:>6} {:>7} {:<20} {:<40}",
        "AGENT", "SESSION_ID", "USER", "ASST", "TOOLS", "LAST_SEEN", "TOP TOOLS"
    );
    for s in stats {
        let last_seen = s
            .last_seen_at_ms
            .map(format_ms_to_date)
            .unwrap_or("—".into());
        let mut top: Vec<(&String, &u64)> = s.tools.iter().collect();
        top.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
        let top_str = top
            .iter()
            .take(4)
            .map(|(name, n)| format!("{name}:{n}"))
            .collect::<Vec<_>>()
            .join(" ");
        println!(
            "{:<10} {:<36} {:>6} {:>6} {:>7} {:<20} {:<40}",
            s.agent.as_kebab(),
            s.session_id,
            s.user_messages,
            s.assistant_messages,
            s.tool_calls,
            last_seen,
            top_str,
        );
    }

    // Aggregate tool usage across all sessions, per agent
    let mut tool_totals: BTreeMap<(String, String), (u64, u64)> = BTreeMap::new(); // (agent, tool) -> (calls, sessions)
    let mut skill_totals: BTreeMap<(String, String), u64> = BTreeMap::new(); // (agent, skill) -> loads
    for s in stats {
        for (tool, n) in &s.tools {
            let e = tool_totals
                .entry((s.agent.as_kebab().to_string(), tool.clone()))
                .or_insert((0, 0));
            e.0 += n;
            e.1 += 1;
        }
        for (skill, n) in &s.skills {
            *skill_totals
                .entry((s.agent.as_kebab().to_string(), skill.clone()))
                .or_insert(0) += n;
        }
    }

    println!();
    println!("── Tool usage (by agent + tool) ────────────────────────────────");
    println!(
        "{:<10} {:<28} {:>8} {:>10}",
        "AGENT", "TOOL", "CALLS", "SESSIONS"
    );
    let mut tools: Vec<_> = tool_totals.into_iter().collect();
    tools.sort_by(|a, b| a.0.0.cmp(&b.0.0).then(b.1.0.cmp(&a.1.0)));
    for ((agent, tool), (calls, sess)) in tools {
        println!("{agent:<10} {tool:<28} {calls:>8} {sess:>10}");
    }

    if !skill_totals.is_empty() {
        println!();
        println!("── Skills loaded ───────────────────────────────────────────────");
        println!("{:<10} {:<40} {:>8}", "AGENT", "SKILL", "LOADS");
        let mut skills: Vec<_> = skill_totals.into_iter().collect();
        skills.sort_by(|a, b| a.0.0.cmp(&b.0.0).then(b.1.cmp(&a.1)));
        for ((agent, skill), loads) in skills {
            println!("{agent:<10} {skill:<40} {loads:>8}");
        }
    }
}

fn format_hour_bucket(hour_bucket: i64) -> String {
    let secs = hour_bucket * 3600;
    let days = secs / 86400;
    let hour = (secs % 86400) / 3600;
    let (y, m, d) = civil_from_days(days);
    format!("{y:04}-{m:02}-{d:02} {hour:02}:00")
}

fn format_ms_to_date(ms: i64) -> String {
    // ponytail: minimal epoch-ms to date+time string; no chrono dep
    let secs = ms / 1000;
    let days = secs / 86400;
    let remainder = secs % 86400;
    let hour = remainder / 3600;
    let minute = (remainder % 3600) / 60;
    let (y, m, d) = civil_from_days(days);
    format!("{y:04}-{m:02}-{d:02} {hour:02}:{minute:02}")
}

fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    let since = match &cli.command {
        Some(Commands::Snapshot {
            start_date, period, ..
        })
        | Some(Commands::Stats {
            start_date, period, ..
        })
        | Some(Commands::Watch {
            start_date, period, ..
        }) => resolve_since(start_date, period),
        None => None,
    };

    match cli.command {
        Some(Commands::Stats { json, agent, .. }) => {
            let mut stats = agent_tokens::stats_snapshot(since);
            if let Some(a) = agent {
                stats.retain(|s| s.agent.as_kebab() == a);
            }
            stats.sort_by(|a, b| {
                (a.agent.as_kebab(), b.last_seen_at_ms)
                    .cmp(&(b.agent.as_kebab(), a.last_seen_at_ms))
            });
            if json {
                println!("{}", serde_json::to_string_pretty(&stats).unwrap());
            } else {
                format_stats_table(&stats);
            }
        }
        Some(Commands::Snapshot { json, period, .. }) => {
            let sessions = agent_tokens::snapshot(since);
            let hourly = period.is_some();
            if json {
                println!("{}", serde_json::to_string_pretty(&sessions).unwrap());
            } else {
                format_table(&sessions, hourly);
            }
        }
        Some(Commands::Watch {
            interval,
            json,
            period,
            ..
        }) => {
            use futures_util::StreamExt;
            let stream = agent_tokens::watch(std::time::Duration::from_secs(interval), since);
            let hourly = period.is_some();
            tokio::pin!(stream);
            loop {
                tokio::select! {
                    Some(sessions) = stream.next() => {
                        if json {
                            println!("{}", serde_json::to_string_pretty(&sessions).unwrap());
                        } else {
                            print!("\x1b[2J\x1b[H");
                            format_table(&sessions, hourly);
                        }
                    }
                    _ = tokio::signal::ctrl_c() => {
                        break;
                    }
                }
            }
        }
        None => {
            let sessions = agent_tokens::snapshot(since);
            format_table(&sessions, false);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_period_hours() {
        let ms = parse_period("1h").unwrap();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64;
        assert!(ms > now - 60 * 60 * 1000 - 1000);
        assert!(ms < now - 60 * 60 * 1000 + 1000);
    }

    #[test]
    fn parse_period_minutes() {
        let ms = parse_period("30m").unwrap();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64;
        let expected = now - 30 * 60 * 1000;
        assert!((ms - expected).abs() < 1000);
    }

    #[test]
    fn parse_period_days() {
        let ms = parse_period("2d").unwrap();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64;
        let expected = now - 2 * 24 * 60 * 60 * 1000;
        assert!((ms - expected).abs() < 1000);
    }

    #[test]
    fn parse_period_seconds() {
        let ms = parse_period("15s").unwrap();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64;
        let expected = now - 15 * 1000;
        assert!((ms - expected).abs() < 1000);
    }

    #[test]
    fn parse_period_invalid() {
        assert!(parse_period("").is_none());
        assert!(parse_period("abc").is_none());
        assert!(parse_period("1x").is_none());
    }

    #[test]
    fn resolve_since_period_takes_precedence() {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64;
        let from_date = parse_start_date("2026-01-01").unwrap();
        let from_period = resolve_since(&Some("2026-01-01".into()), &Some("1h".into())).unwrap();
        // Period should win over start_date
        assert!((from_period - (now - 60 * 60 * 1000)).abs() < 1000);
        assert_ne!(from_period, from_date);
    }

    #[test]
    fn resolve_since_start_date_only() {
        let ms = resolve_since(&Some("2026-06-01".into()), &None).unwrap();
        let expected = parse_start_date("2026-06-01").unwrap();
        assert_eq!(ms, expected);
    }

    #[test]
    fn resolve_since_none() {
        assert!(resolve_since(&None, &None).is_none());
    }
}
