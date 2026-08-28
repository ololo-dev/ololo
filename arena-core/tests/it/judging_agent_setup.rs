//! `build_agent_setup_evidence` — what the workflow judge is handed.
//!
//! The join these tests defend is the one no single source can answer: git
//! says a skill *exists*, telemetry says a name was *loaded*, and only the two
//! together separate a workflow from a directory of markdown. They also pin
//! the `added_in_session` split, without which a player inherits credit for the
//! project's own AGENTS.md.

use crate::common;

use arena_core::entities::task_agent_stats;
use arena_core::judging::agent_setup::build_agent_setup_evidence;
use arena_core::judging::tools::ToolScope;
use arena_core::protocol::AgentSessionStats;
use chrono::Utc;
use sea_orm::{ActiveModelTrait, DatabaseConnection, Set};
use std::collections::BTreeMap;
use std::path::Path;
use uuid::Uuid;

fn write_nested(dir: &Path, rel: &str, content: &str) {
    let path = dir.join(rel);
    std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
    std::fs::write(&path, content).expect("write");
    let out = std::process::Command::new(common::git_bin())
        .arg("-C")
        .arg(dir)
        .args(["add", rel])
        .output()
        .expect("git add");
    assert!(out.status.success());
}

fn agent_stats(tools: &[(&str, u64)], skills: &[(&str, u64)]) -> String {
    let stats = AgentSessionStats {
        agent: "claude".to_string(),
        agent_session_id: "sess-1".to_string(),
        model: Some("claude-opus-5".to_string()),
        input_tokens: 100,
        output_tokens: 50,
        cache_read_tokens: 0,
        cache_write_tokens: 0,
        reasoning_tokens: 0,
        cost: None,
        user_messages: 4,
        assistant_messages: 9,
        tool_calls: tools.iter().map(|(_, n)| n).sum(),
        tools: tools
            .iter()
            .map(|(n, c)| (n.to_string(), *c))
            .collect::<BTreeMap<_, _>>(),
        skills: skills
            .iter()
            .map(|(n, c)| (n.to_string(), *c))
            .collect::<BTreeMap<_, _>>(),
    };
    serde_json::to_string(&vec![stats]).expect("serialize stats")
}

async fn insert_stats(
    db: &DatabaseConnection,
    session_id: Uuid,
    player_id: Uuid,
    agents_json: String,
) {
    task_agent_stats::ActiveModel {
        id: Set(Uuid::new_v4()),
        session_id_fk: Set(session_id),
        player_id_fk: Set(player_id),
        task_id_fk: Set(None),
        task_ordinal: Set(0),
        window_started_at: Set(None),
        window_ended_at: Set(None),
        input_tokens: Set(100),
        output_tokens: Set(50),
        cache_read_tokens: Set(0),
        cache_write_tokens: Set(0),
        reasoning_tokens: Set(0),
        cost: Set(None),
        user_messages: Set(4),
        assistant_messages: Set(9),
        tool_calls: Set(6),
        agents_json: Set(agents_json),
        created_at: Set(Utc::now()),
        updated_at: Set(Utc::now()),
    }
    .insert(db)
    .await
    .expect("insert stats");
}

/// A repo that starts with the project's own AGENTS.md and then gains the
/// player's own workflow: a skill, a subagent, an MCP config, a spec.
fn seeded_repo(dir: &Path) {
    common::make_repo(dir);
    write_nested(dir, "AGENTS.md", "# The project\n\nRun with `make dev`.\n");
    write_nested(dir, "README.md", "hello\n");
    common::commit(dir, "root: session start");

    write_nested(
        dir,
        ".claude/skills/capture-shots/SKILL.md",
        "---\nname: capture-shots\ndescription: \"Takes the screenshots the judges ask for\"\n---\nSteps.\n",
    );
    write_nested(
        dir,
        ".claude/skills/never-used/SKILL.md",
        "---\nname: never-used\ndescription: Decoration\n---\nSteps.\n",
    );
    write_nested(dir, ".claude/agents/reviewer.md", "You review diffs.\n");
    write_nested(
        dir,
        ".mcp.json",
        r#"{"mcpServers": {"playwright": {"command": "npx"}}}"#,
    );
    write_nested(
        dir,
        "docs/plan.md",
        "This is spec-driven development: acceptance criteria first.\n",
    );
    common::commit(dir, "feat(t1): the work");
}

