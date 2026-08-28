use std::path::PathBuf;

/// The root every agent-store path hangs off. `AGENT_TOKENS_HOME` overrides
/// the real home — tests and containers point it at a scratch dir so a
/// snapshot never crawls the developer's actual agent history (a full
/// `~/.claude/projects` turns a "does not panic" test into minutes of IO).
pub fn home_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("AGENT_TOKENS_HOME").filter(|v| !v.is_empty()) {
        return PathBuf::from(dir);
    }
    dirs::home_dir().unwrap_or_else(|| PathBuf::from("."))
}

pub fn claude_projects_dir() -> PathBuf {
    home_dir().join(".claude").join("projects")
}

/// Escape a cwd path the way Claude Code does: replace every `/` with `-`.
/// e.g. /Users/apk/Workspace/lab/tetris -> -Users-apk-Workspace-lab-tetris
pub fn escape_cwd(path: &std::path::Path) -> String {
    path.to_string_lossy().replace('/', "-")
}

pub fn pi_sessions_dir() -> PathBuf {
    home_dir().join(".pi").join("agent").join("sessions")
}

pub fn omp_sessions_dir() -> PathBuf {
    home_dir().join(".omp").join("agent").join("sessions")
}

pub fn kiro_sessions_dir() -> PathBuf {
    home_dir().join(".kiro").join("sessions").join("cli")
}

pub fn copilot_state_dir() -> PathBuf {
    home_dir().join(".copilot").join("session-state")
}

/// Zed's agent-thread store. Location varies by platform; first existing wins.
pub fn zed_threads_db_paths() -> Vec<PathBuf> {
    let mut out = Vec::new();
    // macOS
    out.push(home_dir().join("Library/Application Support/Zed/threads/threads.db"));
    // Linux/BSD: $XDG_DATA_HOME, else ~/.local/share
    let xdg = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home_dir().join(".local/share"));
    out.push(xdg.join("zed/threads/threads.db"));
    // Windows
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        out.push(PathBuf::from(local).join("Zed/threads/threads.db"));
    }
    out
}

/// Codex rollout logs live in two directories with the same schema; archived
/// sessions still count towards a participant's usage.
pub fn codex_session_dirs() -> Vec<PathBuf> {
    let codex = home_dir().join(".codex");
    vec![codex.join("sessions"), codex.join("archived_sessions")]
}

pub fn cursor_projects_dir() -> PathBuf {
    home_dir().join(".cursor").join("projects")
}

/// Cursor IDE global state DB; location varies by platform. First existing wins.
pub fn cursor_state_db_paths() -> Vec<PathBuf> {
    vec![
        home_dir().join("Library/Application Support/Cursor/User/globalStorage/state.vscdb"),
        home_dir().join(".config/Cursor/User/globalStorage/state.vscdb"),
    ]
}

/// tokscale's Cursor usage-export cache (`tokscale cursor sync`): `usage*.csv`.
pub fn tokscale_cursor_cache_dir() -> PathBuf {
    home_dir().join(".config/tokscale/cursor-cache")
}

/// tokscale's Antigravity IDE usage cache (`tokscale antigravity sync`).
pub fn tokscale_antigravity_cache_dir() -> PathBuf {
    home_dir().join(".config/tokscale/antigravity-cache/sessions")
}

/// ololo's own Antigravity IDE usage cache, written by the CLI's built-in
/// sync (`ololo::antigravity_sync`) in the same JSONL artifact format as
/// tokscale's. Kept under `~/.config/ololo` beside the CLI's other state.
pub fn ololo_antigravity_cache_dir() -> PathBuf {
    home_dir().join(".config/ololo/antigravity-cache/sessions")
}

/// Gemini home dir; honors `GEMINI_CLI_HOME` (same override the gemini CLI and
/// Antigravity CLI use), falling back to `~/.gemini`.
pub fn gemini_home_dir() -> PathBuf {
    std::env::var("GEMINI_CLI_HOME")
        .ok()
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| home_dir().join(".gemini"))
}

/// Antigravity CLI conversation DBs: one sqlite `.db` per conversation.
pub fn antigravity_cli_conversations_dir() -> PathBuf {
    gemini_home_dir()
        .join("antigravity-cli")
        .join("conversations")
}

pub fn gemini_tmp_dir() -> PathBuf {
    home_dir().join(".gemini").join("tmp")
}

pub fn qwen_projects_dir() -> PathBuf {
    home_dir().join(".qwen").join("projects")
}

pub fn kimi_sessions_dir() -> PathBuf {
    home_dir().join(".kimi").join("sessions")
}

/// Goose keeps one sqlite DB; location varies by platform/version.
pub fn goose_db_paths() -> Vec<PathBuf> {
    vec![
        home_dir().join(".local/share/goose/sessions/sessions.db"),
        home_dir().join("Library/Application Support/goose/sessions/sessions.db"),
        home_dir().join(".local/share/Block/goose/sessions/sessions.db"),
    ]
}

pub fn amp_threads_dir() -> PathBuf {
    let base = std::env::var("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| home_dir().join(".local/share"));
    base.join("amp").join("threads")
}

pub fn opencode_db_path() -> PathBuf {
    let base = std::env::var("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| home_dir().join(".local/share"));
    base.join("opencode").join("opencode.db")
}
