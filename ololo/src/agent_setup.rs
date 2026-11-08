//! What the player's coding agents are configured with on this machine — the
//! skills and MCP servers they can reach — reported to the server at the
//! session's start, after each finished task and on exit, for the Agentic judge.
//!
//! The repository shows only what the player committed. User-level skills,
//! plugins and MCP servers live in the home directory and never reach git,
//! and the Agentic judge runs after the session, when nobody is left to ask.
//! So the CLI reads them itself. **Names only**: an MCP server's command,
//! arguments, URL and environment often carry secrets and are never read
//! into the report. Best-effort throughout — a file that is missing or does
//! not parse is simply not there.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

use arena_core::protocol::{
    AGENT_SETUP_MAX_ITEMS, AgentSetupConfig, AgentSetupReport, ConfiguredItem,
};

/// Read the configuration of every agent found under `home` and `cwd`.
/// Agents with nothing configured are left out.
pub fn collect(home: &Path, cwd: &Path) -> AgentSetupReport {
    let mut agents = Vec::new();

    // Claude Code: skills in ~/.claude/skills and the project's
    // .claude/skills; MCP servers in ~/.claude.json (user, and per project
    // "local"), the project's .mcp.json; and whatever installed plugins bring.
    let mut skills = Vec::new();
    skills.extend(skill_dir(&home.join(".claude/skills"), "user", None));
    skills.extend(skill_dir(&cwd.join(".claude/skills"), "project", None));
    let mut mcp = Vec::new();
    if let Some(cfg) = read_json(&home.join(".claude.json")) {
        mcp.extend(names(cfg.get("mcpServers"), "user", None));
        let local = cfg
            .get("projects")
            .and_then(|p| p.as_object())
            .and_then(|projects| {
                projects
                    .iter()
                    .find(|(path, _)| same_path(path, cwd))
                    .map(|(_, p)| p)
            })
            .and_then(|p| p.get("mcpServers"));
        mcp.extend(names(local, "local", None));
    }
    mcp.extend(mcp_file(&cwd.join(".mcp.json"), "project", None));
    for (plugin, dir) in claude_plugins(home, cwd) {
        skills.extend(skill_dir(&dir.join("skills"), "plugin", Some(&plugin)));
        mcp.extend(mcp_file(&dir.join(".mcp.json"), "plugin", Some(&plugin)));
    }
    push(&mut agents, "claude-code", skills, mcp);

    // Codex: ~/.codex/skills, and `[mcp_servers.<name>]` in config.toml.
    let skills = skill_dir(&home.join(".codex/skills"), "user", None);
    let mut mcp = toml_servers(&home.join(".codex/config.toml"), "user");
    mcp.extend(toml_servers(&cwd.join(".codex/config.toml"), "project"));
    push(&mut agents, "codex", skills, mcp);

    // Gemini CLI and Cursor: `mcpServers` in their settings files.
    let mut mcp = mcp_file(&home.join(".gemini/settings.json"), "user", None);
    mcp.extend(mcp_file(
        &cwd.join(".gemini/settings.json"),
        "project",
        None,
    ));
    push(&mut agents, "gemini", Vec::new(), mcp);
    let mut mcp = mcp_file(&home.join(".cursor/mcp.json"), "user", None);
    mcp.extend(mcp_file(&cwd.join(".cursor/mcp.json"), "project", None));
    push(&mut agents, "cursor", Vec::new(), mcp);

    // The shared `.agents/skills` convention several agents read.
    let mut skills = skill_dir(&home.join(".agents/skills"), "user", None);
    skills.extend(skill_dir(&cwd.join(".agents/skills"), "project", None));
    push(&mut agents, "agents", skills, Vec::new());

    AgentSetupReport { agents }
}

