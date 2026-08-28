#![allow(dead_code)]

use serde::Serialize;
use sha2::Digest;

#[derive(Debug, Serialize, Clone)]
pub struct ToolEntry {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
}

#[derive(Debug, Serialize, Clone)]
pub struct ProbeMetadata {
    pub ai_agents: Vec<ToolEntry>,
    pub build_tools: Vec<ToolEntry>,
    pub languages: Vec<ToolEntry>,
    pub test_tools: Vec<ToolEntry>,
    pub utility_tools: Vec<ToolEntry>,
    pub probe_duration_ms: u64,
    pub platform: String,
}

pub struct ProbeResult {
    pub fingerprint: String,
    pub metadata: ProbeMetadata,
}

/// Names of AI coding agent binaries we probe for. Re-exported from the
/// `agent-tokens` crate to maintain a single source of truth.
/// When adding a new agent, add it to `agent-tokens/src/lib.rs`.
pub use agent_tokens::AI_AGENT_NAMES;

/// Compute a SHA-256 fingerprint from machine_id and cwd, returned as 64-char lowercase hex.
pub fn compute_fingerprint(machine_id: &str, cwd: &str) -> String {
    let input = format!("{}|{}", machine_id, cwd);
    hex::encode(sha2::Sha256::digest(input.as_bytes()))
}

/// Deduplicate python probing: if python3 or python found, return Some("python"), else None.
pub fn dedup_python(python3_found: bool, python_found: bool) -> Option<&'static str> {
    if python3_found || python_found {
        Some("python")
    } else {
        None
    }
}

/// Run `<name> --version`, return a trimmed version string or None.
fn run_version(name: &str) -> Option<String> {
    let output = std::process::Command::new(name)
        .arg("--version")
        .output()
        .ok()?;
    let text = if !output.stdout.is_empty() {
        String::from_utf8_lossy(&output.stdout).into_owned()
    } else {
        String::from_utf8_lossy(&output.stderr).into_owned()
    };
    extract_version_from_output(&text)
}

/// Extract the first version-like token from a `--version` output string.
pub fn extract_version_from_output(text: &str) -> Option<String> {
    let first_line = text.lines().next()?.trim();
    for token in first_line.split_whitespace() {
        if let Some(v) = parse_version_token(token) {
            return Some(v);
        }
    }
    None
}

/// Parse a single whitespace token and return a version string if it looks like one.
/// Handles formats: "1.2.3", "v1.2.3", "jq-1.2.3", "27.0.3,".
pub fn parse_version_token(token: &str) -> Option<String> {
    // Strip leading 'v' or 'V'
    let stripped = token.trim_start_matches(['v', 'V']);
    // Find first digit position (handles "jq-1.7.1" style)
    let start = stripped.find(|c: char| c.is_ascii_digit())?;
    let from_digit = &stripped[start..];
    // Must contain a separator (dot or dash) followed by a digit
    let has_sep = from_digit.bytes().enumerate().any(|(i, b)| {
        (b == b'.' || b == b'-')
            && i + 1 < from_digit.len()
            && from_digit.as_bytes()[i + 1].is_ascii_digit()
    });
    if !has_sep {
        return None;
    }
    // Collect version characters: digits, dots, dashes, plus signs
    let v: String = from_digit
        .chars()
        .take_while(|&c| c.is_ascii_digit() || c == '.' || c == '-' || c == '+')
        .collect();
    // Trim trailing non-digit chars (e.g. trailing '-')
    let v = v
        .trim_end_matches(|c: char| !c.is_ascii_digit())
        .to_string();
    if v.len() >= 3 { Some(v) } else { None }
}

/// Check if `name` is on PATH, run `--version`, print progress (unless
/// `silent`), and return a `ToolEntry`. Returns `None` if not found.
pub(crate) fn probe_tool(name: &str, silent: bool) -> Option<ToolEntry> {
    if which::which(name).is_err() {
        return None;
    }
    let version = run_version(name);
    if !silent {
        match &version {
            Some(v) => eprintln!("{}... {} ok", name, v),
            None => eprintln!("{}... ok", name),
        }
    }
    Some(ToolEntry {
        name: name.to_string(),
        version,
    })
}

pub async fn run_probe() -> ProbeResult {
    run_probe_with_chosen_agent(None, false).await
}

