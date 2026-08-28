use serde_json::Value;

/// Detect a skill load from a tool invocation.
///
/// Two signals, in priority order:
/// 1. A dedicated skill tool (Claude Code's `Skill`) — the skill name is in the args.
/// 2. Any file-read tool whose args contain a path ending in `SKILL.md` — the
///    skill name is the parent directory (the convention every surveyed agent
///    uses: cursor/pi/omp/copilot load skills by reading `<name>/SKILL.md`).
pub fn skill_from_tool_call(tool: &str, args: &Value) -> Option<String> {
    if tool.eq_ignore_ascii_case("skill") {
        for key in ["skill", "name", "command"] {
            if let Some(s) = args.get(key).and_then(|v| v.as_str()) {
                return Some(s.trim_start_matches('/').to_string());
            }
        }
        return None;
    }
    find_skill_md_path(args)
}

/// Recursively scan all string values for a path ending in `SKILL.md` and
/// return its parent directory name. Recursive because agents nest the path
/// differently (kiro: `operations[].path`, claude: `file_path`, cursor: `path`).
fn find_skill_md_path(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => {
            let s = s.trim();
            if s.ends_with("SKILL.md") && s.len() > "SKILL.md".len() {
                let p = std::path::Path::new(s);
                return p
                    .parent()
                    .and_then(|d| d.file_name())
                    .map(|n| n.to_string_lossy().into_owned());
            }
            None
        }
        Value::Array(a) => a.iter().find_map(find_skill_md_path),
        Value::Object(o) => o.values().find_map(find_skill_md_path),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn skill_tool_by_name() {
        let args = json!({"skill": "gangsta:heist"});
        assert_eq!(
            skill_from_tool_call("Skill", &args),
            Some("gangsta:heist".into())
        );
    }

    #[test]
    fn skill_tool_command_key() {
        let args = json!({"command": "/code-review"});
        assert_eq!(
            skill_from_tool_call("skill", &args),
            Some("code-review".into())
        );
    }

    #[test]
    fn skill_md_read_flat() {
        let args = json!({"path": "/home/u/.agents/skills/reconnaissance/SKILL.md"});
        assert_eq!(
            skill_from_tool_call("ReadFile", &args),
            Some("reconnaissance".into())
        );
    }

    #[test]
    fn skill_md_read_nested() {
        let args =
            json!({"operations": [{"mode": "Line", "path": "/x/skills/drill-tdd/SKILL.md"}]});
        assert_eq!(
            skill_from_tool_call("read", &args),
            Some("drill-tdd".into())
        );
    }

    #[test]
    fn plain_read_is_not_a_skill() {
        let args = json!({"path": "/home/u/project/src/main.rs"});
        assert_eq!(skill_from_tool_call("read", &args), None);
    }

    #[test]
    fn bare_skill_md_without_dir_is_ignored() {
        let args = json!({"path": "SKILL.md"});
        assert_eq!(skill_from_tool_call("read", &args), None);
    }
}
