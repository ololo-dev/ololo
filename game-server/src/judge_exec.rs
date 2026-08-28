//! Execution-judge run path.
//!
//! Where an LLM judge (`judging::run_judge`) *reads* the player's committed
//! code and asks a model to rate it, an execution judge *runs* it: it
//! materializes the task's snapshot commit into a scratch working tree and
//! re-runs the task's probes against it **server-side, in a sandbox**.
//! This turns the otherwise client-self-reported golf result (pass/fail +
//! byte count) into a server-authoritative one.
//!
//! Only probes the player passed client-side are graded: the judge verifies
//! claims. Sections the player never passed (a task interrupted by the
//! session clock, or plain live failures) were already charged by the live
//! probe flow and are excluded from the denominator, so an honest player
//! whose report matches the re-run is never penalized by it.
//!
//! It reuses the live probe machinery wholesale — [`resolve_test_fixtures`]
//! samples fixtures + renders the command exactly as the live flow does, and
//! grading mirrors `player_agent::grading`. The only new step is executing the
//! rendered command in [`arena_core::sandbox`] instead of trusting a
//! client-reported stdout.

use std::path::Path;

use arena_core::entities::{judge_results, probes, sessions, tests};
use arena_core::judging::execution::{aggregate_rating, materialize_commit};
use arena_core::judging::{JudgeError, JudgeRunOutput, TaskJudgeRow};
use arena_core::sandbox::{self, SandboxBackend};
use arena_core::validation::judge_results::RatingScale;
use chrono::Utc;
use sea_orm::{
    ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, QueryOrder, Set,
    sea_query::OnConflict,
};
use uuid::Uuid;

use crate::ws::player_agent::grading::resolve_test_fixtures;

/// Per-probe wall-clock deadline. Golf solutions are tiny; a few seconds is
/// generous. Override with `OLOLO_EXEC_JUDGE_TIMEOUT_MS`.
fn probe_deadline() -> std::time::Duration {
    let ms = std::env::var("OLOLO_EXEC_JUDGE_TIMEOUT_MS")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(10_000);
    std::time::Duration::from_millis(ms.clamp(500, 120_000))
}

/// One probe's server-side outcome, for the feedback/raw-output detail.
#[derive(serde::Serialize)]
struct ProbeOutcome {
    ordinal: i32,
    pass: bool,
    exit_code: i32,
    timed_out: bool,
    /// Truncated for the stored detail.
    stdout: String,
    /// Truncated stderr — why an empty stdout was empty.
    #[serde(skip_serializing_if = "String::is_empty")]
    stderr: String,
    /// Set when the probe could not be run at all, so it was excluded from the
    /// rating instead of being charged to the player.
    #[serde(skip_serializing_if = "Option::is_none")]
    inconclusive: Option<String>,
}

/// Whether a probe produced a gradeable answer, and if not, why not.
enum Verdict {
    Pass,
    Fail,
    /// The probe never produced an answer to grade. Excluded from the
    /// denominator: our inability to run it is not the player's failure.
    Inconclusive(String),
}