/// Like `run_probe`, but when `chosen_agent` is set, the reported
/// `ai_agents` list is filtered to that single agent. ololo knows which
/// agent binary it actually spawned in the PTY; reporting only that one
/// keeps the dashboard agent label accurate (instead of listing every
/// agent binary detected on $PATH). When `silent` is true, tool-progress
/// lines are suppressed (used in TUI mode, where detection is shown in
/// the TUI instead of on the console).
pub async fn run_probe_with_chosen_agent(
    chosen_agent: Option<String>,
    silent: bool,
) -> ProbeResult {
    let start = std::time::Instant::now();

    let result = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        tokio::task::spawn_blocking(move || probe_sync(silent)),
    )
    .await;

    let probe_duration_ms = start.elapsed().as_millis() as u64;

    match result {
        Ok(Ok((fingerprint, mut metadata))) => {
            metadata.probe_duration_ms = probe_duration_ms;
            if let Some(name) = chosen_agent.as_deref() {
                metadata.ai_agents.retain(|t| t.name == name);
                if metadata.ai_agents.is_empty() {
                    // Chosen agent wasn't detected by the PATH probe — still
                    // report it, version unknown. The user explicitly chose it.
                    metadata.ai_agents.push(ToolEntry {
                        name: name.to_string(),
                        version: None,
                    });
                }
            }
            ProbeResult {
                fingerprint,
                metadata,
            }
        }
        _ => {
            // spawn_blocking timed out or panicked — compute best-effort fingerprint.
            let machine_id = machine_uid::get().unwrap_or_else(|_| {
                if let Some(path) = dirs_machine_id_path()
                    && let Ok(content) = std::fs::read_to_string(&path)
                {
                    let t = content.trim().to_string();
                    if !t.is_empty() {
                        return t;
                    }
                }
                uuid::Uuid::new_v4().to_string()
            });
            let cwd = std::env::current_dir()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            let fingerprint = compute_fingerprint(&machine_id, &cwd);
            let ai_agents = chosen_agent
                .map(|name| {
                    vec![ToolEntry {
                        name,
                        version: None,
                    }]
                })
                .unwrap_or_default();
            ProbeResult {
                fingerprint,
                metadata: ProbeMetadata {
                    ai_agents,
                    build_tools: vec![],
                    languages: vec![],
                    test_tools: vec![],
                    utility_tools: vec![],
                    probe_duration_ms,
                    platform: format!("{} {}", std::env::consts::OS, std::env::consts::ARCH),
                },
            }
        }
    }
}

fn probe_sync(silent: bool) -> (String, ProbeMetadata) {
    // Step 1: Get machine ID
    let machine_id = match machine_uid::get() {
        Ok(id) => id,
        Err(_) => get_or_create_machine_id(),
    };

    // Step 2: Get CWD
    let cwd = std::env::current_dir()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();

    // Step 3: Compute fingerprint
    let fingerprint = compute_fingerprint(&machine_id, &cwd);

    // Step 4: Probe tools
    let ai_agents: Vec<ToolEntry> = AI_AGENT_NAMES
        .iter()
        .filter_map(|&name| probe_tool(name, silent))
        .collect();

    let build_tool_names = [
        "cargo", "docker", "make", "cmake", "gradle", "maven", "npm", "yarn", "pnpm", "bun",
        "just", "ninja", "bazel", "uv",
    ];
    let build_tools: Vec<ToolEntry> = build_tool_names
        .iter()
        .filter_map(|&name| probe_tool(name, silent))
        .collect();

    let mut languages: Vec<ToolEntry> = Vec::new();

    // Python special case: dedup python3 and python
    let python3_found = which::which("python3").is_ok();
    let python_found = which::which("python").is_ok();
    if let Some(py) = dedup_python(python3_found, python_found) {
        let bin = if python3_found { "python3" } else { "python" };
        let version = run_version(bin);
        if !silent {
            match &version {
                Some(v) => eprintln!("{}... {} ok", py, v),
                None => eprintln!("{}... ok", py),
            }
        }
        languages.push(ToolEntry {
            name: py.to_string(),
            version,
        });
    }

    // Other languages
    let other_langs = [
        "rustc", "node", "ruby", "go", "java", "dotnet", "swift", "kotlin", "php", "bun",
    ];
    for lang in &other_langs {
        if let Some(entry) = probe_tool(lang, silent) {
            languages.push(entry);
        }
    }

    let test_tool_names = [
        "curl",
        "wget",
        "http",
        "httpie",
        "hurl",
        "xh",
        "playwright",
        "puppeteer",
        "selenium-side-runner",
        "newman",
        "k6",
        "wrk",
        "ab",
        "vegeta",
        "locust",
        "agent-browser",
    ];
    let test_tools: Vec<ToolEntry> = test_tool_names
        .iter()
        .filter_map(|&name| probe_tool(name, silent))
        .collect();

    let utility_tool_names = [
        "cat", "head", "tail", "tee", "jq", "yq", "fx", "jless", "bat", "gron", "mlr", "dasel",
        "rg",
    ];
    let utility_tools: Vec<ToolEntry> = utility_tool_names
        .iter()
        .filter_map(|&name| probe_tool(name, silent))
        .collect();

    let metadata = ProbeMetadata {
        ai_agents,
        build_tools,
        languages,
        test_tools,
        utility_tools,
        probe_duration_ms: 0,
        platform: format!("{} {}", std::env::consts::OS, std::env::consts::ARCH),
    };

    (fingerprint, metadata)
}

