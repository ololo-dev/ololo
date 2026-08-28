//! Command rendering: substitute fixture values into `{{name}}` placeholders.

use std::collections::HashMap;

use minijinja::ErrorKind;

use crate::probe_engine::error::ProbeEngineError;
use crate::probe_engine::fixtures::FixtureSample;
use crate::task_template::{FixtureDef, FixtureKind};

/// Render `template` by substituting `{{name}}` placeholders with fixture values.
///
/// For `NumericRange` fixtures the scalar `value` is substituted.  For `KeyValue`
/// fixtures the sampled `key` is substituted (not the value), so that the command
/// carries the look-up term (e.g. a country name) rather than the answer.
///
/// Requires `defs` to identify `KeyValue` fixtures; `samples` must have been
/// produced by [`sample_fixtures`] on those same defs.
pub fn render_command(
    template: &str,
    defs: &[FixtureDef],
    samples: &HashMap<String, FixtureSample>,
) -> Result<String, ProbeEngineError> {
    render_command_with_memory(template, defs, samples, &Default::default())
}

/// Like [`render_command`], additionally binding `memory` as a nested object
/// so `{{memory.key}}` resolves. Memory values must already be shell-quoted
/// by the caller (they are player-authored).
pub fn render_command_with_memory(
    template: &str,
    defs: &[FixtureDef],
    samples: &HashMap<String, FixtureSample>,
    memory_quoted: &std::collections::BTreeMap<String, String>,
) -> Result<String, ProbeEngineError> {
    let mut ctx = build_scalar_context(defs, samples);
    insert_memory_object(&mut ctx, memory_quoted);
    render_with_minijinja(template, &ctx)
}

/// Like [`render_command`] but accepts a pre-built `name → string` map of already
/// shell-quoted scalars (used by callers that need to shell-quote before rendering).
pub fn render_command_quoted(
    template: &str,
    quoted_scalars: &HashMap<String, String>,
) -> Result<String, ProbeEngineError> {
    render_command_quoted_with_memory(template, quoted_scalars, &Default::default())
}

/// Like [`render_command_quoted`], additionally binding `memory` as a nested
/// object so `{{memory.key}}` resolves. Memory values must already be
/// shell-quoted by the caller.
pub fn render_command_quoted_with_memory(
    template: &str,
    quoted_scalars: &HashMap<String, String>,
    memory_quoted: &std::collections::BTreeMap<String, String>,
) -> Result<String, ProbeEngineError> {
    let mut ctx: serde_json::Value = quoted_scalars
        .iter()
        .map(|(k, v)| (k.clone(), serde_json::Value::String(v.clone())))
        .collect::<serde_json::Map<_, _>>()
        .into();
    insert_memory_object(&mut ctx, memory_quoted);
    render_with_minijinja(template, &ctx)
}