/// Run an execution judge: materialize the commit, re-run every probe of the
/// task in the sandbox, and persist a `judge_results` row rated on the
/// pass-fraction. Returns the same [`JudgeRunOutput`] shape as the LLM path so
/// the caller's broadcast tail is shared.
#[allow(clippy::too_many_arguments)]
pub async fn run_execution_judge(
    db: &DatabaseConnection,
    repo_dir: &Path,
    session_id: Uuid,
    player_id: Uuid,
    task_id: Uuid,
    task_commit_sha: Option<&str>,
    task_judge_row: &TaskJudgeRow,
    // Bounded by `gate_task_judge` at the call site — see `run_judge`. The
    // judge's own scale is no longer read here: the payout ceiling has to be
    // applied once, where the gate runs.
    scale: &RatingScale,
) -> Result<JudgeRunOutput, JudgeError> {
    let started = std::time::Instant::now();
    let scale = *scale;

    // 1. Materialize the task's snapshot commit into a scratch working tree.
    let sha = task_commit_sha.ok_or(JudgeError::PlayerRepoEmpty)?;
    let scratch =
        tempfile::tempdir().map_err(|e| JudgeError::ExecFailed(format!("scratch dir: {e}")))?;
    materialize_commit(repo_dir, sha, scratch.path()).await?;

    // 2. Load the task's probes (one `tests` row per `##` section).
    let test_rows = tests::Entity::find()
        .filter(tests::Column::SessionId.eq(session_id))
        .filter(tests::Column::TaskId.eq(task_id))
        .order_by_asc(tests::Column::Ordinal)
        .all(db)
        .await?;

    // Merged per-player session memory for template rendering — same source
    // the live dispatch uses (current values; a replay after later
    // extractions uses the latest memory, which is the honest state).
    let memory_schema = match sessions::Entity::find_by_id(session_id).one(db).await? {
        Some(s) => arena_core::entities::projects::Entity::find_by_id(s.project_id_fk)
            .one(db)
            .await?
            .and_then(|p| p.memory_schema),
        None => None,
    };
    let memory =
        crate::session_memory::load_memory_map(db, session_id, player_id, memory_schema.as_deref())
            .await;

    // 2b. Only sections the player passed CLIENT-side are verified. This
    //     judge exists to make self-reported passes trustworthy; a section
    //     the player never passed is a known failure the live probes already
    //     charged. Grading it again is guaranteed double punishment when the
    //     session clock interrupts a task mid-rung: the re-run "fails" the
    //     exact sections the player honestly reported as unsolved (session
    //     OD5FJA: -33 for a truthfully-reported 154-byte solution on the
    //     ≤140 rung, with zero discrepancy found). Cheated passes still land
    //     in this set and still fail the re-run.
    let claimed: std::collections::HashSet<Uuid> = probes::Entity::find()
        .filter(probes::Column::SessionId.eq(session_id))
        .filter(probes::Column::PlayerId.eq(player_id))
        .filter(probes::Column::Outcome.eq("pass"))
        .filter(probes::Column::TestId.is_in(test_rows.iter().map(|t| t.id).collect::<Vec<_>>()))
        .all(db)
        .await?
        .into_iter()
        .map(|p| p.test_id)
        .collect();

    // Nothing claimed (e.g. interrupted before the first pass): there is
    // nothing to verify and nothing to charge — record a neutral scored row
    // so the judge pipeline (and the awards settle poll) sees a terminal
    // result instead of an error.
    if claimed.is_empty() {
        let rating = 0.0_f64.clamp(scale.min, scale.max);
        let feedback = format!(
            "Server-side re-run skipped: none of the task's {} probe(s) were passed during the \
             session, so there is nothing to verify.",
            test_rows.len()
        );
        let now = Utc::now();
        let duration_ms = started.elapsed().as_millis() as i64;
        let judge_result_id = persist_execution_result(
            db,
            session_id,
            player_id,
            task_judge_row.id,
            rating,
            0,
            &feedback,
            "[]",
            "none",
            duration_ms,
            now,
        )
        .await?;
        return Ok(JudgeRunOutput {
            rating,
            point_delta: 0,
            feedback,
            raw_output: "[]".to_string(),
            model: "execution:none".to_string(),
            judge_result_id,
            duration_ms,
        });
    }

    // 3. Backend + per-probe deadline. Fail safe: if no process sandbox is
    //    available (e.g. `bwrap` not installed in the deploy image) the judge
    //    refuses to run rather than silently executing untrusted code
    //    unsandboxed. Dev/CI opts in with OLOLO_ALLOW_UNSANDBOXED_EXEC=1.
    let backend = sandbox::detect_backend();
    if backend == SandboxBackend::Unsandboxed {
        let allowed = matches!(
            std::env::var("OLOLO_ALLOW_UNSANDBOXED_EXEC")
                .ok()
                .as_deref(),
            Some("1") | Some("true")
        );
        if !allowed {
            return Err(JudgeError::ExecFailed(
                "no process sandbox available (install bwrap); refusing to execute untrusted \
                 code unsandboxed. Set OLOLO_ALLOW_UNSANDBOXED_EXEC=1 to override in trusted envs."
                    .to_string(),
            ));
        }
        tracing::warn!(
            session_id = %session_id, player_id = %player_id,
            "execution judge running UNSANDBOXED (opt-in) — untrusted code executes without isolation"
        );
    }
    let deadline = probe_deadline();

    // 3b. Prove the sandbox can run *anything* before grading the player on
    //     its output. Without this, a non-functional sandbox (e.g. `bwrap`
    //     present but unable to create user namespaces inside the container)
    //     returns empty stdout for every probe and the pass-fraction grader
    //     reads that as "the committed solution is wrong" — penalizing honest
    //     players for our own broken infrastructure. Fail the run instead: a
    //     `failed` judge_result carries `point_delta = 0`.
    if let Err(why) = sandbox::self_check(backend, scratch.path()).await {
        tracing::error!(
            session_id = %session_id, player_id = %player_id, task_id = %task_id,
            backend = ?backend, reason = %why,
            "execution judge sandbox is non-functional — refusing to score"
        );
        return Err(JudgeError::ExecFailed(why));
    }

    // 4. Re-run each probe server-side against the materialized tree.
    let mut passed = 0usize;
    let mut gradeable = 0usize;
    let mut outcomes: Vec<ProbeOutcome> = Vec::with_capacity(test_rows.len());
    for test in &test_rows {
        if !claimed.contains(&test.id) {
            // Known-failed (or never-reached) section: the live probes
            // already charged it. Not part of the verification denominator.
            outcomes.push(ProbeOutcome {
                ordinal: test.ordinal,
                pass: false,
                exit_code: -1,
                timed_out: false,
                stdout: String::new(),
                stderr: String::new(),
                inconclusive: Some(
                    "not passed during the session; excluded from verification".to_string(),
                ),
            });
            continue;
        }
        let resolved = match resolve_test_fixtures(test, session_id, &memory).await {
            Some(r) => r,
            None => {
                // Our fixtures/command are unbuildable — nothing to do with
                // the player's code, so it must not count against them.
                outcomes.push(ProbeOutcome {
                    ordinal: test.ordinal,
                    pass: false,
                    exit_code: -1,
                    timed_out: false,
                    stdout: String::new(),
                    stderr: String::new(),
                    inconclusive: Some("probe fixtures could not be resolved".to_string()),
                });
                continue;
            }
        };

        let exec = sandbox::run(
            backend,
            &resolved.rendered_command,
            scratch.path(),
            deadline,
        )
        .await
        .map_err(|e| JudgeError::ExecFailed(e.to_string()))?;

        // Grade exactly as the live flow grades a client-reported stdout —
        // deliberately including its indifference to the exit code, so the
        // judge can never fail a solution the live probe passed.
        let verdict = if exec.stdout.is_empty() && matches!(exec.exit_code, 126 | 127) {
            // The declared `run:` command is missing or not executable *here*.
            // The player's own machine ran it fine; our image just lacks the
            // interpreter. Not a verdict on their code.
            Verdict::Inconclusive(format!(
                "declared run command unavailable in the judge sandbox (exit {})",
                exec.exit_code
            ))
        } else {
            let (pass, _, _) = crate::ws::player_agent::grading::grade_stdout(
                &test.answer_template,
                resolved.js_validation_mode,
                &resolved.js_fixtures_for_eval,
                &resolved.fixture_defs,
                &resolved.fixture_values_for_eval,
                &resolved.expected_answer_display,
                &memory,
                &exec.stdout,
                Some(exec.exit_code as i64),
            );
            if pass { Verdict::Pass } else { Verdict::Fail }
        };

        match &verdict {
            Verdict::Pass => {
                passed += 1;
                gradeable += 1;
            }
            Verdict::Fail => gradeable += 1,
            Verdict::Inconclusive(_) => {}
        }
        outcomes.push(ProbeOutcome {
            ordinal: test.ordinal,
            pass: matches!(verdict, Verdict::Pass),
            exit_code: exec.exit_code,
            timed_out: exec.timed_out,
            stdout: arena_core::judging::truncate_chars(&exec.stdout, 256),
            stderr: arena_core::judging::truncate_chars(&exec.stderr, 256),
            inconclusive: match verdict {
                Verdict::Inconclusive(why) => Some(why),
                _ => None,
            },
        });
    }

    // 5. Rating from the pass-fraction — over the probes we could actually
    //    grade. If none were gradeable there is no verdict to give: scoring
    //    0/0 would hand out the scale's failing end (a full penalty) for a
    //    run that verified nothing.
    if gradeable == 0 {
        let why = format!(
            "no probe could be executed ({} claimed probe(s), all inconclusive)",
            claimed.len()
        );
        tracing::error!(
            session_id = %session_id, player_id = %player_id, task_id = %task_id,
            detail = %serde_json::to_string(&outcomes).unwrap_or_default(),
            "execution judge could not run any probe — refusing to score"
        );
        return Err(JudgeError::ExecFailed(why));
    }
    let (rating, point_delta) = aggregate_rating(passed, gradeable, &scale);
    let backend_label = match backend {
        SandboxBackend::Bubblewrap => "bubblewrap",
        SandboxBackend::Unsandboxed => "unsandboxed",
    };
    let unclaimed = test_rows.len() - claimed.len();
    let unrunnable = claimed.len() - gradeable;
    let mut notes = Vec::new();
    if unrunnable > 0 {
        notes.push(format!(
            "{unrunnable} could not be run and were not counted"
        ));
    }
    if unclaimed > 0 {
        notes.push(format!(
            "{unclaimed} not passed during the session and not verified"
        ));
    }
    let feedback = if notes.is_empty() {
        format!("Server-side re-run: {passed}/{gradeable} probes passed.")
    } else {
        format!(
            "Server-side re-run: {passed}/{gradeable} probes passed ({}).",
            notes.join("; ")
        )
    };
    let raw_output = serde_json::to_string(&outcomes).unwrap_or_default();
    let duration_ms = started.elapsed().as_millis() as i64;

    // 6. Persist (same upsert key as the LLM path).
    let now = Utc::now();
    let judge_result_id = persist_execution_result(
        db,
        session_id,
        player_id,
        task_judge_row.id,
        rating,
        point_delta,
        &feedback,
        &raw_output,
        backend_label,
        duration_ms,
        now,
    )
    .await?;

    Ok(JudgeRunOutput {
        rating,
        point_delta,
        feedback,
        raw_output,
        model: format!("execution:{backend_label}"),
        judge_result_id,
        duration_ms,
    })
}