fn get_or_create_machine_id() -> String {
    let config_dir = dirs_machine_id_path();

    if let Some(path) = &config_dir
        && path.exists()
        && let Ok(content) = std::fs::read_to_string(path)
    {
        let trimmed = content.trim().to_string();
        if !trimmed.is_empty() {
            return trimmed;
        }
    }

    let new_id = uuid::Uuid::new_v4().to_string();

    if let Some(path) = &config_dir {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(()) = std::fs::write(path, &new_id) {
            #[cfg(not(windows))]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
            }
        }
    }

    new_id
}

fn dirs_machine_id_path() -> Option<std::path::PathBuf> {
    let home = std::env::var("HOME").ok()?;
    let mut path = std::path::PathBuf::from(home);
    path.push(".config/ololo/machine_id");
    Some(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fingerprint_is_64_hex_chars() {
        let fp = compute_fingerprint("test-machine-id", "/some/cwd");
        assert_eq!(fp.len(), 64);
        assert!(
            fp.chars()
                .all(|c| c.is_ascii_hexdigit() && !c.is_uppercase())
        );
    }

    #[test]
    fn fingerprint_is_deterministic() {
        let fp1 = compute_fingerprint("machine-abc", "/home/user/project");
        let fp2 = compute_fingerprint("machine-abc", "/home/user/project");
        assert_eq!(fp1, fp2);
    }

    #[test]
    fn python_dedup_both_present() {
        let result = dedup_python(true, true);
        assert_eq!(result, Some("python"));
        let list: Vec<&str> = result.into_iter().collect();
        assert_eq!(list.len(), 1);
    }

    #[test]
    fn python_dedup_only_python3() {
        let result = dedup_python(true, false);
        assert_eq!(result, Some("python"));
    }

    #[test]
    fn python_dedup_neither() {
        let result = dedup_python(false, false);
        assert_eq!(result, None);
    }

    #[test]
    fn version_extraction_semver() {
        assert_eq!(
            extract_version_from_output("cargo 1.85.0 (2024-06-01)"),
            Some("1.85.0".into())
        );
    }

    #[test]
    fn version_extraction_v_prefix() {
        assert_eq!(
            extract_version_from_output("v22.1.0"),
            Some("22.1.0".into())
        );
    }

    #[test]
    fn version_extraction_embedded() {
        assert_eq!(
            extract_version_from_output("jq-1.7.1"),
            Some("1.7.1".into())
        );
    }

    #[test]
    fn version_extraction_trailing_comma() {
        assert_eq!(
            extract_version_from_output("Docker version 27.0.3, build abc"),
            Some("27.0.3".into())
        );
    }

    #[test]
    fn version_extraction_no_version() {
        assert_eq!(
            extract_version_from_output("usage: cat [options] [file]"),
            None
        );
    }

    #[test]
    fn version_extraction_empty() {
        assert_eq!(extract_version_from_output(""), None);
    }
}
