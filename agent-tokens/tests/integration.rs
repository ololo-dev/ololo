use agent_tokens::extractors::Claude;
use agent_tokens::{AgentId, TokenExtractor, snapshot};
use rusqlite::Connection;
use std::fs;
use std::path::Path;

#[test]
fn claude_extractor_parses_fixture() {
    // Set up a fake ~/.claude/projects/<escaped-cwd>/ dir with the fixture
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path();
    let cwd = home.join("test-project");
    fs::create_dir_all(&cwd).unwrap();
    std::env::set_current_dir(&cwd).unwrap();

    // Escape what the extractor will actually see: on macOS the tempdir is
    // reached through the /var → /private/var symlink, and current_dir()
    // returns the canonical form. Escaping the raw path used to mismatch
    // the dir name, detect() came back false, and the whole body silently
    // skipped — the test was green while covering nothing.
    let escaped = agent_tokens::paths::escape_cwd(&std::env::current_dir().unwrap());
    let project_dir = home.join(".claude/projects").join(&escaped);
    fs::create_dir_all(&project_dir).unwrap();
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/claude_sample.jsonl");
    fs::copy(&fixture, project_dir.join("session.jsonl")).unwrap();

    // SAFETY: process-per-test under nextest; concurrent cargo-test threads
    // in this binary either set their own override or use explicit paths.
    unsafe { std::env::set_var("AGENT_TOKENS_HOME", home) };

    let claude = Claude;
    assert!(
        claude.detect(),
        "the fixture project dir must be detected — a false negative here \
         used to skip every assertion below"
    );
    let sessions = claude.extract(None);
    assert_eq!(sessions.len(), 1);
    let s = &sessions[0];
    assert_eq!(s.agent, AgentId::Claude);
    assert_eq!(s.session_id, "test-session-001");
    assert_eq!(s.model.as_deref(), Some("gpt-4-test"));
    assert_eq!(s.counts.input, 300); // 100 + 200
    assert_eq!(s.counts.output, 150); // 50 + 100
    assert_eq!(s.counts.cache_write, 30); // 10 + 20
    assert_eq!(s.counts.cache_read, 15); // 5 + 10

    unsafe { std::env::remove_var("AGENT_TOKENS_HOME") };
    std::env::set_current_dir("/").unwrap();
}

#[test]
fn opencode_extractor_parses_db() {
    let tmp = tempfile::tempdir().unwrap();
    // The layout opencode_db_path() expects: $XDG_DATA_HOME/opencode/opencode.db
    let data_dir = tmp.path().join("data");
    fs::create_dir_all(data_dir.join("opencode")).unwrap();
    let db_path = data_dir.join("opencode/opencode.db");
    let conn = Connection::open(&db_path).unwrap();

    conn.execute_batch(
        "CREATE TABLE session (
            id TEXT PRIMARY KEY,
            project_id TEXT NOT NULL DEFAULT '',
            parent_id TEXT,
            slug TEXT NOT NULL DEFAULT '',
            directory TEXT NOT NULL DEFAULT '',
            title TEXT NOT NULL DEFAULT '',
            version TEXT NOT NULL DEFAULT '',
            share_url TEXT,
            summary_additions INTEGER,
            summary_deletions INTEGER,
            summary_files INTEGER,
            summary_diffs TEXT,
            revert TEXT,
            permission TEXT,
            time_created INTEGER NOT NULL DEFAULT 0,
            time_updated INTEGER NOT NULL DEFAULT 0,
            time_compacting INTEGER,
            time_archived INTEGER,
            workspace_id TEXT,
            path TEXT,
            agent TEXT,
            model TEXT,
            cost REAL DEFAULT 0 NOT NULL,
            tokens_input INTEGER DEFAULT 0 NOT NULL,
            tokens_output INTEGER DEFAULT 0 NOT NULL,
            tokens_reasoning INTEGER DEFAULT 0 NOT NULL,
            tokens_cache_read INTEGER DEFAULT 0 NOT NULL,
            tokens_cache_write INTEGER DEFAULT 0 NOT NULL,
            metadata TEXT
        );",
    )
    .unwrap();

    conn.execute(
        "INSERT INTO session (id, model, tokens_input, tokens_output, tokens_reasoning, tokens_cache_read, tokens_cache_write, cost, directory, time_created, time_updated)
         VALUES ('oc-1', 'GLM-5.2', 500, 250, 10, 100, 50, 0.05, '/tmp/proj', 1718800000000, 1718800100000)",
        [],
    )
    .unwrap();
    conn.close().unwrap();

    // Point the extractor's fixed path at the fixture: opencode_db_path()
    // prefers $XDG_DATA_HOME, so both overrides pin it to the tempdir.
    // SAFETY: process-per-test under nextest; concurrent cargo-test threads
    // in this binary either set their own override or use explicit paths.
    unsafe {
        std::env::set_var("XDG_DATA_HOME", &data_dir);
        std::env::set_var("AGENT_TOKENS_HOME", tmp.path());
    }

    use agent_tokens::extractors::OpenCode;
    let opencode = OpenCode;
    assert!(opencode.detect(), "fixture db must be detected");
    let sessions = opencode.extract(None);
    assert_eq!(sessions.len(), 1);
    let s = &sessions[0];
    assert_eq!(s.agent, AgentId::OpenCode);
    assert_eq!(s.session_id, "oc-1");
    assert_eq!(s.model.as_deref(), Some("GLM-5.2"));
    assert_eq!(s.counts.input, 500);
    assert_eq!(s.counts.output, 250);
    assert_eq!(s.counts.reasoning, 10);
    assert_eq!(s.counts.cache_read, 100);
    assert_eq!(s.counts.cache_write, 50);
    assert_eq!(s.cwd.as_deref(), Some("/tmp/proj"));

    unsafe {
        std::env::remove_var("XDG_DATA_HOME");
        std::env::remove_var("AGENT_TOKENS_HOME");
    }
}

