//! Git-read tools for the judge agent loop.
//!
//! Each tool shells out to the `git` binary inside `spawn_blocking` and is
//! bounded by a 10s timeout. On timeout the tool returns an error string to
//! the agent — the loop continues (FR-004).

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::time::timeout;

use super::JudgeError;

const TOOL_TIMEOUT: Duration = Duration::from_secs(10);
const READ_FILE_CAP: usize = 32 * 1024;
const DIFF_CAP: usize = 64 * 1024;
const TRUNC_MARKER: &str = "\n...[truncated]...\n";

/// Repo paths a judge's git tools must not open.
///
/// The player's snapshot carries the platform's own runtime tree under
/// `.ololo/`: delivered artifacts, completion flags, and whatever scratch a
/// task's fixtures wrote. For the UX review that tree *is* the evidence — the
/// screenshots live there. For a judge that only reads the player's code it is
/// noise, and expensive noise: in a five-part campaign `.ololo/tmp` held 468 of
/// the repo's 494 files, so a plain file listing came back 93% scratch data and
/// then rode along in the context for the rest of the run.
///
/// A judge declares its own blind spots in `judges.ignore_paths`; everything
/// else stays visible, so this can only narrow one judge, never the panel.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ToolScope {
    prefixes: Vec<String>,
}

impl ToolScope {
    /// Nothing hidden — the judge sees the whole snapshot.
    pub fn everything() -> Self {
        Self::default()
    }

    /// From the judge's stored `ignore_paths` (a JSON array of prefixes).
    /// Absent or unparseable hides nothing: a judge that cannot say what to
    /// skip still gets to do its job.
    pub fn from_json(raw: Option<&str>) -> Self {
        let prefixes = raw
            .and_then(|r| serde_json::from_str::<Vec<String>>(r).ok())
            .unwrap_or_default()
            .into_iter()
            .map(|p| p.trim().trim_start_matches("./").to_string())
            .filter(|p| !p.is_empty())
            .collect();
        Self { prefixes }
    }

    pub fn is_empty(&self) -> bool {
        self.prefixes.is_empty()
    }

    /// True when `path` sits under one of the ignored prefixes. A prefix
    /// without a trailing slash still hides a whole directory (`.ololo` hides
    /// `.ololo/tmp/x`) but never a sibling that merely starts with the same
    /// letters (`.ololorc` stays visible).
    pub fn hides(&self, path: &str) -> bool {
        let path = path.trim_start_matches("./");
        self.prefixes.iter().any(|raw| {
            let p = raw.trim_end_matches('/');
            path == p || path.starts_with(&format!("{p}/"))
        })
    }

    /// Git pathspecs that keep the ignored trees out of a diff.
    fn pathspecs(&self) -> Vec<String> {
        self.prefixes
            .iter()
            .map(|p| format!(":(exclude){}", p.trim_end_matches('/')))
            .collect()
    }

    /// What a tool answers instead of hidden content. Spelled out, because a
    /// judge that cannot see a file must know it hit a rule rather than an
    /// empty repo — the prompts tell it to score 0 rather than guess.
    fn refusal(&self, path: &str) -> String {
        format!(
            "error: '{path}' is outside this judge's scope — it is the \
             platform's own runtime tree, not the player's code. Judge the \
             player's sources; infer nothing from what you cannot read here."
        )
    }
}

/// Resolve the effective ref: `task_commit_sha` if provided, else "HEAD".
fn default_ref(task_commit_sha: Option<&str>) -> &str {
    task_commit_sha.unwrap_or("HEAD")
}

/// `-- . :(exclude)<p> …` for a git command, or nothing when the judge sees
/// the whole tree. The `.` keeps the rest of the repo in the diff: an
/// exclude-only pathspec would match nothing at all.
fn pathspec_args(excludes: &[String]) -> Vec<String> {
    if excludes.is_empty() {
        return Vec::new();
    }
    let mut args = vec!["--".to_string(), ".".to_string()];
    args.extend(excludes.iter().cloned());
    args
}