/// Collect and POST the report for this session. Best-effort, bounded.
pub async fn report(server_url: &str, pat: &str, join_code: &str) {
    let Some(home) = dirs::home_dir() else {
        return;
    };
    let Ok(cwd) = std::env::current_dir() else {
        return;
    };
    let Ok(setup) = tokio::task::spawn_blocking(move || collect(&home, &cwd)).await else {
        return;
    };
    let url = format!(
        "{}/api/sessions/{join_code}/agent-setup",
        crate::util::http_base_url(server_url)
    );
    let sent = reqwest::Client::new()
        .post(&url)
        .bearer_auth(pat)
        .json(&setup)
        .timeout(Duration::from_secs(10))
        .send()
        .await;
    match sent {
        Ok(resp) if resp.status().is_success() => {
            tracing::info!("agent setup reported: {} agents", setup.agents.len())
        }
        Ok(resp) => tracing::warn!("agent setup report rejected: {}", resp.status()),
        Err(e) => tracing::warn!("agent setup report failed: {e}"),
    }
}

fn push(
    agents: &mut Vec<AgentSetupConfig>,
    agent: &str,
    skills: Vec<ConfiguredItem>,
    mcp_servers: Vec<ConfiguredItem>,
) {
    let tidy = |items: Vec<ConfiguredItem>| {
        let mut set: Vec<ConfiguredItem> = items
            .into_iter()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        // Half the server's limit: names of up to 128 bytes keep even five
        // agents' lists well inside the request body cap.
        set.truncate(AGENT_SETUP_MAX_ITEMS / 2);
        set
    };
    let (skills, mcp_servers) = (tidy(skills), tidy(mcp_servers));
    if skills.is_empty() && mcp_servers.is_empty() {
        return;
    }
    agents.push(AgentSetupConfig {
        agent: agent.to_string(),
        skills,
        mcp_servers,
    });
}

fn item(name: &str, scope: &str, plugin: Option<&str>) -> ConfiguredItem {
    ConfiguredItem {
        name: clip(name),
        scope: scope.to_string(),
        plugin: plugin.map(clip),
    }
}

/// At most `TASK_STATS_MAX_NAME_LEN` bytes — the server counts bytes — cut
/// on a character boundary.
fn clip(s: &str) -> String {
    let max = arena_core::protocol::TASK_STATS_MAX_NAME_LEN;
    if s.len() <= max {
        return s.to_string();
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    s[..end].to_string()
}

/// Whether a path an agent recorded names the working directory: the same
/// place after resolving symlinks, or — when it cannot be resolved — the
/// same spelling up to separators (and, on Windows, case).
fn same_path(recorded: &str, cwd: &Path) -> bool {
    let canon = |p: &Path| std::fs::canonicalize(p).ok();
    if let (Some(a), Some(b)) = (canon(Path::new(recorded)), canon(cwd)) {
        return a == b;
    }
    let norm = |s: &str| {
        let s = s.replace('\\', "/").trim_end_matches('/').to_string();
        if cfg!(windows) { s.to_lowercase() } else { s }
    };
    norm(recorded) == norm(&cwd.to_string_lossy())
}

/// The skills under `dir`: each subdirectory holding a `SKILL.md`, named by
/// its frontmatter `name:` or else the directory.
fn skill_dir(dir: &Path, scope: &str, plugin: Option<&str>) -> Vec<ConfiguredItem> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter_map(|e| {
            let path = e.path();
            let dir_name = e.file_name().to_string_lossy().to_string();
            if dir_name.starts_with('.') {
                return None;
            }
            let manifest = ["SKILL.md", "skill.md"]
                .iter()
                .map(|f| path.join(f))
                .find(|p| p.is_file())?;
            let name = std::fs::read_to_string(&manifest)
                .ok()
                .and_then(|body| frontmatter_name(&body))
                .unwrap_or(dir_name);
            Some(item(&name, scope, plugin))
        })
        .collect()
}

