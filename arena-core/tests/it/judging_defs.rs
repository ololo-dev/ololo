//! Tool-definition + dispatch tests for the judging module.

use arena_core::judging::dispatch_tool;
use arena_core::judging::tool_defs;
use arena_core::judging::tools::ToolScope;

#[test]
fn tool_defs_has_seven_tools() {
    let defs = tool_defs(&ToolScope::everything());
    assert_eq!(defs.len(), 7);
    let names: Vec<&str> = defs.iter().map(|d| d.name.as_str()).collect();
    assert_eq!(
        names,
        vec![
            "get_task_stats",
            "list_files",
            "read_file",
            "get_diff",
            "get_last_commit_diff",
            "find_task_commit",
            "get_commit_log",
        ]
    );
}

#[tokio::test]
async fn dispatch_get_task_stats_returns_payload_or_note() {
    let dir = tempfile::tempdir().unwrap();
    let stats = r#"{"task_ordinal":1,"totals":{"input_tokens":10}}"#;
    let with = dispatch_tool(
        dir.path(),
        "get_task_stats",
        &serde_json::json!({}),
        None,
        Some(stats),
        &ToolScope::everything(),
    )
    .await;
    assert_eq!(with, stats);

    let without = dispatch_tool(
        dir.path(),
        "get_task_stats",
        &serde_json::json!({}),
        None,
        None,
        &ToolScope::everything(),
    )
    .await;
    assert!(without.contains("no agent implementation statistics"));
}

#[tokio::test]
async fn dispatch_tool_unknown_returns_error_string() {
    let dir = tempfile::tempdir().unwrap();
    let res = dispatch_tool(
        dir.path(),
        "nope",
        &serde_json::json!({}),
        None,
        None,
        &ToolScope::everything(),
    )
    .await;
    assert!(res.starts_with("error:"));
}