/// Locate the `git` binary once.
fn git_bin() -> Result<PathBuf, JudgeError> {
    which::which("git").map_err(|e| JudgeError::GitReadError(e.to_string()))
}

/// `list_files` tool output entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileEntry {
    pub path: String,
    pub size_bytes: u64,
}

/// `find_task_commit` / `get_commit_log` output entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommitEntry {
    pub sha: String,
    pub subject: String,
}

/// `list_files(repo_dir, ref)` — `git ls-tree -r --long <ref>`.
///
/// Returns `Vec<{path, size_bytes}>`. On empty/unborn repo or missing path,
/// returns an empty list (FR-005).
pub async fn list_files(
    repo_dir: &Path,
    r#ref: Option<&str>,
    task_commit_sha: Option<&str>,
    scope: &ToolScope,
) -> Result<Vec<FileEntry>, String> {
    let scope = scope.clone();
    let repo_dir = repo_dir.to_path_buf();
    let ref_arg = r#ref
        .map(str::to_string)
        .unwrap_or_else(|| default_ref(task_commit_sha).to_string());
    run_git_tool("list_files", move || {
        let git = git_bin().map_err(|e| e.to_string())?;
        let out = std::process::Command::new(&git)
            .arg("-C")
            .arg(&repo_dir)
            .arg("ls-tree")
            .arg("-r")
            .arg("--long")
            .arg("--end-of-options")
            .arg(&ref_arg)
            .output()
            .map_err(|e| format!("git ls-tree: {e}"))?;
        if !out.status.success() {
            return Ok(Vec::new());
        }
        let text = String::from_utf8_lossy(&out.stdout);
        let mut entries = Vec::new();
        for line in text.lines() {
            // Format: <mode> <type> <blob> <size>\t<path>
            let line = line.trim_end_matches('\r');
            if line.is_empty() {
                continue;
            }
            let (meta, path) = match line.split_once('\t') {
                Some(p) => p,
                None => continue,
            };
            let parts: Vec<&str> = meta.split_whitespace().collect();
            // <mode> <type> <sha> <size>
            let size = parts
                .get(3)
                .and_then(|s| s.parse::<u64>().ok())
                .unwrap_or(0);
            if scope.hides(path) {
                continue;
            }
            entries.push(FileEntry {
                path: path.to_string(),
                size_bytes: size,
            });
        }
        Ok(entries)
    })
    .await
}

/// `read_file(repo_dir, path, ref)` — `git cat-file blob <ref>:<path>`.
///
/// Returns the file content as a String, truncated to 32KB with a marker
/// (NFR-008). On missing file or empty repo, returns an error string.
pub async fn read_file(
    repo_dir: &Path,
    path: &str,
    r#ref: Option<&str>,
    task_commit_sha: Option<&str>,
    scope: &ToolScope,
) -> Result<String, String> {
    if scope.hides(path) {
        return Ok(scope.refusal(path));
    }
    let repo_dir = repo_dir.to_path_buf();
    let path = path.to_string();
    let ref_arg = r#ref
        .map(str::to_string)
        .unwrap_or_else(|| default_ref(task_commit_sha).to_string());
    run_git_tool("read_file", move || {
        let git = git_bin().map_err(|e| e.to_string())?;
        let spec = format!("{ref_arg}:{path}");
        let out = std::process::Command::new(&git)
            .arg("-C")
            .arg(&repo_dir)
            .arg("cat-file")
            .arg("blob")
            .arg("--end-of-options")
            .arg(&spec)
            .output()
            .map_err(|e| format!("git cat-file: {e}"))?;
        if !out.status.success() {
            let stderr = String::from_utf8_lossy(&out.stderr);
            return Ok(format!(
                "error: could not read file '{path}' at {ref_arg}: {}",
                stderr.trim()
            ));
        }
        let body = String::from_utf8_lossy(&out.stdout).into_owned();
        Ok(truncate(&body, READ_FILE_CAP))
    })
    .await
}