#[test]
fn detect_does_not_panic() {
    use agent_tokens::extractors::{
        Antigravity, AntigravityCli, Codebuff, Codex, Copilot, Cursor, CursorCli, Droid, Grok,
        Hermes, Kilo, Kiro, Omp, OpenClaw, Pi, ZCode,
    };
    // detect() depends on what agents are installed on the machine — just
    // verify none of them panic.
    let _ = Codex.detect();
    let _ = Cursor.detect();
    let _ = CursorCli.detect();
    let _ = Pi.detect();
    let _ = Omp.detect();
    let _ = Kiro.detect();
    let _ = Copilot.detect();
    let _ = Antigravity.detect();
    let _ = AntigravityCli.detect();
    let _ = Droid.detect();
    let _ = Codebuff.detect();
    let _ = Hermes.detect();
    let _ = OpenClaw.detect();
    let _ = Kilo.detect();
    let _ = Grok.detect();
    let _ = ZCode.detect();
}

#[test]
fn snapshot_fills_list_price_cost_for_known_models() {
    use agent_tokens::{SessionCounts, TokenCounts};
    let mut sessions = vec![
        SessionCounts {
            agent: AgentId::Claude,
            session_id: "priced".into(),
            model: Some("claude-opus-5".into()),
            cwd: None,
            started_at_ms: None,
            last_seen_at_ms: None,
            counts: TokenCounts {
                input: 1_000_000,
                ..Default::default()
            },
            cost: None,
            source_file: None,
        },
        SessionCounts {
            agent: AgentId::Claude,
            session_id: "recorded".into(),
            model: Some("claude-opus-5".into()),
            cwd: None,
            started_at_ms: None,
            last_seen_at_ms: None,
            counts: TokenCounts::default(),
            cost: Some(0.42),
            source_file: None,
        },
        SessionCounts {
            agent: AgentId::Claude,
            session_id: "unknown".into(),
            model: Some("totally-unknown-model-9000".into()),
            cwd: None,
            started_at_ms: None,
            last_seen_at_ms: None,
            counts: TokenCounts::default(),
            cost: None,
            source_file: None,
        },
    ];
    agent_tokens::fill_estimated_costs(&mut sessions);
    let price = agent_tokens::pricing::lookup("claude-opus-5").unwrap();
    assert!((sessions[0].cost.unwrap() - price.input).abs() < 1e-9);
    // A cost the agent recorded itself is never overwritten by the estimate.
    assert_eq!(sessions[1].cost, Some(0.42));
    // An unknown model stays unknown instead of reading as free.
    assert_eq!(sessions[2].cost, None);
}

