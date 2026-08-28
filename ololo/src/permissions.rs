//! Probe-command permission gate.
//!
//! Probes are shell commands sent by the platform and executed on the
//! player's machine, so the CLI asks before running one it has not seen
//! approved. The player can Allow once, Always allow (persists a rule), or
//! Decline (the probe is reported as failed without running).
//!
//! Approvals live in `.ololo/settings.json` in the workspace, shaped like a
//! Claude Code settings file:
//!
//! ```json
//! { "permissions": { "allow": ["sh answer.sh *"], "deny": [] } }
//! ```
//!
//! Rules match the rendered command: `*` alone matches everything, a
//! trailing `*` is a prefix match, anything else is exact. `deny` wins over
//! `allow` and declines without prompting. The file is re-read before every
//! probe, so hand edits apply immediately.

use anyhow::{Context, Result};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// What the player (or a rule) decided about one command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    Allow,
    AlwaysAllow,
    /// Approve this probe and every later one until the session ends —
    /// in-memory only, nothing is written to `.ololo/settings.json`.
    AllowAllSession,
    Decline,
}

/// "Approve all for the session": a process-local switch, deliberately not
/// persisted — the next session asks again.
static SESSION_ALLOW_ALL: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn allow_all_for_session() {
    SESSION_ALLOW_ALL.store(true, std::sync::atomic::Ordering::Relaxed);
}

pub fn session_allow_all() -> bool {
    SESSION_ALLOW_ALL.load(std::sync::atomic::Ordering::Relaxed)
}

/// What the settings file says about a command before any prompt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Allowed,
    Denied,
    Ask,
}

/// `<workspace>/.ololo/settings.json` — the workspace is the CLI's cwd.
pub fn settings_path() -> PathBuf {
    std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join(".ololo")
        .join("settings.json")
}

/// Check `command` against the workspace settings file. A `deny` rule still
/// wins over the session-wide approval — deny is an explicit human "never".
pub fn check(command: &str) -> Verdict {
    let (allow, deny) = load_lists(&settings_path());
    let v = verdict(&allow, &deny, command);
    if v == Verdict::Ask && session_allow_all() {
        return Verdict::Allowed;
    }
    v
}

/// Persist the run-everything rule — the `--allow-all` opt-in.
pub fn record_allow_all() -> Result<()> {
    record_allow("*")
}

