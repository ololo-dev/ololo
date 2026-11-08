//! Task implementation statistics — client-reported by the ololo CLI when a
//! task completes, persisted in `task_agent_stats`, and served to the player
//! page. Trusted client data (honest-trust model); never used for scoring.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use uuid::Uuid;

/// Statistics for one AI-agent session that was active during a task window.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentSessionStats {
    /// Agent identifier in kebab case (e.g. "claude", "opencode").
    pub agent: String,
    pub agent_session_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
    pub reasoning_tokens: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost: Option<f64>,
    pub user_messages: u64,
    pub assistant_messages: u64,
    pub tool_calls: u64,
    /// tool name -> call count
    #[serde(default)]
    pub tools: BTreeMap<String, u64>,
    /// skill name -> load count
    #[serde(default)]
    pub skills: BTreeMap<String, u64>,
}

/// CLI -> server: statistics captured for one completed task.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskStatsReport {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<Uuid>,
    pub task_ordinal: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window_started_at_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window_ended_at_ms: Option<i64>,
    pub agents: Vec<AgentSessionStats>,
}

/// Server -> frontend: one stored per-task stats row.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskStatsEntry {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<Uuid>,
    pub task_ordinal: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window_started_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window_ended_at: Option<String>,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
    pub reasoning_tokens: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost: Option<f64>,
    pub user_messages: u64,
    pub assistant_messages: u64,
    pub tool_calls: u64,
    pub agents: Vec<AgentSessionStats>,
    pub created_at: String,
    pub updated_at: String,
}

/// Server -> frontend: GET response for a player's task statistics.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskStatsResponse {
    pub entries: Vec<TaskStatsEntry>,
}

/// Bounds enforced by the server on incoming reports.
pub const TASK_STATS_MAX_AGENTS: usize = 32;
pub const TASK_STATS_MAX_TOOLS: usize = 128;
pub const TASK_STATS_MAX_SKILLS: usize = 128;
pub const TASK_STATS_MAX_NAME_LEN: usize = 128;

impl TaskStatsReport {
    /// Validate size bounds so a client cannot store unbounded JSON.
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.agents.len() > TASK_STATS_MAX_AGENTS {
            return Err("too many agent sessions");
        }
        for a in &self.agents {
            if a.agent.len() > TASK_STATS_MAX_NAME_LEN
                || a.agent_session_id.len() > TASK_STATS_MAX_NAME_LEN
                || a.model.as_deref().map_or(0, str::len) > TASK_STATS_MAX_NAME_LEN
            {
                return Err("agent field exceeds max length");
            }
            if a.tools.len() > TASK_STATS_MAX_TOOLS {
                return Err("too many tools");
            }
            if a.skills.len() > TASK_STATS_MAX_SKILLS {
                return Err("too many skills");
            }
            if a.tools
                .keys()
                .chain(a.skills.keys())
                .any(|k| k.len() > TASK_STATS_MAX_NAME_LEN)
            {
                return Err("tool/skill name exceeds max length");
            }
        }
        Ok(())
    }
}

/// CLI -> server: what the player's coding agents were configured with on
/// their machine — skills and MCP servers, names only — read by the CLI at
/// the session's start, after each finished task and on exit. The repository shows only what the player
/// committed; user-level and plugin configuration never reaches git. Never
/// commands, arguments, URLs or environment: MCP configs carry secrets.
/// Client-reported (honest-trust), never used for scoring directly — the
/// Agentic judge reads it as evidence.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentSetupReport {
    pub agents: Vec<AgentSetupConfig>,
}

/// One coding agent's configuration found on the machine.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentSetupConfig {
    /// Agent identifier in kebab case (`claude-code`, `codex`, `gemini`,
    /// `cursor`, `agents` for the shared `~/.agents` convention).
    pub agent: String,
    #[serde(default)]
    pub skills: Vec<ConfiguredItem>,
    #[serde(default)]
    pub mcp_servers: Vec<ConfiguredItem>,
}

/// A skill or MCP server, where it is configured.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConfiguredItem {
    pub name: String,
    /// `user` (the player's home config), `project` (a file in the working
    /// directory, usually committed), `local` (the agent's per-project
    /// setting kept outside the repository) or `plugin`.
    pub scope: String,
    /// The plugin that provides it, for `scope: plugin`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plugin: Option<String>,
}

