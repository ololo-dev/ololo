//! Server-side execution of one system probe (`mode: deterministic`).
//!
//! Where the live loop pushes a rendered command to the participant and
//! trusts the reported stdout, this path runs the command itself: it
//! materializes the player's last pushed HEAD into a scratch tree and
//! executes inside [`arena_core::sandbox`] — the same materialize/sandbox/
//! grade chain the execution judge uses, applied to a single probe instead
//! of a verification pass.
//!
//! Failure vocabulary (decided in the spec):
//! - the player's command failing its validation ⇒ outcome `error` — the
//!   ordinary "measurement says no";
//! - *our* inability to measure — no sandbox, broken sandbox, empty repo,
//!   unresolvable fixtures — ⇒ outcome `unavailable`, `point_delta = 0`,
//!   with the reason recorded in `result_json`. Judges read that as an
//!   observation; it is never a penalty.
//!
//! The measurement lands as an ordinary `probes` row, dispatched and
//! resolved in one step; `result_json` carries the structured extras
//! (snapshot sha/age, stderr tail, timeout flag).

use std::path::Path;

use arena_core::entities::{probes, tests};
use arena_core::evaluation::{OUTCOME_UNAVAILABLE, ProbeConfig};
use arena_core::judging::execution::materialize_commit;
use arena_core::judging::task_commit::resolve_head;
use arena_core::sandbox::{self, SandboxBackend};
use chrono::Utc;
use sea_orm::{ActiveModelTrait, DatabaseConnection, Set};
use tokio::sync::Semaphore;
use uuid::Uuid;

use crate::ws::player_agent::grading::{grade_stdout, resolve_test_fixtures};

/// Concurrency cap for server-side probe executions across the process.
/// Sandboxed runs are cheap but not free; without a cap a burst of interval
/// probes across sessions could stack dozens of `bwrap` trees.
static PROBE_EXEC_PERMITS: Semaphore = Semaphore::const_new(0);
static PROBE_EXEC_INIT: std::sync::Once = std::sync::Once::new();

fn probe_exec_permits() -> &'static Semaphore {
    PROBE_EXEC_INIT.call_once(|| {
        let n = std::env::var("ARENA_PROBE_MAX_CONCURRENT")
            .ok()
            .and_then(|s| s.parse::<usize>().ok())
            .unwrap_or(4)
            .clamp(1, 64);
        PROBE_EXEC_PERMITS.add_permits(n);
    });
    &PROBE_EXEC_PERMITS
}

/// Per-probe wall-clock deadline; same knob family as the execution judge.
fn probe_deadline() -> std::time::Duration {
    let ms = std::env::var("OLOLO_SERVER_PROBE_TIMEOUT_MS")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(15_000);
    std::time::Duration::from_millis(ms.clamp(500, 120_000))
}

/// What one server-side run measured, ready to persist.
#[derive(Debug)]
pub struct ServerProbeRun {
    /// `pass` | `error` | `unavailable`.
    pub outcome: &'static str,
    pub rendered_command: String,
    pub fixture_values_json: String,
    pub expected_answer: Option<String>,
    pub resolved_answer: Option<String>,
    pub output: String,
    pub exit_code: Option<i32>,
    pub duration_ms: Option<i64>,
    pub point_delta: i32,
    /// Structured measurement: snapshot sha/age, stderr tail, timeout flag,
    /// or the `unavailable` reason.
    pub result_json: serde_json::Value,
}

impl ServerProbeRun {
    fn unavailable(reason: impl Into<String>) -> Self {
        let reason = reason.into();
        Self {
            outcome: OUTCOME_UNAVAILABLE,
            rendered_command: String::new(),
            fixture_values_json: "{}".to_string(),
            expected_answer: None,
            resolved_answer: None,
            output: String::new(),
            exit_code: None,
            duration_ms: None,
            point_delta: 0,
            result_json: serde_json::json!({ "unavailable_reason": reason }),
        }
    }
}

/// A materialized, sandbox-verified snapshot of the player's pushed HEAD —
/// the shared front half of every server-side probe mode.
pub(crate) struct Snapshot {
    pub scratch: tempfile::TempDir,
    pub sha: String,
    pub age_secs: i64,
    pub backend: SandboxBackend,
}

