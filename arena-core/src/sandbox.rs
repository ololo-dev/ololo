//! Lightweight process sandbox for executing untrusted player code
//! server-side (execution judges).
//!
//! The platform's trust model is otherwise "no server-side sandbox — tests
//! run on the joiner's machine" (see `README.md`). Execution judges are the
//! one place the server runs committed player code, so it must be isolated.
//!
//! Two backends:
//! - [`SandboxBackend::Bubblewrap`] (Linux, production): each run is wrapped
//!   in `bwrap` with all namespaces unshared (no network, own PID ns), a
//!   scrubbed environment, the host system dirs bound read-only, and only the
//!   scratch working directory writable. This is the intended backend.
//! - [`SandboxBackend::Unsandboxed`] (dev/CI, e.g. macOS where `bwrap` does
//!   not exist): a plain `sh -c` with a scrubbed env and a wall-clock
//!   deadline, **no isolation**. Never use with untrusted code in production;
//!   `detect_backend` only selects it when `bwrap` is absent, and callers log
//!   a warning.
//!
//! Isolation is defense-in-depth, not a guarantee: `bwrap` provides no CPU or
//! memory cap, so the wall-clock deadline (which kills the whole PID namespace
//! via the `bwrap` parent) is the backstop against runaway code.

use std::path::Path;
use std::time::{Duration, Instant};

use crate::protocol::STDOUT_TAIL_MAX_BYTES;

/// Which isolation backend a run uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SandboxBackend {
    /// `bwrap`-wrapped, namespaced, no network. Production.
    Bubblewrap,
    /// Bare `sh -c`, scrubbed env + deadline only. Dev/CI fallback.
    Unsandboxed,
}

/// Result of a sandboxed command.
#[derive(Debug, Clone)]
pub struct ExecOutput {
    /// Tail-truncated, trailing-whitespace-trimmed stdout.
    pub stdout: String,
    /// Tail-truncated stderr. Captured (not discarded) because it is the only
    /// evidence of *why* a run produced no stdout — a broken sandbox, a
    /// missing interpreter, and a wrong answer are otherwise indistinguishable.
    pub stderr: String,
    /// Process exit code; `-1` on timeout or missing status.
    pub exit_code: i32,
    pub duration_ms: u64,
    pub timed_out: bool,
}

/// Stderr is diagnostic only — keep the tail small.
const STDERR_TAIL_MAX_BYTES: usize = 2048;

/// Token the [`self_check`] canary expects back from the sandbox.
pub const CANARY_TOKEN: &str = "ololo-sandbox-ok";

#[derive(Debug, thiserror::Error)]
pub enum SandboxError {
    #[error("spawn failed: {0}")]
    Spawn(String),
    #[error("io error: {0}")]
    Io(String),
}

/// Default read-only host directories exposed to sandboxed code. Chosen to
/// cover common interpreter/tool locations while excluding `/home`, `/root`,
/// `/var`, and `/tmp` (the git-repo store and any on-disk secrets live under
/// those). Override with `OLOLO_SANDBOX_ROBIND` (colon-separated).
const DEFAULT_ROBINDS: &[&str] = &[
    "/usr", "/bin", "/sbin", "/lib", "/lib64", "/etc", "/opt", "/nix",
];

fn robind_dirs() -> Vec<String> {
    match std::env::var("OLOLO_SANDBOX_ROBIND") {
        Ok(v) if !v.trim().is_empty() => v
            .split(':')
            .map(|s| s.to_string())
            .filter(|s| !s.is_empty())
            .collect(),
        _ => DEFAULT_ROBINDS.iter().map(|s| s.to_string()).collect(),
    }
}

/// Select the sandbox backend. Honors `OLOLO_SANDBOX` (`bwrap`|`none`); else
/// uses `bwrap` when it is on `PATH`, otherwise the unsandboxed fallback.
pub fn detect_backend() -> SandboxBackend {
    match std::env::var("OLOLO_SANDBOX").ok().as_deref() {
        Some("bwrap") | Some("bubblewrap") => return SandboxBackend::Bubblewrap,
        Some("none") | Some("unsandboxed") => return SandboxBackend::Unsandboxed,
        _ => {}
    }
    if which::which("bwrap").is_ok() {
        SandboxBackend::Bubblewrap
    } else {
        SandboxBackend::Unsandboxed
    }
}

