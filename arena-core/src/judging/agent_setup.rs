//! Deterministic evidence about how the player set up their coding agent.
//!
//! The session dossier answers "what did this player build". This answers the
//! orthogonal question a workflow judge is asked: *how did they engineer the
//! agent that built it* — the instructions committed for it to read, the
//! skills and subagents defined, the MCP servers wired up, the knowledge base
//! written down — and, crucially, which of that was **actually used**.
//!
//! Both halves matter and only one of them lives in git. A repository can
//! carry a beautiful `.claude/skills/` tree that no run ever loaded; the
//! client-reported agent statistics (`task_agent_stats.agents_json`, one entry
//! per agent session with `tools` and `skills` histograms) say which names were
//! invoked and how often. Cross-referencing the two turns "they have skills"
//! into "they defined four and ran two of them eleven times", which is the
//! difference between decoration and workflow.
//!
//! Everything here is fact. Presence, counts, `path:line` grep hits — never a
//! conclusion. The statistics half is client-reported under the honest-trust
//! model, so it is labelled as such in the prompt and absence of it is weak
//! evidence of nothing.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::entities::task_agent_stats;
use crate::protocol::AgentSessionStats;

use super::tools::{self, GrepHit, ToolScope};

/// Instruction files an agent reads on entering a repository. Matched on the
/// basename for the markdown ones (a monorepo puts one per package) and on the
/// exact path for the tool-specific dotfiles.
const INSTRUCTION_BASENAMES: [&str; 8] = [
    "AGENTS.md",
    "AGENT.md",
    "CLAUDE.md",
    "CLAUDE.local.md",
    "GEMINI.md",
    "QWEN.md",
    "CONVENTIONS.md",
    "COPILOT.md",
];

/// Exact instruction paths (tool dotfiles) and prefixes for rule directories.
const INSTRUCTION_PATHS: [&str; 6] = [
    ".cursorrules",
    ".windsurfrules",
    ".clinerules",
    ".goosehints",
    ".github/copilot-instructions.md",
    ".junie/guidelines.md",
];
const INSTRUCTION_PREFIXES: [&str; 3] = [".cursor/rules/", ".agents/rules/", ".claude/rules/"];

/// Where skills, subagents and slash commands live across the agents in use.
const SKILL_PREFIXES: [&str; 4] = [
    ".claude/skills/",
    ".agents/skills/",
    "skills/",
    ".opencode/skill/",
];
const SUBAGENT_PREFIXES: [&str; 4] = [
    ".claude/agents/",
    ".agents/agents/",
    ".opencode/agent/",
    ".gemini/agents/",
];
const COMMAND_PREFIXES: [&str; 4] = [
    ".claude/commands/",
    ".agents/commands/",
    ".opencode/command/",
    ".gemini/commands/",
];

/// Automation that runs without being asked.
const HOOK_PREFIXES: [&str; 4] = [".claude/hooks/", ".githooks/", ".husky/", ".agents/hooks/"];
const HOOK_FILES: [&str; 4] = [
    ".pre-commit-config.yaml",
    "lefthook.yml",
    "lefthook.yaml",
    ".git-hooks.toml",
];

/// Config files that can declare MCP servers.
const MCP_FILES: [&str; 8] = [
    ".mcp.json",
    ".cursor/mcp.json",
    ".vscode/mcp.json",
    ".claude/settings.json",
    ".claude/settings.local.json",
    ".gemini/settings.json",
    "opencode.json",
    ".windsurf/mcp_config.json",
];

/// Directories that read as a written-down knowledge base rather than source.
const KNOWLEDGE_PREFIXES: [&str; 8] = [
    "docs/",
    "doc/",
    "wiki/",
    "knowledge/",
    "kb/",
    "specs/",
    ".agents/memory/",
    ".claude/memory/",
];

/// Tool names that mean "this agent delegated to another agent".
const SUBAGENT_TOOL_NAMES: [&str; 6] = [
    "task",
    "agent",
    "subagent",
    "dispatch_agent",
    "spawn_agent",
    "delegate",
];

/// Grep probes for the practices a workflow judge is asked about. Each is an
/// extended regex, matched case-insensitively over the final tree.
const METHOD_PROBES: [(&str, &str); 5] = [
    (
        "react_loop",
        r"ReAct|reason.{0,3}\+.{0,3}act|thought *(->|→|then) *action",
    ),
    (
        "spec_driven",
        r"spec-driven|specification-first|\bSDD\b|acceptance criteri|given/when/then|constitution\.md",
    ),
    (
        "multi_agent",
        r"sub-?agent|multi-?agent|agent (team|swarm|fleet)|orchestrat(e|or|ion)|parallel agents",
    ),
    (
        "retrieval",
        r"\bRAG\b|retrieval.augmented|embedding|vector (store|db|database|search)|chromadb|qdrant|pgvector|faiss|lancedb|llamaindex",
    ),
    (
        "mcp",
        r"mcpServers|modelcontextprotocol|model context protocol|\bMCP\b",
    ),
];

