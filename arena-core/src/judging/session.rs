//! Session-scoped judge run: one pass per (player, session) that scores every
//! task the player reached.
//!
//! A task-scoped judge runs once per commit and sees one task. Cheat signal,
//! though, lives in the trajectory — code appearing without matching agent
//! activity, a solution already present at the root commit — so a per-task
//! judge is structurally unable to see it, and pays N agentic runs per player
//! to re-derive the same history N times.
//!
//! This path instead assembles the evidence once ([`dossier`], deterministic
//! and token-free) and then asks the model one narrow question per task:
//! *given this evidence, does this task's work look genuine?* No tools, so no
//! turn budget to exhaust and nothing to hallucinate a join with.
//!
//! ## The contract with the settle poll
//!
//! `session_completion::expired_session_pending_judges` waits for a terminal
//! `judge_results` row per (player, task_judge) over the tasks the player
//! reached, and Arena Points are not awarded until it reads zero. So this
//! function must write a terminal row for **every** task it was handed —
//! including the ones whose verdict call failed, which get a `failed` row with
//! `point_delta = 0`. Returning early on the first error would hang the award
//! flow until its deadline.

use std::collections::HashMap;
use std::path::Path;

use sea_orm::DatabaseConnection;
use uuid::Uuid;

use crate::validation::judge_results::{validate_feedback, validate_rating_output};

use super::dossier::{PointsAwarded, SessionDossier, TaskEvidence, build_session_dossier};
use super::{
    AgentResponse, JudgeError, JudgeRow, TaskJudgeRow, parse_verdict, persist_scored_verdict,
    record_judge_run_status,
};

/// One task's outcome from a session-scoped run.
#[derive(Debug, Clone)]
pub struct TaskVerdict {
    pub task_id: Uuid,
    pub task_judge_id: Uuid,
    /// `None` when the verdict call failed and a `failed` row was written.
    pub point_delta: Option<i32>,
    pub feedback: String,
}

/// Result of a session-scoped run.
#[derive(Debug, Clone)]
pub struct SessionJudgeOutput {
    pub verdicts: Vec<TaskVerdict>,
    pub scored: usize,
    pub failed: usize,
    /// Tasks that earned nothing, so no verdict was needed and no LLM was called.
    pub skipped: usize,
    pub duration_ms: i64,
    /// The evidence handed to the model, for the run log / admin detail.
    pub dossier_json: String,
}