/// Grade an execution judge on its own declared probes' recorded outcomes
/// (§4.5, the no-LLM panel). The measurements were already taken by
/// `materialize_judge_probes`; this aggregates the latest run per declared
/// test onto the scale. `unavailable` runs leave the denominator — our
/// broken tooling is never the player's failure.
#[allow(clippy::too_many_arguments)]
pub async fn run_declared_execution_judge(
    db: &DatabaseConnection,
    session_id: Uuid,
    player_id: Uuid,
    task_judge_row: &TaskJudgeRow,
    scale: &RatingScale,
    declared_test_ids: &[Uuid],
) -> Result<JudgeRunOutput, JudgeError> {
    use sea_orm::QueryOrder;
    let started = std::time::Instant::now();

    let mut passed = 0usize;
    let mut gradeable = 0usize;
    let mut detail: Vec<serde_json::Value> = Vec::with_capacity(declared_test_ids.len());
    for test_id in declared_test_ids {
        let latest = probes::Entity::find()
            .filter(probes::Column::TestId.eq(*test_id))
            .filter(probes::Column::PlayerId.eq(player_id))
            .order_by_desc(probes::Column::DispatchedAt)
            .one(db)
            .await?;
        let outcome = latest.as_ref().and_then(|p| p.outcome.clone());
        match outcome.as_deref() {
            Some("pass") => {
                passed += 1;
                gradeable += 1;
            }
            Some("error") => gradeable += 1,
            // unavailable / no_response / never ran: excluded.
            _ => {}
        }
        detail.push(serde_json::json!({
            "test_id": test_id,
            "outcome": outcome,
            "summary": latest.as_ref().and_then(|p| p
                .result_json
                .as_ref()
                .and_then(|r| r.get("summary").cloned())),
        }));
    }

    let now = Utc::now();
    let duration_ms = started.elapsed().as_millis() as i64;
    let raw_output = serde_json::to_string(&detail).unwrap_or_else(|_| "[]".to_string());

    let (rating, point_delta, feedback) = if gradeable == 0 {
        (
            0.0_f64.clamp(scale.min, scale.max),
            0,
            "No declared probe produced a measurable outcome; nothing to grade.".to_string(),
        )
    } else {
        let (rating, point_delta) = aggregate_rating(passed, gradeable, scale);
        (
            rating,
            point_delta,
            format!("Declared probes: {passed}/{gradeable} passed."),
        )
    };

    let judge_result_id = persist_execution_result(
        db,
        session_id,
        player_id,
        task_judge_row.id,
        rating,
        point_delta,
        &feedback,
        &raw_output,
        "declared",
        duration_ms,
        now,
    )
    .await?;
    Ok(JudgeRunOutput {
        rating,
        point_delta,
        feedback,
        raw_output,
        model: "execution:declared".to_string(),
        judge_result_id,
        duration_ms,
    })
}

