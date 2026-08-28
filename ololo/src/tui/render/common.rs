//! Shared helpers for render submodules.

use agent_tokens::SessionCounts;

/// Extract `(provider_id, model_id)` from a session's `model` JSON blob.
/// Falls back to `"—"` when the model is absent or not parseable.
pub(crate) fn provider_model(s: &SessionCounts) -> (String, String) {
    match &s.model {
        Some(m) => match serde_json::from_str::<serde_json::Value>(m) {
            Ok(v) => (
                v.get("providerID")
                    .and_then(|p| p.as_str())
                    .unwrap_or("—")
                    .to_string(),
                v.get("id")
                    .and_then(|i| i.as_str())
                    .unwrap_or(m)
                    .to_string(),
            ),
            Err(_) => ("—".to_string(), m.clone()),
        },
        None => ("—".to_string(), "—".to_string()),
    }
}

/// Greedy word-wrap `s` to lines of at most `width` chars. Words longer
/// than `width` are hard-broken. Newlines in `s` force line breaks.
/// Always returns at least one (possibly empty) line.
pub(crate) fn wrap_text(s: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut lines: Vec<String> = Vec::new();
    for raw_line in s.lines() {
        let mut cur = String::new();
        for word in raw_line.split_whitespace() {
            let mut word_len = word.chars().count();
            let mut word = word;
            // Hard-break an over-long word into width-sized chunks.
            while word_len > width {
                if !cur.is_empty() {
                    lines.push(std::mem::take(&mut cur));
                }
                let head: String = word.chars().take(width).collect();
                lines.push(head.clone());
                word = &word[head.len()..];
                word_len -= width;
            }
            let cur_len = cur.chars().count();
            if !cur.is_empty() && cur_len + 1 + word_len > width {
                lines.push(std::mem::take(&mut cur));
            }
            if word.is_empty() {
                continue;
            }
            if !cur.is_empty() {
                cur.push(' ');
            }
            cur.push_str(word);
        }
        // Skip a stray empty tail left after hard-breaking, but keep
        // genuinely blank lines.
        if !cur.is_empty() || raw_line.trim().is_empty() {
            lines.push(cur);
        }
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}

/// Truncate `s` to `width` chars, appending an ellipsis when cut.
pub(crate) fn truncate(s: &str, width: usize) -> String {
    if s.chars().count() > width {
        let mut out = s.chars().take(width.saturating_sub(1)).collect::<String>();
        out.push('…');
        out
    } else {
        s.to_string()
    }
}

/// Compact token count matching the web stats tab: 957 → "957",
/// 90_500 → "90.5k", 2_400_000 → "2.4M".
pub(crate) fn fmt_tokens(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.1}k", n as f64 / 1_000.0)
    } else {
        n.to_string()
    }
}

/// "Xm Ys" wall-clock formatting for the stats summary ("0m 38s").
pub(crate) fn fmt_mins_secs(secs: u64) -> String {
    format!("{}m {}s", secs / 60, secs % 60)
}

#[cfg(test)]
mod tests {
    use super::{fmt_mins_secs, fmt_tokens, wrap_text};

    #[test]
    fn fmt_tokens_matches_web_stats_tab() {
        assert_eq!(fmt_tokens(0), "0");
        assert_eq!(fmt_tokens(957), "957");
        assert_eq!(fmt_tokens(1_000), "1.0k");
        assert_eq!(fmt_tokens(90_500), "90.5k");
        assert_eq!(fmt_tokens(2_400_000), "2.4M");
    }

    #[test]
    fn fmt_mins_secs_formats_minutes_and_seconds() {
        assert_eq!(fmt_mins_secs(0), "0m 0s");
        assert_eq!(fmt_mins_secs(38), "0m 38s");
        assert_eq!(fmt_mins_secs(125), "2m 5s");
    }

    #[test]
    fn wrap_short_line_is_untouched() {
        assert_eq!(wrap_text("hello world", 20), vec!["hello world"]);
    }

    #[test]
    fn wrap_breaks_on_word_boundaries() {
        assert_eq!(
            wrap_text("one two three four", 9),
            vec!["one two", "three", "four"]
        );
    }

    #[test]
    fn wrap_hard_breaks_overlong_words() {
        assert_eq!(wrap_text("abcdefgh", 3), vec!["abc", "def", "gh"]);
    }

    #[test]
    fn wrap_preserves_all_content() {
        // Width chosen so no single word exceeds it (no hard-breaks).
        let text = "a description that is fairly long and wraps a few times";
        let joined = wrap_text(text, 12).join(" ");
        assert_eq!(joined, text);
    }

    #[test]
    fn wrap_empty_input_yields_one_empty_line() {
        assert_eq!(wrap_text("", 10), vec![""]);
    }

    #[test]
    fn wrap_respects_embedded_newlines() {
        assert_eq!(wrap_text("a\nb", 10), vec!["a", "b"]);
    }
}