/// Build the `bwrap` argv that runs `sh -c <command>` with `workdir` as the
/// only writable path. Pure (no process spawn) so the isolation flags are
/// unit-testable. Returns the full argv including the leading `bwrap`.
pub fn bwrap_argv(command: &str, workdir: &Path, robinds: &[String]) -> Vec<String> {
    let wd = workdir.to_string_lossy().to_string();
    let mut a: Vec<String> = vec![
        "bwrap".into(),
        // No network, own PID/IPC/UTS/user/mount namespaces.
        "--unshare-all".into(),
        // Child dies when the parent (this bwrap) is killed on timeout —
        // reaps the whole PID namespace, our defense against persistence.
        "--die-with-parent".into(),
        "--new-session".into(),
        // Scrub the environment; provide a minimal PATH only.
        "--clearenv".into(),
        "--setenv".into(),
        "PATH".into(),
        "/usr/bin:/bin:/usr/local/bin:/usr/sbin:/sbin".into(),
        "--setenv".into(),
        "HOME".into(),
        wd.clone(),
    ];
    for dir in robinds {
        if Path::new(dir).exists() {
            a.push("--ro-bind".into());
            a.push(dir.clone());
            a.push(dir.clone());
        }
    }
    // Virtual filesystems and a writable scratch tmpfs.
    a.push("--proc".into());
    a.push("/proc".into());
    a.push("--dev".into());
    a.push("/dev".into());
    a.push("--tmpfs".into());
    a.push("/tmp".into());
    // The materialized solution: the only writable host path.
    a.push("--bind".into());
    a.push(wd.clone());
    a.push(wd.clone());
    a.push("--chdir".into());
    a.push(wd);
    a.push("--".into());
    a.push("sh".into());
    a.push("-c".into());
    a.push(command.to_string());
    a
}

/// Run `command` against `workdir` under `backend`, capturing trimmed stdout,
/// exit code, and duration, killed at `deadline`.
pub async fn run(
    backend: SandboxBackend,
    command: &str,
    workdir: &Path,
    deadline: Duration,
) -> Result<ExecOutput, SandboxError> {
    let start = Instant::now();

    let mut cmd = match backend {
        SandboxBackend::Bubblewrap => {
            let argv = bwrap_argv(command, workdir, &robind_dirs());
            let mut c = tokio::process::Command::new(&argv[0]);
            c.args(&argv[1..]);
            c
        }
        SandboxBackend::Unsandboxed => {
            let mut c = tokio::process::Command::new("sh");
            c.args(["-c", command]);
            c.current_dir(workdir);
            // Scrub inherited env even in the fallback: no leaked secrets.
            c.env_clear();
            c.env("PATH", "/usr/bin:/bin:/usr/local/bin:/usr/sbin:/sbin");
            c.env("HOME", workdir);
            c
        }
    };
    cmd.stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .stdin(std::process::Stdio::null());
    // Own process group so a timeout kill takes the whole tree (the
    // unsandboxed fallback has no PID namespace to rely on).
    #[cfg(unix)]
    cmd.process_group(0);

    let mut child = cmd
        .spawn()
        .map_err(|e| SandboxError::Spawn(e.to_string()))?;
    let stdout_pipe = child.stdout.take().expect("stdout piped");
    let stderr_pipe = child.stderr.take().expect("stderr piped");

    // Drain both pipes concurrently — reading them in sequence deadlocks as
    // soon as the child fills the pipe we are not reading yet. Uses `spawn`
    // rather than `tokio::join!` so this crate needs no `macros` feature.
    let stderr_task = tokio::spawn(async move {
        use tokio::io::AsyncReadExt;
        let mut err = Vec::new();
        let mut reader = tokio::io::BufReader::new(stderr_pipe);
        let _ = reader.read_to_end(&mut err).await;
        err
    });

    let collect = async move {
        use tokio::io::AsyncReadExt;
        let mut buf = Vec::new();
        let mut reader = tokio::io::BufReader::new(stdout_pipe);
        reader.read_to_end(&mut buf).await.map(|_| buf)
    };

    let stdout_bytes = match tokio::time::timeout(deadline, collect).await {
        Ok(Ok(bytes)) => bytes,
        Ok(Err(e)) => {
            stderr_task.abort();
            return Err(SandboxError::Io(e.to_string()));
        }
        Err(_elapsed) => {
            kill_tree(&mut child).await;
            stderr_task.abort();
            return Ok(ExecOutput {
                stdout: String::new(),
                stderr: String::new(),
                exit_code: -1,
                duration_ms: start.elapsed().as_millis() as u64,
                timed_out: true,
            });
        }
    };

    let status = child
        .wait()
        .await
        .map_err(|e| SandboxError::Io(e.to_string()))?;
    // Safe to await now: the child has exited, so its stderr pipe is closed
    // and the drain task has reached EOF.
    let stderr_bytes = stderr_task.await.unwrap_or_default();
    let duration_ms = start.elapsed().as_millis() as u64;

    let tail = |bytes: &[u8], max: usize| -> String {
        let raw = if bytes.len() > max {
            &bytes[bytes.len() - max..]
        } else {
            bytes
        };
        String::from_utf8_lossy(raw).trim_end().to_string()
    };

    Ok(ExecOutput {
        stdout: tail(&stdout_bytes, STDOUT_TAIL_MAX_BYTES),
        stderr: tail(&stderr_bytes, STDERR_TAIL_MAX_BYTES),
        exit_code: status.code().unwrap_or(-1),
        duration_ms,
        timed_out: false,
    })
}