/// Caps, so one pathological repository cannot fill the judge's context.
const MAX_INSTRUCTIONS: usize = 6;
const MAX_LISTED: usize = 20;
const MAX_KNOWLEDGE: usize = 25;
const MAX_TOOLS_LISTED: usize = 25;
const EXCERPT_CHARS: usize = 1_500;
const DESCRIPTION_CHARS: usize = 200;

/// A file in the final snapshot, with whether the session put it there.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoFileFact {
    pub path: String,
    pub size_bytes: u64,
    /// False when the file was already in the root (session-start) commit —
    /// the player inherited it rather than writing it.
    pub added_in_session: bool,
    /// Leading content, for the files whose substance is the evidence.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub excerpt: Option<String>,
}

/// A skill definition found in the repository, with how often it ran.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillFact {
    pub name: String,
    pub path: String,
    pub added_in_session: bool,
    /// Its `description:` frontmatter line, when it has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Loads recorded for this skill across the session's agent sessions.
    pub loads: u64,
}

/// An MCP config file and the servers it declares.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpFact {
    pub path: String,
    pub added_in_session: bool,
    pub servers: Vec<String>,
}

/// Grep hits for one practice probe.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MethodSignal {
    pub topic: String,
    pub hits: Vec<GrepHit>,
}

/// One coding agent the player ran, folded across its sessions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentUse {
    pub agent: String,
    pub models: Vec<String>,
    pub sessions: usize,
    pub tool_calls: u64,
    pub assistant_messages: u64,
}

/// Skill loads inside one task's work window — the evidence that lets a
/// judge ask whether a skill was useful *for the task it ran under*, rather
/// than merely present in the session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskSkillUse {
    pub task_ordinal: i32,
    /// skill name → loads within this task's window.
    pub skills: BTreeMap<String, u64>,
}

/// What the player's agents actually did, folded over the whole session.
///
/// Client-reported (honest-trust model): a player whose CLI could not read
/// their agent's logs reports nothing, so an empty block means "not observed",
/// never "did not happen".
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AgenticUsage {
    pub agents: Vec<AgentUse>,
    pub agent_sessions: usize,
    pub tool_calls_total: u64,
    /// skill name → loads, across every agent session.
    pub skills_loaded: BTreeMap<String, u64>,
    /// The same loads, sliced per task window, in ordinal order. Empty when
    /// no window recorded a skill.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub skills_by_task: Vec<TaskSkillUse>,
    /// MCP tool name (`mcp__<server>__<tool>`) → calls.
    pub mcp_tools_called: BTreeMap<String, u64>,
    /// MCP server names derived from the tool names actually called.
    pub mcp_servers_called: Vec<String>,
    /// Calls to a tool that spawns another agent.
    pub subagent_calls: u64,
    /// The most-used tools, name → calls, capped.
    pub top_tools: Vec<(String, u64)>,
    /// True when the session recorded no statistics at all.
    pub reported: bool,
}

/// Everything known about the player's agentic setup and its use.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AgentSetupEvidence {
    pub instructions: Vec<RepoFileFact>,
    pub skills: Vec<SkillFact>,
    pub subagents: Vec<RepoFileFact>,
    pub commands: Vec<RepoFileFact>,
    pub hooks: Vec<RepoFileFact>,
    pub mcp_configs: Vec<McpFact>,
    /// Knowledge-base files, by directory: prefix → paths (capped).
    pub knowledge_base: Vec<String>,
    pub method_signals: Vec<MethodSignal>,
    pub usage: AgenticUsage,
    /// Skill names that ran but have no definition in the repository — a
    /// personal or plugin skill, not something the submission carries.
    pub skills_loaded_without_definition: Vec<String>,
}

impl AgentSetupEvidence {
    /// JSON for the judge prompt.
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_else(|_| "{}".to_string())
    }

    /// True when the snapshot carries no agent configuration whatsoever —
    /// the observation a workflow judge scores lowest, and one worth stating
    /// once rather than making the model derive from six empty lists.
    pub fn is_bare(&self) -> bool {
        self.instructions.is_empty()
            && self.skills.is_empty()
            && self.subagents.is_empty()
            && self.commands.is_empty()
            && self.hooks.is_empty()
            && self.mcp_configs.is_empty()
    }
}