#[allow(clippy::too_many_arguments)]
async fn persist_execution_result(
    db: &DatabaseConnection,
    session_id: Uuid,
    player_id: Uuid,
    task_judge_id: Uuid,
    rating: f64,
    point_delta: i32,
    feedback: &str,
    raw_output: &str,
    backend_label: &str,
    duration_ms: i64,
    now: chrono::DateTime<Utc>,
) -> Result<Uuid, JudgeError> {
    let insert = judge_results::ActiveModel {
        id: Set(Uuid::new_v4()),
        session_id_fk: Set(session_id),
        player_id_fk: Set(player_id),
        task_judge_id: Set(task_judge_id),
        rating: Set(serde_json::json!(rating)),
        point_delta: Set(point_delta),
        feedback: Set(feedback.to_string()),
        model: Set(format!("execution:{backend_label}")),
        provider: Set("execution".to_string()),
        raw_output: Set(raw_output.to_string()),
        duration_ms: Set(Some(duration_ms)),
        run_log: Set(None),
        tokens_input: Set(None),
        tokens_output: Set(None),
        tokens_cache_read: Set(None),
        tokens_cache_write: Set(None),
        status: Set("scored".to_string()),
        error: Set(None),
        verdict_kind: Set(Some(arena_core::evaluation::VERDICT_KIND_FULL.to_string())),
        created_at: Set(now),
        updated_at: Set(now),
    };
    judge_results::Entity::insert(insert)
        .on_conflict(
            OnConflict::columns([
                judge_results::Column::TaskJudgeId,
                judge_results::Column::PlayerIdFk,
            ])
            .update_columns([
                judge_results::Column::Rating,
                judge_results::Column::Feedback,
                judge_results::Column::RawOutput,
                judge_results::Column::PointDelta,
                judge_results::Column::Model,
                judge_results::Column::Provider,
                judge_results::Column::DurationMs,
                judge_results::Column::TokensInput,
                judge_results::Column::TokensOutput,
                judge_results::Column::TokensCacheRead,
                judge_results::Column::TokensCacheWrite,
                judge_results::Column::Status,
                judge_results::Column::Error,
                judge_results::Column::VerdictKind,
                judge_results::Column::UpdatedAt,
            ])
            .to_owned(),
        )
        .exec_without_returning(db)
        .await?;

    let row = judge_results::Entity::find()
        .filter(judge_results::Column::TaskJudgeId.eq(task_judge_id))
        .filter(judge_results::Column::PlayerIdFk.eq(player_id))
        .one(db)
        .await?
        .ok_or_else(|| {
            JudgeError::Db(sea_orm::DbErr::Custom(
                "upsert returned no row (judge_results)".to_string(),
            ))
        })?;
    Ok(row.id)
}