/// Materialize HEAD into a scratch tree with the sandbox proven functional.
/// `Err` is always an `unavailable` reason, never the player's fault.
pub(crate) async fn prepare_snapshot(
    repo_dir: &Path,
    session_id: Uuid,
    player_id: Uuid,
) -> Result<Snapshot, String> {
    // The snapshot: last pushed HEAD. No pushes yet = nothing to measure.
    let (sha, commit_time) = match resolve_head(repo_dir).await {
        Ok(Some(head)) => head,
        Ok(None) => return Err("player repo has no commits yet".to_string()),
        Err(e) => return Err(format!("git head: {e}")),
    };
    let age_secs = (Utc::now().timestamp() - commit_time).max(0);

    // Sandbox gating — identical policy to the execution judge: refuse to
    // run untrusted code unsandboxed unless explicitly opted in, and prove
    // the sandbox works before attributing any output to the code.
    let backend = sandbox::detect_backend();
    if backend == SandboxBackend::Unsandboxed {
        let allowed = matches!(
            std::env::var("OLOLO_ALLOW_UNSANDBOXED_EXEC")
                .ok()
                .as_deref(),
            Some("1") | Some("true")
        );
        if !allowed {
            return Err(
                "no process sandbox available (install bwrap); refusing to execute untrusted code"
                    .to_string(),
            );
        }
        tracing::warn!(
            session_id = %session_id, player_id = %player_id,
            "server probe running UNSANDBOXED (opt-in)"
        );
    }

    let scratch = tempfile::tempdir().map_err(|e| format!("scratch dir: {e}"))?;
    materialize_commit(repo_dir, &sha, scratch.path())
        .await
        .map_err(|e| format!("materialize {sha}: {e}"))?;
    if let Err(why) = sandbox::self_check(backend, scratch.path()).await {
        tracing::error!(
            session_id = %session_id, player_id = %player_id, backend = ?backend, reason = %why,
            "server probe sandbox is non-functional — recording unavailable"
        );
        return Err(why);
    }
    Ok(Snapshot {
        scratch,
        sha,
        age_secs,
        backend,
    })
}

/// Execute one deterministic probe server-side against the player's last
/// pushed HEAD. Infallible by design at the "our fault" boundary: everything
/// that prevents measuring becomes an `unavailable` run, not an error.
pub async fn run_deterministic_probe(
    _db: &DatabaseConnection,
    repo_dir: &Path,
    session_id: Uuid,
    player_id: Uuid,
    test: &tests::Model,
    config: &ProbeConfig,
    memory: &std::collections::BTreeMap<String, String>,
) -> ServerProbeRun {
    let _permit = probe_exec_permits().acquire().await;

    let snapshot = match prepare_snapshot(repo_dir, session_id, player_id).await {
        Ok(s) => s,
        Err(reason) => return ServerProbeRun::unavailable(reason),
    };
    let Snapshot {
        scratch,
        sha,
        age_secs: snapshot_age_secs,
        backend,
    } = snapshot;

    // Fixtures + command, exactly as the live dispatch renders them.
    let resolved = match resolve_test_fixtures(test, session_id, memory).await {
        Some(r) => r,
        None => return ServerProbeRun::unavailable("probe fixtures could not be resolved"),
    };

    // 4. Run and grade with the shared grader.
    let exec = match sandbox::run(
        backend,
        &resolved.rendered_command,
        scratch.path(),
        probe_deadline(),
    )
    .await
    {
        Ok(e) => e,
        Err(e) => return ServerProbeRun::unavailable(format!("sandbox run: {e}")),
    };

    let (pass, display_expected, _display_actual) = grade_stdout(
        &test.answer_template,
        resolved.js_validation_mode,
        &resolved.js_fixtures_for_eval,
        &resolved.fixture_defs,
        &resolved.fixture_values_for_eval,
        &resolved.expected_answer_display,
        memory,
        &exec.stdout,
        Some(exec.exit_code as i64),
    );

    let points = config.points.unwrap_or_default();
    let point_delta = if pass { points.pass } else { points.fail };

    let fixture_values_json =
        serde_json::to_string(&resolved.fixture_scalars).unwrap_or_else(|_| "{}".to_string());
    let fixture_values_json = resolved
        .secret_meta
        .redact_fixture_values(&fixture_values_json);

    ServerProbeRun {
        outcome: if pass { "pass" } else { "error" },
        rendered_command: resolved.rendered_command,
        fixture_values_json,
        expected_answer: resolved.expected_answer_display.clone(),
        resolved_answer: display_expected,
        output: exec.stdout,
        exit_code: Some(exec.exit_code),
        duration_ms: Some(exec.duration_ms as i64),
        point_delta,
        result_json: serde_json::json!({
            "snapshot_commit": sha,
            "snapshot_age_secs": snapshot_age_secs,
            "timed_out": exec.timed_out,
            "stderr": exec.stderr,
        }),
    }
}

