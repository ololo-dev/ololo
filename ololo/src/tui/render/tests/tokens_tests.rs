use super::*;
use agent_tokens::{AgentId, SessionCounts, SessionStats, TokenCounts};
use std::collections::BTreeMap;

/// A `SessionCounts` mirroring the web stats-tab example: 90.5k in,
/// 957 out, 20.2k cache read.
fn mk_counts(session_id: &str) -> SessionCounts {
    SessionCounts {
        agent: AgentId::OpenCode,
        session_id: session_id.to_string(),
        model: Some(
            r#"{"providerID":"openrouter","id":"poolside/laguna-xs-2.1:free"}"#.to_string(),
        ),
        cwd: None,
        started_at_ms: Some(1_000_000),
        last_seen_at_ms: Some(1_038_000), // 38s later
        counts: TokenCounts {
            input: 90_500,
            output: 957,
            cache_read: 20_200,
            cache_write: 0,
            reasoning: 0,
        },
        cost: None,
        source_file: None,
    }
}

fn mk_stats(session_id: &str) -> SessionStats {
    let mut tools = BTreeMap::new();
    tools.insert("bash".to_string(), 2u64);
    tools.insert("read".to_string(), 2u64);
    tools.insert("edit".to_string(), 1u64);
    tools.insert("glob".to_string(), 1u64);
    SessionStats {
        agent: AgentId::OpenCode,
        session_id: session_id.to_string(),
        cwd: None,
        started_at_ms: Some(1_000_000),
        last_seen_at_ms: Some(1_038_000),
        user_messages: 3,
        assistant_messages: 7,
        tool_calls: 6,
        tools,
        skills: BTreeMap::new(),
        source_file: None,
    }
}

#[test]
fn tokens_panel_renders_stats_tab_summary_and_session_block() {
    let mut app = fresh_app(160, 50);
    let sid = "ses_0710adf91ffexsPlNLIrhhUF63";
    app.tokens = Some(vec![mk_counts(sid)]);
    app.token_stats = Some(vec![mk_stats(sid)]);
    let flat = header_flat(&app, 160, 50);

    // Summary block (tile row on the web stats tab).
    assert!(
        flat.contains("in/out     90.5k / 957"),
        "tokens in/out summary (got: {flat:?})"
    );
    assert!(
        flat.contains("cache r/w  20.2k / 0"),
        "cache read/write summary (got: {flat:?})"
    );
    assert!(
        flat.contains("msgs u/a   3 / 7"),
        "messages user/agent summary (got: {flat:?})"
    );
    assert!(
        flat.contains("tool calls 6"),
        "tool calls summary (got: {flat:?})"
    );
    assert!(
        flat.contains("time       0m 38s"),
        "implementation time summary (got: {flat:?})"
    );

    // Per-session block.
    assert!(
        flat.contains("opencode · openrouter/poolside/laguna-xs-2.1:free"),
        "agent · model header (got: {flat:?})"
    );
    assert!(flat.contains(sid), "session id line (got: {flat:?})");
    assert!(
        flat.contains("90.5k in · 957 out"),
        "per-session token line (got: {flat:?})"
    );
    assert!(
        flat.contains("tools (6): bash 2, read 2, edit 1, glob 1"),
        "tool breakdown, count-desc then alphabetical (got: {flat:?})"
    );
    assert!(
        flat.contains("skills: —"),
        "empty skills line (got: {flat:?})"
    );
}

#[test]
fn tokens_panel_shows_skill_loads_with_counts() {
    let mut app = fresh_app(160, 50);
    let sid = "s1";
    let mut stats = mk_stats(sid);
    stats.skills.insert("verify".to_string(), 2);
    stats.skills.insert("dataviz".to_string(), 1);
    app.tokens = Some(vec![mk_counts(sid)]);
    app.token_stats = Some(vec![stats]);
    let flat = header_flat(&app, 160, 50);
    assert!(
        flat.contains("skills: verify ×2, dataviz"),
        "skills sorted by count with ×N multiplier (got: {flat:?})"
    );
}