/// `get_diff(repo_dir, base_ref, head_ref)` — `git diff <base>..<head>`.
///
/// `base_ref` defaults to the root commit of `head_ref`
/// (`git rev-list --max-parents=0 <head>`). `head_ref` defaults to the task
/// commit SHA or HEAD. Truncates to 64KB (NFR-008).
pub async fn get_diff(
    repo_dir: &Path,
    base_ref: Option<&str>,
    head_ref: Option<&str>,
    task_commit_sha: Option<&str>,
    scope: &ToolScope,
) -> Result<String, String> {
    let repo_dir = repo_dir.to_path_buf();
    let excludes = scope.pathspecs();
    let head = head_ref
        .map(str::to_string)
        .unwrap_or_else(|| default_ref(task_commit_sha).to_string());
    let base_given = base_ref.map(str::to_string);
    run_git_tool("get_diff", move || {
        let git = git_bin().map_err(|e| e.to_string())?;
        let base = match base_given {
            Some(b) => b,
            None => {
                let out = std::process::Command::new(&git)
                    .arg("-C")
                    .arg(&repo_dir)
                    .arg("rev-list")
                    .arg("--max-parents=0")
                    .arg("--end-of-options")
                    .arg(&head)
                    .output()
                    .map_err(|e| format!("git rev-list: {e}"))?;
                if !out.status.success() || out.stdout.is_empty() {
                    return Ok(format!("error: could not resolve root commit for {head}"));
                }
                String::from_utf8_lossy(&out.stdout)
                    .lines()
                    .next()
                    .unwrap_or("")
                    .trim()
                    .to_string()
            }
        };
        let out = std::process::Command::new(&git)
            .arg("-C")
            .arg(&repo_dir)
            .arg("diff")
            .arg("--end-of-options")
            .arg(format!("{base}..{head}"))
            .args(pathspec_args(&excludes))
            .output()
            .map_err(|e| format!("git diff: {e}"))?;
        if !out.status.success() {
            let stderr = String::from_utf8_lossy(&out.stderr);
            return Ok(format!("error: git diff {base}..{head}: {}", stderr.trim()));
        }
        let body = String::from_utf8_lossy(&out.stdout).into_owned();
        Ok(truncate(&body, DIFF_CAP))
    })
    .await
}

/// `get_last_commit_diff(repo_dir, ref)` — `git show <ref> --format=`.
///
/// Truncates to 64KB (NFR-008).
pub async fn get_last_commit_diff(
    repo_dir: &Path,
    r#ref: Option<&str>,
    task_commit_sha: Option<&str>,
    scope: &ToolScope,
) -> Result<String, String> {
    let repo_dir = repo_dir.to_path_buf();
    let excludes = scope.pathspecs();
    let ref_arg = r#ref
        .map(str::to_string)
        .unwrap_or_else(|| default_ref(task_commit_sha).to_string());
    run_git_tool("get_last_commit_diff", move || {
        let git = git_bin().map_err(|e| e.to_string())?;
        let out = std::process::Command::new(&git)
            .arg("-C")
            .arg(&repo_dir)
            .arg("show")
            .arg("--format=")
            .arg("--end-of-options")
            .arg(&ref_arg)
            .args(pathspec_args(&excludes))
            .output()
            .map_err(|e| format!("git show: {e}"))?;
        if !out.status.success() {
            let stderr = String::from_utf8_lossy(&out.stderr);
            return Ok(format!("error: git show {ref_arg}: {}", stderr.trim()));
        }
        let body = String::from_utf8_lossy(&out.stdout).into_owned();
        Ok(truncate(&body, DIFF_CAP))
    })
    .await
}

/// `find_task_commit(repo_dir, task_id)` —
/// `git log --all --grep='<task_id>' --format='%H %s'`.
pub async fn find_task_commit(repo_dir: &Path, task_id: &str) -> Result<Vec<CommitEntry>, String> {
    let repo_dir = repo_dir.to_path_buf();
    let task_id = task_id.to_string();
    run_git_tool("find_task_commit", move || {
        let git = git_bin().map_err(|e| e.to_string())?;
        let out = std::process::Command::new(&git)
            .arg("-C")
            .arg(&repo_dir)
            .arg("log")
            .arg("--all")
            .arg(format!("--grep={task_id}"))
            .arg("--format=%H %s")
            .output()
            .map_err(|e| format!("git log: {e}"))?;
        if !out.status.success() {
            return Ok(Vec::new());
        }
        let text = String::from_utf8_lossy(&out.stdout);
        Ok(parse_commit_lines(&text))
    })
    .await
}

