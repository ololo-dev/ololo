use serde::Serialize;
use std::collections::BTreeMap;
use std::path::PathBuf;

// SessionTokens and TokenCounts are local data types, not wire protocol structs.
// When a server-report path is wired, wrap them in a wire-specific struct with
// #[serde(deny_unknown_fields)].

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub enum AgentId {
    #[serde(rename = "claude")]
    Claude,
    #[serde(rename = "opencode")]
    OpenCode,
    #[serde(rename = "codex")]
    Codex,
    /// Cursor IDE (global state DB + tokscale cursor-cache CSVs).
    #[serde(rename = "cursor")]
    Cursor,
    /// Cursor CLI agent (`~/.cursor/projects/**/agent-transcripts`).
    #[serde(rename = "cursor-cli")]
    CursorCli,
    #[serde(rename = "pi")]
    Pi,
    #[serde(rename = "omp")]
    Omp,
    #[serde(rename = "kiro")]
    Kiro,
    #[serde(rename = "copilot")]
    Copilot,
    #[serde(rename = "gemini")]
    Gemini,
    #[serde(rename = "qwen")]
    Qwen,
    #[serde(rename = "kimi")]
    Kimi,
    #[serde(rename = "goose")]
    Goose,
    #[serde(rename = "amp")]
    Amp,
    /// Zed's built-in agent (`threads/threads.db`, hosted zed.dev models only).
    #[serde(rename = "zed")]
    Zed,
    /// Antigravity IDE (usage synced by `tokscale antigravity sync`).
    #[serde(rename = "antigravity")]
    Antigravity,
    /// Antigravity CLI (`~/.gemini/antigravity-cli/conversations/*.db`).
    #[serde(rename = "antigravity-cli")]
    AntigravityCli,
}