fn fixture(name: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

#[test]
fn pi_parses_counts_and_stats() {
    use agent_tokens::extractors::pi_common;

    let f = fixture("pi_sample.jsonl");
    let s = pi_common::parse_counts(&f, AgentId::Pi, None).unwrap();
    assert_eq!(s.agent, AgentId::Pi);
    assert_eq!(s.session_id, "pi-session-001");
    assert_eq!(s.cwd.as_deref(), Some("/tmp/proj"));
    assert_eq!(s.counts.input, 300); // 100 + 200
    assert_eq!(s.counts.output, 75); // 50 + 25
    assert_eq!(s.counts.cache_read, 10);
    assert_eq!(s.counts.cache_write, 5);
    assert!((s.cost.unwrap() - 0.03).abs() < 1e-9);
    let model = s.model.unwrap();
    assert!(model.contains("kimi-k2.5:cloud"));
    assert!(model.contains("ollama"));

    let st = pi_common::parse_stats(&f, AgentId::Pi, None).unwrap();
    assert_eq!(st.user_messages, 1);
    assert_eq!(st.assistant_messages, 2);
    assert_eq!(st.tool_calls, 2);
    assert_eq!(st.tools.get("bash"), Some(&1));
    assert_eq!(st.tools.get("read"), Some(&1));
    assert_eq!(st.skills.get("drill-tdd"), Some(&1));

    // Window after everything -> no sessions
    assert!(pi_common::parse_counts(&f, AgentId::Pi, Some(i64::MAX)).is_none());

    // Window covering only the second assistant message (ts 1778419490000)
    let s2 = pi_common::parse_counts(&f, AgentId::Pi, Some(1778419480000)).unwrap();
    assert_eq!(s2.counts.input, 200);
    assert_eq!(s2.counts.output, 25);
}

#[test]
fn codex_parses_rollout() {
    use agent_tokens::extractors::codex;

    let f = fixture("codex_rollout.jsonl");
    let s = codex::parse_counts(&f, None).unwrap();
    assert_eq!(s.agent, AgentId::Codex);
    assert_eq!(s.session_id, "codex-session-001");
    assert_eq!(s.cwd.as_deref(), Some("/tmp/proj"));
    assert_eq!(s.model.as_deref(), Some("gpt-5.1-codex"));
    // Lifetime mode uses last cumulative total: input 2500 (900 cached)
    assert_eq!(s.counts.input, 1600);
    assert_eq!(s.counts.cache_read, 900);
    assert_eq!(s.counts.output, 200);
    assert_eq!(s.counts.reasoning, 50);

    // Window covering only the second token_count event (10:01:02Z)
    let cutoff = 1780308060000; // 2026-06-01T10:01:00Z
    let s2 = codex::parse_counts(&f, Some(cutoff)).unwrap();
    assert_eq!(s2.counts.input, 1000); // 1500 - 500 cached
    assert_eq!(s2.counts.cache_read, 500);
    assert_eq!(s2.counts.output, 120);

    let st = codex::parse_stats(&f, None).unwrap();
    assert_eq!(st.user_messages, 1);
    assert_eq!(st.assistant_messages, 1);
    assert_eq!(st.tools.get("shell"), Some(&1));
}

#[test]
fn kiro_parses_meta_and_transcript() {
    use agent_tokens::extractors::kiro;

    let f = fixture("kiro_sample.json");
    let s = kiro::parse_counts(&f, None).unwrap();
    assert_eq!(s.agent, AgentId::Kiro);
    assert_eq!(s.session_id, "kiro-session-001");
    assert_eq!(s.cwd.as_deref(), Some("/tmp/proj"));
    assert_eq!(s.model.as_deref(), Some("claude-sonnet-4.5"));
    assert_eq!(s.counts.input, 1200); // 500 + 700
    assert_eq!(s.counts.output, 300); // 100 + 200
    assert!(s.cost.is_none()); // kiro meters credits, not dollars

    // Window covering only the second turn
    let cutoff = 1783098400000; // between 17:03:47Z and 17:09:33Z on 2026-07-03
    let s2 = kiro::parse_counts(&f, Some(cutoff)).unwrap();
    assert_eq!(s2.counts.input, 700);

    let st = kiro::parse_stats(&f, None).unwrap();
    assert_eq!(st.user_messages, 1);
    assert_eq!(st.assistant_messages, 2);
    assert_eq!(st.tool_calls, 3);
    assert_eq!(st.tools.get("read"), Some(&2));
    assert_eq!(st.tools.get("shell"), Some(&1));
    assert_eq!(st.skills.get("omerta"), Some(&1));
}

#[test]
fn copilot_parses_events() {
    use agent_tokens::extractors::copilot;

    let f = fixture("copilot_events/copilot-session-001/events.jsonl");
    let s = copilot::parse_counts(&f, None).unwrap();
    assert_eq!(s.agent, AgentId::Copilot);
    assert_eq!(s.session_id, "copilot-session-001");
    assert_eq!(s.cwd.as_deref(), Some("/tmp/proj"));
    assert_eq!(s.model.as_deref(), Some("gpt-5.4"));
    // Copilot doesn't persist token usage locally
    assert_eq!(s.counts.input, 0);
    assert!(s.started_at_ms.is_some());
    assert!(s.last_seen_at_ms.unwrap() > s.started_at_ms.unwrap());

    let st = copilot::parse_stats(&f, None).unwrap();
    assert_eq!(st.user_messages, 1);
    assert_eq!(st.assistant_messages, 2);
    assert_eq!(st.tools.get("view"), Some(&2));
    assert_eq!(st.skills.get("using-gangsta"), Some(&1));

    // Window after everything -> filtered out
    assert!(copilot::parse_counts(&f, Some(i64::MAX)).is_none());
}

#[test]
fn cursor_cli_parses_transcript() {
    use agent_tokens::extractors::cursor_cli;

    let f = fixture("cursor_transcript.jsonl");
    let s = cursor_cli::parse_counts(&f, None).unwrap();
    assert_eq!(s.agent, AgentId::CursorCli);
    assert_eq!(s.session_id, "cursor_transcript");
    // Cursor CLI persists neither tokens nor timestamps; mtime stands in for last_seen
    assert!(s.last_seen_at_ms.is_some());

    let st = cursor_cli::parse_stats(&f, None).unwrap();
    assert_eq!(st.user_messages, 1);
    assert_eq!(st.assistant_messages, 3);
    assert_eq!(st.tool_calls, 3);
    assert_eq!(st.tools.get("Glob"), Some(&2));
    assert_eq!(st.tools.get("ReadFile"), Some(&1));
    assert_eq!(st.skills.get("using-gangsta"), Some(&1));
}

#[test]
fn gemini_parses_chat_checkpoint() {
    use agent_tokens::extractors::gemini;

    let f = fixture("gemini_chats/hash1/chats/session-1.json");
    let s = gemini::parse_counts(&f, None).unwrap();
    assert_eq!(s.agent, AgentId::Gemini);
    assert_eq!(s.session_id, "gemini-session-001");
    assert_eq!(s.model.as_deref(), Some("gemini-2.5-pro"));
    // input excludes cached: (1000-300) + (2000-1500)
    assert_eq!(s.counts.input, 1200);
    assert_eq!(s.counts.cache_read, 1800);
    assert_eq!(s.counts.output, 300);
    assert_eq!(s.counts.reasoning, 50);

    let st = gemini::parse_stats(&f, None).unwrap();
    assert_eq!(st.user_messages, 1);
    assert_eq!(st.assistant_messages, 2);
    assert_eq!(st.tools.get("run_shell_command"), Some(&1));
}

#[test]
fn qwen_parses_chat_jsonl() {
    use agent_tokens::extractors::qwen;

    let f = fixture("qwen_sample.jsonl");
    let s = qwen::parse_counts(&f, None).unwrap();
    assert_eq!(s.agent, AgentId::Qwen);
    assert_eq!(s.session_id, "qwen-session-001");
    assert_eq!(s.model.as_deref(), Some("qwen3-coder-plus"));
    assert_eq!(s.counts.input, 2000); // 800 + 1200
    assert_eq!(s.counts.output, 240); // 150 + 90
    assert_eq!(s.counts.reasoning, 40);
    assert_eq!(s.counts.cache_read, 1400); // 500 + 900

    let st = qwen::parse_stats(&f, None).unwrap();
    assert_eq!(st.user_messages, 1);
    assert_eq!(st.assistant_messages, 2);
    assert_eq!(st.tools.get("read_file"), Some(&1));
    assert_eq!(st.skills.get("omerta"), Some(&1));
}

#[test]
fn kimi_parses_wire_with_dedup() {
    use agent_tokens::extractors::kimi;

    let f = fixture("kimi_wire/group1/kimi-session-001/wire.jsonl");
    let s = kimi::parse_counts(&f, None).unwrap();
    assert_eq!(s.agent, AgentId::Kimi);
    assert_eq!(s.session_id, "kimi-session-001");
    assert_eq!(s.model.as_deref(), Some("kimi-k2.5"));
    // m1 deduped to its larger update (output 120), plus m2
    assert_eq!(s.counts.input, 1200); // 500 + 700
    assert_eq!(s.counts.output, 200); // 120 + 80
    assert_eq!(s.counts.cache_read, 400); // 100 + 300
    assert_eq!(s.counts.cache_write, 20);
    assert_eq!(s.started_at_ms, Some(1781000000500));
}

#[test]
fn amp_parses_thread() {
    use agent_tokens::extractors::amp;

    let f = fixture("amp_threads/T-amp-001.json");
    let s = amp::parse_counts(&f, None).unwrap();
    assert_eq!(s.agent, AgentId::Amp);
    assert_eq!(s.session_id, "T-amp-001");
    assert_eq!(s.model.as_deref(), Some("claude-sonnet-4.6"));
    assert_eq!(s.counts.input, 2000); // 900 + 1100
    assert_eq!(s.counts.output, 200); // 130 + 70
    assert_eq!(s.counts.cache_read, 1200);
    assert_eq!(s.counts.cache_write, 60);
    assert!((s.cost.unwrap() - 0.20).abs() < 1e-9);

    let st = amp::parse_stats(&f, None).unwrap();
    assert_eq!(st.user_messages, 1);
    assert_eq!(st.assistant_messages, 2);
    assert_eq!(st.tools.get("Bash"), Some(&1));
}

#[test]
fn goose_parses_sessions_db() {
    use agent_tokens::extractors::goose;

    let tmp = tempfile::tempdir().unwrap();
    let db_path = tmp.path().join("sessions.db");
    let conn = Connection::open(&db_path).unwrap();
    conn.execute_batch(
        "CREATE TABLE sessions (
            id TEXT PRIMARY KEY,
            provider_name TEXT,
            model_config_json TEXT,
            working_dir TEXT,
            created_at INTEGER,
            updated_at INTEGER,
            input_tokens INTEGER,
            output_tokens INTEGER,
            total_tokens INTEGER,
            accumulated_input_tokens INTEGER,
            accumulated_output_tokens INTEGER,
            accumulated_total_tokens INTEGER
        );
        INSERT INTO sessions VALUES
            ('goose-1', 'anthropic', '{\"model_name\":\"claude-sonnet-4-5\"}', '/tmp/proj',
             1781200000, 1781200600, 100, 20, 120, 5000, 800, 6000),
            ('goose-empty', 'anthropic', NULL, NULL, 1781200000, NULL, 0, 0, 0, 0, 0, 0);",
    )
    .unwrap();
    conn.close().unwrap();

    let sessions = goose::parse_db(&db_path, None);
    assert_eq!(sessions.len(), 1); // zero-token session filtered out
    let s = &sessions[0];
    assert_eq!(s.agent, AgentId::Goose);
    assert_eq!(s.session_id, "goose-1");
    // accumulated_* preferred over plain columns
    assert_eq!(s.counts.input, 5000);
    assert_eq!(s.counts.output, 800);
    assert_eq!(s.counts.reasoning, 200); // 6000 - 5800 gap
    assert_eq!(s.cwd.as_deref(), Some("/tmp/proj"));
    // seconds normalized to ms
    assert_eq!(s.started_at_ms, Some(1781200000000));
    let model = s.model.as_deref().unwrap();
    assert!(model.contains("claude-sonnet-4-5"));
    assert!(model.contains("anthropic"));
}