/// Run a session-scoped judge for one player.
///
/// Two different task sets are in play, and conflating them is a bug:
///
/// - `reached_tasks` is the **evidence** scope — every task the player reached,
///   from `session_completion::reached_task_ids`. The dossier spans all of it,
///   because seeing the whole trajectory is the entire reason this judge is
///   session-scoped. A judge attached to one task still gets the full picture.
/// - `task_judges` is the **scoring** scope — task id → this judge's attached
///   `task_judges` row. One verdict row is written per entry, and only these
///   pairs are what the settle poll waits on (see the module note). Projects
///   attach a session judge to one task for a single overall verdict, or to
///   every task for a per-step one.
#[allow(clippy::too_many_arguments)]
pub async fn run_session_judge(
    db: &DatabaseConnection,
    judge_llm: &dyn super::JudgeLlm,
    repo_dir: &Path,
    session_id: Uuid,
    player_id: Uuid,
    reached_tasks: &[Uuid],
    task_judges: &HashMap<Uuid, TaskJudgeRow>,
    judge_row: &JudgeRow,
    model: &str,
    provider: &str,
) -> Result<SessionJudgeOutput, JudgeError> {
    let started = std::time::Instant::now();

    // Evidence over the whole session, once, for free.
    let scope = crate::judging::tools::ToolScope::from_json(judge_row.ignore_paths.as_deref());
    let dossier =
        build_session_dossier(db, repo_dir, session_id, player_id, reached_tasks, &scope).await?;
    let dossier_json = dossier.to_json();

    let mut verdicts = Vec::with_capacity(dossier.tasks.len());
    let mut scored = 0usize;
    let mut failed = 0usize;
    let mut skipped = 0usize;

    for evidence in &dossier.tasks {
        // Score only the tasks this judge is attached to; the rest of the
        // dossier is context the model reads but is not asked to rate.
        let Some(task_judge) = task_judges.get(&evidence.task_id) else {
            continue;
        };
        let scale = clawback_scale(&evidence.points_awarded, &task_judge.rating_scale_override);

        // Nothing was banked on this task, so there is nothing to reverse and
        // no judgment to make. Skip the call — a terminal row is still written
        // so the settle poll clears.
        if scale.min >= 0.0 {
            let feedback = format!(
                "No points were awarded for this task ({} from probes, {} bonus), so there is \
                 nothing to withdraw.",
                evidence.points_awarded.from_probes, evidence.points_awarded.completion_bonus
            );
            persist_scored_verdict(
                db,
                session_id,
                player_id,
                task_judge.id,
                0.0,
                0,
                &feedback,
                "",
                model,
                provider,
                started.elapsed().as_millis() as i64,
            )
            .await?;
            skipped += 1;
            verdicts.push(TaskVerdict {
                task_id: evidence.task_id,
                task_judge_id: task_judge.id,
                point_delta: Some(0),
                feedback,
            });
            continue;
        }

        match score_one_task(judge_llm, &judge_row.prompt, &dossier, evidence, &scale)
            .await
            .and_then(|(rating, feedback, raw)| {
                // Out-of-range pulls to the nearest bound instead of failing:
                // magnitude alone must not cost a verdict (or a re-billed
                // re-run). What remains of validation fails THIS task, not
                // the whole pass — aborting here left every later task
                // without a terminal row, which the recovery sweeps then
                // re-ran (and re-billed) forever.
                let rating =
                    crate::validation::judge_results::clamp_rating_to_scale(rating, &scale);
                validate_rating_output(rating, &scale)
                    .map(|point_delta| (rating, point_delta, feedback, raw))
                    .map_err(|_| JudgeError::AiRatingOutOfRange)
            }) {
            Ok((rating, point_delta, feedback, raw)) => {
                persist_scored_verdict(
                    db,
                    session_id,
                    player_id,
                    task_judge.id,
                    rating,
                    point_delta,
                    &feedback,
                    &raw,
                    model,
                    provider,
                    started.elapsed().as_millis() as i64,
                )
                .await?;
                scored += 1;
                verdicts.push(TaskVerdict {
                    task_id: evidence.task_id,
                    task_judge_id: task_judge.id,
                    point_delta: Some(point_delta),
                    feedback,
                });
            }
            Err(e) => {
                // Isolate the failure to this task: a terminal row keeps the
                // settle poll moving, and point_delta 0 means the player is
                // neither rewarded nor penalized for our failure to judge.
                tracing::warn!(
                    session_id = %session_id, player_id = %player_id,
                    task_id = %evidence.task_id, error = ?e,
                    "session judge: task verdict failed; recording it as failed"
                );
                let _ = record_judge_run_status(
                    db,
                    session_id,
                    player_id,
                    task_judge.id,
                    model,
                    provider,
                    "failed",
                    Some(&e.to_string()),
                    None,
                )
                .await;
                failed += 1;
                verdicts.push(TaskVerdict {
                    task_id: evidence.task_id,
                    task_judge_id: task_judge.id,
                    point_delta: None,
                    feedback: String::new(),
                });
            }
        }
    }

    Ok(SessionJudgeOutput {
        verdicts,
        scored,
        failed,
        skipped,
        duration_ms: started.elapsed().as_millis() as i64,
        dossier_json,
    })
}

/// The rating scale for one task under clawback semantics — `[-earned, 0]`.
///
/// A cheating verdict reverses a reward, so it can never take more than the
/// task actually paid: rate `0` to leave the points alone, `-earned` to strip
/// them entirely, or anything between to remove part. This bounds the whole
/// judge by construction — attached to every task of a session, the worst it
/// can do is return the player to zero, which is why a fixed `-500`-style scale
/// is neither needed nor wanted here.
///
/// An admin override on the `task_judges` row still applies as a floor, so a
/// project can cap the clawback below the full amount.
fn clawback_scale(
    awarded: &PointsAwarded,
    override_: &Option<serde_json::Value>,
) -> crate::validation::judge_results::RatingScale {
    let earned = awarded.total.max(0) as f64;
    let mut scale = crate::validation::judge_results::RatingScale {
        min: -earned,
        max: 0.0,
        step: 1.0,
    };
    // An explicit override tightens the floor; it never loosens it.
    if let Some(v) = override_
        && let Some(min) = v.get("min").and_then(|m| m.as_f64())
        && min > scale.min
    {
        scale.min = min;
    }
    scale
}