/// Warn — loudly, before any session exists — when this run cannot approve
/// probes. Headless play with a non-TTY stdin auto-declines every command no
/// allow rule covers, and each decline is scored as a failed probe; players
/// discovered that from a stack of `-5`s in the log. Silent when a prompt is
/// possible or at least one allow rule exists (rules are prefix matches, so
/// one rule usually covers the session's whole probe family).
pub fn warn_if_cannot_approve_probes() {
    use std::io::IsTerminal;
    if std::io::stdin().is_terminal() {
        return;
    }
    let (allow, _) = load_lists(&settings_path());
    if !allow.is_empty() {
        return;
    }
    let path = settings_path();
    crate::ui::warn(
        "no terminal and no pre-approved commands: every probe will be DECLINED and scored as a failure",
    );
    crate::ui::warn(format!(
        "either allow the session's commands up front in {}:",
        path.display()
    ));
    crate::ui::warn(r#"  {"permissions":{"allow":["*"],"deny":[]}}"#);
    crate::ui::warn("or re-run with --allow-all to write that file now");
}

fn verdict(allow: &[String], deny: &[String], command: &str) -> Verdict {
    if deny.iter().any(|rule| rule_matches(rule, command)) {
        return Verdict::Denied;
    }
    if allow.iter().any(|rule| rule_matches(rule, command)) {
        return Verdict::Allowed;
    }
    Verdict::Ask
}

/// `*` matches everything; a trailing `*` is a prefix match; else exact.
fn rule_matches(rule: &str, command: &str) -> bool {
    let rule = rule.trim();
    if rule == "*" {
        return true;
    }
    match rule.strip_suffix('*') {
        Some(prefix) => command.starts_with(prefix),
        None => command == rule,
    }
}

/// The rule "Always allow" persists. Probe commands embed randomized
/// fixture values, so an exact command would never match again; the stable
/// part is how the command starts. Two-word prefix (`sh answer.sh *`)
/// covers the common `runner script args…` shape; a command of one or two
/// words is stored exactly.
pub fn always_rule(command: &str) -> String {
    let words: Vec<&str> = command.split_whitespace().collect();
    if words.len() <= 2 {
        command.trim().to_string()
    } else {
        format!("{} {} *", words[0], words[1])
    }
}

/// Append `rule` to `permissions.allow`, creating `.ololo/settings.json`
/// if needed. Unknown keys in the file are preserved.
pub fn record_allow(rule: &str) -> Result<()> {
    record_allow_at(&settings_path(), rule)
}

fn record_allow_at(path: &Path, rule: &str) -> Result<()> {
    let mut root: Value = match std::fs::read_to_string(path) {
        Ok(text) => serde_json::from_str(&text)
            .with_context(|| format!("{} is not valid JSON", path.display()))?,
        Err(_) => json!({}),
    };
    if !root.is_object() {
        anyhow::bail!("{} must contain a JSON object", path.display());
    }
    let permissions = root
        .as_object_mut()
        .expect("checked above")
        .entry("permissions")
        .or_insert_with(|| json!({}));
    if !permissions.is_object() {
        anyhow::bail!("\"permissions\" in {} must be an object", path.display());
    }
    let allow = permissions
        .as_object_mut()
        .expect("checked above")
        .entry("allow")
        .or_insert_with(|| json!([]));
    let Some(list) = allow.as_array_mut() else {
        anyhow::bail!(
            "\"permissions.allow\" in {} must be an array",
            path.display()
        );
    };
    if !list.iter().any(|v| v.as_str() == Some(rule)) {
        list.push(Value::String(rule.to_string()));
    }

    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    let mut text = serde_json::to_string_pretty(&root)?;
    text.push('\n');
    std::fs::write(path, text).with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}

fn load_lists(path: &Path) -> (Vec<String>, Vec<String>) {
    let Ok(text) = std::fs::read_to_string(path) else {
        return (Vec::new(), Vec::new());
    };
    let Ok(root) = serde_json::from_str::<Value>(&text) else {
        tracing::warn!("{} is not valid JSON; ignoring it", path.display());
        return (Vec::new(), Vec::new());
    };
    let list = |key: &str| -> Vec<String> {
        root.get("permissions")
            .and_then(|p| p.get(key))
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default()
    };
    (list("allow"), list("deny"))
}

/// Prompt on the controlling terminal — the plain-text and headless paths.
/// A non-TTY stdin declines immediately: an unattended run cannot consent
/// (seed `.ololo/settings.json` with `{"permissions":{"allow":["*"]}}` for
/// automation). Times out to Decline when the probe deadline passes.
pub async fn prompt_via_stdin(command: &str, rule: &str, deadline: Duration) -> Decision {
    use std::io::IsTerminal;
    if !std::io::stdin().is_terminal() {
        crate::ui::warn(
            "probe declined: no terminal to ask for permission \
             (allow commands in .ololo/settings.json to run unattended)",
        );
        return Decision::Decline;
    }

    let command = command.to_string();
    let rule = rule.to_string();
    let read = tokio::task::spawn_blocking(move || {
        use std::io::BufRead;
        eprintln!();
        eprintln!("  The session wants to run this command in your workspace:");
        eprintln!("    {command}");
        eprintln!(
            "  [a] allow once   [w] always allow ({rule})   [s] approve all for the session   [d] decline"
        );
        eprint!("  > ");
        let mut line = String::new();
        if std::io::stdin().lock().read_line(&mut line).is_err() {
            return Decision::Decline;
        }
        match line.trim().to_lowercase().as_str() {
            "a" | "y" | "allow" | "yes" => Decision::Allow,
            "w" | "always" => Decision::AlwaysAllow,
            "s" | "session" => Decision::AllowAllSession,
            _ => Decision::Decline,
        }
    });
    match tokio::time::timeout(deadline, read).await {
        Ok(Ok(decision)) => decision,
        // Timeout leaves the blocking read holding stdin until the next
        // newline — a cosmetic leak; the probe deadline has passed anyway.
        _ => {
            crate::ui::warn("permission prompt timed out — probe declined");
            Decision::Decline
        }
    }
}

/// Headless-mode responder for a TUI-bus permission request: prompt on
/// stdin, then answer through the request's channel. Persisting an
/// always-allow rule is the `player_ws` gate's job when the answer arrives.
pub async fn respond_from_stdin(prompt: crate::tui::event::PermissionPrompt) {
    let deadline = Duration::from_secs(prompt.deadline_secs.max(1) as u64);
    let decision = prompt_via_stdin(&prompt.command, &prompt.always_rule, deadline).await;
    if let Ok(mut guard) = prompt.responder.lock()
        && let Some(tx) = guard.take()
    {
        let _ = tx.send(decision);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rule_matching_exact_prefix_and_star() {
        assert!(rule_matches("*", "anything at all"));
        assert!(rule_matches(
            "sh answer.sh *",
            "sh answer.sh -q \"5 plus 3\""
        ));
        assert!(!rule_matches("sh answer.sh *", "bash evil.sh"));
        assert!(rule_matches("git status", "git status"));
        assert!(!rule_matches("git status", "git status --short"));
    }

    #[test]
    fn deny_wins_over_allow() {
        let allow = vec!["*".to_string()];
        let deny = vec!["rm *".to_string()];
        assert_eq!(verdict(&allow, &deny, "rm -rf /"), Verdict::Denied);
        assert_eq!(verdict(&allow, &deny, "ls"), Verdict::Allowed);
        assert_eq!(verdict(&[], &[], "ls"), Verdict::Ask);
    }

    #[test]
    fn always_rule_is_two_word_prefix() {
        assert_eq!(
            always_rule("sh answer.sh -q \"what is 5 plus 3\""),
            "sh answer.sh *"
        );
        assert_eq!(always_rule("git status"), "git status");
        assert_eq!(always_rule("make"), "make");
    }

    #[test]
    fn record_allow_creates_updates_dedupes_and_preserves() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".ololo").join("settings.json");

        record_allow_at(&path, "sh answer.sh *").unwrap();
        record_allow_at(&path, "sh answer.sh *").unwrap();
        record_allow_at(&path, "node run.js *").unwrap();
        let (allow, deny) = load_lists(&path);
        assert_eq!(allow, vec!["sh answer.sh *", "node run.js *"]);
        assert!(deny.is_empty());

        // Unknown keys and deny entries survive a rewrite.
        std::fs::write(
            &path,
            r#"{"custom": true, "permissions": {"allow": ["a *"], "deny": ["rm *"]}}"#,
        )
        .unwrap();
        record_allow_at(&path, "b *").unwrap();
        let root: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(root["custom"], true);
        assert_eq!(root["permissions"]["deny"][0], "rm *");
        let (allow, deny) = load_lists(&path);
        assert_eq!(allow, vec!["a *", "b *"]);
        assert_eq!(deny, vec!["rm *"]);
    }

    /// `--allow-all` writes the same rule the matcher treats as
    /// run-everything, and adding it twice keeps the list clean.
    #[test]
    fn allow_all_rule_covers_everything_and_dedupes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".ololo").join("settings.json");

        record_allow_at(&path, "*").unwrap();
        record_allow_at(&path, "*").unwrap();
        let (allow, deny) = load_lists(&path);
        assert_eq!(allow, vec!["*"]);
        assert!(deny.is_empty());
        assert_eq!(
            verdict(&allow, &deny, "sh answer.sh -q \"anything\""),
            Verdict::Allowed
        );
    }

    #[test]
    fn malformed_settings_fail_closed_to_ask() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(&path, "not json").unwrap();
        let (allow, deny) = load_lists(&path);
        assert!(allow.is_empty() && deny.is_empty());
        // record_allow refuses to clobber a file it cannot parse.
        assert!(record_allow_at(&path, "x").is_err());
    }
}

#[cfg(test)]
mod session_allow_tests {
    use super::*;

    #[test]
    fn session_wide_approval_turns_ask_into_allowed_but_not_deny() {
        // Empty rule lists: everything is Ask until the switch flips.
        assert_eq!(verdict(&[], &[], "sh answer.sh"), Verdict::Ask);
        allow_all_for_session();
        assert!(session_allow_all());
        // check() consults the switch only for Ask — an explicit deny rule
        // still wins.
        let denied = verdict(&[], &["sh answer.sh".to_string()], "sh answer.sh");
        assert_eq!(denied, Verdict::Denied);
    }
}