/// `get_commit_log(repo_dir, ref, limit)` —
/// `git log <ref> --format='%H %s' -n <limit>`.
pub async fn get_commit_log(
    repo_dir: &Path,
    r#ref: Option<&str>,
    limit: Option<u32>,
    task_commit_sha: Option<&str>,
) -> Result<Vec<CommitEntry>, String> {
    let repo_dir = repo_dir.to_path_buf();
    let ref_arg = r#ref
        .map(str::to_string)
        .unwrap_or_else(|| default_ref(task_commit_sha).to_string());
    let limit = limit.unwrap_or(20);
    run_git_tool("get_commit_log", move || {
        let git = git_bin().map_err(|e| e.to_string())?;
        let out = std::process::Command::new(&git)
            .arg("-C")
            .arg(&repo_dir)
            .arg("log")
            .arg("--format=%H %s")
            .arg("-n")
            .arg(limit.to_string())
            .arg("--end-of-options")
            .arg(&ref_arg)
            .output()
            .map_err(|e| format!("git log: {e}"))?;
        if !out.status.success() {
            return Ok(Vec::new());
        }
        let text = String::from_utf8_lossy(&out.stdout);
        Ok(parse_commit_lines(&text))
    })
    .await
}

/// Execute a sync git closure inside `spawn_blocking` with a 10s timeout.
/// On timeout, returns `Err("git read timed out")` (FR-004).
async fn run_git_tool<F, T>(name: &str, f: F) -> Result<T, String>
where
    F: FnOnce() -> Result<T, String> + Send + 'static,
    T: Send + 'static,
{
    let name = name.to_string();
    let join = tokio::task::spawn_blocking(f);
    match timeout(TOOL_TIMEOUT, join).await {
        Ok(Ok(res)) => res,
        Ok(Err(join_err)) => Err(format!("git {name} join error: {join_err}")),
        Err(_) => Err(format!("git {name} timed out")),
    }
}

fn parse_commit_lines(text: &str) -> Vec<CommitEntry> {
    let mut entries = Vec::new();
    for line in text.lines() {
        let line = line.trim_end_matches('\r');
        if line.is_empty() {
            continue;
        }
        if let Some((sha, subject)) = line.split_once(' ') {
            entries.push(CommitEntry {
                sha: sha.to_string(),
                subject: subject.to_string(),
            });
        }
    }
    entries
}

/// Truncate `s` to `cap` bytes, appending a truncation marker if cut.
fn truncate(s: &str, cap: usize) -> String {
    if s.len() <= cap {
        return s.to_string();
    }
    let mut cut = cap;
    while cut > 0 && !s.is_char_boundary(cut) {
        cut -= 1;
    }
    let mut out = String::with_capacity(cut + TRUNC_MARKER.len());
    out.push_str(&s[..cut]);
    out.push_str(TRUNC_MARKER);
    out
}

/// One `git grep` hit: where it matched and the line it matched on.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GrepHit {
    pub path: String,
    pub line: u32,
    /// The matching line, trimmed and capped at [`GREP_LINE_CAP`] chars.
    pub text: String,
}

/// Cap on hits returned per pattern, so one noisy word cannot fill a prompt.
pub const GREP_MAX_HITS: usize = 20;
/// Cap on the reported length of a matching line.
pub const GREP_LINE_CAP: usize = 200;

