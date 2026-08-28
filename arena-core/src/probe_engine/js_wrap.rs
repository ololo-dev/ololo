//! JS script wrapping: turn a loose predicate/object script into an IIFE
//! that returns the trailing expression, respecting top-level `return`, `;`,
//! and block boundaries.

pub fn wrap_js(script: &str) -> String {
    let t = script.trim();
    // A `return` at brace depth 0 means the script is already written as
    // a block body with its own return (e.g. `return got === expected`).
    // A `return` nested inside an arrow/inner-function body does NOT
    // count — the top-level still needs the trailing expression returned.
    // ponytail: brace-depth scan; doesn't handle template-literal braces
    // or comments, but validation templates here are plain JS expressions.
    if has_top_level_return(t) {
        return format!("(() => {{\n{t}\n}})()");
    }
    if let Some(split) = last_top_level_statement_boundary(t) {
        // Block body: ensure the trailing expression statement is returned
        // so predicate templates like `const code = ...; code >= 200 && code < 500`
        // evaluate. A "statement boundary" is either a top-level `;` or the
        // closing `}` of a top-level block (e.g. a `for(...){...}` body) —
        // whichever comes last. This avoids splitting inside a `for(;;)`
        // header or an inner arrow body.
        // ponytail: depth-aware scan over ()[]{} groupings; template-literal
        // braces/comments unsupported, validation templates are plain JS.
        let (head, expr) = match split.kind {
            BoundaryKind::Semi => (&t[..split.idx], &t[split.idx + 1..]),
            BoundaryKind::BlockClose => (&t[..split.idx + 1], &t[split.idx + 1..]),
        };
        let expr = expr.trim();
        if expr.is_empty() {
            format!("(() => {{\n{t}\n}})()")
        } else {
            format!("(() => {{\n{head}\nreturn ({expr});\n}})()")
        }
    } else {
        format!("(() => ({t}))()")
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum BoundaryKind {
    Semi,
    BlockClose,
}

struct Boundary {
    idx: usize,
    kind: BoundaryKind,
}

/// Find the last top-level statement boundary in `script`: either a `;` at
/// the top level, or the closing `}` of a top-level block (e.g. a `for` body
/// or standalone block statement). Returns the boundary so the caller can
/// split `head|trailing-expression`.
/// ponytail: tracks `()`, `[]`, `{}` depth together so `;` inside a `for(;;)`
/// header or array literal is not mistaken for a top-level terminator, and
/// the `}` closing a `for`/`while` body is recognised as a statement boundary.
/// Template-literal braces and comments are not handled — validation
/// templates in this codebase are plain JS expressions.
fn last_top_level_statement_boundary(script: &str) -> Option<Boundary> {
    let bytes = script.as_bytes();
    let mut depth = 0i32;
    let mut last: Option<Boundary> = None;
    let mut i = 0usize;
    while i < bytes.len() {
        let b = bytes[i];
        match b {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => {
                let was_depth = depth;
                depth -= 1;
                // A `}` that returns depth to 0 closes a top-level block
                // statement (for/while/if body, standalone block). Treat it
                // as a statement boundary so a trailing expression after the
                // block is returned, e.g. `for(...){...} assertEqual(...)`.
                if was_depth == 1 && depth == 0 && b == b'}' {
                    last = Some(Boundary {
                        idx: i,
                        kind: BoundaryKind::BlockClose,
                    });
                }
            }
            b'"' | b'\'' | b'`' => {
                let quote = b;
                i += 1;
                while i < bytes.len() && bytes[i] != quote {
                    if bytes[i] == b'\\' {
                        i += 1;
                    }
                    i += 1;
                }
            }
            _ => {
                if depth == 0 && b == b';' {
                    last = Some(Boundary {
                        idx: i,
                        kind: BoundaryKind::Semi,
                    });
                }
            }
        }
        i += 1;
    }
    last
}

/// True if `script` contains a `return` statement at brace depth 0
/// (not nested inside a `{ ... }` body like an arrow function).
fn has_top_level_return(script: &str) -> bool {
    let bytes = script.as_bytes();
    let mut depth = 0i32;
    let mut i = 0usize;
    while i < bytes.len() {
        let b = bytes[i];
        match b {
            b'{' => depth += 1,
            b'}' => depth -= 1,
            b'"' | b'\'' => {
                // Skip string literal to avoid counting braces/returns inside it.
                let quote = b;
                i += 1;
                while i < bytes.len() && bytes[i] != quote {
                    if bytes[i] == b'\\' {
                        i += 1;
                    }
                    i += 1;
                }
            }
            _ => {
                if depth == 0
                    && b == b'r'
                    && bytes[i..].starts_with(b"return")
                    && (i + 6 >= bytes.len()
                        || !bytes[i + 6].is_ascii_alphanumeric() && bytes[i + 6] != b'_')
                {
                    return true;
                }
            }
        }
        i += 1;
    }
    false
}

pub(crate) fn normalize_validation_script(script: &str) -> String {
    let mut s = script.trim().to_string();
    if let Some(rest) = s.strip_prefix("assert ") {
        s = rest.trim().to_string();
    }
    // `result` is the stdout string, but judge-authored validations keep
    // reaching for `result.exit_code`; steer that spelling onto the real
    // `exit_code` global instead of letting it read `undefined`.
    s = s.replace("result.exit_code", "exit_code");
    replace_braced_tokens(&s)
}

fn replace_braced_tokens(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'{' {
            let mut j = i + 1;
            while j < bytes.len() && bytes[j] != b'}' {
                j += 1;
            }
            if j < bytes.len() {
                let token = s[i + 1..j].trim();
                if token
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.')
                {
                    out.push_str(token);
                    i = j + 1;
                    continue;
                }
            }
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expression_body_for_plain_object() {
        let w = wrap_js("({ a: 1 })");
        assert!(w.starts_with("(() => ("));
    }

    #[test]
    fn top_level_return_uses_block_body() {
        let w = wrap_js("return got === expected");
        assert!(w.starts_with("(() => {"));
        assert!(!w.contains("return ("));
    }

    #[test]
    fn returns_trailing_predicate_after_block() {
        let script = "let a = 0; for (let i = 0; i < n; i++) { a += i; } got === expected";
        let w = wrap_js(script);
        assert!(w.contains("return (got === expected)"));
    }

    #[test]
    fn has_top_level_return_detects_only_brace_depth_zero() {
        let nested = "const f = x => { return x; }; y";
        assert!(!has_top_level_return(nested));
        assert!(has_top_level_return("return y"));
    }
}