#[test]
fn tokens_panel_absent_without_token_data() {
    let mut app = fresh_app(160, 50);
    // Stats without counts must not bring up the panel (gated on tokens).
    app.token_stats = Some(vec![mk_stats("s1")]);
    let flat = header_flat(&app, 160, 50);
    assert!(
        !flat.contains("tool calls"),
        "no tokens panel without token counts (got: {flat:?})"
    );
}

#[test]
fn merge_joins_counts_and_stats_by_agent_and_session() {
    use crate::tui::render::tokens::merge_session_views;
    let views = merge_session_views(&[mk_counts("s1")], &[mk_stats("s1")]);
    assert_eq!(views.len(), 1);
    let v = &views[&("opencode".to_string(), "s1".to_string())];
    assert_eq!(v.input, 90_500);
    assert_eq!(v.output, 957);
    assert_eq!(v.cache_read, 20_200);
    assert_eq!(v.user_messages, 3);
    assert_eq!(v.assistant_messages, 7);
    assert_eq!(v.tool_calls, 6);
    let models: Vec<&String> = v.models.keys().collect();
    assert_eq!(models, ["openrouter/poolside/laguna-xs-2.1:free"]);
}

#[test]
fn merge_keeps_per_model_usage_when_session_switches_models() {
    use crate::tui::render::tokens::merge_session_views;
    // Extractors report one SessionCounts row per (session, model); a
    // model switch must not overwrite the earlier model's usage.
    let mut second = mk_counts("s1");
    second.model = Some(r#"{"providerID":"openrouter","id":"qwen/qwen3:free"}"#.to_string());
    second.counts.input = 12_000;
    second.counts.output = 300;
    let views = merge_session_views(&[mk_counts("s1"), second], &[mk_stats("s1")]);
    assert_eq!(views.len(), 1, "still one session view");
    let v = &views[&("opencode".to_string(), "s1".to_string())];
    assert_eq!(v.models.len(), 2, "both models kept");
    assert_eq!(
        v.models["openrouter/poolside/laguna-xs-2.1:free"].input,
        90_500
    );
    assert_eq!(v.models["openrouter/qwen/qwen3:free"].input, 12_000);
    assert_eq!(v.input, 102_500, "session total sums both models");
    assert_eq!(v.tool_calls, 6, "stats attach once, not per model");
}

#[test]
fn tokens_panel_lists_every_model_of_a_multi_model_session() {
    let mut app = fresh_app(160, 50);
    let sid = "s1";
    let mut second = mk_counts(sid);
    second.model = Some(r#"{"providerID":"openrouter","id":"qwen/qwen3:free"}"#.to_string());
    second.counts.input = 12_000;
    second.counts.output = 300;
    app.tokens = Some(vec![mk_counts(sid), second]);
    app.token_stats = Some(vec![mk_stats(sid)]);
    let flat = header_flat(&app, 160, 50);
    assert!(
        flat.contains("opencode · 2 models"),
        "header counts the models (got: {flat:?})"
    );
    assert!(
        flat.contains("openrouter/poolside/laguna-xs-2.1:free"),
        "first model listed (got: {flat:?})"
    );
    assert!(
        flat.contains("90.5k in · 957 out"),
        "first model's own counts (got: {flat:?})"
    );
    assert!(
        flat.contains("openrouter/qwen/qwen3:free"),
        "second model listed (got: {flat:?})"
    );
    assert!(
        flat.contains("12.0k in · 300 out"),
        "second model's own counts (got: {flat:?})"
    );
}

#[test]
fn merge_skips_stats_only_sessions() {
    use crate::tui::render::tokens::merge_session_views;
    // Stats for a session with no token counts: dropped (panel is
    // token-gated), and stats for a DIFFERENT session must not leak into
    // an existing one.
    let views = merge_session_views(&[mk_counts("s1")], &[mk_stats("other")]);
    assert_eq!(views.len(), 1);
    let v = &views[&("opencode".to_string(), "s1".to_string())];
    assert_eq!(v.tool_calls, 0, "stats from 'other' must not merge into s1");
}