/// Ask the model for one task's verdict. Returns `(rating, feedback, raw)`.
///
/// No tools are offered: the dossier already carries the evidence, and a model
/// that cannot call tools cannot spend a turn budget or invent a join.
async fn score_one_task(
    judge_llm: &dyn super::JudgeLlm,
    judge_prompt: &str,
    dossier: &SessionDossier,
    evidence: &TaskEvidence,
    scale: &crate::validation::judge_results::RatingScale,
) -> Result<(f64, String, String), JudgeError> {
    let user = build_task_prompt(dossier, evidence, scale);
    let resp = judge_llm
        .run_agent(judge_prompt, &user, Vec::new(), None)
        .await?;
    let raw = match resp {
        AgentResponse::Final { text } => text,
        // Tools were not offered; a tool call is a protocol violation.
        AgentResponse::ToolCall { name, .. } => {
            return Err(JudgeError::Llm(format!(
                "session judge received an unexpected tool call '{name}'"
            )));
        }
    };
    let verdict = parse_verdict(&raw)?;
    validate_feedback(&verdict.feedback).map_err(|_| JudgeError::FeedbackTooLong)?;
    Ok((verdict.rating, verdict.feedback, raw))
}

/// The per-task user prompt: session-wide evidence, then the task to rate.
///
/// The dossier JSON leads so it stays a stable prefix across every task of the
/// session — provider prompt caching then makes the second and later calls
/// much cheaper than the first.
fn build_task_prompt(
    dossier: &SessionDossier,
    evidence: &TaskEvidence,
    scale: &crate::validation::judge_results::RatingScale,
) -> String {
    format!(
        "## Session evidence (server-collected, all tasks)\n\n\
         This is the complete factual record for this player's session: what the \
         repository looked like at the root commit, and per task, the snapshot \
         commit with the diff of its whole work window, the probe outcomes, and \
         the agent activity reported for that window.\n\n\
         ```json\n{dossier_json}\n```\n\n\
         ### Reading the evidence\n\n\
         - `root_commit.files` is what the player started from. Functionality \
         the task asked for that already exists there was NOT written during \
         the session.\n\
         - `flags` are objective observations, not conclusions. \
         `no_agent_stats` and `zero_agent_activity` come from client-reported \
         telemetry which players are trusted to send: absent or zero activity \
         is weak supporting evidence and NEVER sufficient on its own.\n\
         - `passed_without_changes` means probes passed for a task whose \
         whole work window — every commit from the previous task's snapshot \
         to its own — changed nothing. The strongest signal here, because it \
         is derived from git, not from client reports.\n\n\
         ## Task to rate now\n\n\
         Task {ordinal}: {title}\n\n\
         Evidence for this task:\n\n\
         ```json\n{task_json}\n```\n\n\
         ## Your response\n\n\
         This player banked **{earned} points** on this task ({probes} from probes, \
         {bonus} completion bonus). Your rating is how much of that to WITHDRAW:\n\n\
         - `0` — keep all {earned} points. The work looks genuine. This is the \
         normal answer.\n\
         - `{min}` — withdraw everything the task paid, because the evidence \
         shows it was not earned during this session.\n\
         - anything between — withdraw that many points, when only part of the \
         task was pre-existing.\n\n\
         You cannot take more than the task paid, so a wrong call costs the \
         player at most these {earned} points — but it does cost them real \
         points, so require real evidence.\n\n\
         Respond with ONLY a JSON object, no other text:\n\
         {{\"rating\": <number between {min} and 0, whole numbers>, \
         \"feedback\": \"<one or two sentences citing the specific evidence you \
         relied on>\"}}",
        dossier_json = dossier.to_json(),
        ordinal = evidence.ordinal,
        title = evidence.title,
        task_json = serde_json::to_string_pretty(evidence).unwrap_or_else(|_| "{}".to_string()),
        earned = evidence.points_awarded.total,
        probes = evidence.points_awarded.from_probes,
        bonus = evidence.points_awarded.completion_bonus,
        min = scale.min,
    )
}