/// Execute one analysis probe: the named tool adapter over the snapshot.
/// An unknown or broken tool records `unavailable` — never a penalty. When
/// the section carries a validation, it is graded over the metrics JSON
/// (threshold checks like `JSON.parse(result).duplicated_pct < 5`).
pub async fn run_analysis_probe(
    repo_dir: &Path,
    session_id: Uuid,
    player_id: Uuid,
    test: &tests::Model,
    config: &ProbeConfig,
    memory: &std::collections::BTreeMap<String, String>,
) -> ServerProbeRun {
    let _permit = probe_exec_permits().acquire().await;

    let tool_name = config.tool.as_deref().unwrap_or("").trim().to_string();
    let Some(adapter) = crate::analysis::find_adapter(&tool_name) else {
        return ServerProbeRun::unavailable(format!("unknown analysis tool `{tool_name}`"));
    };
    if let Err(reason) = adapter.self_check().await {
        return ServerProbeRun::unavailable(reason);
    }

    let snapshot = match prepare_snapshot(repo_dir, session_id, player_id).await {
        Ok(s) => s,
        Err(reason) => return ServerProbeRun::unavailable(reason),
    };

    let started = std::time::Instant::now();
    let result = match adapter
        .run(snapshot.scratch.path(), snapshot.backend, probe_deadline())
        .await
    {
        Ok(r) => r,
        Err(reason) => return ServerProbeRun::unavailable(reason),
    };
    let duration_ms = started.elapsed().as_millis() as i64;

    // The metrics JSON doubles as the probe's "stdout" so an optional
    // validation expression can threshold it with the ordinary grader.
    let metrics_text = result.metrics.to_string();
    let (pass, _, _) = {
        let resolved = resolve_test_fixtures(test, session_id, memory).await;
        match resolved {
            Some(resolved) => crate::ws::player_agent::grading::grade_stdout(
                &test.answer_template,
                resolved.js_validation_mode,
                &resolved.js_fixtures_for_eval,
                &resolved.fixture_defs,
                &resolved.fixture_values_for_eval,
                &resolved.expected_answer_display,
                memory,
                &metrics_text,
                None,
            ),
            // No validation resolvable: the measurement itself is the point.
            None => (true, None, None),
        }
    };
    // A section with no validation at all is a pure measurement: pass.
    let pass = pass || test.answer_template.trim().is_empty();

    let points = config.points.unwrap_or_default();
    ServerProbeRun {
        outcome: if pass { "pass" } else { "error" },
        rendered_command: format!("analysis:{tool_name}"),
        fixture_values_json: "{}".to_string(),
        expected_answer: None,
        resolved_answer: None,
        output: metrics_text,
        exit_code: Some(0),
        duration_ms: Some(duration_ms),
        point_delta: if pass { points.pass } else { points.fail },
        result_json: serde_json::json!({
            "snapshot_commit": snapshot.sha,
            "snapshot_age_secs": snapshot.age_secs,
            "analysis": result.metrics,
            "summary": result.summary,
        }),
    }
}

/// Dispatch one server-side probe by mode — the single entry the ticker,
/// the done-probe pass, and judge-declared probes all share.
pub async fn run_server_probe(
    state: &crate::state::GameServerState,
    repo_dir: &Path,
    session_id: Uuid,
    player_id: Uuid,
    test: &tests::Model,
    config: &ProbeConfig,
    memory: &std::collections::BTreeMap<String, String>,
) -> ServerProbeRun {
    use arena_core::evaluation::ProbeMode;
    match config.mode {
        ProbeMode::Deterministic => {
            run_deterministic_probe(
                &state.db, repo_dir, session_id, player_id, test, config, memory,
            )
            .await
        }
        ProbeMode::Analysis => {
            run_analysis_probe(repo_dir, session_id, player_id, test, config, memory).await
        }
        // `mode: llm` was removed (judges are the only LLM evaluation);
        // definitions that predate the removal resolve without penalty.
        ProbeMode::Llm => {
            ServerProbeRun::unavailable("`mode: llm` probes were removed — judges evaluate now")
        }
        // Interactive probes are participant-driven; the caller filters them.
        ProbeMode::Interactive => {
            ServerProbeRun::unavailable("interactive probes are not server-executed")
        }
    }
}

/// Persist a server-side run as an ordinary `probes` row — dispatched and
/// resolved in one step, history preserved like every other probe.
pub async fn record_server_probe(
    db: &DatabaseConnection,
    session_id: Uuid,
    player_id: Uuid,
    test_id: Uuid,
    run: &ServerProbeRun,
) -> Result<probes::Model, sea_orm::DbErr> {
    let now = Utc::now();
    probes::ActiveModel {
        id: Set(Uuid::new_v4()),
        test_id: Set(test_id),
        player_id: Set(player_id),
        session_id: Set(session_id),
        attempt: Set(1),
        rendered_command: Set(run.rendered_command.clone()),
        fixture_values: Set(run.fixture_values_json.clone()),
        expected_answer: Set(run.expected_answer.clone()),
        resolved_answer: Set(run.resolved_answer.clone()),
        secret_meta: Set(None),
        outcome: Set(Some(run.outcome.to_string())),
        dispatched_at: Set(now),
        deadline_at: Set(now),
        resolved_at: Set(Some(now)),
        updated_at: Set(Some(now)),
        output: Set(Some(run.output.clone())),
        exit_code: Set(run.exit_code),
        duration_ms: Set(run.duration_ms),
        point_delta: Set(Some(run.point_delta)),
        result_json: Set(Some(run.result_json.clone())),
        artifact_path: Set(None),
    }
    .insert(db)
    .await
}