/// `name:` in a leading `---` frontmatter block.
fn frontmatter_name(body: &str) -> Option<String> {
    let mut lines = body.lines();
    if lines.next()?.trim() != "---" {
        return None;
    }
    for line in lines {
        let line = line.trim();
        if line == "---" {
            return None;
        }
        if let Some(rest) = line.strip_prefix("name:") {
            let name = rest.trim().trim_matches(['"', '\'']).trim();
            return (!name.is_empty()).then(|| name.to_string());
        }
    }
    None
}

fn read_json(path: &Path) -> Option<serde_json::Value> {
    serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
}

/// The keys of a server map — the names, never the definitions.
fn names(
    map: Option<&serde_json::Value>,
    scope: &str,
    plugin: Option<&str>,
) -> Vec<ConfiguredItem> {
    map.and_then(|m| m.as_object())
        .map(|m| m.keys().map(|k| item(k, scope, plugin)).collect())
        .unwrap_or_default()
}

/// An MCP JSON file: `{"mcpServers": {...}}`, or (in a plugin) the map
/// itself.
fn mcp_file(path: &Path, scope: &str, plugin: Option<&str>) -> Vec<ConfiguredItem> {
    let Some(cfg) = read_json(path) else {
        return Vec::new();
    };
    match cfg.get("mcpServers") {
        Some(servers) => names(Some(servers), scope, plugin),
        None if plugin.is_some() => names(Some(&cfg), scope, plugin),
        None => Vec::new(),
    }
}

/// `[mcp_servers.<name>]` tables in a Codex `config.toml`.
fn toml_servers(path: &Path, scope: &str) -> Vec<ConfiguredItem> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let Ok(cfg) = text.parse::<toml::Table>() else {
        return Vec::new();
    };
    cfg.get("mcp_servers")
        .and_then(|v| v.as_table())
        .map(|t| t.keys().map(|k| item(k, scope, None)).collect())
        .unwrap_or_default()
}