/// `grep_repo(repo_dir, pattern, ref)` — `git grep -n -I -i -E` over a tree.
///
/// Case-insensitive extended regex, text files only. Used to turn "is there
/// any trace of X in this repository" into a fact with a `path:line` behind
/// it, rather than a question the model answers from the shape of the file
/// listing. A pattern that matches nothing — or an empty repo — returns an
/// empty list, never an error.
pub async fn grep_repo(
    repo_dir: &Path,
    pattern: &str,
    r#ref: Option<&str>,
    scope: &ToolScope,
) -> Result<Vec<GrepHit>, String> {
    let scope_owned = scope.clone();
    let repo_dir = repo_dir.to_path_buf();
    let pattern = pattern.to_string();
    let ref_arg = r#ref
        .map(str::to_string)
        .unwrap_or_else(|| "HEAD".to_string());
    let excludes = scope.pathspecs();
    run_git_tool("grep", move || {
        let git = git_bin().map_err(|e| e.to_string())?;
        let mut cmd = std::process::Command::new(&git);
        cmd.arg("-C")
            .arg(&repo_dir)
            .arg("grep")
            .arg("-n")
            .arg("-I")
            .arg("-i")
            .arg("-E")
            // No `--end-of-options` here: git grep treats everything after
            // it as a path, and the ref is positional. `-e` already keeps a
            // pattern starting with `-` from being read as an option, and the
            // ref is ours, never the model's.
            .arg("--max-count=5")
            .arg("-e")
            .arg(&pattern)
            .arg(&ref_arg);
        for arg in pathspec_args(&excludes) {
            cmd.arg(arg);
        }
        let out = cmd.output().map_err(|e| format!("git grep: {e}"))?;
        // Exit code 1 is "no matches", not a failure.
        let text = String::from_utf8_lossy(&out.stdout);
        let mut hits = Vec::new();
        for line in text.lines() {
            // Format: <ref>:<path>:<line>:<text>
            let rest = line.strip_prefix(&format!("{ref_arg}:")).unwrap_or(line);
            let Some((path, rest)) = rest.split_once(':') else {
                continue;
            };
            let Some((lineno, body)) = rest.split_once(':') else {
                continue;
            };
            if scope_owned.hides(path) {
                continue;
            }
            let body = body.trim();
            let mut cut = GREP_LINE_CAP.min(body.len());
            while cut > 0 && !body.is_char_boundary(cut) {
                cut -= 1;
            }
            hits.push(GrepHit {
                path: path.to_string(),
                line: lineno.parse().unwrap_or(0),
                text: body[..cut].to_string(),
            });
            if hits.len() >= GREP_MAX_HITS {
                break;
            }
        }
        Ok(hits)
    })
    .await
}

/// Tool parameter schema (name + description + JSON-Schema-ish params).
/// The `JudgeLlm` trait consumes these to describe tools to the LLM.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDef {
    pub name: String,
    pub description: String,
    pub params: Value,
    /// The judge's blind spots, carried with the tool it is offered rather
    /// than plumbed through every LLM builder: a tool call is where the rule
    /// has to hold, and rig executes tools out of the caller's sight.
    pub scope: ToolScope,
}

/// A live probe-registration backend, provided by the game-server when the
/// judge may register probes mid-run. Kept as a trait so arena-core stays
/// free of DB/orchestration concerns and the manual test loop can fake it.
#[async_trait::async_trait]
pub trait ProbeRegistrar: Send + Sync {
    /// Handle one `register_probe` tool call. Always returns tool output
    /// (JSON text) — refusals (limits, bad args) are messages to the model,
    /// never run failures.
    async fn register(&self, args: &Value) -> String;
    /// True when an interactive probe was registered this run and its
    /// artifact has not arrived — the run must end `waiting`.
    fn interactive_pending(&self) -> bool;
}

