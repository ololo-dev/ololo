//! Session-scoped **appraisal**: one verdict about the whole session, on the
//! judge's own positive scale.
//!
//! Three session-scoped run paths now exist and they answer different
//! questions. [`super::session::run_session_judge`] reverses rewards — its
//! scale is `[-earned, 0]` per task, because a cheating verdict can only take
//! back what a task paid. [`super::report::run_session_report`] scores nothing
//! and writes prose. This one *awards*: it asks one question about the session
//! as a whole and pays out of the attached task's panel share.
//!
//! Why a session judge rather than a task one. Some properties are simply not
//! task-shaped. How the participant engineered their agent — the instructions
//! they committed, the skills they wrote and actually ran, the MCP servers
//! they wired up — is one setup that spans the ladder, so judging it per task
//! re-asks the same question N times, bills N runs, and gives N slightly
//! different answers. Worse, a task-scoped judge attached to the last task
//! never runs at all for the majority of players, who never reach it: the
//! session ends when the clock does, not when the ladder does.
//!
//! ## Where the points land
//!
//! The judge is attached to task rows like any other, and the settle poll
//! waits for a terminal `judge_results` row on **every** attached pair. But
//! the verdict is one verdict, so paying it out on each pair would multiply it
//! by however many tasks the project attached. Instead the run picks one
//! primary pair ([`primary_pair`]), scores it on that task's panel share, and
//! writes the same feedback with `point_delta = 0` on the rest — the poll
//! clears, the player reads the verdict wherever they open it, and the points
//! are paid once.

use std::collections::HashMap;
use std::path::Path;

use sea_orm::DatabaseConnection;
use uuid::Uuid;

use crate::validation::judge_results::{RatingScale, validate_feedback, validate_rating_output};

use super::agent_setup::{AgentSetupEvidence, build_agent_setup_evidence};
use super::criteria::CriteriaContext;
use super::dossier::{SessionDossier, build_session_dossier};
use super::session::{SessionJudgeOutput, TaskVerdict};
use super::{
    AgentResponse, JudgeError, JudgeRow, TaskJudgeRow, parse_and_score_criteria, parse_verdict,
    persist_scored_verdict_json, record_judge_run_status,
};

/// What the caller resolved before the run: where the points land, on what
/// scale, and whether the judge asked for the agent-setup evidence.
pub struct AppraisalInputs<'a> {
    /// The pair that carries the score. Every other attached pair gets the
    /// same feedback at zero points.
    pub primary_task_judge_id: Uuid,
    /// The primary pair's panel share — the same scale the task path would
    /// have given this judge on that task.
    pub scale: RatingScale,
    /// The criteria contract, when the judge scores a sheet.
    pub criteria: Option<&'a CriteriaContext>,
    /// Include the agent-setup evidence pack (`needs: [agent_setup]`).
    pub agent_setup: bool,
}

/// True when this session-scoped judge awards rather than claws back.
///
/// The distinction is the sign of its declared scale: a judge whose maximum is
/// zero can only remove points (anti-cheat), and one whose maximum is positive
/// is scoring something worth paying for. A report judge — zero-width scale —
/// is neither, and is dispatched before this is asked.
pub fn is_appraisal_judge(judge_row: &JudgeRow) -> bool {
    let scale = super::effective_rating_scale(&judge_row.rating_scale, &None);
    scale.max > 0.0
}

/// The pair that carries the score: the first task the player reached that
/// this judge is attached to, falling back to the lowest-ordinal attachment.
///
/// Deterministic on purpose — a re-run must pay the same task, or a replay
/// moves the player's points from one task to another.
pub fn primary_pair(
    reached_tasks: &[Uuid],
    task_judges: &HashMap<Uuid, TaskJudgeRow>,
) -> Option<TaskJudgeRow> {
    reached_tasks
        .iter()
        .find_map(|t| task_judges.get(t))
        .or_else(|| {
            let mut all: Vec<&TaskJudgeRow> = task_judges.values().collect();
            all.sort_by_key(|tj| tj.id);
            all.first().copied()
        })
        .cloned()
}