impl AgentId {
    /// Display name matching the agent's binary name (e.g. "opencode" not "open-code").
    pub fn as_kebab(&self) -> &'static str {
        match self {
            AgentId::Claude => "claude",
            AgentId::OpenCode => "opencode",
            AgentId::Codex => "codex",
            AgentId::Cursor => "cursor",
            AgentId::CursorCli => "cursor-cli",
            AgentId::Pi => "pi",
            AgentId::Omp => "omp",
            AgentId::Kiro => "kiro",
            AgentId::Copilot => "copilot",
            AgentId::Gemini => "gemini",
            AgentId::Qwen => "qwen",
            AgentId::Kimi => "kimi",
            AgentId::Goose => "goose",
            AgentId::Amp => "amp",
            AgentId::Zed => "zed",
            AgentId::Antigravity => "antigravity",
            AgentId::AntigravityCli => "antigravity-cli",
        }
    }
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct TokenCounts {
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
    pub reasoning: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct SessionCounts {
    pub agent: AgentId,
    pub session_id: String,
    pub model: Option<String>,
    pub cwd: Option<String>,
    pub started_at_ms: Option<i64>,
    pub last_seen_at_ms: Option<i64>,
    pub counts: TokenCounts,
    pub cost: Option<f64>,
    pub source_file: Option<PathBuf>,
}

/// Behavioural statistics for one agent session: message counts, which tools
/// were called how often, and which skills were loaded.
#[derive(Debug, Clone, Serialize)]
pub struct SessionStats {
    pub agent: AgentId,
    pub session_id: String,
    pub cwd: Option<String>,
    pub started_at_ms: Option<i64>,
    pub last_seen_at_ms: Option<i64>,
    pub user_messages: u64,
    pub assistant_messages: u64,
    pub tool_calls: u64,
    /// tool name -> call count
    pub tools: BTreeMap<String, u64>,
    /// skill name -> load count (Skill tool invocations or SKILL.md reads)
    pub skills: BTreeMap<String, u64>,
    pub source_file: Option<PathBuf>,
}

impl SessionStats {
    pub fn new(agent: AgentId, session_id: String) -> Self {
        Self {
            agent,
            session_id,
            cwd: None,
            started_at_ms: None,
            last_seen_at_ms: None,
            user_messages: 0,
            assistant_messages: 0,
            tool_calls: 0,
            tools: BTreeMap::new(),
            skills: BTreeMap::new(),
            source_file: None,
        }
    }

    /// Record one tool invocation; detects skill loads from the tool name/args.
    pub fn record_tool(&mut self, name: &str, args: &serde_json::Value) {
        self.tool_calls += 1;
        *self.tools.entry(name.to_string()).or_insert(0) += 1;
        if let Some(skill) = crate::skills::skill_from_tool_call(name, args) {
            *self.skills.entry(skill).or_insert(0) += 1;
        }
    }

    pub fn touch_ts(&mut self, ts: Option<i64>) {
        if let Some(t) = ts {
            if self.started_at_ms.is_none_or(|s| t < s) {
                self.started_at_ms = Some(t);
            }
            if self.last_seen_at_ms.is_none_or(|l| t > l) {
                self.last_seen_at_ms = Some(t);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::AI_AGENT_NAMES;

    #[test]
    fn agent_id_as_kebab() {
        assert_eq!(AgentId::Claude.as_kebab(), "claude");
        assert_eq!(AgentId::OpenCode.as_kebab(), "opencode");
        assert_eq!(AgentId::Copilot.as_kebab(), "copilot");
    }

    #[test]
    fn agent_id_serde_kebab() {
        let json = serde_json::to_string(&AgentId::OpenCode).unwrap();
        assert_eq!(json, "\"opencode\"");
    }

    #[test]
    fn ai_agent_names_preserves_binary_names() {
        assert!(AI_AGENT_NAMES.contains(&"opencode"));
        assert!(AI_AGENT_NAMES.contains(&"aider"));
        assert!(!AI_AGENT_NAMES.contains(&"open-code"));
    }

    #[test]
    fn ai_agent_names_contains_kiro_cli() {
        assert!(AI_AGENT_NAMES.contains(&"kiro-cli"));
    }

    #[test]
    fn cursor_and_antigravity_clis_are_detectable() {
        // Regression: v0.4.0 added token extraction for these agents but not
        // their binaries, so `ololo` never scanned for them and they never
        // appeared in the start-of-session picker.
        for binary in [
            "cursor-agent",
            "cursor-cli",
            "agy",
            "antigravity",
            "zed",
            "droid",
        ] {
            assert!(
                AI_AGENT_NAMES.contains(&binary),
                "{binary} must be scanned on $PATH or the picker cannot offer it"
            );
        }
    }

    #[test]
    fn every_token_store_agent_has_a_binary_to_launch() {
        // An agent whose tokens we know how to read is one a participant may
        // well want to play with — so each must have at least one candidate
        // binary on the scan list. Guards the exact drift this list suffered:
        // `AgentId` grew, `AI_AGENT_NAMES` did not.
        let expected: &[(AgentId, &[&str])] = &[
            (AgentId::Claude, &["claude"]),
            (AgentId::OpenCode, &["opencode"]),
            (AgentId::Codex, &["codex"]),
            (AgentId::Cursor, &["cursor"]),
            (AgentId::CursorCli, &["cursor-agent", "cursor-cli"]),
            (AgentId::Pi, &["pi"]),
            (AgentId::Omp, &["omp"]),
            (AgentId::Kiro, &["kiro-cli"]),
            (AgentId::Copilot, &["copilot"]),
            (AgentId::Gemini, &["gemini"]),
            (AgentId::Qwen, &["qwen"]),
            (AgentId::Kimi, &["kimi"]),
            (AgentId::Goose, &["goose"]),
            (AgentId::Amp, &["amp"]),
            (AgentId::Zed, &["zed"]),
            (AgentId::Antigravity, &["antigravity"]),
            (AgentId::AntigravityCli, &["agy"]),
        ];
        for (agent, binaries) in expected {
            assert!(
                binaries.iter().any(|b| AI_AGENT_NAMES.contains(b)),
                "{} has a token extractor but none of {:?} is on the scan list",
                agent.as_kebab(),
                binaries
            );
        }
    }

    #[test]
    fn session_counts_serializes() {
        let s = SessionCounts {
            agent: AgentId::Claude,
            session_id: "s1".into(),
            model: Some("gpt-4".into()),
            cwd: Some("/tmp".into()),
            started_at_ms: Some(1000),
            last_seen_at_ms: Some(2000),
            counts: TokenCounts {
                input: 10,
                output: 20,
                ..Default::default()
            },
            cost: None,
            source_file: None,
        };
        let json = serde_json::to_string(&s).unwrap();
        assert!(json.contains("\"claude\""));
        assert!(json.contains("\"input\":10"));
    }
}