/// Point every store path at an empty scratch dir for the duration of the
/// test. Without this, snapshot tests crawl the developer's REAL agent
/// history (`~/.claude/projects` and friends) — minutes of IO on a lived-in
/// machine, and a nondeterministic result.
fn with_empty_home<T>(f: impl FnOnce() -> T) -> T {
    let tmp = tempfile::tempdir().unwrap();
    // SAFETY: single-threaded within this test; concurrent tests in this
    // binary either set the same override or use explicit fixture paths.
    unsafe { std::env::set_var("AGENT_TOKENS_HOME", tmp.path()) };
    let out = f();
    unsafe { std::env::remove_var("AGENT_TOKENS_HOME") };
    out
}

#[test]
fn stats_snapshot_does_not_panic() {
    with_empty_home(|| {
        let _ = agent_tokens::stats_snapshot(None);
        let _ = agent_tokens::stats_snapshot(Some(0));
    });
}

#[test]
fn snapshot_returns_empty_when_no_data() {
    // With an empty home, snapshot must return empty without panicking.
    with_empty_home(|| {
        assert!(snapshot(None).is_empty());
    });
}

#[test]
fn snapshot_with_since_filter() {
    // Verify since filter doesn't panic
    with_empty_home(|| {
        let _ = snapshot(Some(0));
        let _ = snapshot(Some(9999999999999));
    });
}