/// Installed Claude Code plugins that apply here: user-scoped ones, and
/// project- or local-scoped ones installed for this working directory.
fn claude_plugins(home: &Path, cwd: &Path) -> Vec<(String, PathBuf)> {
    let Some(manifest) = read_json(&home.join(".claude/plugins/installed_plugins.json")) else {
        return Vec::new();
    };
    let Some(plugins) = manifest.get("plugins").unwrap_or(&manifest).as_object() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for (key, installs) in plugins {
        let Some(installs) = installs.as_array() else {
            continue;
        };
        for install in installs {
            let scope = install
                .get("scope")
                .and_then(|s| s.as_str())
                .unwrap_or("user");
            let applies = scope == "user"
                || install
                    .get("projectPath")
                    .and_then(|p| p.as_str())
                    .is_some_and(|p| same_path(p, cwd));
            if !applies {
                continue;
            }
            if let Some(path) = install.get("installPath").and_then(|p| p.as_str()) {
                out.push((key.clone(), PathBuf::from(path)));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(path: &Path, body: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, body).unwrap();
    }

    #[test]
    fn reads_names_from_every_agent_and_never_a_definition() {
        let home = tempfile::tempdir().unwrap();
        let cwd = tempfile::tempdir().unwrap();
        let (h, c) = (home.path(), cwd.path());
        write(
            &h.join(".claude/skills/capture/SKILL.md"),
            "---\nname: capture-shots\n---\nTake shots",
        );
        write(&h.join(".claude/skills/plain/SKILL.md"), "no frontmatter");
        write(&h.join(".claude/skills/not-a-skill/notes.md"), "x");
        write(
            &c.join(".claude/skills/deliver/SKILL.md"),
            "---\ndescription: d\n---\n",
        );
        write(
            &h.join(".claude.json"),
            &serde_json::json!({
                "mcpServers": { "context7": { "command": "npx", "env": { "TOKEN": "secret" } } },
                "projects": { c.to_string_lossy(): { "mcpServers": { "local-db": {} } },
                              "/elsewhere": { "mcpServers": { "other": {} } } }
            })
            .to_string(),
        );
        write(
            &c.join(".mcp.json"),
            r#"{"mcpServers":{"playwright":{"command":"npx"}}}"#,
        );
        // A user plugin, and a local one installed for another project.
        let plugin = h.join(".claude/plugins/cache/m/pw/1.0.0");
        write(
            &plugin.join("skills/drive/SKILL.md"),
            "---\nname: drive\n---\n",
        );
        write(&plugin.join(".mcp.json"), r#"{"browser":{"command":"x"}}"#);
        let elsewhere = h.join(".claude/plugins/cache/m/other/1.0.0");
        write(&elsewhere.join("skills/nope/SKILL.md"), "x");
        write(
            &h.join(".claude/plugins/installed_plugins.json"),
            &serde_json::json!({ "version": 2, "plugins": {
                "pw@m": [{ "scope": "user", "installPath": plugin }],
                "other@m": [{ "scope": "local", "installPath": elsewhere, "projectPath": "/elsewhere" }]
            }})
            .to_string(),
        );
        write(
            &h.join(".codex/config.toml"),
            "model = \"x\"\n[mcp_servers.github]\ncommand = \"gh\"\nenv = { TOKEN = \"secret\" }\n",
        );
        write(&h.join(".agents/skills/shared/SKILL.md"), "x");

        let report = collect(h, c);
        let json = serde_json::to_string(&report).unwrap();
        assert!(
            !json.contains("secret") && !json.contains("npx"),
            "names only: {json}"
        );

        let agent = |name: &str| report.agents.iter().find(|a| a.agent == name).expect(name);
        let pairs = |items: &[ConfiguredItem]| {
            items
                .iter()
                .map(|i| format!("{}:{}", i.scope, i.name))
                .collect::<Vec<_>>()
        };
        let claude = agent("claude-code");
        assert_eq!(
            pairs(&claude.skills),
            vec![
                "user:capture-shots",
                "project:deliver",
                "plugin:drive",
                "user:plain"
            ]
        );
        assert_eq!(
            pairs(&claude.mcp_servers),
            vec![
                "plugin:browser",
                "user:context7",
                "local:local-db",
                "project:playwright"
            ]
        );
        assert_eq!(claude.skills[2].plugin.as_deref(), Some("pw@m"));
        assert_eq!(pairs(&agent("codex").mcp_servers), vec!["user:github"]);
        assert_eq!(pairs(&agent("agents").skills), vec!["user:shared"]);
        // Nothing configured: no entry at all.
        assert!(
            report
                .agents
                .iter()
                .all(|a| a.agent != "gemini" && a.agent != "cursor")
        );
        assert!(report.validate().is_ok());
    }

    #[test]
    fn an_empty_machine_reports_no_agents() {
        let home = tempfile::tempdir().unwrap();
        let cwd = tempfile::tempdir().unwrap();
        assert!(collect(home.path(), cwd.path()).agents.is_empty());
    }

    #[test]
    fn long_names_are_cut_by_bytes_on_a_character_boundary() {
        let cyrillic = "ж".repeat(100); // 200 bytes
        let cut = clip(&cyrillic);
        assert!(cut.len() <= 128 && cut.chars().all(|c| c == 'ж'));
        assert_eq!(clip("short"), "short");
    }

    #[test]
    fn a_project_path_matches_through_a_symlink() {
        let real = tempfile::tempdir().unwrap();
        let links = tempfile::tempdir().unwrap();
        let link = links.path().join("ws");
        #[cfg(unix)]
        std::os::unix::fs::symlink(real.path(), &link).unwrap();
        #[cfg(unix)]
        assert!(same_path(&link.to_string_lossy(), real.path()));
        assert!(same_path(
            &format!("{}/", real.path().display()),
            real.path()
        ));
        assert!(!same_path("/elsewhere", real.path()));
    }
}
