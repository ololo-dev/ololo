//! The shell probe and test commands run under.
//!
//! Every project writes its probes as POSIX `sh` scripts (`#` comments,
//! `mkdir -p`, `sed`, `$R -q …`). On Windows `cmd.exe` cannot run a single
//! one of them: each probe "ran" in a few milliseconds, printed nothing and
//! failed, and the player watched ololo grade nothing they built. Windows
//! players almost always have a POSIX shell anyway — Claude Code on Windows
//! requires Git for Windows — so find its `sh.exe` and use that; `cmd.exe`
//! stays only as the last resort, announced by [`missing_warning`].

use std::path::{Path, PathBuf};

/// A command running `script` under the POSIX shell.
pub fn command(script: &str) -> tokio::process::Command {
    #[cfg(unix)]
    {
        let mut cmd = tokio::process::Command::new("sh");
        cmd.args(["-c", script]);
        cmd
    }
    #[cfg(windows)]
    {
        match windows_sh() {
            Some(sh) => {
                let mut cmd = tokio::process::Command::new(sh);
                cmd.args(["-c", script]);
                cmd
            }
            None => {
                let mut cmd = tokio::process::Command::new("cmd.exe");
                cmd.args(["/C", script]);
                cmd
            }
        }
    }
}

/// What to tell the player when no POSIX shell was found — `None` on unix,
/// and on Windows when Git Bash (or another `sh`) is there.
pub fn missing_warning() -> Option<&'static str> {
    #[cfg(windows)]
    if windows_sh().is_none() {
        return Some(
            "No POSIX shell found (Git Bash's sh.exe): probe commands are sh scripts and will \
             fail under cmd.exe. Install Git for Windows, or point CLAUDE_CODE_GIT_BASH_PATH \
             at its bash.exe, then restart ololo.",
        );
    }
    None
}

#[cfg(windows)]
fn windows_sh() -> Option<&'static Path> {
    static SH: std::sync::OnceLock<Option<PathBuf>> = std::sync::OnceLock::new();
    SH.get_or_init(|| {
        let env = |k: &str| std::env::var_os(k).map(PathBuf::from);
        let candidates = sh_candidates(
            env("CLAUDE_CODE_GIT_BASH_PATH"),
            which::which("git").ok(),
            which::which("sh").ok(),
            [
                env("ProgramFiles"),
                env("ProgramFiles(x86)"),
                env("LOCALAPPDATA").map(|p| p.join("Programs")),
            ]
            .into_iter()
            .flatten()
            .collect(),
        );
        candidates.into_iter().find(|p| p.is_file())
    })
    .as_deref()
}

/// Where a POSIX shell may live on Windows, best first:
/// 1. `CLAUDE_CODE_GIT_BASH_PATH` — the bash Claude Code itself was told to use;
/// 2. the Git install that owns the `git` on PATH (`<root>\cmd\git.exe` →
///    `<root>\bin\sh.exe`, the wrapper that puts the coreutils on PATH);
/// 3. an `sh` on PATH (MSYS2, Cygwin, Git's `usr\bin`) — never one under
///    `System32`, which is WSL's and runs in another filesystem;
/// 4. Git's default install directories.
#[cfg_attr(not(windows), allow(dead_code))]
fn sh_candidates(
    bash_override: Option<PathBuf>,
    git_on_path: Option<PathBuf>,
    sh_on_path: Option<PathBuf>,
    install_roots: Vec<PathBuf>,
) -> Vec<PathBuf> {
    let mut out = Vec::new();
    out.extend(bash_override);
    if let Some(git) = git_on_path {
        // git.exe sits in `cmd\`, `bin\` or `mingw64\bin\` of the install:
        // the root is two or three levels above the file.
        for root in git.ancestors().skip(2).take(2) {
            out.push(root.join("bin").join("sh.exe"));
        }
    }
    if let Some(sh) = sh_on_path.filter(|p| !is_system32(p)) {
        out.push(sh);
    }
    for root in install_roots {
        out.push(root.join("Git").join("bin").join("sh.exe"));
    }
    out
}

#[cfg_attr(not(windows), allow(dead_code))]
fn is_system32(path: &Path) -> bool {
    path.components().any(|c| {
        c.as_os_str()
            .to_str()
            .is_some_and(|s| s.eq_ignore_ascii_case("system32"))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> PathBuf {
        PathBuf::from(s)
    }

    #[test]
    fn the_git_on_path_leads_to_its_bin_sh() {
        let c = sh_candidates(None, Some(p("/Git/cmd/git.exe")), None, vec![]);
        assert_eq!(c[0], p("/Git/bin/sh.exe"));
        let c = sh_candidates(None, Some(p("/Git/mingw64/bin/git.exe")), None, vec![]);
        assert!(c.contains(&p("/Git/bin/sh.exe")), "{c:?}");
    }

    #[test]
    fn claude_codes_bash_comes_first_and_install_dirs_last() {
        let c = sh_candidates(
            Some(p("/custom/bash.exe")),
            Some(p("/Git/cmd/git.exe")),
            Some(p("/msys64/usr/bin/sh.exe")),
            vec![p("/Program Files")],
        );
        assert_eq!(c.first(), Some(&p("/custom/bash.exe")));
        assert_eq!(c.last(), Some(&p("/Program Files/Git/bin/sh.exe")));
        assert!(c.contains(&p("/msys64/usr/bin/sh.exe")));
    }

    #[test]
    fn wsl_bash_in_system32_is_never_a_candidate() {
        let c = sh_candidates(None, None, Some(p("/Windows/System32/sh.exe")), vec![]);
        assert!(c.is_empty(), "{c:?}");
    }

    #[cfg(unix)]
    #[test]
    fn unix_runs_sh_and_never_warns() {
        assert!(missing_warning().is_none());
        let cmd = command("echo hi");
        assert_eq!(cmd.as_std().get_program(), "sh");
    }
}