#[tokio::test]
async fn evidence_joins_definitions_with_what_actually_ran() {
    let db = common::setup_db().await;
    let owner = common::insert_user(&db).await;
    let project = common::insert_project(&db, owner).await;
    let session = common::insert_session(&db, project).await;
    let player = common::insert_player(&db, session).await;
    insert_stats(
        &db,
        session,
        player,
        agent_stats(
            &[
                ("Bash", 3),
                ("mcp__playwright__navigate", 2),
                ("Task", 1),
                ("Read", 7),
            ],
            &[("capture-shots", 2), ("gangsta:heist", 1)],
        ),
    )
    .await;

    let tmp = tempfile::tempdir().expect("tempdir");
    seeded_repo(tmp.path());

    let ev = build_agent_setup_evidence(
        &db,
        tmp.path(),
        session,
        player,
        &["AGENTS.md".to_string(), "README.md".to_string()],
        &ToolScope::everything(),
    )
    .await;

    // Inherited configuration is context, not credit.
    let agents_md = ev
        .instructions
        .iter()
        .find(|f| f.path == "AGENTS.md")
        .expect("AGENTS.md found");
    assert!(!agents_md.added_in_session);
    assert!(
        agents_md
            .excerpt
            .as_deref()
            .unwrap_or_default()
            .contains("make dev"),
        "the instructions' substance is the evidence, so it is read"
    );

    // A skill that ran and a skill that did not, told apart.
    let used = ev
        .skills
        .iter()
        .find(|s| s.name == "capture-shots")
        .expect("defined skill");
    assert_eq!(used.loads, 2);
    assert!(used.added_in_session);
    assert_eq!(
        used.description.as_deref(),
        Some("Takes the screenshots the judges ask for")
    );
    let idle = ev
        .skills
        .iter()
        .find(|s| s.name == "never-used")
        .expect("defined skill");
    assert_eq!(idle.loads, 0);

    // A skill loaded from the player's own machine is not part of the
    // submission, and is reported as such rather than credited to the repo.
    assert_eq!(ev.skills_loaded_without_definition, vec!["gangsta:heist"]);

    assert_eq!(ev.subagents.len(), 1);
    assert_eq!(ev.usage.subagent_calls, 1);

    // MCP: configured in git, called in telemetry — the two sides scored apart.
    let mcp = ev.mcp_configs.first().expect("mcp config");
    assert_eq!(mcp.servers, vec!["playwright"]);
    assert!(mcp.added_in_session);
    assert_eq!(ev.usage.mcp_servers_called, vec!["playwright"]);
    assert_eq!(
        ev.usage.mcp_tools_called.get("mcp__playwright__navigate"),
        Some(&2)
    );

    assert!(ev.usage.reported);
    // The per-task slice: the judge puts a skill next to the task it ran
    // under, which is what turns "loaded twice" into "loaded for the task
    // that needed it".
    let by_task = ev.usage.skills_by_task.first().expect("task slice");
    assert_eq!(by_task.task_ordinal, 0);
    assert_eq!(by_task.skills.get("capture-shots"), Some(&2));
    assert_eq!(ev.usage.agent_sessions, 1);
    assert_eq!(ev.usage.agents.first().expect("agent").agent, "claude");
    assert_eq!(ev.usage.top_tools.first().expect("top tool").0, "Read");
    assert!(ev.knowledge_base.contains(&"docs/plan.md".to_string()));
    assert!(
        ev.method_signals.iter().any(|s| s.topic == "spec_driven"),
        "a spec-driven mention is surfaced with its file:line"
    );
    assert!(!ev.is_bare());
}

#[tokio::test]
async fn a_bare_repo_reports_bare_rather_than_empty() {
    let db = common::setup_db().await;
    let owner = common::insert_user(&db).await;
    let project = common::insert_project(&db, owner).await;
    let session = common::insert_session(&db, project).await;
    let player = common::insert_player(&db, session).await;

    let tmp = tempfile::tempdir().expect("tempdir");
    common::make_repo(tmp.path());
    write_nested(tmp.path(), "main.py", "print('hi')\n");
    common::commit(tmp.path(), "feat(t1): the work");

    let ev = build_agent_setup_evidence(
        &db,
        tmp.path(),
        session,
        player,
        &[],
        &ToolScope::everything(),
    )
    .await;

    assert!(ev.is_bare());
    // No statistics reported is "not observed", never "did nothing" — the
    // prompt leans on this flag to keep the judge from penalizing silence.
    assert!(!ev.usage.reported);
    assert!(ev.usage.agents.is_empty());
}