#[test]
fn gemini_project_hash_matches_cli_scheme() {
    use agent_tokens::extractors::gemini;
    use std::path::Path;

    // gemini-cli's getProjectHash: sha256 hex of the absolute project path.
    assert_eq!(
        gemini::project_hash(Path::new("/Users/demo/workspace/extreme-startup")),
        "be977c7075d1394c1a6521316bec34c940c299ca495ac13881830525a9c2b115"
    );
}

#[test]
fn gemini_parses_logs_json_fallback() {
    use agent_tokens::extractors::gemini;

    let f = fixture("gemini_chats/hash2/logs.json");
    let sessions = gemini::parse_logs_stats(&f, None);
    assert_eq!(sessions.len(), 2, "one entry per sessionId");

    let s1 = sessions
        .iter()
        .find(|s| s.session_id == "gemini-log-session-001")
        .expect("first session present");
    assert_eq!(s1.agent, AgentId::Gemini);
    assert_eq!(s1.user_messages, 2);
    assert_eq!(s1.assistant_messages, 0);
    // 2026-06-11T10:00:00Z / 10:05:00Z
    assert_eq!(s1.started_at_ms, Some(1781172000000));
    assert_eq!(s1.last_seen_at_ms, Some(1781172300000));

    let s2 = sessions
        .iter()
        .find(|s| s.session_id == "gemini-log-session-002")
        .expect("second session present");
    assert_eq!(s2.user_messages, 1);
}

#[test]
fn gemini_logs_fallback_respects_since() {
    use agent_tokens::extractors::gemini;

    let f = fixture("gemini_chats/hash2/logs.json");
    // Cutoff after the first session's activity: only session 002 remains.
    let sessions = gemini::parse_logs_stats(&f, Some(1781174000000));
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].session_id, "gemini-log-session-002");
}

// ---------------------------------------------------------------------------
// Cursor IDE (state.vscdb + tokscale cursor-cache CSVs)
// ---------------------------------------------------------------------------

/// Build a synthetic Cursor `state.vscdb` with two composers and per-bubble
/// token counts (only c1:b2 non-zero, mirroring real DBs where most bubble
/// tokenCounts are zeros).
fn build_cursor_state_db(dir: &Path) -> std::path::PathBuf {
    let db_path = dir.join("state.vscdb");
    let conn = Connection::open(&db_path).unwrap();
    conn.execute_batch("CREATE TABLE cursorDiskKV (key TEXT PRIMARY KEY, value BLOB);")
        .unwrap();
    let rows: &[(&str, &str)] = &[
        (
            "composerData:c1",
            r#"{"composerId":"c1","createdAt":1781000000000,"fullConversationHeadersOnly":[
                {"bubbleId":"b1","type":1},{"bubbleId":"b2","type":2},{"bubbleId":"b3","type":2}]}"#,
        ),
        (
            "composerData:c2",
            r#"{"composerId":"c2","createdAt":1782000000000,"fullConversationHeadersOnly":[
                {"bubbleId":"b1","type":1},{"bubbleId":"b2","type":2}]}"#,
        ),
        (
            "bubbleId:c1:b2",
            r#"{"tokenCount":{"inputTokens":120,"outputTokens":30}}"#,
        ),
        (
            "bubbleId:c1:b3",
            r#"{"tokenCount":{"inputTokens":0,"outputTokens":0}}"#,
        ),
        (
            "bubbleId:c2:b2",
            r#"{"tokenCount":{"inputTokens":7,"outputTokens":2}}"#,
        ),
        ("bubbleId:junk", "not json at all"),
        ("someOtherKey", r#"{"unrelated":true}"#),
    ];
    for (key, value) in rows {
        conn.execute(
            "INSERT INTO cursorDiskKV (key, value) VALUES (?1, ?2)",
            rusqlite::params![key, value.as_bytes()],
        )
        .unwrap();
    }
    conn.close().unwrap();
    db_path
}

#[test]
fn cursor_ide_parses_state_db() {
    use agent_tokens::extractors::cursor;

    let tmp = tempfile::tempdir().unwrap();
    let db = build_cursor_state_db(tmp.path());

    let mut stats = cursor::parse_state_db_stats(&db, None);
    stats.sort_by(|a, b| a.session_id.cmp(&b.session_id));
    assert_eq!(stats.len(), 2);
    assert_eq!(stats[0].session_id, "c1");
    assert_eq!(stats[0].agent, AgentId::Cursor);
    assert_eq!(stats[0].user_messages, 1);
    assert_eq!(stats[0].assistant_messages, 2);
    assert_eq!(stats[0].started_at_ms, Some(1781000000000));
    assert_eq!(stats[1].session_id, "c2");
    assert_eq!(stats[1].user_messages, 1);
    assert_eq!(stats[1].assistant_messages, 1);

    let mut counts = cursor::parse_state_db_counts(&db, None);
    counts.sort_by(|a, b| a.session_id.cmp(&b.session_id));
    assert_eq!(counts.len(), 2);
    // c1: only the non-zero bubble contributes; the all-zero bubble is skipped
    assert_eq!(counts[0].session_id, "c1");
    assert_eq!(counts[0].counts.input, 120);
    assert_eq!(counts[0].counts.output, 30);
    assert_eq!(counts[0].started_at_ms, Some(1781000000000));
    assert_eq!(counts[1].session_id, "c2");
    assert_eq!(counts[1].counts.input, 7);
    assert_eq!(counts[1].counts.output, 2);
}