pub const AGENT_SETUP_MAX_AGENTS: usize = 16;
pub const AGENT_SETUP_MAX_ITEMS: usize = 256;

impl AgentSetupReport {
    /// Reject reports past the size limits (names as for task stats).
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.agents.len() > AGENT_SETUP_MAX_AGENTS {
            return Err("too many agents");
        }
        for a in &self.agents {
            if a.agent.len() > TASK_STATS_MAX_NAME_LEN {
                return Err("agent name exceeds max length");
            }
            if a.skills.len() > AGENT_SETUP_MAX_ITEMS || a.mcp_servers.len() > AGENT_SETUP_MAX_ITEMS
            {
                return Err("too many skills or MCP servers");
            }
            let too_long = |i: &ConfiguredItem| {
                i.name.len() > TASK_STATS_MAX_NAME_LEN
                    || i.scope.len() > 16
                    || i.plugin.as_deref().map_or(0, str::len) > TASK_STATS_MAX_NAME_LEN
            };
            if a.skills.iter().chain(&a.mcp_servers).any(too_long) {
                return Err("skill or MCP server field exceeds max length");
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn agent(n_tools: usize) -> AgentSessionStats {
        AgentSessionStats {
            agent: "claude".into(),
            agent_session_id: "s1".into(),
            model: Some("claude-fable-5".into()),
            input_tokens: 1,
            output_tokens: 2,
            cache_read_tokens: 3,
            cache_write_tokens: 4,
            reasoning_tokens: 0,
            cost: None,
            user_messages: 1,
            assistant_messages: 2,
            tool_calls: 3,
            tools: (0..n_tools).map(|i| (format!("t{i}"), 1)).collect(),
            skills: BTreeMap::new(),
        }
    }

    #[test]
    fn report_roundtrips() {
        let report = TaskStatsReport {
            task_id: None,
            task_ordinal: 3,
            window_started_at_ms: Some(1_000),
            window_ended_at_ms: Some(2_000),
            agents: vec![agent(2)],
        };
        let json = serde_json::to_string(&report).unwrap();
        let back: TaskStatsReport = serde_json::from_str(&json).unwrap();
        assert_eq!(back.task_ordinal, 3);
        assert_eq!(back.agents.len(), 1);
        assert_eq!(back.agents[0].tools.len(), 2);
    }

    #[test]
    fn validate_rejects_oversized() {
        let mut report = TaskStatsReport {
            task_id: None,
            task_ordinal: 0,
            window_started_at_ms: None,
            window_ended_at_ms: None,
            agents: vec![agent(TASK_STATS_MAX_TOOLS + 1)],
        };
        assert!(report.validate().is_err());
        report.agents = vec![agent(1)];
        assert!(report.validate().is_ok());
    }

    #[test]
    fn unknown_fields_rejected() {
        let json = r#"{"task_ordinal":1,"agents":[],"bogus":true}"#;
        assert!(serde_json::from_str::<TaskStatsReport>(json).is_err());
    }

    #[test]
    fn agent_setup_reports_round_trip_and_are_bounded() {
        let json = r#"{"agents":[{"agent":"claude-code","skills":[{"name":"capture","scope":"user"}],
            "mcp_servers":[{"name":"playwright","scope":"plugin","plugin":"pw@market"}]}]}"#;
        let report: AgentSetupReport = serde_json::from_str(json).unwrap();
        assert!(report.validate().is_ok());
        assert_eq!(
            report.agents[0].mcp_servers[0].plugin.as_deref(),
            Some("pw@market")
        );
        // Nothing but names: a command or env field is refused outright.
        let leaky = r#"{"agents":[{"agent":"x","mcp_servers":[{"name":"a","scope":"user","command":"npx"}]}]}"#;
        assert!(serde_json::from_str::<AgentSetupReport>(leaky).is_err());
        let mut big = report.clone();
        big.agents[0].skills = vec![
            ConfiguredItem {
                name: "s".into(),
                scope: "user".into(),
                plugin: None
            };
            AGENT_SETUP_MAX_ITEMS + 1
        ];
        assert!(big.validate().is_err());
    }
}
