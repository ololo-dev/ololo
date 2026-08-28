//! Shared command normalization and splitting policy for adaptation paths.

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandPolicyError {
    EmptyCommand,
    InvalidMarkdownFence,
    MultiConcernAmbiguous,
}

impl CommandPolicyError {
    pub fn as_code(&self) -> &'static str {
        match self {
            CommandPolicyError::EmptyCommand => "empty_command",
            CommandPolicyError::InvalidMarkdownFence => "invalid_markdown_fence",
            CommandPolicyError::MultiConcernAmbiguous => "multi_concern_ambiguous",
        }
    }
}

pub fn normalize_shell_command(raw: &str) -> Result<String, CommandPolicyError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(CommandPolicyError::EmptyCommand);
    }

    if let Some(rest) = trimmed.strip_prefix("```") {
        let body = if let Some(newline_idx) = rest.find('\n') {
            let after_open = &rest[newline_idx + 1..];
            after_open
                .strip_suffix("```")
                .map(str::trim)
                .ok_or(CommandPolicyError::InvalidMarkdownFence)?
                .to_string()
        } else {
            return Err(CommandPolicyError::InvalidMarkdownFence);
        };

        if body.is_empty() {
            return Err(CommandPolicyError::EmptyCommand);
        }
        return Ok(body);
    }

    let without_inline_ticks =
        if trimmed.starts_with('`') && trimmed.ends_with('`') && trimmed.len() >= 2 {
            trimmed[1..trimmed.len() - 1].trim().to_string()
        } else {
            trimmed.to_string()
        };

    if without_inline_ticks.is_empty() {
        return Err(CommandPolicyError::EmptyCommand);
    }

    Ok(without_inline_ticks)
}

pub fn split_when_safe(command: &str) -> Result<Vec<String>, CommandPolicyError> {
    let normalized = normalize_shell_command(command)?;

    if normalized.contains('\n')
        || normalized.contains("||")
        || normalized.contains('|')
        || normalized.contains(';')
        || contains_single_ampersand(&normalized)
    {
        return Err(CommandPolicyError::MultiConcernAmbiguous);
    }

    if !normalized.contains("&&") {
        return Ok(vec![normalized]);
    }

    let parts = split_on_unquoted_and_and(&normalized);

    if parts.len() < 2 {
        return Err(CommandPolicyError::MultiConcernAmbiguous);
    }

    Ok(parts)
}

fn split_on_unquoted_and_and(input: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut in_single = false;
    let mut in_double = false;
    let mut chars = input.chars().peekable();

    while let Some(ch) = chars.next() {
        match ch {
            '\'' if !in_double => {
                in_single = !in_single;
                current.push(ch);
            }
            '"' if !in_single => {
                in_double = !in_double;
                current.push(ch);
            }
            '&' if !in_single && !in_double && chars.peek() == Some(&'&') => {
                let _ = chars.next();
                let trimmed = current.trim();
                if !trimmed.is_empty() {
                    parts.push(trimmed.to_string());
                }
                current.clear();
            }
            _ => current.push(ch),
        }
    }

    let trimmed = current.trim();
    if !trimmed.is_empty() {
        parts.push(trimmed.to_string());
    }

    parts
}

fn contains_single_ampersand(input: &str) -> bool {
    let bytes = input.as_bytes();
    for i in 0..bytes.len() {
        if bytes[i] != b'&' {
            continue;
        }
        let left_is_amp = i > 0 && bytes[i - 1] == b'&';
        let right_is_amp = i + 1 < bytes.len() && bytes[i + 1] == b'&';
        if !left_is_amp && !right_is_amp {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::{CommandPolicyError, normalize_shell_command, split_when_safe};

    #[test]
    fn strips_markdown_fence() {
        let normalized =
            normalize_shell_command("```bash\ntest -f ./README.md\n```").expect("normalized");
        assert_eq!(normalized, "test -f ./README.md");
    }

    #[test]
    fn rejects_invalid_fence() {
        let err =
            normalize_shell_command("```bash\ntest -f ./README.md").expect_err("invalid fence");
        assert_eq!(err, CommandPolicyError::InvalidMarkdownFence);
    }

    #[test]
    fn splits_simple_and_chain() {
        let split = split_when_safe("test -f ./AGENTS.md && test -f ./README.md").expect("split");
        assert_eq!(split, vec!["test -f ./AGENTS.md", "test -f ./README.md"]);
    }

    #[test]
    fn rejects_ambiguous_split() {
        let err = split_when_safe("cat ./AGENTS.md | grep -q TODO || cat ./README.md")
            .expect_err("ambiguous");
        assert_eq!(err, CommandPolicyError::MultiConcernAmbiguous);
    }

    #[test]
    fn rejects_background_ampersand_split() {
        let err =
            split_when_safe("test -f ./AGENTS.md & test -f ./README.md").expect_err("ambiguous");
        assert_eq!(err, CommandPolicyError::MultiConcernAmbiguous);
    }

    #[test]
    fn rejects_multiline_command_split() {
        let err =
            split_when_safe("test -f ./AGENTS.md\ntest -f ./README.md").expect_err("ambiguous");
        assert_eq!(err, CommandPolicyError::MultiConcernAmbiguous);
    }

    #[test]
    fn does_not_split_quoted_and_and() {
        let split = split_when_safe("printf 'a && b' && test -f ./README.md").expect("split");
        assert_eq!(split, vec!["printf 'a && b'", "test -f ./README.md"]);
    }
}