/// Render `{{name}}` / `{{memory.key}}` placeholders with quoting decided by
/// where each one sits in the *shell* text around it.
///
/// A single quoting pass cannot be right for every position: templates write
/// both `$R "{list}"` (inside double quotes) and `R={memory.run}` (bare).
/// Pre-quoting every value with shlex made the first case deliver literal
/// quotes to the player's program — `'97,390'`, commas trip shlex — while
/// not quoting at all would leave the second case open to word-splitting.
/// So the template is scanned as shell text, and each substitution escapes
/// for its own context:
///
/// - bare (unquoted) position → `shlex::try_quote`, exactly as before;
/// - inside `"…"` → backslash-escape the four characters the shell treats
///   specially there (`\`, `"`, `$`, `` ` ``);
/// - inside `'…'` → the `'\''` close-insert-reopen idiom.
///
/// Backslash escapes in the template are honoured (`\"` inside a
/// double-quoted string does not close it — the anagram tasks rely on that).
/// An unknown placeholder is an [`ProbeEngineError::UndefinedVariable`],
/// matching the Strict minijinja behavior of the other render paths.
pub fn render_command_shell_aware(
    template: &str,
    scalars: &HashMap<String, String>,
    memory: &std::collections::BTreeMap<String, String>,
) -> Result<String, ProbeEngineError> {
    #[derive(Clone, Copy, PartialEq)]
    enum Ctx {
        Bare,
        Single,
        Double,
    }

    let lookup = |token: &str| -> Option<&str> {
        match token.strip_prefix("memory.") {
            Some(key) => memory.get(key).map(String::as_str),
            None => scalars.get(token).map(String::as_str),
        }
    };
    let escape = |value: &str, ctx: Ctx, token: &str| -> Result<String, ProbeEngineError> {
        Ok(match ctx {
            Ctx::Bare => shlex::try_quote(value)
                .map_err(|_| {
                    ProbeEngineError::SyntaxError(format!(
                        "value of {{{token}}} cannot be shell-quoted"
                    ))
                })?
                .into_owned(),
            Ctx::Double => {
                let mut out = String::with_capacity(value.len());
                for c in value.chars() {
                    if matches!(c, '\\' | '"' | '$' | '`') {
                        out.push('\\');
                    }
                    out.push(c);
                }
                out
            }
            Ctx::Single => value.replace('\'', "'\\''"),
        })
    };

    let mut out = String::with_capacity(template.len() + 16);
    let mut ctx = Ctx::Bare;
    let mut rest = template;
    while let Some(c) = rest.chars().next() {
        // Placeholders substitute in any context; everything else is shell
        // text that only drives the context tracking.
        if ctx != Ctx::Single && c == '\\' {
            let escaped_len = 1 + rest[1..].chars().next().map_or(0, char::len_utf8);
            out.push_str(&rest[..escaped_len]);
            rest = &rest[escaped_len..];
            continue;
        }
        if let Some(inner) = rest.strip_prefix("{{") {
            let Some(end) = inner.find("}}") else {
                return Err(ProbeEngineError::SyntaxError(
                    "unclosed {{placeholder}} in command template".to_string(),
                ));
            };
            let token = inner[..end].trim();
            let Some(value) = lookup(token) else {
                return Err(ProbeEngineError::UndefinedVariable(token.to_string()));
            };
            out.push_str(&escape(value, ctx, token)?);
            rest = &inner[end + 2..];
            continue;
        }
        match (ctx, c) {
            (Ctx::Bare, '\'') => ctx = Ctx::Single,
            (Ctx::Bare, '"') => ctx = Ctx::Double,
            (Ctx::Single, '\'') => ctx = Ctx::Bare,
            (Ctx::Double, '"') => ctx = Ctx::Bare,
            _ => {}
        }
        let len = c.len_utf8();
        out.push_str(&rest[..len]);
        rest = &rest[len..];
    }
    Ok(out)
}

/// Bind `memory` in a render context as a nested object (skipped when the
/// map is empty so templates without memory keep Strict-undefined behavior
/// for a stray `{{memory.x}}`).
fn insert_memory_object(
    ctx: &mut serde_json::Value,
    memory: &std::collections::BTreeMap<String, String>,
) {
    if memory.is_empty() {
        return;
    }
    if let Some(map) = ctx.as_object_mut() {
        let obj: serde_json::Map<String, serde_json::Value> = memory
            .iter()
            .map(|(k, v)| (k.clone(), serde_json::Value::String(v.clone())))
            .collect();
        map.insert("memory".to_string(), serde_json::Value::Object(obj));
    }
}