/// The `register_probe` tool definition, offered only when a registrar is
/// wired (i.e. the judge declared `max_interactive` or the task allows it).
pub fn register_probe_def() -> ToolDef {
    let scope = ToolScope::everything();
    ToolDef {
        name: "register_probe".to_string(),
        description: "Register an additional probe when the evidence you have cannot answer \
                      your rubric. BOTH modes ask the PARTICIPANT, on their own machine, and \
                      BOTH return {test_id, status: \"queued\"} rather than a result: nothing \
                      is executed for you here. mode=deterministic asks them to run a shell \
                      command — their toolchain, their dependencies, their running server are \
                      the only place it means what you think it means. mode=interactive asks \
                      them for an artifact (screenshot, recording, export). Either way, give \
                      your best verdict from what you have now; this run is revisited when the \
                      probe resolves or the session ends. Interactive \
                      registrations are strictly limited per task and per judge.                       REUSE FIRST: if artifacts were already delivered for this task the                       call returns their inventory instead of registering — use them when                       they cover your need. If an equivalent request (yours or another                       judge's) is already OPEN, no new request goes out: you are attached                       to the open one and revisited when it lands — the reply carries its                       open_instruction. Repeat the call with confirm:true only for a                       capture the delivered or open requests genuinely do not cover.                       Visual artifacts (image/*, video/*) can only be requested by judges                       whose evidence carries images — others would never see the file."
            .to_string(),
        params: serde_json::json!({
            "type": "object",
            "properties": {
                "mode": {"type": "string", "enum": ["deterministic", "interactive"]},
                "purpose": {"type": "string", "description": "one line: what this probe verifies and why — shown to the player above the command"},
                "command": {"type": "string", "description": "deterministic: shell command to run"},
                "validation": {"type": "string", "description": "deterministic: a JS expression graded as pass when it evaluates to true. In scope: `result` — the command's trimmed stdout as a STRING — and `exit_code` (a number). Example: `exit_code === 0 && result.includes(\"ok\")`."},
                "instruction": {"type": "string", "description": "interactive: what to ask the participant for. Address the participant directly, in short markdown. Name every file they must produce in **bold** (e.g. **desktop.png**) and say what each must show. Do NOT mention git or where to save the files — delivery mechanics are appended automatically."},
                "content_type": {"type": "string", "description": "interactive: expected artifact type, e.g. image/png"},
                "max_bytes": {"type": "integer", "description": "interactive: artifact size cap"},
                "deadline_secs": {"type": "integer", "description": "interactive: ignored — a request stands until the session ends, and the session does not finish while one is open"},
                "confirm": {"type": "boolean", "description": "interactive: set true to insist on a NEW capture after the call returned already-delivered artifacts or attached you to an open request that does not cover your need"}
            },
            "required": ["mode"]
        }),
        // Registration reads nothing from the repo, so it has nothing to hide.
        scope,
    }
}

/// Build the tool definitions for the judge agent loop (FR-003).
pub fn tool_defs(scope: &ToolScope) -> Vec<ToolDef> {
    vec![
        ToolDef {
            name: "get_task_stats".to_string(),
            description: "Get the player's AI-agent implementation statistics for this task: \
                          token usage, user/assistant message counts, per-tool call counts, and \
                          skills loaded, per agent session active during the task's work window. \
                          Client-reported telemetry — treat as supporting evidence, not proof."
                .to_string(),
            params: serde_json::json!({
                "type": "object",
                "properties": {}
            }),
            scope: scope.clone(),
        },
        ToolDef {
            name: "list_files".to_string(),
            description:
                "List all files in the player's repo at a given ref. Returns [{path, size_bytes}]."
                    .to_string(),
            params: serde_json::json!({
                "type": "object",
                "properties": {
                    "ref": {"type": "string", "description": "Git ref (commit SHA or branch). Defaults to the task commit."}
                }
            }),
            scope: scope.clone(),
        },
        ToolDef {
            name: "read_file".to_string(),
            description: "Read a file's content at a given ref. Truncated to 32KB.".to_string(),
            params: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string", "description": "File path relative to repo root."},
                    "ref": {"type": "string", "description": "Git ref. Defaults to the task commit."}
                },
                "required": ["path"]
            }),
            scope: scope.clone(),
        },
        ToolDef {
            name: "get_diff".to_string(),
            description: "Get the diff between two refs. Truncated to 64KB.".to_string(),
            params: serde_json::json!({
                "type": "object",
                "properties": {
                    "base_ref": {"type": "string"},
                    "head_ref": {"type": "string"}
                }
            }),
            scope: scope.clone(),
        },
        ToolDef {
            name: "get_last_commit_diff".to_string(),
            description: "Get the diff of the last commit at a ref. Truncated to 64KB.".to_string(),
            params: serde_json::json!({
                "type": "object",
                "properties": {
                    "ref": {"type": "string"}
                }
            }),
            scope: scope.clone(),
        },
        ToolDef {
            name: "find_task_commit".to_string(),
            description: "Find commits whose message contains a task_id. Returns [{sha, subject}]."
                .to_string(),
            params: serde_json::json!({
                "type": "object",
                "properties": {
                    "task_id": {"type": "string", "description": "UUID of the task to search for."}
                },
                "required": ["task_id"]
            }),
            scope: scope.clone(),
        },
        ToolDef {
            name: "get_commit_log".to_string(),
            description: "Get recent commit log at a ref. Returns [{sha, subject}].".to_string(),
            params: serde_json::json!({
                "type": "object",
                "properties": {
                    "ref": {"type": "string"},
                    "limit": {"type": "integer", "description": "Max commits (default 20)."}
                }
            }),
            scope: scope.clone(),
        },
    ]
}