/// Assemble the evidence for one (session, player).
///
/// `root_files` is the root-commit file listing from the session dossier: it
/// is what tells "the player wrote this" from "the project shipped with it".
/// Pass an empty slice when the root commit could not be read — every file
/// then reads as inherited, which is the conservative direction.
pub async fn build_agent_setup_evidence(
    db: &DatabaseConnection,
    repo_dir: &Path,
    session_id: Uuid,
    player_id: Uuid,
    root_files: &[String],
    scope: &ToolScope,
) -> AgentSetupEvidence {
    let root: BTreeSet<&str> = root_files.iter().map(String::as_str).collect();
    let files = tools::list_files(repo_dir, None, None, scope)
        .await
        .unwrap_or_default();

    let mut ev = AgentSetupEvidence::default();
    let mut knowledge: Vec<String> = Vec::new();

    for entry in &files {
        let path = entry.path.as_str();
        let added = !root.contains(path);
        let basename = path.rsplit('/').next().unwrap_or(path);

        if is_instruction(path, basename) {
            if ev.instructions.len() < MAX_INSTRUCTIONS {
                ev.instructions.push(RepoFileFact {
                    path: path.to_string(),
                    size_bytes: entry.size_bytes,
                    added_in_session: added,
                    excerpt: None,
                });
            }
            continue;
        }
        if let Some(name) = skill_name(path) {
            if ev.skills.len() < MAX_LISTED {
                ev.skills.push(SkillFact {
                    name,
                    path: path.to_string(),
                    added_in_session: added,
                    description: None,
                    loads: 0,
                });
            }
            continue;
        }
        if has_prefix(path, &SUBAGENT_PREFIXES) {
            push_capped(&mut ev.subagents, entry, added);
            continue;
        }
        if has_prefix(path, &COMMAND_PREFIXES) {
            push_capped(&mut ev.commands, entry, added);
            continue;
        }
        if has_prefix(path, &HOOK_PREFIXES) || HOOK_FILES.contains(&path) {
            push_capped(&mut ev.hooks, entry, added);
            continue;
        }
        if MCP_FILES.contains(&path) {
            ev.mcp_configs.push(McpFact {
                path: path.to_string(),
                added_in_session: added,
                servers: Vec::new(),
            });
            continue;
        }
        if has_prefix(path, &KNOWLEDGE_PREFIXES) && knowledge.len() < MAX_KNOWLEDGE {
            knowledge.push(path.to_string());
        }
    }
    ev.knowledge_base = knowledge;

    // Read the substance of the few files whose content is the evidence: an
    // AGENTS.md that contradicts the build is worse than none, and the model
    // cannot tell the difference from a file listing.
    for file in ev.instructions.iter_mut() {
        if let Ok(body) = tools::read_file(repo_dir, &file.path, None, None, scope).await {
            file.excerpt = Some(truncate_chars(&body, EXCERPT_CHARS));
        }
    }
    for skill in ev.skills.iter_mut() {
        if let Ok(body) = tools::read_file(repo_dir, &skill.path, None, None, scope).await {
            skill.description = frontmatter_description(&body);
        }
    }
    for cfg in ev.mcp_configs.iter_mut() {
        if let Ok(body) = tools::read_file(repo_dir, &cfg.path, None, None, scope).await {
            cfg.servers = mcp_server_names(&body);
        }
    }
    // A settings file that declares no MCP server is not MCP evidence.
    ev.mcp_configs.retain(|c| !c.servers.is_empty());

    for (topic, pattern) in METHOD_PROBES {
        match tools::grep_repo(repo_dir, pattern, None, scope).await {
            Ok(hits) if !hits.is_empty() => ev.method_signals.push(MethodSignal {
                topic: topic.to_string(),
                hits,
            }),
            _ => {}
        }
    }

    ev.usage = load_usage(db, session_id, player_id).await;
    reconcile_skills(&mut ev);
    ev
}