#[test]
fn cursor_ide_state_db_respects_since_at_composer_granularity() {
    use agent_tokens::extractors::cursor;

    let tmp = tempfile::tempdir().unwrap();
    let db = build_cursor_state_db(tmp.path());

    // Cutoff between c1 (createdAt 1781...) and c2 (createdAt 1782...): the
    // whole c1 composer drops out (headers have no per-message timestamps).
    let stats = cursor::parse_state_db_stats(&db, Some(1781500000000));
    assert_eq!(stats.len(), 1);
    assert_eq!(stats[0].session_id, "c2");

    let counts = cursor::parse_state_db_counts(&db, Some(1781500000000));
    assert_eq!(counts.len(), 1);
    assert_eq!(counts[0].session_id, "c2");

    // Cutoff after everything -> nothing.
    assert!(cursor::parse_state_db_stats(&db, Some(i64::MAX)).is_empty());
    assert!(cursor::parse_state_db_counts(&db, Some(i64::MAX)).is_empty());
}

#[test]
fn cursor_ide_parses_cache_csv_v2() {
    use agent_tokens::extractors::cursor;

    let f = fixture("cursor_cache/usage.csv");
    let rows = cursor::parse_cache_csv(&f, None);
    assert_eq!(rows.len(), 2);

    // Rows come back in file order; legacy usage.csv maps to account "active".
    let first = &rows[0];
    assert_eq!(first.agent, AgentId::Cursor);
    assert_eq!(
        first.session_id,
        "cursor-csv:active:2025-11-13T18:36:05.846Z"
    );
    assert_eq!(first.model.as_deref(), Some("auto"));
    assert_eq!(first.counts.input, 775);
    assert_eq!(first.counts.output, 21282);
    assert_eq!(first.counts.cache_read, 105891);
    assert_eq!(first.counts.cache_write, 28342 - 775);
    assert!((first.cost.unwrap() - 0.19).abs() < 1e-9);

    let second = &rows[1];
    assert_eq!(second.model.as_deref(), Some("gpt-5-codex"));
    assert_eq!(second.counts.input, 8263);
    assert_eq!(second.counts.cache_write, 0); // 0 - 8263 clamps to 0

    // since between the two rows keeps only the newer one.
    let ts_second = second.started_at_ms.unwrap();
    let ts_first = first.started_at_ms.unwrap();
    assert!(ts_first > ts_second);
    let filtered = cursor::parse_cache_csv(&f, Some(ts_second + 1));
    assert_eq!(filtered.len(), 1);
    assert_eq!(filtered[0].model.as_deref(), Some("auto"));
}

#[test]
fn cursor_ide_parses_cache_csv_v3() {
    use agent_tokens::extractors::cursor;

    let f = fixture("cursor_cache/usage.work.csv");
    let rows = cursor::parse_cache_csv(&f, None);
    assert_eq!(rows.len(), 3);

    // usage.<account>.csv carries the account name into the session key.
    assert!(rows[0].session_id.starts_with("cursor-csv:work:"));
    assert_eq!(rows[0].model.as_deref(), Some("composer-2"));
    assert_eq!(rows[0].counts.input, 343446);
    assert_eq!(rows[0].counts.cache_read, 29045760);
    assert_eq!(rows[0].counts.output, 915201);
    assert!(rows[0].cost.is_none()); // "Included" -> no dollar cost

    assert_eq!(rows[1].counts.cache_write, 50000 - 43478);
    assert!((rows[1].cost.unwrap() - 0.11).abs() < 1e-9);

    assert!(rows[2].cost.is_none()); // "-" (Errored, No Charge) -> no cost
}

// ---------------------------------------------------------------------------
// Antigravity IDE (tokscale antigravity-cache JSONL)
// ---------------------------------------------------------------------------