/// Dispatch a tool call by name with raw JSON args. Returns a string result
/// (data serialized as JSON, or an error message). Bounded by 10s timeout.
/// `task_stats_json` is the prefetched `task_agent_stats` payload for the
/// (session, player, task) being judged — resolved by the caller because
/// this layer has no DB access.
pub async fn dispatch_tool(
    repo_dir: &Path,
    name: &str,
    args: &Value,
    task_commit_sha: Option<&str>,
    task_stats_json: Option<&str>,
    scope: &ToolScope,
) -> String {
    match name {
        "get_task_stats" => task_stats_json.map(str::to_string).unwrap_or_else(|| {
            "no agent implementation statistics were reported for this task \
                 (the player's CLI may not have submitted them yet, or at all)"
                .to_string()
        }),
        "list_files" => {
            let r#ref = args.get("ref").and_then(|v| v.as_str());
            match list_files(repo_dir, r#ref, task_commit_sha, scope).await {
                Ok(v) => serde_json::to_string(&v).unwrap_or_else(|_| "[]".to_string()),
                Err(e) => e,
            }
        }
        "read_file" => {
            let path = args.get("path").and_then(|v| v.as_str()).unwrap_or("");
            let r#ref = args.get("ref").and_then(|v| v.as_str());
            read_file(repo_dir, path, r#ref, task_commit_sha, scope)
                .await
                .unwrap_or_else(|e| e)
        }
        "get_diff" => {
            let base = args.get("base_ref").and_then(|v| v.as_str());
            let head = args.get("head_ref").and_then(|v| v.as_str());
            get_diff(repo_dir, base, head, task_commit_sha, scope)
                .await
                .unwrap_or_else(|e| e)
        }
        "get_last_commit_diff" => {
            let r#ref = args.get("ref").and_then(|v| v.as_str());
            get_last_commit_diff(repo_dir, r#ref, task_commit_sha, scope)
                .await
                .unwrap_or_else(|e| e)
        }
        "find_task_commit" => {
            let task_id = args.get("task_id").and_then(|v| v.as_str()).unwrap_or("");
            match find_task_commit(repo_dir, task_id).await {
                Ok(v) => serde_json::to_string(&v).unwrap_or_else(|_| "[]".to_string()),
                Err(e) => e,
            }
        }
        "get_commit_log" => {
            let r#ref = args.get("ref").and_then(|v| v.as_str());
            let limit = args.get("limit").and_then(|v| v.as_u64()).map(|n| n as u32);
            match get_commit_log(repo_dir, r#ref, limit, task_commit_sha).await {
                Ok(v) => serde_json::to_string(&v).unwrap_or_else(|_| "[]".to_string()),
                Err(e) => e,
            }
        }
        other => format!("error: unknown tool '{other}'"),
    }
}