/// Fold `task_agent_stats.agents_json` over the whole session.
async fn load_usage(db: &DatabaseConnection, session_id: Uuid, player_id: Uuid) -> AgenticUsage {
    let rows = task_agent_stats::Entity::find()
        .filter(task_agent_stats::Column::SessionIdFk.eq(session_id))
        .filter(task_agent_stats::Column::PlayerIdFk.eq(player_id))
        .all(db)
        .await
        .unwrap_or_default();

    let mut usage = AgenticUsage::default();
    // One agent session spans tasks, so the same id is reported once per task
    // window it was active in. Counting rows would multiply it by the ladder.
    let mut seen_sessions: BTreeSet<(String, String)> = BTreeSet::new();
    let mut per_agent: BTreeMap<String, AgentUse> = BTreeMap::new();
    let mut tools_total: BTreeMap<String, u64> = BTreeMap::new();

    let mut by_task: BTreeMap<i32, BTreeMap<String, u64>> = BTreeMap::new();
    for row in &rows {
        usage.reported = true;
        let agents: Vec<AgentSessionStats> =
            serde_json::from_str(&row.agents_json).unwrap_or_default();
        for a in agents {
            let fresh = seen_sessions.insert((a.agent.clone(), a.agent_session_id.clone()));
            let entry = per_agent
                .entry(a.agent.clone())
                .or_insert_with(|| AgentUse {
                    agent: a.agent.clone(),
                    models: Vec::new(),
                    sessions: 0,
                    tool_calls: 0,
                    assistant_messages: 0,
                });
            if fresh {
                entry.sessions += 1;
            }
            if let Some(model) = a.model.as_ref()
                && !entry.models.iter().any(|m| m == model)
            {
                entry.models.push(model.clone());
            }
            entry.tool_calls += a.tool_calls;
            entry.assistant_messages += a.assistant_messages;
            usage.tool_calls_total += a.tool_calls;
            for (name, count) in &a.tools {
                *tools_total.entry(name.clone()).or_insert(0) += count;
            }
            for (name, count) in &a.skills {
                *usage.skills_loaded.entry(name.clone()).or_insert(0) += count;
                *by_task
                    .entry(row.task_ordinal)
                    .or_default()
                    .entry(name.clone())
                    .or_insert(0) += count;
            }
        }
    }
    usage.skills_by_task = by_task
        .into_iter()
        .map(|(task_ordinal, skills)| TaskSkillUse {
            task_ordinal,
            skills,
        })
        .collect();

    let mut servers: BTreeSet<String> = BTreeSet::new();
    for (name, count) in &tools_total {
        let lower = name.to_ascii_lowercase();
        if let Some(server) = mcp_server_of(name) {
            usage.mcp_tools_called.insert(name.clone(), *count);
            servers.insert(server);
        }
        if SUBAGENT_TOOL_NAMES.contains(&lower.as_str()) {
            usage.subagent_calls += count;
        }
    }
    usage.mcp_servers_called = servers.into_iter().collect();
    usage.agent_sessions = seen_sessions.len();
    usage.agents = per_agent.into_values().collect();

    let mut ranked: Vec<(String, u64)> = tools_total.into_iter().collect();
    ranked.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    ranked.truncate(MAX_TOOLS_LISTED);
    usage.top_tools = ranked;
    usage
}

/// Attach load counts to the skills the repository defines, and list the ones
/// that ran without a definition here.
fn reconcile_skills(ev: &mut AgentSetupEvidence) {
    let loaded = ev.usage.skills_loaded.clone();
    let mut matched: BTreeSet<String> = BTreeSet::new();
    for skill in ev.skills.iter_mut() {
        for (name, count) in &loaded {
            if skill_names_match(&skill.name, name) {
                skill.loads += count;
                matched.insert(name.clone());
            }
        }
    }
    ev.skills_loaded_without_definition = loaded
        .keys()
        .filter(|n| !matched.contains(*n))
        .cloned()
        .collect();
}

/// `gangsta:heist` and `heist` name the same skill: agents report a skill by
/// its plugin-qualified name, the repository stores it as a directory.
fn skill_names_match(defined: &str, loaded: &str) -> bool {
    let tail = |s: &str| {
        s.rsplit([':', '/'])
            .next()
            .unwrap_or(s)
            .to_ascii_lowercase()
    };
    tail(defined) == tail(loaded)
}

fn is_instruction(path: &str, basename: &str) -> bool {
    INSTRUCTION_BASENAMES
        .iter()
        .any(|b| basename.eq_ignore_ascii_case(b))
        || INSTRUCTION_PATHS.contains(&path)
        || has_prefix(path, &INSTRUCTION_PREFIXES)
}

/// The skill's name when `path` defines one: the directory for a `SKILL.md`,
/// the file stem for a flat `<skills>/<name>.md`.
fn skill_name(path: &str) -> Option<String> {
    let under_skills = has_prefix(path, &SKILL_PREFIXES) || path.contains("/skills/");
    if !under_skills {
        return None;
    }
    let basename = path.rsplit('/').next().unwrap_or(path);
    if basename.eq_ignore_ascii_case("SKILL.md") {
        return path
            .trim_end_matches(basename)
            .trim_end_matches('/')
            .rsplit('/')
            .next()
            .map(str::to_string);
    }
    if !basename.ends_with(".md") {
        return None;
    }
    Some(basename.trim_end_matches(".md").to_string())
}