#[test]
fn antigravity_parses_cache_jsonl() {
    use agent_tokens::extractors::antigravity;

    let f = fixture("antigravity_cache/sessions/ag-sample.jsonl");
    let mut counts = antigravity::parse_cache_counts(&f, None);
    counts.sort_by(|a, b| (&a.session_id, &a.model).cmp(&(&b.session_id, &b.model)));
    // (ag-1, claude-sonnet-4.6 from session_meta), (ag-1, gemini-3-pro own
    // modelId), (ag-2, no model). The duplicate responseId r2 row, the
    // all-zero row, and the row without a timestamp are all dropped.
    assert_eq!(counts.len(), 3);

    let s1 = &counts[0];
    assert_eq!(s1.agent, AgentId::Antigravity);
    assert_eq!(s1.session_id, "ag-1");
    assert_eq!(s1.model.as_deref(), Some("claude-sonnet-4.6"));
    assert_eq!(s1.counts.input, 100);
    assert_eq!(s1.counts.output, 20);
    assert_eq!(s1.counts.cache_read, 50);
    assert_eq!(s1.counts.cache_write, 5);
    assert_eq!(s1.counts.reasoning, 10);
    assert_eq!(s1.started_at_ms, Some(1781000000000));

    let s2 = &counts[1];
    assert_eq!(s2.session_id, "ag-1");
    assert_eq!(s2.model.as_deref(), Some("gemini-3-pro"));
    assert_eq!(s2.counts.input, 200);

    let s3 = &counts[2];
    assert_eq!(s3.session_id, "ag-2");
    // No own modelId: inherits the file-global session_meta model (the
    // fallback is not scoped per sessionId, matching tokscale).
    assert_eq!(s3.model.as_deref(), Some("claude-sonnet-4.6"));
    assert_eq!(s3.counts.input, 10);

    let mut stats = antigravity::parse_cache_stats(&f, None);
    stats.sort_by(|a, b| a.session_id.cmp(&b.session_id));
    assert_eq!(stats.len(), 2);
    assert_eq!(stats[0].session_id, "ag-1");
    assert_eq!(stats[0].assistant_messages, 2); // r1 + r2, dup r2 not double-counted
    assert_eq!(stats[0].started_at_ms, Some(1781000000000));
    assert_eq!(stats[0].last_seen_at_ms, Some(1781000060000));
    assert_eq!(stats[1].session_id, "ag-2");
    assert_eq!(stats[1].assistant_messages, 1);
}

#[test]
fn antigravity_cache_respects_since() {
    use agent_tokens::extractors::antigravity;

    let f = fixture("antigravity_cache/sessions/ag-sample.jsonl");
    // Cutoff drops the first usage row (1781000000000): the (ag-1, claude)
    // entry disappears; (ag-1, gemini-3-pro) and ag-2 remain.
    let mut counts = antigravity::parse_cache_counts(&f, Some(1781000060000));
    counts.sort_by(|a, b| a.session_id.cmp(&b.session_id));
    assert_eq!(counts.len(), 2);
    assert_eq!(counts[0].session_id, "ag-1");
    assert_eq!(counts[0].model.as_deref(), Some("gemini-3-pro"));
    assert_eq!(counts[1].session_id, "ag-2");

    assert!(antigravity::parse_cache_counts(&f, Some(i64::MAX)).is_empty());
    assert!(antigravity::parse_cache_stats(&f, Some(i64::MAX)).is_empty());
}

// ---------------------------------------------------------------------------
// Antigravity CLI (protobuf-in-sqlite conversations)
// ---------------------------------------------------------------------------

/// Test-side protobuf wire encoder: varint fields and length-delimited fields.
mod proto_enc {
    pub fn varint(mut value: u64) -> Vec<u8> {
        let mut out = Vec::new();
        loop {
            let mut byte = (value & 0x7f) as u8;
            value >>= 7;
            if value != 0 {
                byte |= 0x80;
            }
            out.push(byte);
            if value == 0 {
                break;
            }
        }
        out
    }

    /// Encode `field` as a varint (wire type 0).
    pub fn field_varint(field: u64, value: u64) -> Vec<u8> {
        let mut out = varint(field << 3);
        out.extend(varint(value));
        out
    }

    /// Encode `field` as length-delimited bytes (wire type 2).
    pub fn field_len(field: u64, payload: &[u8]) -> Vec<u8> {
        let mut out = varint((field << 3) | 2);
        out.extend(varint(payload.len() as u64));
        out.extend_from_slice(payload);
        out
    }

    /// `{#1: seconds, #2: nanos}` protobuf Timestamp payload.
    pub fn timestamp(seconds: u64, nanos: u64) -> Vec<u8> {
        let mut out = field_varint(1, seconds);
        out.extend(field_varint(2, nanos));
        out
    }
}

/// Build one gen_metadata blob. `gen_ts` is the optional per-generation
/// `chatModel.#9.#4` wall-clock stamp.
#[allow(clippy::too_many_arguments)]
fn build_gen_blob(
    system_prompt: u64,
    input: u64,
    cache_read: u64,
    output: u64,
    reasoning: u64,
    response_id: &str,
    model: &str,
    gen_ts: Option<(u64, u64)>,
) -> Vec<u8> {
    use proto_enc::*;
    let mut usage = Vec::new();
    usage.extend(field_varint(1, system_prompt)); // fixed system prompt
    usage.extend(field_varint(2, input)); // new input
    usage.extend(field_varint(5, cache_read)); // cacheRead
    usage.extend(field_varint(9, output)); // output
    usage.extend(field_varint(10, reasoning)); // thinking
    usage.extend(field_len(11, response_id.as_bytes())); // responseId

    let mut chat_model = Vec::new();
    chat_model.extend(field_len(4, &usage));
    if let Some((seconds, nanos)) = gen_ts {
        // #9 wraps a sub-message whose #4 is the {seconds, nanos} Timestamp.
        let gen9 = field_len(4, &timestamp(seconds, nanos));
        chat_model.extend(field_len(9, &gen9));
    }
    chat_model.extend(field_len(19, model.as_bytes()));

    field_len(1, &chat_model)
}

fn build_trajectory_blob(created_seconds: u64, workspace_uri: &str) -> Vec<u8> {
    use proto_enc::*;
    let workspace = field_len(1, workspace_uri.as_bytes());
    let mut blob = Vec::new();
    blob.extend(field_len(1, &workspace));
    blob.extend(field_len(2, &timestamp(created_seconds, 0)));
    blob
}

