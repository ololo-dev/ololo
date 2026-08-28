//! Agent launch specs.
//!
//! `--agent` and `--launch` accept a full command line, not just a binary
//! name: `--agent "claude --model glm-5.2:cloud"`. The first token is the
//! program (resolved on `$PATH`); the rest are passed to it verbatim.

use anyhow::{Result, bail};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentCommand {
    /// First token as given (bare name or path); resolved via `which` later.
    pub program: String,
    pub args: Vec<String>,
}

impl AgentCommand {
    pub fn parse(spec: &str) -> Result<Self> {
        let tokens = split_command(spec)?;
        let Some((program, args)) = tokens.split_first() else {
            bail!("agent command is empty");
        };
        Ok(Self {
            program: program.clone(),
            args: args.to_vec(),
        })
    }

    /// Bare program name (file stem) — what gets reported to the server as
    /// the agent and matched against known-agent lists (`agent_kind`,
    /// antigravity sync). `/usr/local/bin/claude` and `claude` both → "claude".
    pub fn program_name(&self) -> &str {
        std::path::Path::new(&self.program)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or(&self.program)
    }

    /// The command line for display: program plus args, space-joined.
    pub fn display(&self) -> String {
        std::iter::once(self.program.as_str())
            .chain(self.args.iter().map(String::as_str))
            .collect::<Vec<_>>()
            .join(" ")
    }
}

/// Split a command line into tokens: whitespace-separated, with `'…'` /
/// `"…"` quoting and `\` escapes outside single quotes. Errors on an
/// unterminated quote so a typo fails loudly instead of launching a
/// half-parsed command.
fn split_command(s: &str) -> Result<Vec<String>> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut in_token = false;
    let mut chars = s.chars().peekable();

    while let Some(c) = chars.next() {
        match c {
            c if c.is_whitespace() => {
                if in_token {
                    tokens.push(std::mem::take(&mut current));
                    in_token = false;
                }
            }
            '\'' | '"' => {
                in_token = true;
                let quote = c;
                loop {
                    match chars.next() {
                        None => bail!("unterminated {quote} quote in agent command: {s:?}"),
                        Some(c) if c == quote => break,
                        Some('\\') if quote == '"' => match chars.next() {
                            None => bail!("trailing backslash in agent command: {s:?}"),
                            Some(e) => current.push(e),
                        },
                        Some(c) => current.push(c),
                    }
                }
            }
            '\\' => {
                in_token = true;
                match chars.next() {
                    None => bail!("trailing backslash in agent command: {s:?}"),
                    Some(e) => current.push(e),
                }
            }
            c => {
                in_token = true;
                current.push(c);
            }
        }
    }
    if in_token {
        tokens.push(current);
    }
    Ok(tokens)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bare_name_has_no_args() {
        let cmd = AgentCommand::parse("claude").unwrap();
        assert_eq!(cmd.program, "claude");
        assert!(cmd.args.is_empty());
        assert_eq!(cmd.program_name(), "claude");
        assert_eq!(cmd.display(), "claude");
    }

    #[test]
    fn command_with_args_splits() {
        let cmd = AgentCommand::parse("claude --model glm-5.2:cloud").unwrap();
        assert_eq!(cmd.program, "claude");
        assert_eq!(cmd.args, vec!["--model", "glm-5.2:cloud"]);
        assert_eq!(cmd.display(), "claude --model glm-5.2:cloud");
    }

    #[test]
    fn quoted_arg_keeps_spaces() {
        let cmd = AgentCommand::parse(r#"claude --system-prompt "be fast, win""#).unwrap();
        assert_eq!(cmd.args, vec!["--system-prompt", "be fast, win"]);
    }

    #[test]
    fn single_quotes_are_literal() {
        // Backslash inside single quotes is a plain character, so this
        // quote closes at the final tick and the backslash survives.
        let cmd = AgentCommand::parse(r"agent -m 'it\'").unwrap();
        assert_eq!(cmd.args, vec!["-m", r"it\"]);
        let cmd = AgentCommand::parse(r"agent -m 'a \n b'").unwrap();
        assert_eq!(cmd.args, vec!["-m", r"a \n b"]);
    }

    #[test]
    fn escaped_space_outside_quotes() {
        let cmd = AgentCommand::parse(r"my\ agent --go").unwrap();
        assert_eq!(cmd.program, "my agent");
        assert_eq!(cmd.args, vec!["--go"]);
    }

    #[test]
    fn empty_and_whitespace_fail() {
        assert!(AgentCommand::parse("").is_err());
        assert!(AgentCommand::parse("   ").is_err());
    }

    #[test]
    fn unterminated_quote_fails() {
        assert!(AgentCommand::parse(r#"claude --model "glm"#).is_err());
    }

    #[test]
    fn program_name_strips_path_and_extension() {
        let cmd = AgentCommand::parse("/usr/local/bin/claude --verbose").unwrap();
        assert_eq!(cmd.program_name(), "claude");
        // `\` is not a path separator on unix, so only the extension is
        // testable cross-platform here.
        let cmd = AgentCommand::parse("codex.exe run").unwrap();
        assert_eq!(cmd.program_name(), "codex");
    }

    #[test]
    fn empty_quoted_arg_is_preserved() {
        let cmd = AgentCommand::parse(r#"agent --flag """#).unwrap();
        assert_eq!(cmd.args, vec!["--flag", ""]);
    }
}