/// Normalize `{name}` placeholders into `{{name}}` for known variable names,
/// leaving existing `{{name}}` templates untouched.
pub fn normalize_brace_placeholders(template: &str, names: &[String]) -> String {
    let set: std::collections::HashSet<&str> = names.iter().map(String::as_str).collect();
    let bytes = template.as_bytes();
    let mut out = String::with_capacity(template.len() + 16);
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'{' {
            if i + 1 < bytes.len() && bytes[i + 1] == b'{' {
                out.push_str("{{");
                i += 2;
                continue;
            }
            let mut j = i + 1;
            while j < bytes.len() && bytes[j] != b'}' && bytes[j] != b'{' {
                j += 1;
            }
            // A nested `{` before any `}` means the outer `{` opens a shell
            // brace group (`|| { echo ...; }`), not a placeholder. Emit it
            // literally and rescan from the inner `{`, so placeholders inside
            // the group still normalize.
            if j < bytes.len() && bytes[j] == b'{' {
                out.push_str(&template[i..j]);
                i = j;
                continue;
            }
            if j < bytes.len() {
                let token = &template[i + 1..j];
                if set.contains(token.trim()) {
                    out.push_str("{{");
                    out.push_str(token.trim());
                    out.push_str("}}");
                } else {
                    out.push('{');
                    out.push_str(token);
                    out.push('}');
                }
                i = j + 1;
                continue;
            }
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}

/// Build a scalar-only context: `X` → string suitable for command rendering.
/// For `KeyValue` fixtures, the scalar is the sampled key.
/// For `NumericRange` fixtures, the scalar is the number string.
pub(crate) fn build_scalar_context(
    defs: &[FixtureDef],
    samples: &HashMap<String, FixtureSample>,
) -> serde_json::Value {
    let mut map = serde_json::Map::new();
    for def in defs {
        if let Some(s) = samples.get(&def.name) {
            let scalar = match &def.kind {
                FixtureKind::KeyValue { .. } => s.key.as_deref().unwrap_or(&s.value).to_string(),
                FixtureKind::NumericRange { .. } => s.value.clone(),
            };
            map.insert(def.name.clone(), serde_json::Value::String(scalar));
        }
    }
    serde_json::Value::Object(map)
}

pub(crate) fn render_with_minijinja(
    template: &str,
    ctx: &serde_json::Value,
) -> Result<String, ProbeEngineError> {
    let mut env = minijinja::Environment::new();
    env.set_auto_escape_callback(|_| minijinja::AutoEscape::None);
    env.set_undefined_behavior(minijinja::UndefinedBehavior::Strict);
    env.render_str(template, ctx).map_err(minijinja_err)
}

pub(crate) fn minijinja_err(e: minijinja::Error) -> ProbeEngineError {
    match e.kind() {
        ErrorKind::UndefinedError => ProbeEngineError::UndefinedVariable(e.to_string()),
        _ => ProbeEngineError::SyntaxError(e.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::probe_engine::test_util::{fixed, kv, numeric};

    #[test]
    fn renders_numeric_value() {
        let defs = vec![numeric("n", 5, 5)];
        let samples = fixed("n", "5");
        assert_eq!(
            render_command("echo {{n}}", &defs, &samples).unwrap(),
            "echo 5"
        );
    }

    #[test]
    fn renders_kv_key_not_value() {
        let defs = vec![kv("country", &[("France", "Paris")])];
        let mut samples = HashMap::new();
        samples.insert(
            "country".to_string(),
            FixtureSample {
                key: Some("France".to_string()),
                value: "Paris".to_string(),
            },
        );
        assert_eq!(
            render_command("q={{country}}", &defs, &samples).unwrap(),
            "q=France"
        );
    }

    #[test]
    fn undefined_var_is_error() {
        let defs = vec![numeric("n", 1, 1)];
        let samples = fixed("n", "1");
        assert!(render_command("echo {{missing}}", &defs, &samples).is_err());
    }

    #[test]
    fn normalizes_single_brace() {
        let names = vec!["a".to_string(), "b".to_string()];
        assert_eq!(
            normalize_brace_placeholders("echo {a}-{b}", &names),
            "echo {{a}}-{{b}}"
        );
    }

    #[test]
    fn brace_group_does_not_swallow_placeholders() {
        // `|| { echo ... }` is a shell brace group: the `{` before `echo`
        // must stay literal while the `{doneFile}` inside it still renders.
        let names = vec!["doneFile".to_string()];
        assert_eq!(
            normalize_brace_placeholders(
                "test -f {doneFile} || { echo \"missing {doneFile}\"; exit 0; }",
                &names
            ),
            "test -f {{doneFile}} || { echo \"missing {{doneFile}}\"; exit 0; }"
        );
    }

    #[test]
    fn normalizes_dotted_memory_token() {
        let names = vec!["memory.command".to_string()];
        assert_eq!(
            normalize_brace_placeholders("{memory.command} -q {id}", &names),
            "{{memory.command}} -q {id}"
        );
    }

    #[test]
    fn renders_memory_object_quoted_branch() {
        let mut memory = std::collections::BTreeMap::new();
        memory.insert("command".to_string(), "'sh answer.sh'".to_string());
        let scalars = HashMap::from([("id".to_string(), "abc".to_string())]);
        assert_eq!(
            render_command_quoted_with_memory("{{memory.command}} -q {{id}}", &scalars, &memory)
                .unwrap(),
            "'sh answer.sh' -q abc"
        );
    }

    #[test]
    fn renders_memory_object_fixture_branch() {
        let defs = vec![numeric("n", 5, 5)];
        let samples = fixed("n", "5");
        let mut memory = std::collections::BTreeMap::new();
        memory.insert("port".to_string(), "8088".to_string());
        assert_eq!(
            render_command_with_memory("curl :{{memory.port}}/{{n}}", &defs, &samples, &memory)
                .unwrap(),
            "curl :8088/5"
        );
    }

    #[test]
    fn missing_memory_key_is_strict_error() {
        let mut memory = std::collections::BTreeMap::new();
        memory.insert("port".to_string(), "8088".to_string());
        let scalars = HashMap::new();
        assert!(render_command_quoted_with_memory("{{memory.nope}}", &scalars, &memory).is_err());
    }

    mod shell_aware {
        use super::*;

        fn scalars(pairs: &[(&str, &str)]) -> HashMap<String, String> {
            pairs
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect()
        }

        fn memory(pairs: &[(&str, &str)]) -> std::collections::BTreeMap<String, String> {
            pairs
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect()
        }

        /// The regression this renderer exists for: a comma-carrying list
        /// substituted inside the template's own double quotes used to arrive
        /// wrapped in literal shlex quotes (`"'97,390'"`), so every correct
        /// array answer was graded against a corrupted input.
        #[test]
        fn value_inside_template_double_quotes_is_not_requoted() {
            let s = scalars(&[("list", "97,390,244")]);
            let m = memory(&[("run", "sh answer.sh")]);
            assert_eq!(
                render_command_shell_aware("R={{memory.run}}\n$R \"{{list}}\"", &s, &m).unwrap(),
                "R='sh answer.sh'\n$R \"97,390,244\""
            );
        }

        #[test]
        fn bare_position_is_shlex_quoted() {
            let s = scalars(&[("safe", "abc"), ("listy", "1,2"), ("spacey", "a b")]);
            let m = memory(&[]);
            assert_eq!(
                render_command_shell_aware("x {{safe}} {{listy}} {{spacey}}", &s, &m).unwrap(),
                "x abc '1,2' 'a b'"
            );
        }

        #[test]
        fn double_quote_context_escapes_shell_specials() {
            let s = scalars(&[("v", r#"a"b$c`d\e"#)]);
            assert_eq!(
                render_command_shell_aware(r#"echo "{{v}}""#, &s, &memory(&[])).unwrap(),
                r#"echo "a\"b\$c\`d\\e""#
            );
        }

        #[test]
        fn single_quote_context_uses_close_insert_reopen() {
            let s = scalars(&[("v", "it's")]);
            assert_eq!(
                render_command_shell_aware("echo '{{v}}'", &s, &memory(&[])).unwrap(),
                r#"echo 'it'\''s'"#
            );
        }

        /// `\"` inside a double-quoted string does not close it — the anagram
        /// templates embed escaped quotes around one placeholder while later
        /// placeholders are still inside the same string.
        #[test]
        fn escaped_quote_does_not_flip_context() {
            let s = scalars(&[("word", "listen"), ("options", "inlets, google")]);
            assert_eq!(
                render_command_shell_aware(
                    r#"$R "anagram of \"{{word}}\": {{options}}""#,
                    &s,
                    &memory(&[]),
                )
                .unwrap(),
                r#"$R "anagram of \"listen\": inlets, google""#
            );
        }

        #[test]
        fn context_tracking_spans_plain_shell_text() {
            // Quote state must survive text between placeholders: after the
            // closing quote the next placeholder is bare again.
            let s = scalars(&[("a", "x,y"), ("b", "p q")]);
            assert_eq!(
                render_command_shell_aware(r#"f "{{a}}" {{b}}"#, &s, &memory(&[])).unwrap(),
                r#"f "x,y" 'p q'"#
            );
        }

        #[test]
        fn unknown_placeholder_is_strict_error() {
            let err = render_command_shell_aware("echo {{nope}}", &scalars(&[]), &memory(&[]))
                .unwrap_err();
            assert!(matches!(err, ProbeEngineError::UndefinedVariable(_)));
        }

        #[test]
        fn unclosed_placeholder_is_an_error() {
            assert!(
                render_command_shell_aware("echo {{oops", &scalars(&[]), &memory(&[])).is_err()
            );
        }

        #[test]
        fn non_ascii_template_text_passes_through() {
            let s = scalars(&[("n", "7")]);
            assert_eq!(
                render_command_shell_aware("echo «π≈{{n}}»", &s, &memory(&[])).unwrap(),
                "echo «π≈7»"
            );
        }
    }
}
