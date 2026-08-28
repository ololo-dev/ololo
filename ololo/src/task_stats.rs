//! Per-task agent statistics reporting.
//!
//! When the TUI detects a task is done (the scheduler advanced past its
//! ordinal, or the session completed), it sends a `CompletedTask` to the
//! reporter task spawned here. The reporter collects agent activity from
//! local agent logs (via the `agent-tokens` crate) for the task's time
//! window and POSTs a `TaskStatsReport` to the main server, where it is
//! stored and shown on the player's session page. Best-effort: failures
//! are logged and never block gameplay.

use arena_core::protocol::{AgentSessionStats, TaskStatsReport};
use std::collections::BTreeMap;
use std::path::PathBuf;
use uuid::Uuid;

/// A task the TUI considers done, ready for stats collection.
#[derive(Debug, Clone)]
pub struct CompletedTask {
    pub task_id: Uuid,
    pub ordinal: i32,
    pub player_id: Uuid,
    /// First probe for this task arrived (epoch ms) — window start.
    pub window_start_ms: i64,
    /// Completion detected (epoch ms) — window end.
    pub window_end_ms: i64,
}

/// Spawn the reporter task. Returns the sender the TUI app uses and the
/// join handle (awaited with a timeout on shutdown to drain the queue).
pub fn spawn_reporter(
    server_url: String,
    pat: String,
    session_code: String,
) -> (
    tokio::sync::mpsc::UnboundedSender<CompletedTask>,
    tokio::task::JoinHandle<()>,
) {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<CompletedTask>();
    let handle = tokio::spawn(async move {
        let client = reqwest::Client::new();
        let http_base = crate::util::http_base_url(&server_url);
        let cwd = std::env::current_dir().ok();
        while let Some(first) = rx.recv().await {
            // Drain everything already queued into one batch. The scan
            // dominates the cost — agent-tokens walks EVERY local agent
            // store, tens of seconds on a machine with a season of logs —
            // and a session finish drops all remaining tasks on the queue
            // at once. One scan serves the whole batch; per-task windows
            // are cut from the same raw data. Scanning per task made the
            // queue outrun the scanner and the shutdown drain dropped
            // every report of every session.
            let mut batch = vec![first];
            while let Ok(more) = rx.try_recv() {
                batch.push(more);
            }
            let min_start = batch
                .iter()
                .map(|t| t.window_start_ms)
                .min()
                .unwrap_or_default();
            // agent-tokens scans local files/DBs — keep it off the runtime.
            let raw = tokio::task::spawn_blocking(move || {
                (
                    agent_tokens::snapshot(Some(min_start)),
                    agent_tokens::stats_snapshot(Some(min_start)),
                )
            })
            .await
            .unwrap_or_default();

            for task in batch {
                let agents = merge_agent_stats(
                    raw.0.clone(),
                    raw.1.clone(),
                    task.window_start_ms,
                    task.window_end_ms,
                    cwd.clone(),
                );
                let report = TaskStatsReport {
                    task_id: Some(task.task_id),
                    task_ordinal: task.ordinal,
                    window_started_at_ms: Some(task.window_start_ms),
                    window_ended_at_ms: Some(task.window_end_ms),
                    agents,
                };
                let url = format!(
                    "{http_base}/api/sessions/{session_code}/players/{}/task-stats",
                    task.player_id
                );
                match client
                    .post(&url)
                    .bearer_auth(&pat)
                    .json(&report)
                    .send()
                    .await
                {
                    Ok(resp) if resp.status().is_success() => {
                        tracing::info!(
                            "task stats reported: ordinal={} agents={}",
                            task.ordinal,
                            report.agents.len()
                        );
                    }
                    Ok(resp) => {
                        tracing::warn!(
                            "task stats report rejected: ordinal={} status={}",
                            task.ordinal,
                            resp.status()
                        );
                    }
                    Err(e) => {
                        tracing::warn!("task stats report failed: ordinal={}: {e}", task.ordinal);
                    }
                }
            }
        }
    });
    (tx, handle)
}

/// Pure merge step, separated for tests.
pub fn merge_agent_stats(
    counts: Vec<agent_tokens::SessionCounts>,
    stats: Vec<agent_tokens::SessionStats>,
    window_start_ms: i64,
    window_end_ms: i64,
    project_cwd: Option<PathBuf>,
) -> Vec<AgentSessionStats> {
    let cwd_str = project_cwd.map(|p| p.to_string_lossy().into_owned());
    // Sessions whose logs record a cwd must match the project dir; sessions
    // without one (some agents don't persist it) pass through.
    let cwd_ok = |cwd: &Option<String>| match (&cwd_str, cwd) {
        (Some(want), Some(got)) => want == got,
        _ => true,
    };
    // Window overlap: activity at/after start (guaranteed by the `since`
    // filter) that began before the window closed.
    let in_window = |started: Option<i64>, last_seen: Option<i64>| {
        let effective = started.or(last_seen);
        effective.is_none_or(|t| t <= window_end_ms)
            && last_seen.is_none_or(|t| t >= window_start_ms)
    };

    let mut merged: BTreeMap<(String, String), AgentSessionStats> = BTreeMap::new();

    for c in counts {
        if !cwd_ok(&c.cwd) || !in_window(c.started_at_ms, c.last_seen_at_ms) {
            continue;
        }
        let key = (c.agent.as_kebab().to_string(), c.session_id.clone());
        let entry = merged
            .entry(key.clone())
            .or_insert_with(|| empty_entry(&key));
        entry.model = c.model.as_deref().map(clean_model);
        entry.input_tokens += c.counts.input;
        entry.output_tokens += c.counts.output;
        entry.cache_read_tokens += c.counts.cache_read;
        entry.cache_write_tokens += c.counts.cache_write;
        entry.reasoning_tokens += c.counts.reasoning;
        if let Some(cost) = c.cost {
            entry.cost = Some(entry.cost.unwrap_or(0.0) + cost);
        }
    }

    for s in stats {
        if !cwd_ok(&s.cwd) || !in_window(s.started_at_ms, s.last_seen_at_ms) {
            continue;
        }
        let key = (s.agent.as_kebab().to_string(), s.session_id.clone());
        let entry = merged
            .entry(key.clone())
            .or_insert_with(|| empty_entry(&key));
        entry.user_messages += s.user_messages;
        entry.assistant_messages += s.assistant_messages;
        entry.tool_calls += s.tool_calls;
        for (tool, n) in s.tools {
            *entry.tools.entry(tool).or_insert(0) += n;
        }
        for (skill, n) in s.skills {
            *entry.skills.entry(skill).or_insert(0) += n;
        }
    }

    // Drop sessions with no signal at all (e.g. an idle agent store).
    merged
        .into_values()
        .filter(|e| {
            e.input_tokens
                + e.output_tokens
                + e.cache_read_tokens
                + e.cache_write_tokens
                + e.user_messages
                + e.assistant_messages
                + e.tool_calls
                > 0
        })
        .collect()
}