/// Verify the backend can actually execute a command, by round-tripping
/// [`CANARY_TOKEN`] through it. Returns `Err(diagnostic)` when it cannot.
///
/// This exists because a *present but non-functional* sandbox is the dangerous
/// failure mode: `bwrap` on `PATH` in a container without permission to create
/// user namespaces exits non-zero with empty stdout for every command, which a
/// caller grading on stdout alone reads as "the code under test is wrong".
/// Callers must run this before attributing any empty output to the code.
pub async fn self_check(backend: SandboxBackend, workdir: &Path) -> Result<(), String> {
    let command = format!("printf %s {CANARY_TOKEN}");
    let out = run(backend, &command, workdir, Duration::from_secs(10))
        .await
        .map_err(|e| format!("canary could not be spawned: {e}"))?;

    if out.stdout.trim() == CANARY_TOKEN {
        return Ok(());
    }

    let mut why = format!(
        "sandbox cannot execute commands: canary returned stdout {:?} (exit {})",
        out.stdout, out.exit_code
    );
    if out.timed_out {
        why.push_str(", timed out");
    }
    if !out.stderr.is_empty() {
        why.push_str(&format!("; stderr: {}", out.stderr));
    }
    Err(why)
}

#[cfg(unix)]
async fn kill_tree(child: &mut tokio::process::Child) {
    // Best-effort: signal the process group, then the child itself.
    if let Some(pid) = child.id() {
        // Negative pid targets the group created by process_group(0).
        unsafe {
            libc::kill(-(pid as i32), libc::SIGKILL);
        }
    }
    let _ = child.kill().await;
    let _ = child.wait().await;
}

#[cfg(not(unix))]
async fn kill_tree(child: &mut tokio::process::Child) {
    let _ = child.kill().await;
    let _ = child.wait().await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bwrap_argv_isolates_network_and_scopes_writable_dir() {
        let argv = bwrap_argv("echo hi", Path::new("/scratch/x"), &[]);
        assert_eq!(argv[0], "bwrap");
        assert!(
            argv.contains(&"--unshare-all".to_string()),
            "no network/namespaces"
        );
        assert!(argv.contains(&"--clearenv".to_string()), "env scrubbed");
        assert!(argv.contains(&"--die-with-parent".to_string()));
        // The scratch dir is bound writable and is the working dir.
        let bind = argv
            .windows(3)
            .any(|w| w == ["--bind", "/scratch/x", "/scratch/x"]);
        assert!(bind, "scratch dir bound writable");
        let chdir = argv.windows(2).any(|w| w == ["--chdir", "/scratch/x"]);
        assert!(chdir, "chdir into scratch");
        // The command is the final `sh -c <command>`.
        let n = argv.len();
        assert_eq!(
            &argv[n - 3..],
            &["sh".to_string(), "-c".to_string(), "echo hi".to_string()]
        );
    }

    #[test]
    fn detect_backend_honors_env_override() {
        // Note: relies on process-global env; safe as a single assertion.
        unsafe { std::env::set_var("OLOLO_SANDBOX", "none") };
        assert_eq!(detect_backend(), SandboxBackend::Unsandboxed);
        unsafe { std::env::set_var("OLOLO_SANDBOX", "bwrap") };
        assert_eq!(detect_backend(), SandboxBackend::Bubblewrap);
        unsafe { std::env::remove_var("OLOLO_SANDBOX") };
    }

    #[tokio::test]
    async fn unsandboxed_runs_command_and_captures_stdout() {
        let dir = tempfile::tempdir().unwrap();
        let out = run(
            SandboxBackend::Unsandboxed,
            "printf 'hello-%s' world",
            dir.path(),
            Duration::from_secs(5),
        )
        .await
        .unwrap();
        assert_eq!(out.stdout, "hello-world");
        assert_eq!(out.exit_code, 0);
        assert!(!out.timed_out);
    }

    #[tokio::test]
    async fn unsandboxed_honors_workdir_and_scrubs_env() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("answer.js"), "x").unwrap();
        // Working dir is the scratch dir (file is visible); a secret in the
        // parent env is not.
        unsafe { std::env::set_var("OLOLO_TEST_SECRET", "leaked") };
        let out = run(
            SandboxBackend::Unsandboxed,
            "test -f answer.js && printf 'found'; printf ':%s' \"$OLOLO_TEST_SECRET\"",
            dir.path(),
            Duration::from_secs(5),
        )
        .await
        .unwrap();
        unsafe { std::env::remove_var("OLOLO_TEST_SECRET") };
        assert_eq!(out.stdout, "found:", "workdir file visible, env scrubbed");
    }

    #[tokio::test]
    async fn deadline_times_out_runaway_command() {
        let dir = tempfile::tempdir().unwrap();
        let out = run(
            SandboxBackend::Unsandboxed,
            "sleep 10",
            dir.path(),
            Duration::from_millis(300),
        )
        .await
        .unwrap();
        assert!(out.timed_out);
        assert_eq!(out.exit_code, -1);
    }
}