fn has_prefix(path: &str, prefixes: &[&str]) -> bool {
    prefixes.iter().any(|p| path.starts_with(p))
}

fn push_capped(into: &mut Vec<RepoFileFact>, entry: &tools::FileEntry, added: bool) {
    if into.len() >= MAX_LISTED {
        return;
    }
    into.push(RepoFileFact {
        path: entry.path.clone(),
        size_bytes: entry.size_bytes,
        added_in_session: added,
        excerpt: None,
    });
}

/// `mcp__<server>__<tool>` → `<server>`; anything else is not an MCP tool.
fn mcp_server_of(tool: &str) -> Option<String> {
    let rest = tool.strip_prefix("mcp__")?;
    let (server, _) = rest.split_once("__")?;
    (!server.is_empty()).then(|| server.to_string())
}

/// Server names from an MCP config: the keys of `mcpServers` (or `mcp.servers`,
/// which is how VS Code and Gemini spell it).
fn mcp_server_names(body: &str) -> Vec<String> {
    let Ok(json) = serde_json::from_str::<serde_json::Value>(body) else {
        return Vec::new();
    };
    let candidates = [
        json.get("mcpServers"),
        json.get("servers"),
        json.get("mcp").and_then(|m| m.get("servers")),
        json.get("mcp").and_then(|m| m.get("mcpServers")),
    ];
    for candidate in candidates.into_iter().flatten() {
        if let Some(map) = candidate.as_object()
            && !map.is_empty()
        {
            return map.keys().cloned().collect();
        }
    }
    Vec::new()
}

/// The `description:` line of a markdown file's YAML frontmatter.
fn frontmatter_description(body: &str) -> Option<String> {
    let rest = body.strip_prefix("---")?;
    let end = rest.find("\n---")?;
    for line in rest[..end].lines() {
        if let Some(value) = line.trim().strip_prefix("description:") {
            let value = value.trim().trim_matches(['"', '\''].as_ref()).trim();
            if !value.is_empty() {
                return Some(truncate_chars(value, DESCRIPTION_CHARS));
            }
        }
    }
    None
}

fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let cut: String = s.chars().take(max).collect();
    format!("{cut}…")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skill_names_come_from_dir_or_stem() {
        assert_eq!(
            skill_name(".claude/skills/play-session/SKILL.md").as_deref(),
            Some("play-session")
        );
        assert_eq!(
            skill_name(".agents/skills/drill-tdd.md").as_deref(),
            Some("drill-tdd")
        );
        assert_eq!(skill_name("src/skills/mod.rs"), None);
        assert_eq!(skill_name("README.md"), None);
    }

    #[test]
    fn plugin_qualified_skill_matches_its_directory() {
        assert!(skill_names_match("heist", "gangsta:heist"));
        assert!(skill_names_match("play-session", "play-session"));
        assert!(!skill_names_match("heist", "recon"));
    }

    #[test]
    fn mcp_servers_read_from_either_spelling() {
        let claude = r#"{"mcpServers": {"zeplin": {}, "dokploy": {}}}"#;
        assert_eq!(mcp_server_names(claude), vec!["dokploy", "zeplin"]);
        let vscode = r#"{"mcp": {"servers": {"playwright": {}}}}"#;
        assert_eq!(mcp_server_names(vscode), vec!["playwright"]);
        assert!(mcp_server_names(r#"{"model": "x"}"#).is_empty());
        assert!(mcp_server_names("not json").is_empty());
    }

    #[test]
    fn mcp_tool_names_carry_their_server() {
        assert_eq!(
            mcp_server_of("mcp__zeplin__get_screen").as_deref(),
            Some("zeplin")
        );
        assert_eq!(mcp_server_of("Bash"), None);
        assert_eq!(mcp_server_of("mcp__"), None);
    }

    #[test]
    fn instruction_files_match_by_basename_and_path() {
        assert!(is_instruction("AGENTS.md", "AGENTS.md"));
        assert!(is_instruction("packages/api/CLAUDE.md", "CLAUDE.md"));
        assert!(is_instruction(".cursor/rules/style.mdc", "style.mdc"));
        assert!(!is_instruction("README.md", "README.md"));
    }

    #[test]
    fn frontmatter_description_is_optional() {
        let body = "---\nname: x\ndescription: \"Runs the thing\"\n---\nbody";
        assert_eq!(
            frontmatter_description(body).as_deref(),
            Some("Runs the thing")
        );
        assert_eq!(frontmatter_description("no frontmatter"), None);
    }
}