fn empty_entry(key: &(String, String)) -> AgentSessionStats {
    AgentSessionStats {
        agent: key.0.clone(),
        agent_session_id: key.1.clone(),
        model: None,
        input_tokens: 0,
        output_tokens: 0,
        cache_read_tokens: 0,
        cache_write_tokens: 0,
        reasoning_tokens: 0,
        cost: None,
        user_messages: 0,
        assistant_messages: 0,
        tool_calls: 0,
        tools: BTreeMap::new(),
        skills: BTreeMap::new(),
    }
}

/// OpenCode/pi store the model as JSON `{"id":..,"providerID":..}`; flatten
/// to `provider/id` for display. Plain strings pass through.
fn clean_model(model: &str) -> String {
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(model)
        && let Some(id) = v.get("id").and_then(|i| i.as_str())
    {
        return match v.get("providerID").and_then(|p| p.as_str()) {
            Some(provider) => format!("{provider}/{id}"),
            None => id.to_string(),
        };
    }
    model.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_tokens::{AgentId, SessionCounts, SessionStats, TokenCounts};

    fn counts(agent_session: &str, cwd: Option<&str>, input: u64) -> SessionCounts {
        SessionCounts {
            agent: AgentId::Claude,
            session_id: agent_session.to_string(),
            model: Some("claude-fable-5".into()),
            cwd: cwd.map(String::from),
            started_at_ms: Some(1_000),
            last_seen_at_ms: Some(2_000),
            counts: TokenCounts {
                input,
                output: 10,
                ..Default::default()
            },
            cost: None,
            source_file: None,
        }
    }

    fn stats(agent_session: &str, tool: &str) -> SessionStats {
        let mut s = SessionStats::new(AgentId::Claude, agent_session.to_string());
        s.started_at_ms = Some(1_000);
        s.last_seen_at_ms = Some(2_000);
        s.user_messages = 1;
        s.assistant_messages = 2;
        s.record_tool(tool, &serde_json::Value::Null);
        s
    }

    #[test]
    fn merges_counts_and_stats_by_session() {
        let out = merge_agent_stats(
            vec![counts("s1", Some("/proj"), 100)],
            vec![stats("s1", "Bash")],
            0,
            10_000,
            Some(PathBuf::from("/proj")),
        );
        assert_eq!(out.len(), 1);
        let e = &out[0];
        assert_eq!(e.agent, "claude");
        assert_eq!(e.input_tokens, 100);
        assert_eq!(e.user_messages, 1);
        assert_eq!(e.tools.get("Bash"), Some(&1));
        assert_eq!(e.model.as_deref(), Some("claude-fable-5"));
    }

    #[test]
    fn filters_by_cwd_when_recorded() {
        let out = merge_agent_stats(
            vec![counts("s1", Some("/other"), 100)],
            vec![],
            0,
            10_000,
            Some(PathBuf::from("/proj")),
        );
        assert!(out.is_empty());
        // Sessions without a recorded cwd pass through.
        let out = merge_agent_stats(
            vec![counts("s2", None, 100)],
            vec![],
            0,
            10_000,
            Some(PathBuf::from("/proj")),
        );
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn drops_sessions_outside_window() {
        let mut c = counts("s1", None, 100);
        c.started_at_ms = Some(50_000); // started after the window closed
        c.last_seen_at_ms = Some(60_000);
        let out = merge_agent_stats(vec![c], vec![], 0, 10_000, None);
        assert!(out.is_empty());
    }

    #[test]
    fn drops_zero_signal_sessions() {
        let mut c = counts("s1", None, 0);
        c.counts.output = 0;
        let out = merge_agent_stats(vec![c], vec![], 0, 10_000, None);
        assert!(out.is_empty());
    }

    #[test]
    fn clean_model_flattens_json() {
        assert_eq!(
            clean_model(r#"{"id":"glm-5.2","providerID":"ollama-cloud"}"#),
            "ollama-cloud/glm-5.2"
        );
        assert_eq!(clean_model("claude-fable-5"), "claude-fable-5");
    }
}