/// Run a session appraisal for one player.
#[allow(clippy::too_many_arguments)]
pub async fn run_session_appraisal(
    db: &DatabaseConnection,
    judge_llm: &dyn super::JudgeLlm,
    repo_dir: &Path,
    session_id: Uuid,
    player_id: Uuid,
    reached_tasks: &[Uuid],
    task_judges: &HashMap<Uuid, TaskJudgeRow>,
    judge_row: &JudgeRow,
    inputs: &AppraisalInputs<'_>,
    model: &str,
    provider: &str,
) -> Result<SessionJudgeOutput, JudgeError> {
    let started = std::time::Instant::now();
    let scope = super::tools::ToolScope::from_json(judge_row.ignore_paths.as_deref());

    let dossier =
        build_session_dossier(db, repo_dir, session_id, player_id, reached_tasks, &scope).await?;
    let setup = if inputs.agent_setup {
        let root_files: Vec<String> = dossier
            .root_commit
            .as_ref()
            .map(|r| r.files.iter().map(|f| f.path.clone()).collect())
            .unwrap_or_default();
        Some(
            build_agent_setup_evidence(db, repo_dir, session_id, player_id, &root_files, &scope)
                .await,
        )
    } else {
        None
    };
    let dossier_json = evidence_json(&dossier, setup.as_ref());

    let system = build_system_prompt(&judge_row.prompt, inputs.criteria);
    let user = build_user_prompt(&dossier, setup.as_ref(), &inputs.scale, inputs.criteria);

    // No tools: everything the judge was promised is in the pack, and a
    // session run has no per-task retry budget to spend on an investigation.
    let raw = ask(
        judge_llm,
        &system,
        &user,
        inputs.criteria.map(|c| c.keys.as_slice()),
    )
    .await?;

    let (rating_json, rating, feedback) = match inputs.criteria {
        Some(ctx) => {
            // A sheet that parses but scores the wrong keys is a contract
            // error, and reads the same to the caller as unparseable JSON.
            let scored = parse_and_score_criteria(&raw, ctx, &inputs.scale)
                .map_err(|_| JudgeError::AiParseError)?;
            let rating = crate::validation::judge_results::clamp_rating_to_scale(
                scored.mapped_rating,
                &inputs.scale,
            );
            (
                serde_json::json!({"overall": rating, "criteria": scored.verdict.criteria}),
                rating,
                scored.feedback,
            )
        }
        None => {
            let verdict = parse_verdict(&raw)?;
            let rating = crate::validation::judge_results::clamp_rating_to_scale(
                verdict.rating,
                &inputs.scale,
            );
            (serde_json::json!(rating), rating, verdict.feedback)
        }
    };
    let point_delta = validate_rating_output(rating, &inputs.scale)
        .map_err(|_| JudgeError::AiRatingOutOfRange)?;
    validate_feedback(&feedback).map_err(|_| JudgeError::FeedbackTooLong)?;

    // Where the verdict actually lives, for the rows that only exist to clear
    // the settle poll. Without this line a reader meets a zero-point verdict
    // with no explanation of why it is zero.
    let primary_task = task_judges
        .values()
        .find(|tj| tj.id == inputs.primary_task_judge_id)
        .map(|tj| tj.task_id);
    let pointer = match primary_task.and_then(|id| dossier.tasks.iter().find(|t| t.task_id == id)) {
        Some(t) => format!(
            "This judge scores the session as a whole. Its verdict and its points are \
             recorded on task {} — {}.",
            t.ordinal, t.title
        ),
        None => "This judge scores the session as a whole; its verdict and its points are \
                 recorded on another task of this session."
            .to_string(),
    };

    let duration_ms = started.elapsed().as_millis() as i64;
    let mut verdicts = Vec::with_capacity(task_judges.len());
    let mut scored = 0usize;
    let mut failed = 0usize;
    for tj in task_judges.values() {
        let primary = tj.id == inputs.primary_task_judge_id;
        // One verdict, one payout. The other rows carry a pointer rather than
        // a copy: the same scorecard rendered under two tasks reads as two
        // assessments, and a duplicated verdict doubles the judge's voice in
        // the report.
        let (delta, rating_for_row, text) = if primary {
            (point_delta, rating_json.clone(), feedback.as_str())
        } else {
            (0, serde_json::Value::Null, pointer.as_str())
        };
        match persist_scored_verdict_json(
            db,
            session_id,
            player_id,
            tj.id,
            rating_for_row,
            delta,
            text,
            if primary { &raw } else { "" },
            model,
            provider,
            duration_ms,
        )
        .await
        {
            Ok(()) => {
                scored += 1;
                verdicts.push(TaskVerdict {
                    task_id: tj.task_id,
                    task_judge_id: tj.id,
                    point_delta: Some(delta),
                    feedback: text.to_string(),
                });
            }
            Err(e) => {
                tracing::warn!(
                    session_id = %session_id, player_id = %player_id, error = ?e,
                    "session appraisal: persisting a verdict row failed"
                );
                let _ = record_judge_run_status(
                    db,
                    session_id,
                    player_id,
                    tj.id,
                    model,
                    provider,
                    "failed",
                    Some(&e.to_string()),
                    None,
                )
                .await;
                failed += 1;
                verdicts.push(TaskVerdict {
                    task_id: tj.task_id,
                    task_judge_id: tj.id,
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
        skipped: 0,
        duration_ms,
        dossier_json,
    })
}

/// One completion, then — when the model ends on prose — the extractor
/// call the task path uses: the analysis is done, only its transcription
/// into JSON is missing, and that is a separate small task rather than a
/// second full run (see `super::extract`).
async fn ask(
    judge_llm: &dyn super::JudgeLlm,
    system: &str,
    user: &str,
    keys: Option<&[String]>,
) -> Result<String, JudgeError> {
    let first = match judge_llm.run_agent(system, user, Vec::new(), None).await? {
        AgentResponse::Final { text } => text,
        AgentResponse::ToolCall { name, .. } => {
            return Err(JudgeError::Llm(format!(
                "session appraisal received an unexpected tool call '{name}'"
            )));
        }
    };
    if super::criteria::parse_any_verdict(&first).is_ok() {
        return Ok(first);
    }
    super::extract::extract_verdict(judge_llm, &first, keys).await
}

/// The evidence as stored on the run: the dossier plus, when it was gathered,
/// the agent-setup pack. Kept together so the admin run detail shows exactly
/// what the model was given.
fn evidence_json(dossier: &SessionDossier, setup: Option<&AgentSetupEvidence>) -> String {
    match setup {
        None => dossier.to_json(),
        Some(setup) => serde_json::to_string_pretty(&serde_json::json!({
            "session": serde_json::to_value(dossier).unwrap_or(serde_json::Value::Null),
            "agent_setup": serde_json::to_value(setup).unwrap_or(serde_json::Value::Null),
        }))
        .unwrap_or_else(|_| dossier.to_json()),
    }
}

fn build_system_prompt(judge_prompt: &str, criteria: Option<&CriteriaContext>) -> String {
    let contract = match criteria {
        Some(ctx) => {
            let keys = ctx
                .keys
                .iter()
                .map(|k| format!("\"{k}\""))
                .collect::<Vec<_>>()
                .join(", ");
            format!(
                "Respond with a final JSON verdict and nothing else:\n\
                 ```json\n\
                 {{\"criteria\": [{{\"key\": \"<key>\", \"score\": <0.0-10.0 or null>, \
                 \"rationale\": \"<why>\", \"evidence\": [\"file:<path>:<line>\", \
                 \"commit:<sha>\"]}}, ...], \"feedback\": \"<short review>\"}}\n\
                 ```\n\
                 Score EXACTLY these criteria keys: {keys}. Use `null` only when you \
                 genuinely cannot assess a criterion, and say why in its rationale."
            )
        }
        None => "Respond with a final JSON verdict and nothing else:\n\
                 ```json\n\
                 {\"rating\": <number>, \"feedback\": \"<short review>\"}\n\
                 ```"
        .to_string(),
    };
    format!(
        "{judge_prompt}\n\n\
         You are an AI judge writing ONE verdict about a player's whole session — \
         every task they reached, and the repository as it stands at the end of it. \
         All available evidence is already in your briefing. There are no tools to \
         call and no way to fetch more, so judge only what you can see and treat a \
         missing section as unknown rather than as absent.\n\n\
         {contract}\n\
         Answer in one response."
    )
}

fn build_user_prompt(
    dossier: &SessionDossier,
    setup: Option<&AgentSetupEvidence>,
    scale: &RatingScale,
    criteria: Option<&CriteriaContext>,
) -> String {
    let mut out = String::new();
    out.push_str(
        "## Session evidence (server-collected)\n\n\
         The factual record of this player's session: what the repository looked \
         like at the root commit, and per task, the snapshot commit with its work \
         window diff, the probe outcomes, and the agent activity reported for that \
         window.\n\n",
    );
    out.push_str(&format!("```json\n{}\n```\n\n", dossier.to_json()));

    if let Some(setup) = setup {
        out.push_str(
            "## Agent setup and its use (server-collected)\n\n\
             The left half comes from git — the agent configuration present in the \
             final snapshot, with `added_in_session` telling what the player wrote \
             from what the project shipped with. The right half (`usage`) is the \
             agent telemetry their CLI reported: which tools and skills were \
             actually invoked, by which agent, how often. `skills[].loads` joins \
             the two — a skill defined here and never loaded is decoration, and a \
             loaded skill with no definition came from the player's own machine, \
             not from this submission.\n\n\
             The telemetry is client-reported under the honest-trust model: when \
             `usage.reported` is false, nothing was captured, which says nothing \
             about what the player did. Never penalize a missing report.\n\n\
             `method_signals` are case-insensitive `git grep` hits over the final \
             tree. A hit is a mention, not a practice — read the file:line and \
             judge what is actually there.\n\n",
        );
        out.push_str(&format!("```json\n{}\n```\n\n", setup.to_json()));
        if setup.is_bare() {
            out.push_str(
                "Note: the snapshot carries no agent instructions, skills, \
                 subagents, hooks or MCP configuration at all. That is an \
                 observation to score, not a reason to abstain.\n\n",
            );
        }
    }

    out.push_str("## Your verdict\n\n");
    match criteria {
        Some(_) => out.push_str(&format!(
            "Score each criterion 0.0–10.0 for the session as a whole. Your sheet is \
             mapped onto this session's point range for this judge: {min} to {max} \
             points. Cite `file:<path>:<line>` or `commit:<sha>` evidence that \
             exists in the pack above.\n",
            min = scale.min,
            max = scale.max,
        )),
        None => out.push_str(&format!(
            "Rate the session between {min} and {max}, and say why in one or two \
             sentences citing the evidence you relied on.\n",
            min = scale.min,
            max = scale.max,
        )),
    }
    out
}