/// Synthetic conversations/<uuid>.db: two real generations (the first without
/// a per-generation timestamp, the second with one), a duplicate responseId,
/// and a malformed blob.
fn build_antigravity_cli_db(dir: &Path) -> std::path::PathBuf {
    let db_path = dir.join("11111111-2222-3333-4444-555555555555.db");
    let conn = Connection::open(&db_path).unwrap();
    conn.execute_batch(
        "CREATE TABLE gen_metadata (idx INTEGER, data BLOB, size INTEGER);
         CREATE TABLE trajectory_metadata_blob (id TEXT, data BLOB);",
    )
    .unwrap();

    let gen1 = build_gen_blob(
        1132,
        500,
        16000,
        300,
        40,
        "resp-1",
        "gemini-3-flash-a",
        None,
    );
    let gen2 = build_gen_blob(
        0,
        200,
        0,
        100,
        25,
        "resp-2",
        "gemini-3-flash-a",
        Some((1_781_502_700, 250_000_000)),
    );
    let dup_of_gen1 = build_gen_blob(
        1132,
        500,
        16000,
        300,
        40,
        "resp-1",
        "gemini-3-flash-a",
        None,
    );
    let malformed = vec![0xffu8, 0xff, 0xff, 0xff];
    for (idx, blob) in [(0, &gen1), (1, &gen2), (2, &dup_of_gen1), (3, &malformed)] {
        conn.execute(
            "INSERT INTO gen_metadata (idx, data, size) VALUES (?1, ?2, 0)",
            rusqlite::params![idx, blob],
        )
        .unwrap();
    }
    conn.execute(
        "INSERT INTO trajectory_metadata_blob (id, data) VALUES ('main', ?1)",
        rusqlite::params![build_trajectory_blob(1_781_502_653, "file:///tmp/proj-x")],
    )
    .unwrap();
    conn.close().unwrap();
    db_path
}

#[test]
fn antigravity_cli_parses_db() {
    use agent_tokens::extractors::antigravity_cli;

    let tmp = tempfile::tempdir().unwrap();
    let db = build_antigravity_cli_db(tmp.path());

    let counts = antigravity_cli::parse_db_counts(&db, None);
    assert_eq!(counts.len(), 1); // one model -> one row
    let s = &counts[0];
    assert_eq!(s.agent, AgentId::AntigravityCli);
    assert_eq!(s.session_id, "11111111-2222-3333-4444-555555555555");
    assert_eq!(s.model.as_deref(), Some("gemini-3-flash-a"));
    assert_eq!(s.cwd.as_deref(), Some("/tmp/proj-x"));
    // gen1 (1132+500 input, dedup drops its duplicate) + gen2 (200 input)
    assert_eq!(s.counts.input, 1632 + 200);
    assert_eq!(s.counts.output, 300 + 100);
    assert_eq!(s.counts.cache_read, 16000);
    assert_eq!(s.counts.reasoning, 40 + 25);
    assert_eq!(s.counts.cache_write, 0);
    // gen1 has no per-generation stamp -> session created-at; gen2 carries
    // its own #9.#4 stamp (seconds + 250ms).
    assert_eq!(s.started_at_ms, Some(1_781_502_653_000));
    assert_eq!(s.last_seen_at_ms, Some(1_781_502_700_250));

    let st = antigravity_cli::parse_db_stats(&db, None).unwrap();
    assert_eq!(st.agent, AgentId::AntigravityCli);
    // Deduped generations: gen1 + gen2 (duplicate resp-1 and the malformed
    // blob are both dropped).
    assert_eq!(st.assistant_messages, 2);
    assert_eq!(st.started_at_ms, Some(1_781_502_653_000));
    assert_eq!(st.last_seen_at_ms, Some(1_781_502_700_250));
}

#[test]
fn antigravity_cli_respects_since_on_generation_timestamps() {
    use agent_tokens::extractors::antigravity_cli;

    let tmp = tempfile::tempdir().unwrap();
    let db = build_antigravity_cli_db(tmp.path());

    // Cutoff between gen1 (session created-at 1_781_502_653_000) and gen2
    // (per-generation 1_781_502_700_250): only gen2 remains.
    let counts = antigravity_cli::parse_db_counts(&db, Some(1_781_502_700_000));
    assert_eq!(counts.len(), 1);
    assert_eq!(counts[0].counts.input, 200);
    assert_eq!(counts[0].counts.output, 100);
    assert_eq!(counts[0].counts.reasoning, 25);

    let st = antigravity_cli::parse_db_stats(&db, Some(1_781_502_700_000)).unwrap();
    assert_eq!(st.assistant_messages, 1);

    // Cutoff after everything -> nothing.
    assert!(antigravity_cli::parse_db_counts(&db, Some(i64::MAX)).is_empty());
    assert!(antigravity_cli::parse_db_stats(&db, Some(i64::MAX)).is_none());
}

#[test]
fn antigravity_cli_tolerates_foreign_db() {
    use agent_tokens::extractors::antigravity_cli;

    // A sqlite DB without the gen_metadata table is not an Antigravity CLI
    // conversation — must yield nothing, not error/panic.
    let tmp = tempfile::tempdir().unwrap();
    let db_path = tmp.path().join("other.db");
    let conn = Connection::open(&db_path).unwrap();
    conn.execute_batch("CREATE TABLE unrelated (x INTEGER);")
        .unwrap();
    conn.close().unwrap();

    assert!(antigravity_cli::parse_db_counts(&db_path, None).is_empty());
    assert!(antigravity_cli::parse_db_stats(&db_path, None).is_none());
}
