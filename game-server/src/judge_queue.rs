//! Judge queue: resolves DB rows, acquires the semaphore, runs a judge,
//! and fans results out to observers + the player agent.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use chrono::Utc;
use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter};
use tokio::sync::OwnedSemaphorePermit;
use uuid::Uuid;

use arena_core::entities::{
    activity_event, judge_results, judges, players, task_agent_stats, task_judges, task_results,
    tasks,
};
use arena_core::git_store::{player_repo_path, repos_base_dir};
use arena_core::judging::{
    self, AgentResponse, JUDGE_SCOPE_SESSION, JudgeError, JudgeLlm, JudgeLogEvent, JudgeRow,
    JudgeRunOutput, JudgeRunRecorder, PriorJudgeResult, PriorSessionVerdict, PriorTaskResult,
    TaskJudgeRow, TaskRow, ToolDef, log_now_ms, truncate_chars,
};
use arena_core::llm::ModelConfig;
use arena_core::llm::resolve::LlmOverride;
use arena_core::protocol::{
    JudgeScoredPayload, PlayerAgentFrame, PlayerJudgeStatusPayload, ZmqEvent,
};

use crate::state::GameServerState;

/// Resolved context for a judge run — everything needed to call
/// `execute_judge_run` without touching the DB again (except `run_judge`'s
/// upsert). The semaphore permit is held here so the caller controls
/// concurrency lifetime.
#[derive(Debug)]
pub struct ResolvedJudgeRun {
    pub task_judge_row: TaskJudgeRow,
    pub judge_row: JudgeRow,
    pub task_row: TaskRow,
    pub prior_results: Vec<PriorTaskResult>,
    pub prior_judge_result: Option<PriorJudgeResult>,
    /// This judge's own verdicts on EARLIER tasks of the same session.
    pub prior_session_verdicts: Vec<PriorSessionVerdict>,
    pub repo_dir: PathBuf,
    pub task_commit_sha: Option<String>,
    /// Prefetched `task_agent_stats` payload for the `get_task_stats` judge
    /// tool (client-reported by ololo). `None` when nothing was reported.
    pub task_stats_json: Option<String>,
    pub join_code: String,
    pub _permit: OwnedSemaphorePermit,
}

/// This judge's own scored verdicts on the player's EARLIER tasks in this
/// session, oldest first.
///
/// Scoped hard: same session, same player, same judge, and only tasks that
/// come before the one being judged — a verdict must never be influenced by
/// work the player had not done yet, and a re-run of the CURRENT task is
/// separate context (`prior_judge_result`). Only settled (`scored`) rows
/// carry reasoning worth passing on.
async fn load_prior_session_verdicts(
    db: &DatabaseConnection,
    session_id: Uuid,
    player_id: Uuid,
    judge_id: Uuid,
    current_ordinal: i32,
) -> Result<Vec<PriorSessionVerdict>, JudgeError> {
    // Every (task, judge) pair this judge is attached to.
    let pairs = task_judges::Entity::find()
        .filter(task_judges::Column::JudgeId.eq(judge_id))
        .all(db)
        .await?;
    if pairs.is_empty() {
        return Ok(Vec::new());
    }
    let task_by_pair: std::collections::HashMap<Uuid, Uuid> =
        pairs.iter().map(|tj| (tj.id, tj.task_id)).collect();

    let results = judge_results::Entity::find()
        .filter(judge_results::Column::SessionIdFk.eq(session_id))
        .filter(judge_results::Column::PlayerIdFk.eq(player_id))
        .filter(judge_results::Column::TaskJudgeId.is_in(task_by_pair.keys().copied()))
        .filter(judge_results::Column::Status.eq("scored"))
        .all(db)
        .await?;
    if results.is_empty() {
        return Ok(Vec::new());
    }

    let task_ids: Vec<Uuid> = results
        .iter()
        .filter_map(|r| task_by_pair.get(&r.task_judge_id).copied())
        .collect();
    let task_rows = tasks::Entity::find()
        .filter(tasks::Column::Id.is_in(task_ids))
        .all(db)
        .await?;
    let task_meta: std::collections::HashMap<Uuid, (i32, String)> = task_rows
        .into_iter()
        .map(|t| (t.id, (t.ordinal, t.title)))
        .collect();

    let mut out: Vec<PriorSessionVerdict> = results
        .into_iter()
        .filter_map(|r| {
            let task_id = task_by_pair.get(&r.task_judge_id)?;
            let (ordinal, title) = task_meta.get(task_id)?.clone();
            (ordinal < current_ordinal).then(|| PriorSessionVerdict {
                task_ordinal: ordinal,
                task_title: title,
                rating: judging::rating_scalar(&r.rating),
                point_delta: r.point_delta,
                feedback: r.feedback,
            })
        })
        .collect();
    out.sort_by_key(|v| v.task_ordinal);
    Ok(out)
}

/// Load the task's agent statistics (if reported) and render the JSON payload
/// the `get_task_stats` judge tool returns.
async fn load_task_stats_json(
    db: &DatabaseConnection,
    session_id: Uuid,
    player_id: Uuid,
    task_id: Uuid,
) -> Option<String> {
    let row = task_agent_stats::Entity::find()
        .filter(task_agent_stats::Column::SessionIdFk.eq(session_id))
        .filter(task_agent_stats::Column::PlayerIdFk.eq(player_id))
        .filter(task_agent_stats::Column::TaskIdFk.eq(task_id))
        .one(db)
        .await
        .ok()
        .flatten()?;
    let agents: serde_json::Value =
        serde_json::from_str(&row.agents_json).unwrap_or(serde_json::Value::Array(vec![]));
    let payload = serde_json::json!({
        "task_ordinal": row.task_ordinal,
        "window_started_at": row.window_started_at.map(|t| t.to_rfc3339()),
        "window_ended_at": row.window_ended_at.map(|t| t.to_rfc3339()),
        "totals": {
            "input_tokens": row.input_tokens,
            "output_tokens": row.output_tokens,
            "cache_read_tokens": row.cache_read_tokens,
            "cache_write_tokens": row.cache_write_tokens,
            "reasoning_tokens": row.reasoning_tokens,
            "cost": row.cost,
            "user_messages": row.user_messages,
            "assistant_messages": row.assistant_messages,
            "tool_calls": row.tool_calls,
        },
        "agents": agents,
        "note": "client-reported telemetry from the player's machine; supporting evidence, not proof",
    });
    Some(payload.to_string())
}

/// Process-wide admission gate bounding how many judge-dispatch tasks may be
/// in flight at once. Each `run_task_judges_after_commit` task waits up to ~2min
/// for the player's commit + stats before it even acquires the concurrency
/// semaphore, so under a burst they pile up without a bound. Callers hold an
/// admission permit for the task's lifetime and shed (defer to the recovery
/// sweep, which re-runs any judge that was never recorded) when it is full.
/// Cap: `ARENA_JUDGE_MAX_QUEUED`, default 512.
pub fn judge_admission() -> &'static tokio::sync::Semaphore {
    static ADMISSION: std::sync::OnceLock<tokio::sync::Semaphore> = std::sync::OnceLock::new();
    ADMISSION.get_or_init(|| {
        let cap = std::env::var("ARENA_JUDGE_MAX_QUEUED")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .filter(|n| *n > 0)
            .unwrap_or(512);
        tokio::sync::Semaphore::new(cap)
    })
}

/// Best-effort delivery of a frame to the player-agent WS. The judge score is
/// already persisted and published over ZMQ (the browser path); this is the
/// CLI's real-time nudge. A full channel (agent draining slowly) is logged
/// rather than silently dropped, and a closed channel (agent disconnected) is
/// ignored.
fn send_agent_frame(
    tx: &tokio::sync::mpsc::Sender<PlayerAgentFrame>,
    frame: PlayerAgentFrame,
    player_id: Uuid,
) {
    use tokio::sync::mpsc::error::TrySendError;
    match tx.try_send(frame) {
        Ok(()) => {}
        Err(TrySendError::Full(_)) => {
            tracing::warn!(%player_id, "player-agent channel full; dropped a judge frame");
        }
        Err(TrySendError::Closed(_)) => {}
    }
}

/// Resolve all DB rows, acquire the judge semaphore, and return the context
/// needed to run the judge. Splits the resolution phase from execution so
/// tests can inject a `FakeJudgeLlm` without touching Ollama.
pub async fn resolve_judge_run(
    state: &GameServerState,
    db: &DatabaseConnection,
    session_id: Uuid,
    player_id: Uuid,
    task_id: Uuid,
    judge_id: Uuid,
) -> Result<ResolvedJudgeRun, JudgeError> {
    // 1. task_judges row for (task_id, judge_id).
    let tj = task_judges::Entity::find()
        .filter(task_judges::Column::TaskId.eq(task_id))
        .filter(task_judges::Column::JudgeId.eq(judge_id))
        .one(db)
        .await?
        .ok_or_else(|| JudgeError::GitReadError("task_judge not found".to_string()))?;
    let task_judge_row = TaskJudgeRow {
        id: tj.id,
        task_id: tj.task_id,
        judge_id: tj.judge_id,
        rating_scale_override: tj.rating_scale_override.clone(),
        weight: tj.weight,
    };

    // 2. hydrate judge row.
    let j = judges::Entity::find_by_id(judge_id)
        .one(db)
        .await?
        .ok_or_else(|| JudgeError::GitReadError("judge not found".to_string()))?;
    let judge_row = JudgeRow {
        slug: j.slug,
        name: j.name,
        prompt: j.prompt,
        rating_scale: j.rating_scale,
        kind: j.kind,
        scope: j.scope,
        evidence_mode: j.evidence_mode,
        evidence_needs: j.evidence_needs,
        llm_provider_id: j.llm_provider_id_fk,
        llm_pool_id: j.llm_pool_id_fk,
        llm_source_order: j.llm_source_order.clone(),
        llm_model: j.llm_model,
        ignore_paths: j.ignore_paths,
        criteria: j.criteria,
        max_interactive: j.max_interactive,
    };

    // 3. hydrate task row.
    let t = tasks::Entity::find_by_id(task_judge_row.task_id)
        .one(db)
        .await?
        .ok_or_else(|| JudgeError::GitReadError("task not found".to_string()))?;
    let t_ordinal = t.ordinal;
    let task_row = TaskRow {
        id: t.id,
        title: t.title,
        description: t.content,
        tags: t.tags,
        point_value: t.point_value,
        evaluation: t.evaluation.clone(),
    };

    // 4. prior task_results for (session_id, player_id, task_id).
    let prior_results: Vec<PriorTaskResult> = task_results::Entity::find()
        .filter(task_results::Column::SessionIdFk.eq(session_id))
        .filter(task_results::Column::PlayerIdFk.eq(player_id))
        .filter(task_results::Column::TaskId.eq(task_id))
        .all(db)
        .await?
        .into_iter()
        .map(|r| PriorTaskResult {
            point_delta: r.point_delta,
            answer: r.answer,
        })
        .collect();

    // 5. prior judge_results for (task_judge_id, player_id).
    let prior_judge = judge_results::Entity::find()
        .filter(judge_results::Column::TaskJudgeId.eq(task_judge_row.id))
        .filter(judge_results::Column::PlayerIdFk.eq(player_id))
        .one(db)
        .await?;
    let prior_judge_result = match prior_judge.as_ref() {
        Some(r) => {
            // A waiting row may hold the paused conversation of the run
            // that registered the request; the new run resumes it rather
            // than investigating again. Any other status starts fresh — an
            // admin re-run of a scored judge is a new opinion.
            let transcript = if r.status == "waiting" {
                arena_core::entities::judge_run_transcripts::Entity::find_by_id(r.id)
                    .one(db)
                    .await?
                    .map(|t| t.transcript)
            } else {
                None
            };
            Some(PriorJudgeResult {
                rating: judging::rating_scalar(&r.rating),
                feedback: r.feedback.clone(),
                requests: load_prior_requests(db, session_id, player_id, task_id, judge_id).await?,
                transcript,
            })
        }
        None => None,
    };

    // 5b. What THIS judge already said about this player's earlier tasks in
    // this session. A project's tasks build on one another, so a verdict
    // written blind re-litigates settled ground.
    let prior_session_verdicts =
        load_prior_session_verdicts(db, session_id, player_id, judge_id, t_ordinal).await?;

    // 6. repo_dir + task commit SHA.
    let base = repos_base_dir().ok_or_else(|| JudgeError::PlayerRepoNotFound)?;
    let repo_dir: PathBuf = player_repo_path(&base, session_id, player_id);
    let task_commit_sha = judging::resolve_task_commit(&repo_dir, task_id)
        .await?
        .map(|(sha, _)| sha);
    let task_stats_json = load_task_stats_json(db, session_id, player_id, task_id).await;

    // 7. acquire semaphore permit.
    let permit = state
        .judge_semaphore
        .clone()
        .acquire_owned()
        .await
        .map_err(|e| JudgeError::GitReadError(format!("semaphore closed: {e}")))?;

    // 9. resolve join_code for broadcasts.
    let join_code = arena_core::entities::sessions::Entity::find_by_id(session_id)
        .one(db)
        .await?
        .map(|s| s.join_code)
        .unwrap_or_default();

    Ok(ResolvedJudgeRun {
        task_judge_row,
        judge_row,
        task_row,
        prior_results,
        prior_judge_result,
        prior_session_verdicts,
        repo_dir,
        task_commit_sha,
        task_stats_json,
        join_code,
        _permit: permit,
    })
}

/// Execute the judge pipeline using a provided `JudgeLlm` impl, then
/// broadcast results. Separated from `resolve_judge_run` so tests can inject
/// a fake LLM without needing a running Ollama instance.
#[allow(clippy::too_many_arguments)]
/// Screenshot artifacts already delivered for this (session, player, task),
/// as vision attachments. Capped at 2 images / 4 MiB each: enough for a
/// widget capture, small enough not to blow a provider's request limit.
pub async fn load_artifact_images(
    db: &DatabaseConnection,
    session_id: Uuid,
    player_id: Uuid,
    task_id: Uuid,
) -> Vec<arena_core::judging::JudgeImage> {
    use arena_core::entities::{probes, tests};
    use arena_core::evaluation::ProbeConfig;
    use base64::Engine as _;

    // Per-request files × per-judge requests can pile up; the caps bound
    // what one vision call carries. Per-image and total-byte limits keep a
    // provider's request-size ceiling out of reach.
    const MAX_IMAGES: usize = 6;
    const MAX_BYTES: usize = 4 * 1024 * 1024;
    const MAX_TOTAL_BYTES: usize = 12 * 1024 * 1024;

    let Ok(test_rows) = tests::Entity::find()
        .filter(tests::Column::SessionId.eq(session_id))
        .filter(tests::Column::TaskId.eq(task_id))
        .filter(tests::Column::ProbeConfig.is_not_null())
        .all(db)
        .await
    else {
        return Vec::new();
    };
    let Some(repos_base) = arena_core::git_store::repos_base_dir() else {
        return Vec::new();
    };
    let repo_dir = arena_core::git_store::player_repo_path(&repos_base, session_id, player_id);

    let mut out = Vec::new();
    for test in test_rows {
        if out.len() >= MAX_IMAGES {
            break;
        }
        let Some(config) = test
            .probe_config
            .as_ref()
            .and_then(|c| ProbeConfig::from_json(c).ok())
        else {
            continue;
        };
        let Some(artifact) = &config.artifact else {
            continue;
        };
        let is_image = artifact.content_type.starts_with("image/");
        let is_video = artifact.content_type.starts_with("video/");
        if !is_image && !is_video {
            continue;
        }
        let Ok(Some(probe)) = probes::Entity::find()
            .filter(probes::Column::TestId.eq(test.id))
            .filter(probes::Column::PlayerId.eq(player_id))
            .filter(probes::Column::ArtifactPath.is_not_null())
            .one(db)
            .await
        else {
            continue;
        };
        let Some(reference) = probe.artifact_path else {
            continue;
        };
        // A request may have delivered several files — read them all (the
        // sweep already bounded the list); fall back to the single stored
        // reference for rows recorded before the list existed.
        let refs: Vec<String> = probe
            .result_json
            .as_ref()
            .and_then(|r| {
                let a = r.get("artifact")?;
                let commit = a.get("commit")?.as_str()?;
                let files = a.get("files")?.as_array()?;
                Some(
                    files
                        .iter()
                        .filter_map(|f| f.get("path")?.as_str().map(|p| format!("{commit}:{p}")))
                        .collect::<Vec<_>>(),
                )
            })
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| vec![reference.clone()]);
        let mut total: usize = out
            .iter()
            .map(|i: &arena_core::judging::JudgeImage| i.base64.len() * 3 / 4)
            .sum();
        for reference in refs {
            if out.len() >= MAX_IMAGES || total >= MAX_TOTAL_BYTES {
                break;
            }
            let Some(bytes) = crate::artifacts::read_artifact_blob(&repo_dir, &reference).await
            else {
                continue;
            };
            if bytes.is_empty() {
                continue;
            }
            let path_label = reference
                .split_once(':')
                .map(|(_, p)| p.to_string())
                .unwrap_or_else(|| "?".to_string());
            // The delivered file overrides the declared type: a video/webm
            // request answered with a .gif is still a screencast, and ffmpeg
            // samples GIF frames the same way — while a .png answered to it
            // is a plain screenshot.
            let file_ct = arena_core::evaluation::artifact_content_type_for_path(&path_label)
                .unwrap_or(artifact.content_type.as_str());
            if file_ct.starts_with("video/") || file_ct == "image/gif" {
                // The model cannot watch video — sample the screencast into
                // frames and let the judge see the flow it demonstrates.
                if bytes.len() as u64 > artifact.max_bytes {
                    continue;
                }
                let budget = MAX_IMAGES - out.len();
                let frames = crate::artifacts::extract_video_frames(&bytes, budget.min(4)).await;
                let n = frames.len();
                for (i, frame) in frames.into_iter().enumerate() {
                    if out.len() >= MAX_IMAGES || total + frame.jpeg.len() > MAX_TOTAL_BYTES {
                        break;
                    }
                    total += frame.jpeg.len();
                    out.push(arena_core::judging::JudgeImage {
                        media_type: "image/jpeg".to_string(),
                        base64: base64::engine::general_purpose::STANDARD.encode(&frame.jpeg),
                        label: format!(
                            "screencast key frame {}/{} at {:.1}s from probe {} ({})",
                            i + 1,
                            n,
                            frame.at_secs,
                            probe.id,
                            path_label,
                        ),
                    });
                }
                continue;
            }
            if bytes.len() > MAX_BYTES || total + bytes.len() > MAX_TOTAL_BYTES {
                continue;
            }
            total += bytes.len();
            out.push(arena_core::judging::JudgeImage {
                media_type: file_ct.to_string(),
                base64: base64::engine::general_purpose::STANDARD.encode(&bytes),
                label: format!(
                    "screenshot artifact from probe {} ({})",
                    probe.id, path_label,
                ),
            });
        }
    }

    // Top up with images the participant committed on their own: agents
    // often screenshot into the repo without answering the request probe,
    // and a vision judge should see those rather than nothing.
    //
    // Only images this SESSION added. The tree at HEAD also carries whatever
    // the session started from — a campaign part imports the previous part's
    // whole workspace, stale `.ololo/artifacts/` deliveries included — and
    // attaching those as "participant-delivered" had a judge scoring part 3
    // against part 1's screenshots (plum NJXDD5: "the provided UI screenshots
    // show only the earlier EUR ledger", −28 on Correctness). Diffing against
    // the root snapshot keeps in-session captures and drops inherited ones;
    // an unreadable root attaches nothing rather than someone else's pixels.
    if out.len() < MAX_IMAGES {
        let listed = tokio::task::spawn_blocking({
            let repo_dir = repo_dir.clone();
            move || {
                let git_bin = which::which("git").ok()?;
                let root = std::process::Command::new(&git_bin)
                    .arg("-C")
                    .arg(&repo_dir)
                    .arg("rev-list")
                    .arg("--max-parents=0")
                    .arg("HEAD")
                    .output()
                    .ok()?;
                if !root.status.success() {
                    return None;
                }
                let root_sha = String::from_utf8_lossy(&root.stdout)
                    .lines()
                    .next()
                    .map(str::trim)
                    .map(str::to_string)?;
                let diff = std::process::Command::new(&git_bin)
                    .arg("-C")
                    .arg(&repo_dir)
                    .arg("diff")
                    .arg("--name-only")
                    .arg("--diff-filter=AM")
                    .arg(&root_sha)
                    .arg("HEAD")
                    .output()
                    .ok()?;
                diff.status.success().then(|| {
                    String::from_utf8_lossy(&diff.stdout)
                        .lines()
                        .map(str::to_string)
                        .collect::<Vec<_>>()
                })
            }
        })
        .await
        .ok()
        .flatten()
        .unwrap_or_default();
        let mut candidates: Vec<String> = listed
            .into_iter()
            .filter(|p| {
                let lower = p.to_ascii_lowercase();
                [".png", ".jpg", ".jpeg", ".webp"]
                    .iter()
                    .any(|ext| lower.ends_with(ext))
                    && !lower.contains("node_modules/")
            })
            .collect();
        candidates.sort_by_key(|p| {
            let lower = p.to_ascii_lowercase();
            let looks = lower.contains("screenshot") || lower.contains(".ololo/");
            (!looks, p.clone())
        });
        for path in candidates {
            if out.len() >= MAX_IMAGES {
                break;
            }
            let Some(bytes) =
                crate::artifacts::read_artifact_blob(&repo_dir, &format!("HEAD:{path}")).await
            else {
                continue;
            };
            if bytes.is_empty() || bytes.len() > MAX_BYTES {
                continue;
            }
            let lower = path.to_ascii_lowercase();
            let media_type = if lower.ends_with(".png") {
                "image/png"
            } else if lower.ends_with(".webp") {
                "image/webp"
            } else {
                "image/jpeg"
            };
            out.push(arena_core::judging::JudgeImage {
                media_type: media_type.to_string(),
                base64: base64::engine::general_purpose::STANDARD.encode(&bytes),
                label: format!("screenshot committed during this session ({path})"),
            });
        }
    }
    out
}

/// Publish `EvaluationReady` once every judge attached to an open-ended
/// task has a terminal verdict for this player.
async fn maybe_publish_evaluation_ready(
    state: &GameServerState,
    db: &DatabaseConnection,
    session_id: Uuid,
    player_id: Uuid,
    task_id: Uuid,
    join_code: &str,
) {
    let _ = session_id;
    let Ok(Some(task)) = arena_core::entities::tasks::Entity::find_by_id(task_id)
        .one(db)
        .await
    else {
        return;
    };
    if task.evaluation.is_none() {
        return;
    }
    let Ok(attached) = task_judges::Entity::find()
        .filter(task_judges::Column::TaskId.eq(task_id))
        .all(db)
        .await
    else {
        return;
    };
    if attached.is_empty() {
        return;
    }
    for tj in &attached {
        let terminal = judge_results::Entity::find()
            .filter(judge_results::Column::TaskJudgeId.eq(tj.id))
            .filter(judge_results::Column::PlayerIdFk.eq(player_id))
            .filter(judge_results::Column::Status.is_in(["scored", "failed"]))
            .one(db)
            .await
            .ok()
            .flatten()
            .is_some();
        if !terminal {
            return;
        }
    }
    state
        .event_publisher
        .publish(&ZmqEvent::EvaluationReady {
            join_code: join_code.to_string(),
            player_id,
            task_id,
            timestamp: Utc::now(),
        })
        .await;
}

/// Wire a probe registrar for this run — only for open-ended tasks. The
/// per-task interactive limit comes from the evaluation contract; the
/// judge's own budget from `judges.max_interactive`.
async fn build_registrar(
    state: &GameServerState,
    db: &DatabaseConnection,
    session_id: Uuid,
    player_id: Uuid,
    task_id: Uuid,
    judge_id: Uuid,
) -> Option<std::sync::Arc<dyn judging::ProbeRegistrar>> {
    let task = arena_core::entities::tasks::Entity::find_by_id(task_id)
        .one(db)
        .await
        .ok()
        .flatten()?;
    let contract =
        arena_core::evaluation::EvaluationContract::from_json(task.evaluation.as_ref()?).ok()?;
    let judge = judges::Entity::find_by_id(judge_id)
        .one(db)
        .await
        .ok()
        .flatten()?;
    // The judge's own budget, further capped by the task author's
    // per-judge limit — whichever is tighter wins.
    let per_judge = judge
        .max_interactive
        .unwrap_or(0)
        .min(contract.limits.interactive_probes_per_judge as i32);
    let sees_images =
        judging::evidence::EvidenceNeeds::from_column(judge.evidence_needs.as_deref()).images;
    Some(crate::judge_registrar::JudgeProbeRegistrar::new(
        state.clone(),
        session_id,
        player_id,
        task_id,
        judge_id,
        judge.slug,
        per_judge,
        contract.limits.interactive_probes_per_task,
        sees_images,
    ))
}

/// Materialize a judge's declared probes as `tests` rows
/// (`initiator='judge'`) and run the server-side ones that have not run for
/// this player yet. Returns the declared tests' ids (for the execution
/// judge's aggregate). Idempotent: a re-judge reuses the recorded rows —
/// reproducibility comes from storage, not from re-measuring.
async fn materialize_judge_probes(
    state: &GameServerState,
    db: &DatabaseConnection,
    judge_id: Uuid,
    session_id: Uuid,
    player_id: Uuid,
    task_id: Uuid,
) -> Vec<Uuid> {
    use arena_core::entities::{judges as judges_entity, probes, tests};
    use arena_core::evaluation::{JudgeProbeDef, ProbeExecutor, ProbeMode};

    let Ok(Some(judge)) = judges_entity::Entity::find_by_id(judge_id).one(db).await else {
        return Vec::new();
    };
    let Some(config_json) = judge.probes_config else {
        return Vec::new();
    };
    let defs: Vec<JudgeProbeDef> = match serde_json::from_value(config_json) {
        Ok(d) => d,
        Err(e) => {
            tracing::warn!(judge = %judge.slug, error = %e, "unparseable judges.probes_config");
            return Vec::new();
        }
    };

    let existing = tests::Entity::find()
        .filter(tests::Column::SessionId.eq(session_id))
        .filter(tests::Column::TaskId.eq(task_id))
        .all(db)
        .await
        .unwrap_or_default();
    // Judge probes live above the task's own sections; 1000+ keeps them out
    // of the section ordinal space and the unique (session, task, ordinal)
    // index decides races between concurrent judges.
    let mut next_ordinal = existing
        .iter()
        .map(|t| t.ordinal)
        .filter(|o| *o >= 1000)
        .max()
        .map(|o| o + 1)
        .unwrap_or(1000);

    let mut declared_ids = Vec::with_capacity(defs.len());
    for def in defs {
        // Interactive judge probes go through the registration path (limits,
        // waiting) — not the declaration path.
        if def.mode == ProbeMode::Interactive {
            continue;
        }
        let config = def.to_probe_config();
        let row = existing.iter().find(|t| {
            t.initiator == arena_core::evaluation::INITIATOR_JUDGE
                && t.registered_by_judge_id == Some(judge_id)
                && t.prompt == def.name
        });
        let test_row = match row {
            Some(t) => t.clone(),
            None => {
                let am = tests::ActiveModel {
                    id: sea_orm::Set(Uuid::new_v4()),
                    command_template: sea_orm::Set(def.command.clone().unwrap_or_default()),
                    answer_template: sea_orm::Set(def.validation.clone().unwrap_or_default()),
                    fixture_definitions: sea_orm::Set(
                        r#"{"kind":"js","script":"({})"}"#.to_string(),
                    ),
                    created_at: sea_orm::Set(chrono::Utc::now()),
                    session_id: sea_orm::Set(session_id),
                    task_id: sea_orm::Set(task_id),
                    ordinal: sea_orm::Set(next_ordinal),
                    prompt: sea_orm::Set(def.name.clone()),
                    description: sea_orm::Set(None),
                    probe_config: sea_orm::Set(serde_json::to_value(&config).ok()),
                    initiator: sea_orm::Set(arena_core::evaluation::INITIATOR_JUDGE.to_string()),
                    registered_by_judge_id: sea_orm::Set(Some(judge_id)),
                };
                next_ordinal += 1;
                match sea_orm::ActiveModelTrait::insert(am, db).await {
                    Ok(t) => t,
                    Err(e) => {
                        tracing::warn!(judge = %judge.slug, probe = %def.name, error = %e,
                            "judge probe materialization failed");
                        continue;
                    }
                }
            }
        };
        declared_ids.push(test_row.id);

        if config.effective_executor() != ProbeExecutor::Server {
            continue;
        }
        let already_ran = probes::Entity::find()
            .filter(probes::Column::TestId.eq(test_row.id))
            .filter(probes::Column::PlayerId.eq(player_id))
            .one(db)
            .await
            .ok()
            .flatten()
            .is_some();
        if already_ran {
            continue;
        }
        let Some(repos_base) = arena_core::git_store::repos_base_dir() else {
            continue;
        };
        let repo_dir = arena_core::git_store::player_repo_path(&repos_base, session_id, player_id);
        let memory_schema = {
            use arena_core::entities::{projects, sessions};
            match sessions::Entity::find_by_id(session_id).one(db).await {
                Ok(Some(sess)) => projects::Entity::find_by_id(sess.project_id_fk)
                    .one(db)
                    .await
                    .ok()
                    .flatten()
                    .and_then(|p| p.memory_schema),
                _ => None,
            }
        };
        let memory = crate::session_memory::load_memory_map(
            db,
            session_id,
            player_id,
            memory_schema.as_deref(),
        )
        .await;
        let run = crate::probe_exec::run_server_probe(
            state, &repo_dir, session_id, player_id, &test_row, &config, &memory,
        )
        .await;
        if let Err(e) =
            crate::probe_exec::record_server_probe(db, session_id, player_id, test_row.id, &run)
                .await
        {
            tracing::warn!(error = %e, "judge probe record failed");
        }
    }
    declared_ids
}

/// `(context, panel-share scale)` when this judge scores a per-criterion
/// sheet on an open-ended task; `None` keeps the classic single-rating path.
async fn criteria_context_for(
    db: &DatabaseConnection,
    judge_row: &JudgeRow,
    task_row: &judging::TaskRow,
    task_judge_row: &TaskJudgeRow,
) -> Option<(
    arena_core::judging::criteria::CriteriaContext,
    arena_core::validation::judge_results::RatingScale,
)> {
    let keys: Vec<String> = serde_json::from_str(judge_row.criteria.as_deref()?).ok()?;
    if keys.is_empty() {
        return None;
    }
    let contract =
        arena_core::evaluation::EvaluationContract::from_json(task_row.evaluation.as_ref()?)
            .ok()?;
    let weights: std::collections::BTreeMap<String, f64> = contract
        .criteria
        .iter()
        .map(|c| (c.key.trim().to_string(), c.weight))
        .collect();
    // The panel total: every judge attached to this task, weighted.
    let attached = task_judges::Entity::find()
        .filter(task_judges::Column::TaskId.eq(task_row.id))
        .all(db)
        .await
        .ok()?;
    let total: f64 = attached.iter().map(|tj| tj.weight.unwrap_or(1.0)).sum();
    let share = arena_core::judging::criteria::panel_share_scale(
        task_row.point_value,
        task_judge_row.weight.unwrap_or(1.0),
        total,
    );
    Some((
        arena_core::judging::criteria::CriteriaContext { keys, weights },
        share,
    ))
}

/// Resolve what a session **appraisal** needs before it runs: which attached
/// pair carries the score, on what scale, and under which criteria contract.
///
/// Returns `None` when the judge has no attachment to score — the caller then
/// falls back to the clawback runner rather than paying for a verdict with
/// nowhere to land.
async fn appraisal_inputs(
    db: &DatabaseConnection,
    judge_row: &JudgeRow,
    reached: &[Uuid],
    task_judge_map: &std::collections::HashMap<Uuid, TaskJudgeRow>,
) -> Option<(
    TaskJudgeRow,
    arena_core::validation::judge_results::RatingScale,
    Option<arena_core::judging::criteria::CriteriaContext>,
)> {
    let primary = arena_core::judging::appraisal::primary_pair(reached, task_judge_map)?;
    let t = tasks::Entity::find_by_id(primary.task_id)
        .one(db)
        .await
        .ok()??;
    let task_row = judging::TaskRow {
        id: t.id,
        title: t.title,
        description: t.content,
        tags: t.tags,
        point_value: t.point_value,
        evaluation: t.evaluation.clone(),
    };
    // A criteria judge is paid out of the task's panel share, exactly as the
    // task path would pay it; a plain one keeps its own declared scale.
    match criteria_context_for(db, judge_row, &task_row, &primary).await {
        Some((ctx, share)) => Some((primary, share, Some(ctx))),
        None => {
            let scale = arena_core::judging::effective_rating_scale(
                &judge_row.rating_scale,
                &primary.rating_scale_override,
            );
            Some((primary, scale, None))
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub async fn execute_judge_run(
    state: &GameServerState,
    db: &DatabaseConnection,
    resolved: ResolvedJudgeRun,
    judge_llm: &dyn JudgeLlm,
    model_cfg: &ModelConfig,
    session_id: Uuid,
    player_id: Uuid,
    task_id: Uuid,
    recorder: Option<&JudgeRunRecorder>,
    registrar: Option<&std::sync::Arc<dyn judging::ProbeRegistrar>>,
) -> Result<JudgeRunOutput, JudgeError> {
    let ResolvedJudgeRun {
        task_judge_row,
        judge_row,
        task_row,
        prior_results,
        prior_judge_result,
        prior_session_verdicts,
        repo_dir,
        task_commit_sha,
        task_stats_json,
        join_code,
        _permit,
    } = resolved;

    // What this task actually paid the player. A penalty-only judge reverses
    // a reward, so this is both the reason to run it and the ceiling on what
    // it may take; the session-scoped path has bounded itself this way from
    // the start, and this is the task-scoped twin of that rule.
    let earned: i32 = prior_results.iter().map(|r| r.point_delta).sum();
    let scale = match judging::gate_task_judge(
        &judge_row.rating_scale,
        &task_judge_row.rating_scale_override,
        earned,
    ) {
        judging::TaskJudgeGate::Skip { reason } => {
            let judge_result_id =
                judging::persist_gate_skip(db, session_id, player_id, task_judge_row.id, &reason)
                    .await?;
            tracing::info!(
                judge = %judge_row.slug,
                %task_id,
                %player_id,
                "judge gated: task paid nothing, scored 0 without running"
            );
            // Recorded like any other run, under the gate's own provider: no
            // request was made, so the tokens are zero and the spend cannot
            // be misattributed — but a run nobody can look up afterwards is
            // how "why was this judge silent" became unanswerable.
            record_judge_llm_request(
                db,
                model_cfg,
                session_id,
                player_id,
                Some(task_id),
                &judge_row.slug,
                "scored",
                None,
                0,
                recorder,
                None,
                Some(ENGINE_GATE),
                serde_json::json!({
                    "outcome": "gated",
                    "reason": reason,
                    "task_earned": earned,
                }),
            )
            .await;
            let out = JudgeRunOutput {
                rating: 0.0,
                point_delta: 0,
                feedback: reason,
                raw_output: String::new(),
                model: ENGINE_GATE.model.to_string(),
                judge_result_id,
                duration_ms: 0,
            };
            broadcast_leaderboard_ref(state, session_id, &join_code).await;
            return Ok(out);
        }
        judging::TaskJudgeGate::Run(scale) => scale,
    };

    // Criteria judges on open-ended tasks: the model scores a 0–10 sheet per
    // declared criterion; the points map onto this judge's share of the
    // task's budget (panel weight over the panel total).
    let criteria_ctx = criteria_context_for(db, &judge_row, &task_row, &task_judge_row).await;
    let scale = match &criteria_ctx {
        Some((_, share)) => *share,
        None => scale,
    };

    // The judge's declared probes: materialize and run them before the
    // evidence is assembled, so the snapshot the model (or the execution
    // aggregate) reads already carries their measurements.
    let declared_test_ids = materialize_judge_probes(
        state,
        db,
        task_judge_row.judge_id,
        session_id,
        player_id,
        task_id,
    )
    .await;

    // Assemble the snapshot once the judge is cleared to run, and hand it to
    // the recorder with the fingerprint of the definition that will read it.
    // Recorded before the verdict on purpose: a run that fails still has to
    // say what it was holding.
    let programs = judging::programs::split_programs(&judge_row.prompt).1;
    let program = programs.decide.clone();
    let evidence = match judging::evidence::build_evidence(
        db,
        &repo_dir,
        session_id,
        player_id,
        task_id,
        &judge_row.slug,
        &scale,
        prior_judge_result
            .as_ref()
            .map(|p| judging::evidence::PriorVerdict {
                rating: p.rating,
                feedback: p.feedback.clone(),
            }),
        // Only what this judge declared it needs. An undeclared judge still
        // gets the whole snapshot, which is what every judge had before.
        judging::evidence::EvidenceNeeds::from_column(judge_row.evidence_needs.as_deref()),
        &judging::tools::ToolScope::from_json(judge_row.ignore_paths.as_deref()),
    )
    .await
    {
        Ok(ev) => {
            if let Some(rec) = recorder {
                rec.set_seen(judging::SeenByRun {
                    judge_fingerprint: judging::judge_fingerprint(&judge_row),
                    evidence: serde_json::to_value(&ev).unwrap_or(serde_json::Value::Null),
                });
            }
            Some(ev)
        }
        Err(e) => {
            // A judge with a program cannot run without its input: guessing
            // past a missing snapshot would run the model the program may
            // have meant to skip. A judge without one only loses the record.
            if program.is_some() {
                return Err(e);
            }
            tracing::warn!(
                judge = %judge_row.slug,
                %task_id,
                %player_id,
                error = %e,
                "judge evidence snapshot failed; run continues without it"
            );
            None
        }
    };

    // The judge's own program, before any model or sandbox is reached.
    let mut focus: Option<String> = None;
    if let (Some(program), Some(ev)) = (program.as_deref(), evidence.as_ref()) {
        let started = std::time::Instant::now();
        let decision = judging::programs::run_decide(program, ev);
        let decided_ms = started.elapsed().as_millis() as i64;
        // The decision is a step of the run like any model turn, and the one
        // that explains a verdict reached without a model at all.
        if let Some(rec) = recorder {
            rec.record(stage_event(
                "decide",
                Some("program".to_string()),
                match &decision {
                    Ok(d) => format!("{d:?}"),
                    Err(e) => format!("failed: {e}"),
                },
                decided_ms,
            ));
        }
        match decision? {
            judging::programs::Decision::Skip { reason } => {
                let judge_result_id = judging::persist_gate_skip(
                    db,
                    session_id,
                    player_id,
                    task_judge_row.id,
                    &reason,
                )
                .await?;
                tracing::info!(
                    judge = %judge_row.slug,
                    %task_id,
                    %player_id,
                    %reason,
                    "judge program skipped the run"
                );
                record_judge_llm_request(
                    db,
                    model_cfg,
                    session_id,
                    player_id,
                    Some(task_id),
                    &judge_row.slug,
                    "scored",
                    None,
                    decided_ms,
                    recorder,
                    None,
                    Some(ENGINE_DECIDE),
                    serde_json::json!({
                        "outcome": "skipped_by_program",
                        "reason": reason,
                    }),
                )
                .await;
                broadcast_leaderboard_ref(state, session_id, &join_code).await;
                return Ok(JudgeRunOutput {
                    rating: 0.0,
                    point_delta: 0,
                    feedback: reason,
                    raw_output: String::new(),
                    model: "decide:skip".to_string(),
                    judge_result_id,
                    duration_ms: decided_ms,
                });
            }
            judging::programs::Decision::Score { rating, feedback } => {
                // The rating is CLAMPED to the scale the gate already
                // narrowed, not rejected: rejecting turns into a run failure,
                // the retry budget cannot fix a deterministic program, and
                // the verdict is lost entirely — session 6LA7TX saw the
                // replay penalty vanish on exactly the cheapest tasks
                // ("score(-50) rejected: outside [-20, 0]") while the
                // expensive ones took the full -50. The narrowed scale IS
                // the per-task cap; asking for more than it means
                // "everything the task paid".
                let rating =
                    arena_core::validation::judge_results::clamp_rating_to_scale(rating, &scale);
                let point_delta =
                    arena_core::validation::judge_results::validate_rating_output(rating, &scale)
                        .map_err(|e| {
                        JudgeError::DecideFailed(format!("score({rating}) rejected: {e}"))
                    })?;
                arena_core::validation::judge_results::validate_feedback(&feedback)
                    .map_err(|e| JudgeError::DecideFailed(format!("feedback rejected: {e}")))?;
                let judge_result_id = judging::persist_verdict_and_id(
                    db,
                    session_id,
                    player_id,
                    task_judge_row.id,
                    rating,
                    point_delta,
                    &feedback,
                    "decide:program",
                    "decide",
                )
                .await?;
                tracing::info!(
                    judge = %judge_row.slug,
                    %task_id,
                    %player_id,
                    rating,
                    "judge program scored without a model"
                );
                record_judge_llm_request(
                    db,
                    model_cfg,
                    session_id,
                    player_id,
                    Some(task_id),
                    &judge_row.slug,
                    "scored",
                    None,
                    decided_ms,
                    recorder,
                    None,
                    Some(ENGINE_DECIDE),
                    serde_json::json!({
                        "outcome": "scored_by_program",
                        "rating": rating,
                        "point_delta": point_delta,
                        "feedback": feedback,
                    }),
                )
                .await;
                broadcast_leaderboard_ref(state, session_id, &join_code).await;
                // A program's verdict is a verdict: without the announcement
                // the activity feed has no row and the main server's cached
                // leaderboard never hears about the points.
                let detail = {
                    let f = feedback.trim();
                    (!f.is_empty()).then(|| serde_json::json!({ "feedback": f }))
                };
                announce_verdict(
                    state,
                    db,
                    session_id,
                    player_id,
                    task_id,
                    &task_row.title,
                    &judge_row.slug,
                    &judge_row.name,
                    &join_code,
                    rating,
                    point_delta,
                    &feedback,
                    None,
                    detail,
                )
                .await;
                maybe_publish_evaluation_ready(
                    state, db, session_id, player_id, task_id, &join_code,
                )
                .await;
                return Ok(JudgeRunOutput {
                    rating,
                    point_delta,
                    feedback,
                    raw_output: String::new(),
                    model: ENGINE_DECIDE.model.to_string(),
                    judge_result_id,
                    duration_ms: decided_ms,
                });
            }
            judging::programs::Decision::Ask { focus: f } => focus = f,
        }
    }

    // run the judge pipeline — execution judges run the committed code
    // server-side; LLM judges drive the tool-calling model loop.
    let execution = judge_row.kind == "execution";
    // Set when the model's run paused on a participant request: the
    // conversation so far, to be stored against the waiting row.
    let mut suspended_transcript: Option<serde_json::Value> = None;
    let mut out = if execution {
        let out = if declared_test_ids.is_empty() {
            crate::judge_exec::run_execution_judge(
                db,
                &repo_dir,
                session_id,
                player_id,
                task_id,
                task_commit_sha.as_deref(),
                &task_judge_row,
                &scale,
            )
            .await?
        } else {
            // A judge that declared its own probes is graded on those
            // measurements, not on re-running the task's claimed sections.
            crate::judge_exec::run_declared_execution_judge(
                db,
                session_id,
                player_id,
                &task_judge_row,
                &scale,
                &declared_test_ids,
            )
            .await?
        };
        // A sandbox re-run has as much to account for as a model turn — which
        // probe was re-run, what it printed, whether it could run at all —
        // and none of it was reaching the trace. The judge already reports it
        // as JSON for the verdict row; lift it into per-probe events so the
        // telemetry drawer shows a sandbox run the way it shows a model one.
        if let Some(rec) = recorder {
            for probe in
                serde_json::from_str::<Vec<serde_json::Value>>(&out.raw_output).unwrap_or_default()
            {
                let ordinal = probe.get("ordinal").and_then(|v| v.as_i64());
                rec.record(stage_event(
                    "probe",
                    Some(match ordinal {
                        Some(o) => format!("probe #{o}"),
                        None => "probe".to_string(),
                    }),
                    serde_json::to_string_pretty(&probe).unwrap_or_default(),
                    0,
                ));
            }
        }
        out
    } else {
        // Delivered visual artifacts ride along as vision attachments — but
        // only to judges that declared they look at pixels (`needs: [...,
        // images]`): a text-only model 400s on image content, which is how
        // a code judge on gpt-oss-120b died mid-panel.
        let wants_images = arena_core::judging::evidence::EvidenceNeeds::from_column(
            judge_row.evidence_needs.as_deref(),
        )
        .images;
        let mut images = if task_row.evaluation.is_some() && wants_images {
            load_artifact_images(db, session_id, player_id, task_id).await
        } else {
            Vec::new()
        };
        let mut attempt = judging::run_judge(
            db,
            judge_llm,
            &repo_dir,
            session_id,
            player_id,
            task_id,
            task_commit_sha.as_deref(),
            task_stats_json.as_deref(),
            &task_judge_row,
            &judge_row,
            &task_row,
            &prior_results,
            prior_judge_result.as_ref(),
            &prior_session_verdicts,
            &model_cfg.model,
            &model_cfg.provider,
            recorder,
            &scale,
            focus.as_deref(),
            // The cut after the model. Without the snapshot there is nothing
            // for a review to read, so it is skipped rather than run blind.
            programs.review.as_deref().zip(evidence.as_ref()),
            criteria_ctx.as_ref().map(|(ctx, _)| ctx),
            registrar,
            &images,
        )
        .await;
        // A vision judge can still land on a text-only pool member; a
        // text-only verdict beats a dead one, so strip the attachments and
        // try once more before giving up.
        if let Err(e) = &attempt
            && !images.is_empty()
            && model_rejected_images(&e.to_string())
        {
            tracing::warn!(
                session_id = %session_id, player_id = %player_id,
                judge = %judge_row.slug, model = %model_cfg.model,
                "model rejected image attachments; retrying text-only"
            );
            images.clear();
            attempt = judging::run_judge(
                db,
                judge_llm,
                &repo_dir,
                session_id,
                player_id,
                task_id,
                task_commit_sha.as_deref(),
                task_stats_json.as_deref(),
                &task_judge_row,
                &judge_row,
                &task_row,
                &prior_results,
                prior_judge_result.as_ref(),
                &prior_session_verdicts,
                &model_cfg.model,
                &model_cfg.provider,
                recorder,
                &scale,
                focus.as_deref(),
                programs.review.as_deref().zip(evidence.as_ref()),
                criteria_ctx.as_ref().map(|(ctx, _)| ctx),
                registrar,
                &images,
            )
            .await;
        }
        match attempt {
            // Not a failure: the run paused on a participant request. No
            // verdict exists yet — the row below is `waiting`, and the
            // re-drive resumes the stored conversation.
            Err(JudgeError::Suspended(transcript)) => {
                suspended_transcript = Some(*transcript);
                JudgeRunOutput {
                    rating: 0.0,
                    point_delta: 0,
                    feedback: String::new(),
                    raw_output: String::new(),
                    model: model_cfg.model.clone(),
                    judge_result_id: Uuid::nil(),
                    duration_ms: 0,
                }
            }
            other => other?,
        }
    };

    // A judge that asked the participant for an artifact ends this run
    // `waiting` — no points stand, the settle poll keeps the session open,
    // and the probe ticker re-drives the judge when the artifact lands or
    // its deadline passes. A turn-driven run paused mid-conversation and
    // stores its transcript; a legacy run wrote a provisional verdict
    // whose points are cleared here.
    let ended_waiting = suspended_transcript.is_some()
        || registrar.as_ref().is_some_and(|r| r.interactive_pending());
    if ended_waiting
        && let Err(e) = judging::record_judge_run_status(
            db,
            session_id,
            player_id,
            task_judge_row.id,
            &model_cfg.model,
            &model_cfg.provider,
            "waiting",
            Some("waiting for a participant artifact"),
            recorder,
        )
        .await
    {
        tracing::warn!(error = %e, "judge_queue: failed to persist waiting status");
    }
    match suspended_transcript.take() {
        Some(transcript) => {
            match store_suspended_transcript(db, task_judge_row.id, player_id, transcript).await {
                Ok(id) => out.judge_result_id = id,
                Err(e) => {
                    tracing::warn!(error = %e, "judge_queue: failed to store the paused transcript")
                }
            }
        }
        // A concluded run owes no resume: drop the transcript it may have
        // resumed from, so a later re-run starts fresh.
        None if !ended_waiting => {
            let _ = arena_core::entities::judge_run_transcripts::Entity::delete_by_id(
                out.judge_result_id,
            )
            .exec(db)
            .await;
        }
        None => {}
    }

    // Unified LLM telemetry: one `llm_requests` row per concluded run.
    // Failed runs are recorded by the retry loop in `enqueue_judge_run`
    // (which sees the final error after all attempts); this covers the
    // scored path. Additive — does not replace judge_results/log-store.
    record_judge_llm_request(
        db,
        model_cfg,
        session_id,
        player_id,
        Some(task_id),
        &judge_row.slug,
        "scored",
        None,
        out.duration_ms,
        recorder,
        None,
        // A sandbox run never reached the resolved model; saying it did would
        // report its wall-clock as that provider's latency.
        execution.then_some(ENGINE_EXECUTION),
        serde_json::json!({
            "outcome": if ended_waiting {
                "waiting_for_artifact"
            } else if execution {
                "scored_by_execution"
            } else {
                "scored_by_model"
            },
            "rating": out.rating,
            "point_delta": out.point_delta,
            "feedback": out.feedback,
            "focus": focus,
        }),
    )
    .await;

    broadcast_leaderboard_ref(state, session_id, &join_code).await;

    // Lift a compact summary for the activity feed and the live TaskScored
    // frame: the judge's written feedback for every verdict, plus the
    // per-criterion sheet (overall + scores) for criteria judges.
    let verdict_detail: Option<serde_json::Value> = {
        let mut obj = serde_json::Map::new();
        let feedback = out.feedback.trim();
        if !feedback.is_empty() {
            obj.insert("feedback".to_string(), serde_json::json!(feedback));
        }
        let criteria_row = if criteria_ctx.is_some() {
            judge_results::Entity::find_by_id(out.judge_result_id)
                .one(db)
                .await
                .ok()
                .flatten()
        } else {
            None
        };
        if let Some(row) = criteria_row
            && let Some(sheet) = row.rating.get("criteria").and_then(|v| v.as_array())
        {
            let compact: Vec<serde_json::Value> = sheet
                .iter()
                .map(|c| {
                    serde_json::json!({
                        "key": c["key"],
                        "score": c["score"],
                        // The per-criterion explanation — the dashboard shows
                        // it in a hovercard on the criterion chip.
                        "rationale": c["rationale"],
                    })
                })
                .collect();
            obj.insert(
                "overall".to_string(),
                row.rating
                    .get("overall")
                    .cloned()
                    .unwrap_or(serde_json::Value::Null),
            );
            obj.insert("criteria".to_string(), serde_json::json!(compact));
        }
        if obj.is_empty() {
            None
        } else {
            Some(serde_json::Value::Object(obj))
        }
    };

    // A run that ended waiting has no verdict to announce: its provisional
    // rating was written down and immediately cleared, and broadcasting it
    // anyway is how FBTQYR watched "UX review: 0" stand for six minutes while
    // the screenshots it had just asked for were still being taken. The
    // re-driven run announces the real verdict; until then the player's chat
    // shows the judge as waiting, which is the truth.
    if !ended_waiting {
        announce_verdict(
            state,
            db,
            session_id,
            player_id,
            task_id,
            &task_row.title,
            &judge_row.slug,
            &judge_row.name,
            &join_code,
            out.rating,
            out.point_delta,
            &out.feedback,
            Some(out.duration_ms),
            verdict_detail,
        )
        .await;

        maybe_publish_evaluation_ready(state, db, session_id, player_id, task_id, &join_code).await;
    }

    Ok(out)
}

/// The announcements a settled verdict owes the rest of the system: the
/// activity_event row, the cross-process `JudgeScored` event (which is also
/// what refreshes the main server's cached leaderboard — the score the
/// player page reads), and the player agent's frame. Every path that
/// persists a verdict must come through here: session 6LA7TX's
/// program-scored verdicts returned early without it, and the player page
/// froze on a pre-verdict score while the dashboard recomputed the truth
/// from the database.
#[allow(clippy::too_many_arguments)]
async fn announce_verdict(
    state: &GameServerState,
    db: &DatabaseConnection,
    session_id: Uuid,
    player_id: Uuid,
    task_id: Uuid,
    task_title: &str,
    judge_slug: &str,
    judge_name: &str,
    join_code: &str,
    rating: f64,
    point_delta: i32,
    feedback: &str,
    duration_ms: Option<i64>,
    detail: Option<serde_json::Value>,
) {
    let now = Utc::now();
    let player_display_name = players::Entity::find_by_id(player_id)
        .one(db)
        .await
        .ok()
        .flatten()
        .map(|p| p.display_name)
        .unwrap_or_default();
    let task_ordinal = tasks::Entity::find_by_id(task_id)
        .one(db)
        .await
        .ok()
        .flatten()
        .map(|t| t.ordinal)
        .unwrap_or(0);
    let version = state
        .session_registry
        .get(join_code)
        .and_then(|e| e.cache.read().ok().map(|c| c.version))
        .unwrap_or(0);

    let player_frame = PlayerAgentFrame::JudgeScored(JudgeScoredPayload {
        task_id,
        judge_slug: judge_slug.to_string(),
        judge_name: judge_name.to_string(),
        rating,
        feedback: feedback.to_string(),
        point_delta,
        created_at: now,
    });

    // Persist-first: insert activity_event row before publishing.
    let activity_row = activity_event::ActiveModel {
        id: sea_orm::ActiveValue::Set(Uuid::new_v4()),
        session_id_fk: sea_orm::ActiveValue::Set(session_id),
        player_id_fk: sea_orm::ActiveValue::Set(player_id),
        task_id_fk: sea_orm::ActiveValue::Set(task_id),
        event_kind: sea_orm::ActiveValue::Set("task_scored".to_string()),
        task_ordinal: sea_orm::ActiveValue::Set(task_ordinal),
        task_title: sea_orm::ActiveValue::Set(task_title.to_string()),
        player_display_name: sea_orm::ActiveValue::Set(player_display_name.clone()),
        judge_name: sea_orm::ActiveValue::Set(Some(judge_name.to_string())),
        point_delta: sea_orm::ActiveValue::Set(Some(point_delta)),
        detail: sea_orm::ActiveValue::Set(detail.clone()),
        timestamp: sea_orm::ActiveValue::Set(now),
        version: sea_orm::ActiveValue::Set(version as i64),
    };
    if let Err(e) = activity_event::Entity::insert(activity_row).exec(db).await {
        tracing::warn!(error = %e, "judge_queue: failed to persist activity_event, skipping publish");
        // Still send the player agent frame — that path is independent.
        if let Some(tx) = state.player_agent_registry.get(&player_id) {
            send_agent_frame(tx.value(), player_frame, player_id);
        }
        return;
    }

    let event = ZmqEvent::JudgeScored {
        join_code: join_code.to_string(),
        player_id,
        player_display_name,
        task_id,
        task_ordinal,
        task_title: task_title.to_string(),
        point_delta,
        judge_slug: judge_slug.to_string(),
        judge_name: judge_name.to_string(),
        rating,
        feedback: feedback.to_string(),
        duration_ms,
        detail,
        timestamp: now,
        version,
    };
    state.event_publisher.publish(&event).await;

    if let Some(tx) = state.player_agent_registry.get(&player_id) {
        let _ = tx.try_send(player_frame);
    }
}

/// What ran the judge, when it was not a model.
///
/// Reported in place of the resolved provider/model so a run that never
/// issued a request cannot read as spend against that provider. The strings
/// land in `llm_requests.provider` / `.model`, which is what the telemetry
/// list groups by.
#[derive(Debug, Clone, Copy)]
struct JudgeEngine {
    provider: &'static str,
    model: &'static str,
}

/// The task's own probes, re-run server-side in a sandbox.
const ENGINE_EXECUTION: JudgeEngine = JudgeEngine {
    provider: "execution",
    model: "execution:sandbox",
};
/// The harness decided before the judge was reached — the task paid nothing.
const ENGINE_GATE: JudgeEngine = JudgeEngine {
    provider: "gate",
    model: "gate:not-applicable",
};
/// The judge's own `js decide` program decided, without a model.
const ENGINE_DECIDE: JudgeEngine = JudgeEngine {
    provider: "decide",
    model: "decide:program",
};

/// One [`JudgeLogEvent`] for a stage of a run that is not a model turn.
fn stage_event(
    kind: &str,
    name: Option<String>,
    output: String,
    duration_ms: i64,
) -> JudgeLogEvent {
    JudgeLogEvent {
        at_ms: log_now_ms(),
        kind: kind.to_string(),
        name,
        duration_ms,
        output_chars: Some(output.chars().count() as u64),
        output: Some(truncate_chars(&output, 8_000)),
        ..Default::default()
    }
}

/// Insert one `llm_requests` telemetry row for a judge run (operation
/// "judge").
///
/// Every concluded run lands one, not only the ones that called a model: a
/// sandbox re-run, a run the gate stopped, and a run a judge's own program
/// decided are all things an operator has to be able to look up, and until
/// they were recorded here the only account of them was the database. Runs
/// without a model carry a [`JudgeEngine`] label and zero tokens, so the cost
/// views stay honest while the trace is complete.
///
/// Additive to the `judge_results` + on-disk log-store writes; best-effort
/// (never fails the caller).
#[allow(clippy::too_many_arguments)]
async fn record_judge_llm_request(
    db: &DatabaseConnection,
    cfg: &ModelConfig,
    session_id: Uuid,
    player_id: Uuid,
    task_id: Option<Uuid>,
    judge_slug: &str,
    status: &str,
    error: Option<&str>,
    duration_ms: i64,
    recorder: Option<&JudgeRunRecorder>,
    attempts: Option<u32>,
    // `None` when a model ran: the resolved config is then the truth.
    engine: Option<JudgeEngine>,
    // Merged into `detail_json` — the verdict, the decision, whatever the
    // path knows and the aggregate columns cannot hold.
    extra_detail: serde_json::Value,
) {
    use arena_core::llm::telemetry::{LlmContext, LlmRequestRecord, record_llm_request};

    let (tokens_in, tokens_out) = recorder.map(|r| r.token_totals()).unwrap_or((0, 0));
    let (cache_read, cache_write) = recorder.map(|r| r.cache_token_totals()).unwrap_or((0, 0));
    let seen = recorder.and_then(|r| r.seen());
    let mut detail = serde_json::json!({
        "events": recorder.map(|r| r.events().len()).unwrap_or(0),
    });
    if let Some(attempts) = attempts {
        detail["attempts"] = attempts.into();
    }
    // Which definition of the judge produced this: `judges/*.md` is re-seeded
    // over the same slug on every boot, so the slug alone dates badly.
    if let Some(seen) = &seen {
        detail["judge_fingerprint"] = seen.judge_fingerprint.clone().into();
    }
    if let serde_json::Value::Object(extra) = extra_detail
        && let Some(detail) = detail.as_object_mut()
    {
        detail.extend(extra);
    }

    let ctx = LlmContext {
        session_id: Some(session_id),
        player_id: Some(player_id),
        task_id,
        judge_slug: Some(judge_slug.to_string()),
    };
    let mut rec = LlmRequestRecord::new("judge", cfg, ctx)
        .with_duration_ms(duration_ms)
        .with_tokens(tokens_in, tokens_out, cache_read, cache_write)
        .with_detail_json(detail.to_string());
    if let Some(engine) = engine {
        rec.provider = engine.provider.to_string();
        rec.provider_name = None;
        rec.model = engine.model.to_string();
    }

    // The trace: the evidence the run read, then every stage of what it did
    // with it. Assembled through a recorder so the same two-stage size
    // bounding applies as to a model transcript.
    let trace = JudgeRunRecorder::default();
    if let Some(seen) = seen {
        trace.record(stage_event(
            "evidence",
            Some("snapshot".to_string()),
            serde_json::to_string_pretty(&seen.evidence).unwrap_or_default(),
            0,
        ));
    }
    for event in recorder.map(|r| r.events()).unwrap_or_default() {
        trace.record(event);
    }
    rec = rec.with_events(&trace);

    if status != "scored" {
        rec = rec.failed(error.unwrap_or("judge run failed"));
    }
    record_llm_request(db, rec).await;
}

/// Wait (bounded) for the task's `feat(<task_id>)` snapshot commit to land in
/// the player's repo before judging. ololo commits and pushes a completed
/// task's snapshot only after it learns the scheduler moved on (the next
/// TestPush, or SessionComplete) — so judges enqueued at completion time
/// race the push and would otherwise read a repo without the task's changes.
/// Polls every 2s up to `max_wait`; returns whether the commit was seen.
pub async fn wait_for_task_commit(
    session_id: Uuid,
    player_id: Uuid,
    task_id: Uuid,
    max_wait: std::time::Duration,
) -> bool {
    let Some(base) = repos_base_dir() else {
        return false;
    };
    let repo_dir = player_repo_path(&base, session_id, player_id);
    let deadline = tokio::time::Instant::now() + max_wait;
    loop {
        match judging::resolve_task_commit(&repo_dir, task_id).await {
            Ok(Some(_)) => return true,
            Ok(None) => {}
            Err(e) => {
                tracing::debug!(task_id = %task_id, error = ?e, "judge_queue: task commit poll failed");
            }
        }
        if tokio::time::Instant::now() >= deadline {
            tracing::warn!(
                task_id = %task_id,
                player_id = %player_id,
                "judge_queue: task snapshot commit did not appear within {:?}; judging current repo state",
                max_wait
            );
            return false;
        }
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    }
}

/// Wait (bounded) for the task's client-reported agent statistics row so the
/// `get_task_stats` judge tool has data. ololo collects and POSTs stats in
/// parallel with the snapshot push, so a short wait usually suffices. Polls
/// every 2s up to `max_wait`; returns whether the row was seen.
pub async fn wait_for_task_stats(
    db: &DatabaseConnection,
    session_id: Uuid,
    player_id: Uuid,
    task_id: Uuid,
    max_wait: std::time::Duration,
) -> bool {
    let deadline = tokio::time::Instant::now() + max_wait;
    loop {
        let found = task_agent_stats::Entity::find()
            .filter(task_agent_stats::Column::SessionIdFk.eq(session_id))
            .filter(task_agent_stats::Column::PlayerIdFk.eq(player_id))
            .filter(task_agent_stats::Column::TaskIdFk.eq(task_id))
            .one(db)
            .await
            .ok()
            .flatten()
            .is_some();
        if found {
            return true;
        }
        if tokio::time::Instant::now() >= deadline {
            tracing::info!(
                task_id = %task_id,
                player_id = %player_id,
                "judge_queue: no task stats reported within {:?}; judging without them",
                max_wait
            );
            return false;
        }
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    }
}

/// Public-facing judge failure message. The full provider/system error is
/// persisted in `judge_results.error` (admin-only surface) — players and
/// observers only ever see this generic line.
pub const GENERIC_JUDGE_ERROR: &str =
    "Judge evaluation failed. A session operator can view the details.";

/// Public-facing message for a quota-denied run. Unlike provider/system
/// errors this is the PLAYER's situation, not an internal detail — hiding
/// it behind the generic line left a whole session of failed judges with
/// no visible reason (session KN5JHB).
pub const QUOTA_JUDGE_ERROR: &str = "Monthly judge-run limit reached for this account — \
     reviews resume next month or when the account's limit is raised.";

/// The public message for a persisted failure `message`: quota denials are
/// the player's own state and stay specific; everything else is generic.
fn public_judge_error(message: &str) -> String {
    if arena_core::judging::JudgeError::message_is_quota_denial(message) {
        QUOTA_JUDGE_ERROR.to_string()
    } else {
        GENERIC_JUDGE_ERROR.to_string()
    }
}

/// Process-wide pacer for LLM provider requests. The judge semaphore bounds
/// *concurrent judge runs*, but each run is an agent loop issuing several
/// completions — bursts from interleaved loops are what trip provider rate
/// limits. Every LLM call queues here (FIFO via the mutex) and waits until
/// at least `min_interval` has passed since the previous call started.
/// Interval: `ARENA_JUDGE_LLM_MIN_INTERVAL_MS`, default 1000, 0 disables.
/// In-flight judge runs allowed per game-server process
/// (`ARENA_JUDGE_MAX_CONCURRENT`).
///
/// 3 was sized for one local Ollama serving every request; hosted providers
/// handle far more, and the limit applies to whole runs, each of which spends
/// most of its time waiting on the network.
pub const DEFAULT_JUDGE_MAX_CONCURRENT: u32 = 8;

pub struct LlmPacer {
    /// One token accrues per `min_interval`.
    min_interval: std::time::Duration,
    /// Max tokens (burst) — lets up to this many calls start without waiting.
    capacity: f64,
    state: tokio::sync::Mutex<PacerState>,
}

struct PacerState {
    /// Fractional tokens available, as of `updated`.
    tokens: f64,
    updated: tokio::time::Instant,
}

impl LlmPacer {
    /// `burst` is the number of calls allowed to start back-to-back before
    /// pacing kicks in (0 is treated as 1).
    pub fn new(min_interval: std::time::Duration, burst: u32) -> Self {
        let capacity = burst.max(1) as f64;
        Self {
            min_interval,
            capacity,
            state: tokio::sync::Mutex::new(PacerState {
                tokens: capacity,
                updated: tokio::time::Instant::now(),
            }),
        }
    }

    /// Pacer for one provider.
    ///
    /// Pacing is per-provider, not process-wide: a shared bucket meant one
    /// slow backend throttled every other one, and with several providers
    /// configured that caps total throughput at a single provider's rate for
    /// no reason. Buckets are created on first use and live for the process.
    ///
    /// The default interval is 0 (no pacing). Rate limits are already handled
    /// where they surface — `judge_rate_limit_attempts()` retries 429s with a
    /// 15s→300s ramp — and in-flight calls are bounded by
    /// `ARENA_JUDGE_MAX_CONCURRENT`. A non-zero default only served local
    /// single-process Ollama; set `ARENA_JUDGE_LLM_MIN_INTERVAL_MS` to restore
    /// it for such a backend.
    fn for_provider(provider: &str) -> Arc<LlmPacer> {
        static PACERS: std::sync::OnceLock<std::sync::Mutex<HashMap<String, Arc<LlmPacer>>>> =
            std::sync::OnceLock::new();
        let map = PACERS.get_or_init(|| std::sync::Mutex::new(HashMap::new()));
        let mut guard = match map.lock() {
            Ok(g) => g,
            // A poisoned mutex here would otherwise take down judging; the map
            // is a plain cache, so recovering it is safe.
            Err(poisoned) => poisoned.into_inner(),
        };
        if let Some(p) = guard.get(provider) {
            return Arc::clone(p);
        }
        let ms = std::env::var("ARENA_JUDGE_LLM_MIN_INTERVAL_MS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(0);
        // Burst up to the configured concurrency so the pacer throttles
        // sustained rate without defeating ARENA_JUDGE_MAX_CONCURRENT.
        let burst = std::env::var("ARENA_JUDGE_MAX_CONCURRENT")
            .ok()
            .and_then(|v| v.parse::<u32>().ok())
            .unwrap_or(DEFAULT_JUDGE_MAX_CONCURRENT);
        let pacer = Arc::new(LlmPacer::new(std::time::Duration::from_millis(ms), burst));
        guard.insert(provider.to_string(), Arc::clone(&pacer));
        pacer
    }

    /// Token-bucket rate limiter. Up to `capacity` calls start immediately
    /// (a burst), then the sustained start rate is one per `min_interval`.
    /// Previously the lock was held across a sleep, serializing every LLM call
    /// process-wide (≤1 in flight regardless of ARENA_JUDGE_MAX_CONCURRENT) and
    /// blowing past the award deadline on a many-player session finish.
    pub async fn wait_turn(&self) {
        if self.min_interval.is_zero() {
            return;
        }
        let interval = self.min_interval.as_secs_f64();
        let sleep_until = {
            let mut st = self.state.lock().await;
            let now = tokio::time::Instant::now();
            // Accrue tokens for the elapsed time since we last accounted.
            let refill = now.duration_since(st.updated).as_secs_f64() / interval;
            st.tokens = (st.tokens + refill).min(self.capacity);
            st.updated = now;
            if st.tokens >= 1.0 {
                st.tokens -= 1.0;
                None
            } else {
                // Reserve the token that will accrue `deficit * interval` from
                // now; account it at that future instant so later callers queue
                // behind us instead of all waking together.
                let deficit = 1.0 - st.tokens;
                let wait = self.min_interval.mul_f64(deficit);
                st.tokens = 0.0;
                st.updated = now + wait;
                Some(now + wait)
            }
        };
        if let Some(t) = sleep_until {
            tokio::time::sleep_until(t).await;
        }
    }
}

/// Keep the paused conversation against the judge's `waiting` row (upsert:
/// a run may pause more than once). Returns the row's id.
async fn store_suspended_transcript(
    db: &DatabaseConnection,
    task_judge_id: Uuid,
    player_id: Uuid,
    transcript: serde_json::Value,
) -> Result<Uuid, sea_orm::DbErr> {
    use arena_core::entities::judge_run_transcripts;
    use sea_orm::sea_query::OnConflict;
    let row = judge_results::Entity::find()
        .filter(judge_results::Column::TaskJudgeId.eq(task_judge_id))
        .filter(judge_results::Column::PlayerIdFk.eq(player_id))
        .one(db)
        .await?
        .ok_or_else(|| sea_orm::DbErr::Custom("waiting row missing".to_string()))?;
    let now = Utc::now();
    judge_run_transcripts::Entity::insert(judge_run_transcripts::ActiveModel {
        judge_result_id: sea_orm::Set(row.id),
        transcript: sea_orm::Set(transcript),
        created_at: sea_orm::Set(now),
        updated_at: sea_orm::Set(now),
    })
    .on_conflict(
        OnConflict::column(judge_run_transcripts::Column::JudgeResultId)
            .update_columns([
                judge_run_transcripts::Column::Transcript,
                judge_run_transcripts::Column::UpdatedAt,
            ])
            .to_owned(),
    )
    .exec_without_returning(db)
    .await?;
    Ok(row.id)
}

/// Decorator that routes every `run_agent` call through the global
/// [`LlmPacer`] before hitting the provider.
struct PacedJudgeLlm<'a> {
    inner: &'a dyn JudgeLlm,
    /// Which provider's bucket this run draws from.
    provider: String,
}

#[async_trait::async_trait]
impl JudgeLlm for PacedJudgeLlm<'_> {
    async fn run_agent(
        &self,
        system: &str,
        user: &str,
        tools: Vec<ToolDef>,
        prior_tool_result: Option<&str>,
    ) -> Result<AgentResponse, JudgeError> {
        LlmPacer::for_provider(&self.provider).wait_turn().await;
        self.inner
            .run_agent(system, user, tools, prior_tool_result)
            .await
    }

    async fn run_agent_with_images(
        &self,
        system: &str,
        user: &str,
        tools: Vec<ToolDef>,
        prior_tool_result: Option<&str>,
        images: &[arena_core::judging::JudgeImage],
    ) -> Result<AgentResponse, JudgeError> {
        // Delegation must be explicit: the trait default drops images.
        LlmPacer::for_provider(&self.provider).wait_turn().await;
        self.inner
            .run_agent_with_images(system, user, tools, prior_tool_result, images)
            .await
    }

    fn supports_turns(&self) -> bool {
        self.inner.supports_turns()
    }

    async fn run_turns(
        &self,
        system: &str,
        user: &str,
        tools: Vec<ToolDef>,
        images: &[arena_core::judging::JudgeImage],
        resume: Option<arena_core::judging::ResumeFrom>,
    ) -> Result<arena_core::judging::TurnsOutcome, JudgeError> {
        LlmPacer::for_provider(&self.provider).wait_turn().await;
        self.inner
            .run_turns(system, user, tools, images, resume)
            .await
    }
}

/// How many AI-behavior failures a judge run tolerates before it is marked
/// failed. Overridable via `ARENA_JUDGE_ATTEMPTS`; clamped to [1, 10].
fn judge_attempts() -> u32 {
    std::env::var("ARENA_JUDGE_ATTEMPTS")
        .ok()
        .and_then(|v| v.parse::<u32>().ok())
        .map(|n| n.clamp(1, 10))
        .unwrap_or(3)
}

/// Separate (larger) budget for provider rate-limit failures. Free-tier
/// models can be saturated *upstream* for minutes ("temporarily
/// rate-limited upstream, please retry shortly") — burning the small
/// AI-behavior budget in under a minute guarantees failure. With the
/// escalating `retry_delay` ramp this budget spans ~12 minutes.
/// Overridable via `ARENA_JUDGE_RATE_LIMIT_ATTEMPTS`; clamped to [1, 20].
fn judge_rate_limit_attempts() -> u32 {
    std::env::var("ARENA_JUDGE_RATE_LIMIT_ATTEMPTS")
        .ok()
        .and_then(|v| v.parse::<u32>().ok())
        .map(|n| n.clamp(1, 20))
        .unwrap_or(6)
}

/// The probes this judge registered on the task — its own rows plus the
/// requests it attached to as a watcher — each with the fate its latest
/// probe row spells out, oldest request first.
async fn load_prior_requests(
    db: &DatabaseConnection,
    session_id: Uuid,
    player_id: Uuid,
    task_id: Uuid,
    judge_id: Uuid,
) -> Result<Vec<judging::PriorRequest>, JudgeError> {
    use arena_core::entities::{artifact_request_watchers, probes, tests};
    use arena_core::evaluation::{ProbeConfig, ProbeMode};
    use arena_core::judging::RequestFate;
    use sea_orm::QueryOrder;

    let watched: Vec<Uuid> = artifact_request_watchers::Entity::find()
        .filter(artifact_request_watchers::Column::JudgeId.eq(judge_id))
        .all(db)
        .await?
        .into_iter()
        .map(|w| w.test_id)
        .collect();
    let mut rows = tests::Entity::find()
        .filter(tests::Column::SessionId.eq(session_id))
        .filter(tests::Column::TaskId.eq(task_id))
        .filter(
            sea_orm::Condition::any()
                .add(tests::Column::RegisteredByJudgeId.eq(judge_id))
                .add(tests::Column::Id.is_in(watched)),
        )
        .order_by_asc(tests::Column::CreatedAt)
        .all(db)
        .await?;
    rows.dedup_by_key(|t| t.id);

    let mut out = Vec::with_capacity(rows.len());
    for test in rows {
        let config = test
            .probe_config
            .as_ref()
            .and_then(|c| ProbeConfig::from_json(c).ok());
        let interactive = config
            .as_ref()
            .is_some_and(|c| c.mode == ProbeMode::Interactive);
        let instruction = config
            .as_ref()
            .and_then(|c| c.instruction.clone())
            .or_else(|| test.description.clone())
            .unwrap_or_else(|| test.command_template.clone());
        let latest = probes::Entity::find()
            .filter(probes::Column::TestId.eq(test.id))
            .filter(probes::Column::PlayerId.eq(player_id))
            .order_by_desc(probes::Column::DispatchedAt)
            .all(db)
            .await?;
        let delivered = latest
            .iter()
            .find(|p| p.outcome.as_deref() == Some("pass") || p.artifact_path.is_some());
        let fate = match (interactive, delivered, latest.first()) {
            (true, Some(p), _) => RequestFate::Delivered {
                files: p
                    .result_json
                    .as_ref()
                    .and_then(|j| j["artifact"]["files"].as_array().map(|f| f.len()))
                    .unwrap_or(1),
            },
            (true, None, Some(p)) if p.outcome.as_deref() == Some("no_response") => {
                RequestFate::Expired
            }
            (true, None, _) => RequestFate::Open,
            (false, _, Some(p)) => match p.outcome.as_deref() {
                Some("no_response") => RequestFate::Expired,
                Some(outcome) => RequestFate::Ran {
                    outcome: outcome.to_string(),
                    answer: p.output.clone().unwrap_or_default(),
                },
                None => RequestFate::Open,
            },
            (false, _, None) => RequestFate::Open,
        };
        out.push(judging::PriorRequest { instruction, fate });
    }
    Ok(out)
}

/// Whether a failed judge run is worth re-attempting. AI-behavior errors
/// (malformed verdict, timeout, out-of-range rating, transient provider
/// errors) are — a fresh run re-prompts the model from scratch.
/// Configuration/data errors (missing rows, absent repo, DB failures) are
/// not: they fail the same way every time.
fn is_retryable(err: &JudgeError) -> bool {
    matches!(
        err,
        JudgeError::AiTimeout
            | JudgeError::AiParseError
            | JudgeError::AiRatingOutOfRange
            | JudgeError::FeedbackTooLong
            | JudgeError::TooManyToolCalls
            | JudgeError::Llm(_)
            // The judge's own review threw the verdict out. A fresh run
            // re-prompts the model from scratch, which is exactly the remedy
            // — unlike a broken program, which fails identically every time.
            | JudgeError::VerdictRejected(_)
    )
}

/// Provider rate limiting (HTTP 429) needs a much longer backoff than other
/// transient failures — retrying in seconds lands in the same rate window
/// and just burns attempts. Detected from the provider error text.
fn is_rate_limit(err: &JudgeError) -> bool {
    match err {
        JudgeError::Llm(msg) => {
            let msg = msg.to_lowercase();
            msg.contains("429") || msg.contains("rate limit") || msg.contains("rate_limit")
        }
        _ => false,
    }
}

/// Delay before the next retry, given how many failures of this class have
/// occurred: rate limits back off 15s → 30s → 60s → 120s → 240s → 300s
/// (capped); other retryable failures keep the short 2s·attempt ramp.
fn retry_delay(err: &JudgeError, attempt: u32) -> std::time::Duration {
    if is_rate_limit(err) {
        std::time::Duration::from_secs((15u64 << (attempt.saturating_sub(1)).min(5)).min(300))
    } else {
        std::time::Duration::from_secs(2 * u64::from(attempt))
    }
}

/// Convenience wrapper: resolve rows, resolve the per-run model config
/// (`GameServerState::resolve_llm`), build the judge LLM, execute —
/// retrying AI-behavior failures up to
/// [`judge_attempts`] times (each attempt is a fresh resolve + prompt; the
/// judge semaphore permit is released between attempts).
/// All errors are returned to the caller — the auto-fire loop logs and
/// continues to the next judge. The run's lifecycle is made visible to the
/// player page: a "running" status + `JudgeStarted` event before the first
/// attempt, and a persisted "failed" status + `JudgeFailed` event once all
/// attempts are exhausted (or immediately for non-retryable errors).
///
/// `force` marks a run a human asked for by hand. It only reaches the
/// session-scoped path, where it lifts the "already has a verdict" guard —
/// see [`try_run_session_scoped`]. Every automatic caller passes `false`.
pub async fn enqueue_judge_run(
    state: &GameServerState,
    db: &DatabaseConnection,
    session_id: Uuid,
    player_id: Uuid,
    task_id: Uuid,
    judge_id: Uuid,
    force: bool,
) -> Result<JudgeRunOutput, JudgeError> {
    // A session-scoped judge must never go through the task pipeline: its
    // prompt is built around the session dossier, which that pipeline does not
    // supply, so it would score against a briefing it was not written for.
    // Route it to the session path instead — this is what makes the admin
    // re-run endpoint work for judges like anti-cheater.
    if let Some(out) =
        try_run_session_scoped(state, db, session_id, player_id, task_id, judge_id, force).await?
    {
        return Ok(out);
    }

    // Account plans: one metered unit per run of this pipeline (internal
    // retries included), charged to the judged player's account. Checked
    // before the "running" row exists so a denied run is recorded as failed,
    // never left dangling. A metering DB error fails open — a quota hiccup
    // must not take judging down with it. (The session-scoped path above
    // meters itself inside `run_one_session_judge`.)
    match arena_core::quota::check_and_charge_judge_run(db, session_id, player_id, judge_id).await {
        Ok(None) => {}
        Ok(Some(q)) => {
            let e = JudgeError::QuotaExceeded(format!(
                "monthly judge-run limit reached for this player's account \
                 ({}/{} on the {} plan)",
                q.used, q.limit, q.plan
            ));
            report_judge_failure(state, db, session_id, player_id, task_id, judge_id, &e).await;
            return Err(e);
        }
        Err(e) => {
            tracing::warn!(error = %e, "judge quota check failed; allowing run");
        }
    }

    // Two retry budgets: AI-behavior failures (parse errors, timeouts …)
    // get the small `judge_attempts` budget with a 2s ramp; provider
    // rate limits get their own larger `judge_rate_limit_attempts` budget
    // with a minutes-long ramp — upstream free-tier saturation outlives
    // any seconds-scale retry loop.
    let max_ai_failures = judge_attempts();
    let max_rl_failures = judge_rate_limit_attempts();
    let mut ai_failures = 0u32;
    let mut rl_failures = 0u32;
    let mut attempt = 0u32;
    // One recorder for the WHOLE run, not per attempt: retried attempts'
    // LLM calls still cost tokens, so the persisted totals (the basis for
    // per-session/per-player cost) must include them, and the run log
    // shows every attempt chronologically.
    let recorder = std::sync::Arc::new(JudgeRunRecorder::default());
    // Probe registration is wired only for open-ended tasks — the tool is
    // simply not offered elsewhere. One registrar for the whole run: a retry
    // must see (and not repeat) its own earlier registrations.
    let registrar: Option<std::sync::Arc<dyn judging::ProbeRegistrar>> =
        build_registrar(state, db, session_id, player_id, task_id, judge_id).await;
    // Wall-clock span of the whole run (all attempts + backoff) for the
    // failed-run telemetry row; the scored row uses the winning attempt's
    // own duration from `JudgeRunOutput`.
    let run_started = std::time::Instant::now();
    loop {
        attempt += 1;
        let resolved = match resolve_judge_run(state, db, session_id, player_id, task_id, judge_id)
            .await
        {
            Ok(r) => r,
            Err(e) => {
                report_judge_failure(state, db, session_id, player_id, task_id, judge_id, &e).await;
                return Err(e);
            }
        };
        let task_judge_id = resolved.task_judge_row.id;
        let judge_slug = resolved.judge_row.slug.clone();
        // Per-run model resolution: this judge's override → the
        // `llm_op_judge` assignment → the default assignment → the static
        // Ollama fallback. Resolved before any status row is written so every
        // recorded model/provider matches what the run actually calls.
        //
        // A pool assignment yields several candidates and each attempt takes
        // the next one, so the retry budgets above double as failover: a
        // rate-limited or dead provider is not retried into, the run moves to
        // the next member of the tier (and on to the next tier once the tier
        // is exhausted). With a single-model assignment every attempt gets
        // the same config, exactly as before.
        let candidates = state
            .resolve_llm_candidates(
                "judge",
                &LlmOverride::for_judge(
                    resolved.judge_row.llm_pool_id,
                    resolved.judge_row.llm_provider_id,
                    resolved.judge_row.llm_model.as_deref(),
                    &resolved.judge_row.llm_source_order,
                ),
            )
            .await;
        let cfg: ModelConfig = candidates[(attempt as usize - 1) % candidates.len()].clone();
        let judge_name = resolved.judge_row.name.clone();
        let join_code = resolved.join_code.clone();

        if attempt == 1 {
            // Persist-first "running", then announce the start to the player
            // page. Later attempts keep the existing "running" status.
            if let Err(e) = judging::record_judge_run_status(
                db,
                session_id,
                player_id,
                task_judge_id,
                &cfg.model,
                &cfg.provider,
                "running",
                None,
                None,
            )
            .await
            {
                tracing::warn!(error = %e, "judge_queue: failed to persist running status");
            }
            let started_at = Utc::now();
            state
                .event_publisher
                .publish(&ZmqEvent::JudgeStarted {
                    join_code: join_code.clone(),
                    player_id,
                    task_id,
                    judge_slug: judge_slug.clone(),
                    judge_name: judge_name.clone(),
                    timestamp: started_at,
                })
                .await;
            // The player's own agent hears it too, so the CLI can say who
            // is reviewing what — the web page learns the same from the bus.
            if let Some(tx) = state.player_agent_registry.get(&player_id) {
                send_agent_frame(
                    tx.value(),
                    PlayerAgentFrame::JudgeStarted(PlayerJudgeStatusPayload {
                        task_id,
                        judge_slug: judge_slug.clone(),
                        judge_name: judge_name.clone(),
                        status: "running".to_string(),
                        error: None,
                        updated_at: Some(started_at),
                        judge_result_id: None,
                    }),
                    player_id,
                );
            }
        }

        let run = match cfg
            .build_judge_full(
                resolved.repo_dir.clone(),
                resolved.task_commit_sha.clone(),
                resolved.task_stats_json.clone(),
                Some(recorder.clone()),
                registrar.clone(),
            )
            .map_err(|e| JudgeError::GitReadError(e.to_string()))
        {
            Ok(judge_llm) => {
                let paced = PacedJudgeLlm {
                    inner: judge_llm.as_ref(),
                    // The configured endpoint, not the registry id: keying on
                    // the latter put every `openai_compatible` provider in one
                    // bucket, so unrelated endpoints throttled each other.
                    provider: cfg.provider_label().to_string(),
                };
                execute_judge_run(
                    state,
                    db,
                    resolved,
                    &paced,
                    &cfg,
                    session_id,
                    player_id,
                    task_id,
                    Some(&recorder),
                    registrar.as_ref(),
                )
                .await
            }
            Err(e) => Err(e),
        };

        match run {
            Ok(out) => {
                store_run_log_file(
                    db,
                    session_id,
                    &join_code,
                    player_id,
                    task_id,
                    &judge_slug,
                    &cfg.model,
                    &cfg.provider,
                    "scored",
                    None,
                    Some(&recorder),
                )
                .await;
                return Ok(out);
            }
            Err(e) => {
                // Charge the failure to its class budget; retry while the
                // class has budget left.
                let (failures, cap) = if !is_retryable(&e) {
                    (0, 0) // non-retryable: fail immediately
                } else if is_rate_limit(&e) {
                    rl_failures += 1;
                    (rl_failures, max_rl_failures)
                } else {
                    ai_failures += 1;
                    (ai_failures, max_ai_failures)
                };
                if failures > 0 && failures < cap {
                    let delay = retry_delay(&e, failures);
                    tracing::warn!(
                        session_id = %session_id,
                        task_id = %task_id,
                        judge_id = %judge_id,
                        attempt,
                        failures,
                        cap,
                        delay_secs = delay.as_secs(),
                        rate_limited = is_rate_limit(&e),
                        error = ?e,
                        "judge run failed; retrying"
                    );
                    tokio::time::sleep(delay).await;
                    continue;
                }
                let message = if attempt > 1 {
                    format!("{e} (after {attempt} attempts)")
                } else {
                    e.to_string()
                };
                store_run_log_file(
                    db,
                    session_id,
                    &join_code,
                    player_id,
                    task_id,
                    &judge_slug,
                    &cfg.model,
                    &cfg.provider,
                    "failed",
                    Some(&message),
                    Some(&recorder),
                )
                .await;
                // Unified LLM telemetry: the one row for this failed run
                // (the scored path records in `execute_judge_run`).
                record_judge_llm_request(
                    db,
                    &cfg,
                    session_id,
                    player_id,
                    Some(task_id),
                    &judge_slug,
                    "failed",
                    Some(&message),
                    run_started.elapsed().as_millis() as i64,
                    Some(&recorder),
                    Some(attempt),
                    None,
                    serde_json::json!({ "outcome": "failed" }),
                )
                .await;
                persist_and_publish_failure(
                    state,
                    db,
                    session_id,
                    player_id,
                    task_id,
                    task_judge_id,
                    &judge_slug,
                    &judge_name,
                    &join_code,
                    &cfg.model,
                    &cfg.provider,
                    &message,
                    Some(&recorder),
                )
                .await;
                return Err(e);
            }
        }
    }
}

/// Failure path for errors raised before the run context was resolved
/// (missing rows, absent repo). Looks the context up best-effort so the
/// failure is still persisted and announced; gives up silently only when
/// even the lookups fail.
async fn report_judge_failure(
    state: &GameServerState,
    db: &DatabaseConnection,
    session_id: Uuid,
    player_id: Uuid,
    task_id: Uuid,
    judge_id: Uuid,
    error: &JudgeError,
) {
    let Ok(Some(judge)) = judges::Entity::find_by_id(judge_id).one(db).await else {
        return;
    };
    let join_code = arena_core::entities::sessions::Entity::find_by_id(session_id)
        .one(db)
        .await
        .ok()
        .flatten()
        .map(|s| s.join_code)
        .unwrap_or_default();
    let task_judge_id = task_judges::Entity::find()
        .filter(task_judges::Column::TaskId.eq(task_id))
        .filter(task_judges::Column::JudgeId.eq(judge_id))
        .one(db)
        .await
        .ok()
        .flatten()
        .map(|tj| tj.id);
    if let Some(task_judge_id) = task_judge_id {
        // Resolution failed before a judge-specific config existed; record
        // the default-chain model as the best-effort attribution.
        let cfg = state.resolve_llm("judge", &LlmOverride::none()).await;
        persist_and_publish_failure(
            state,
            db,
            session_id,
            player_id,
            task_id,
            task_judge_id,
            &judge.slug,
            &judge.name,
            &join_code,
            &cfg.model,
            &cfg.provider,
            &error.to_string(),
            None,
        )
        .await;
    }
}

#[allow(clippy::too_many_arguments)]
async fn persist_and_publish_failure(
    state: &GameServerState,
    db: &DatabaseConnection,
    session_id: Uuid,
    player_id: Uuid,
    task_id: Uuid,
    task_judge_id: Uuid,
    judge_slug: &str,
    judge_name: &str,
    join_code: &str,
    model: &str,
    provider: &str,
    message: &str,
    recorder: Option<&JudgeRunRecorder>,
) {
    // Full error + run log go to the DB (admin-only surface); the
    // published event carries only the generic public message.
    if let Err(e) = judging::record_judge_run_status(
        db,
        session_id,
        player_id,
        task_judge_id,
        model,
        provider,
        "failed",
        Some(message),
        recorder,
    )
    .await
    {
        tracing::warn!(error = %e, "judge_queue: failed to persist failed status");
    }
    let failed_at = Utc::now();
    state
        .event_publisher
        .publish(&ZmqEvent::JudgeFailed {
            join_code: join_code.to_string(),
            player_id,
            task_id,
            judge_slug: judge_slug.to_string(),
            judge_name: judge_name.to_string(),
            error: public_judge_error(message),
            timestamp: failed_at,
        })
        .await;
    // Same generic message to the agent, so its "reviewing" line lets go of
    // the judge instead of spinning on a run that already gave up.
    if let Some(tx) = state.player_agent_registry.get(&player_id) {
        send_agent_frame(
            tx.value(),
            PlayerAgentFrame::JudgeFailed(PlayerJudgeStatusPayload {
                task_id,
                judge_slug: judge_slug.to_string(),
                judge_name: judge_name.to_string(),
                status: "failed".to_string(),
                error: Some(public_judge_error(message)),
                updated_at: Some(failed_at),
                judge_result_id: None,
            }),
            player_id,
        );
    }
}

/// Persist a judge run's full telemetry to the on-disk log store
/// (`{join_code}/{username}/{task_id}.json`, keyed by judge slug). The
/// database keeps only the small metadata; this file carries the event
/// log with prompts, transcripts, and tool outputs. The same entry also
/// lands in the player's session event log as a `judge_run` line, so the
/// admin's per-player timeline carries the full run record and not just
/// the verdict events off the bus.
#[allow(clippy::too_many_arguments)]
async fn store_run_log_file(
    db: &DatabaseConnection,
    session_id: Uuid,
    join_code: &str,
    player_id: Uuid,
    task_id: Uuid,
    judge_slug: &str,
    model: &str,
    provider: &str,
    status: &str,
    error: Option<&str>,
    recorder: Option<&JudgeRunRecorder>,
) {
    let username = players::Entity::find_by_id(player_id)
        .one(db)
        .await
        .ok()
        .flatten()
        .map(|p| p.display_name)
        .unwrap_or_else(|| player_id.to_string());
    let events = recorder.map(|r| r.events()).unwrap_or_default();
    let (tokens_in, tokens_out) = recorder.map(|r| r.token_totals()).unwrap_or((0, 0));
    let (cache_read, cache_write) = recorder.map(|r| r.cache_token_totals()).unwrap_or((0, 0));
    let seen = recorder.and_then(|r| r.seen());
    let entry = serde_json::json!({
        "status": status,
        "model": model,
        "provider": provider,
        // What the run was looking at, and which definition of the judge read
        // it. Null on runs that never got as far as gathering evidence.
        "judge_fingerprint": seen.as_ref().map(|s| s.judge_fingerprint.clone()),
        "evidence": seen.map(|s| s.evidence),
        "tokens_input": tokens_in,
        "tokens_output": tokens_out,
        "tokens_cache_read": cache_read,
        "tokens_cache_write": cache_write,
        "error": error,
        "recorded_at": Utc::now(),
        "events": events,
    });
    let mut log_line = serde_json::json!({
        "player_id": player_id,
        "task_id": task_id,
        "judge_slug": judge_slug,
    });
    if let (Some(line), serde_json::Value::Object(run)) = (log_line.as_object_mut(), entry.clone())
    {
        line.extend(run);
    }
    crate::session_log_store::record(
        crate::session_log_store::base_dir(),
        session_id,
        Some(player_id),
        "judge_run",
        log_line,
    )
    .await;
    crate::judge_log_store::store_entry(
        crate::judge_log_store::base_dir(),
        join_code,
        &username,
        task_id,
        judge_slug,
        entry,
    )
    .await;
}

/// Thin wrapper around the existing broadcast_leaderboard in player_agent.rs.
async fn broadcast_leaderboard_ref(state: &GameServerState, session_id: Uuid, join_code: &str) {
    crate::ws::player_agent::broadcast_leaderboard(state, session_id, join_code).await;
}

/// Keep only the `task_judges` rows whose judge is task-scoped.
///
/// On a lookup failure the row is KEPT: a judge that runs when it should not
/// is a wasted call, while a judge that silently never runs leaves the settle
/// poll waiting for a row nobody writes.
pub async fn retain_task_scoped(
    db: &DatabaseConnection,
    rows: Vec<task_judges::Model>,
) -> Vec<task_judges::Model> {
    if rows.is_empty() {
        return rows;
    }
    let ids: Vec<Uuid> = rows.iter().map(|r| r.judge_id).collect();
    let session_scoped: std::collections::HashSet<Uuid> = match judges::Entity::find()
        .filter(judges::Column::Id.is_in(ids))
        .filter(judges::Column::Scope.eq(JUDGE_SCOPE_SESSION))
        .all(db)
        .await
    {
        Ok(found) => found.into_iter().map(|j| j.id).collect(),
        Err(e) => {
            tracing::warn!(error = %e, "judge_queue: scope lookup failed; running all attached judges");
            return rows;
        }
    };
    rows.into_iter()
        .filter(|r| !session_scoped.contains(&r.judge_id))
        .collect()
}

/// Wait (bounded) for the task's snapshot commit and stats row, then run the
/// given attached judges in order. Shared by the task-completion path
/// (player agent socket) and the time-expiry path (interrupted tasks): both
/// must not judge before ololo's push lands.
/// Run an open-ended task's server-side `done` probes against the final
/// commit. Quiet no-op for classic tasks; failures never block the judges.
async fn run_done_probes(
    state: &GameServerState,
    session_id: Uuid,
    player_id: Uuid,
    task_id: Uuid,
) {
    use arena_core::entities::{projects, sessions, tests};
    use arena_core::evaluation::{ProbeConfig, ProbeExecutor, ProbeMode, ScheduleOn};

    let Ok(Some(task)) = arena_core::entities::tasks::Entity::find_by_id(task_id)
        .one(&state.db)
        .await
    else {
        return;
    };
    if task.evaluation.is_none() {
        return;
    }
    let Ok(test_rows) = tests::Entity::find()
        .filter(tests::Column::TaskId.eq(task_id))
        .filter(tests::Column::SessionId.eq(session_id))
        .filter(tests::Column::ProbeConfig.is_not_null())
        .all(&state.db)
        .await
    else {
        return;
    };
    let due: Vec<(tests::Model, ProbeConfig)> = test_rows
        .into_iter()
        .filter_map(|t| {
            let config = ProbeConfig::from_json(t.probe_config.as_ref()?).ok()?;
            (config.effective_executor() == ProbeExecutor::Server
                && config.mode != ProbeMode::Interactive
                && config
                    .schedule
                    .as_ref()
                    .is_some_and(|s| s.on.contains(&ScheduleOn::Done)))
            .then_some((t, config))
        })
        .collect();
    if due.is_empty() {
        return;
    }

    let Ok(Some(session)) = sessions::Entity::find_by_id(session_id)
        .one(&state.db)
        .await
    else {
        return;
    };
    let memory_schema = match projects::Entity::find_by_id(session.project_id_fk)
        .one(&state.db)
        .await
    {
        Ok(p) => p.and_then(|p| p.memory_schema),
        Err(_) => None,
    };
    let memory = crate::session_memory::load_memory_map(
        &state.db,
        session_id,
        player_id,
        memory_schema.as_deref(),
    )
    .await;

    let Some(repos_base) = arena_core::git_store::repos_base_dir() else {
        return;
    };
    let repo_dir = arena_core::git_store::player_repo_path(&repos_base, session_id, player_id);
    for (test, config) in due {
        let run = crate::probe_exec::run_server_probe(
            state, &repo_dir, session_id, player_id, &test, &config, &memory,
        )
        .await;
        if let Err(e) =
            crate::probe_exec::record_server_probe(&state.db, session_id, player_id, test.id, &run)
                .await
        {
            tracing::warn!(error = %e, test_id = %test.id, "done probe: record failed");
        }
    }
}

pub async fn run_task_judges_after_commit(
    state: &GameServerState,
    session_id: Uuid,
    player_id: Uuid,
    task_id: Uuid,
    judges_to_run: Vec<task_judges::Model>,
) {
    let commit_seen = wait_for_task_commit(
        session_id,
        player_id,
        task_id,
        std::time::Duration::from_secs(90),
    )
    .await;
    // A landed snapshot may have changed the player's markdown files — run
    // session-memory extraction in parallel with the judges (best-effort,
    // never blocks them; skips itself when the sources are unchanged or the
    // project declares no memory schema).
    if commit_seen {
        let mem_state = state.clone();
        tokio::spawn(async move {
            crate::session_memory::extract_after_commit(mem_state, session_id, player_id).await;
        });
    }
    // Only wait for the client-reported stats row when the commit landed —
    // a client that pushes snapshots also reports stats; without a push
    // there is likely no reporter, so don't burn the extra wait.
    if commit_seen {
        wait_for_task_stats(
            &state.db,
            session_id,
            player_id,
            task_id,
            std::time::Duration::from_secs(30),
        )
        .await;
    }
    // Open-ended `schedule.on: [done]` probes run now, against the final
    // commit and before the judges — their measurements belong in the
    // evidence the panel reads.
    run_done_probes(state, session_id, player_id, task_id).await;

    // Session-scoped judges are attached per task like any other (that is how
    // the settle poll knows to expect their rows), but they must NOT fire here
    // — they run once per player at session end and score every reached task
    // in one pass. Firing them per commit would defeat the whole point.
    let judges_to_run = retain_task_scoped(&state.db, judges_to_run).await;

    // Run a task's judges concurrently. They are independent, and awaiting
    // them one by one meant a single judge's retry ladder — up to ~12 minutes
    // for a rate-limited provider — stalled every remaining judge for the same
    // task. The semaphore inside `enqueue_judge_run` still bounds how many
    // actually execute at once.
    let mut set = tokio::task::JoinSet::new();
    for tj in judges_to_run {
        let state = state.clone();
        let judge_id = tj.judge_id;
        set.spawn(async move {
            let outcome = enqueue_judge_run(
                &state, &state.db, session_id, player_id, task_id, judge_id, false,
            )
            .await;
            (judge_id, outcome)
        });
    }
    while let Some(joined) = set.join_next().await {
        match joined {
            Ok((_, Ok(_))) => {}
            Ok((judge_id, Err(e))) => {
                tracing::warn!(session_id = %session_id, task_id = %task_id, judge_id = %judge_id, error = ?e, "judge run failed")
            }
            Err(e) => {
                tracing::warn!(session_id = %session_id, task_id = %task_id, error = %e, "judge task panicked")
            }
        }
    }
}

/// Run attached judges for every task interrupted by time expiry: each
/// eligible player whose scheduler row still points at a task gets that
/// task's judges executed against whatever snapshot the client pushed on the
/// session-end signal (ololo's final sweep commits the in-progress task
/// too). Players run concurrently — each has its own bounded commit wait.
pub async fn run_interrupted_task_judges(state: &GameServerState, session_id: Uuid) {
    let interrupted = match arena_core::session_completion::interrupted_tasks(&state.db, session_id)
        .await
    {
        Ok(v) => v,
        Err(e) => {
            tracing::error!(session_id = %session_id, error = %e, "expiry judges: interrupted-task query failed");
            return;
        }
    };
    if interrupted.is_empty() {
        return;
    }
    let runs = interrupted.into_iter().map(|it| {
        let state = state.clone();
        async move {
            use sea_orm::QueryOrder;
            let judges_to_run = task_judges::Entity::find()
                .filter(task_judges::Column::TaskId.eq(it.task_id))
                .order_by(task_judges::Column::Ordinal, sea_orm::Order::Asc)
                .all(&state.db)
                .await
                .unwrap_or_default();
            if judges_to_run.is_empty() {
                return;
            }
            tracing::info!(
                session_id = %session_id,
                player_id = %it.player_id,
                task_id = %it.task_id,
                judges = judges_to_run.len(),
                "expiry judges: running judges for interrupted task"
            );
            let task_judge_ids: Vec<Uuid> = judges_to_run.iter().map(|tj| tj.id).collect();
            run_task_judges_after_commit(
                &state,
                session_id,
                it.player_id,
                it.task_id,
                judges_to_run,
            )
            .await;
            // These verdicts were reached on interrupted work — the judge saw
            // less than the task intended to show it. Mark them `partial` so
            // every reader (player page, admin, later analysis) knows the
            // basis was cut short. Scored rows only: a failed run stays what
            // it is.
            let _ = judge_results::Entity::update_many()
                .col_expr(
                    judge_results::Column::VerdictKind,
                    sea_orm::prelude::Expr::value(arena_core::evaluation::VERDICT_KIND_PARTIAL),
                )
                .filter(judge_results::Column::TaskJudgeId.is_in(task_judge_ids))
                .filter(judge_results::Column::PlayerIdFk.eq(it.player_id))
                .filter(judge_results::Column::Status.eq("scored"))
                .exec(&state.db)
                .await;
        }
    });
    futures::future::join_all(runs).await;
}

// ---------------------------------------------------------------------------
// Session-scoped judges
// ---------------------------------------------------------------------------

/// Run `judge_id` via the session path when it is session-scoped.
///
/// Returns `Ok(None)` when the judge is task-scoped, so the caller falls through
/// to the normal pipeline. On the session path the returned
/// [`JudgeRunOutput`] describes the verdict for `task_id` specifically, since
/// that is what the caller asked about, even though the run scores every task
/// the judge is attached to.
///
/// With `force`, an existing verdict is overwritten rather than returned. The
/// cost guard that normally stops a second pass exists for the recovery
/// sweeps, which fire blind; an admin pressing re-run has already decided to
/// pay for a fresh answer, and silently handing back the old one makes the
/// button look like it worked.
async fn try_run_session_scoped(
    state: &GameServerState,
    db: &DatabaseConnection,
    session_id: Uuid,
    player_id: Uuid,
    task_id: Uuid,
    judge_id: Uuid,
    force: bool,
) -> Result<Option<JudgeRunOutput>, JudgeError> {
    let Some(judge) = judges::Entity::find_by_id(judge_id).one(db).await? else {
        return Ok(None);
    };
    if judge.scope != JUDGE_SCOPE_SESSION {
        return Ok(None);
    }
    let Some(session) = arena_core::entities::sessions::Entity::find_by_id(session_id)
        .one(db)
        .await?
    else {
        return Err(JudgeError::GitReadError("session not found".to_string()));
    };
    let Some(player) = players::Entity::find_by_id(player_id).one(db).await? else {
        return Err(JudgeError::GitReadError("player not found".to_string()));
    };
    let attached = task_judges::Entity::find()
        .filter(task_judges::Column::JudgeId.eq(judge_id))
        .all(db)
        .await?;

    run_one_session_judge(state, &session, &player, &judge, &attached, force).await;

    // Report the row for the task the caller named.
    let tj = attached.iter().find(|tj| tj.task_id == task_id);
    let row = match tj {
        Some(tj) => {
            judge_results::Entity::find()
                .filter(judge_results::Column::TaskJudgeId.eq(tj.id))
                .filter(judge_results::Column::PlayerIdFk.eq(player_id))
                .one(db)
                .await?
        }
        None => None,
    };
    let row = row.ok_or_else(|| {
        JudgeError::ExecFailed(
            "session judge produced no verdict for the requested task (the player may not have \
             reached it)"
                .to_string(),
        )
    })?;
    if row.status != "scored" {
        return Err(JudgeError::ExecFailed(
            row.error
                .unwrap_or_else(|| "session judge did not score".to_string()),
        ));
    }
    Ok(Some(JudgeRunOutput {
        rating: judging::rating_scalar(&row.rating),
        point_delta: row.point_delta,
        feedback: row.feedback,
        raw_output: row.raw_output,
        model: row.model,
        judge_result_id: row.id,
        duration_ms: row.duration_ms.unwrap_or(0),
    }))
}

/// Run every session-scoped judge attached to this session's project, once per
/// eligible player, scoring all the tasks that player reached in a single pass.
///
/// Called at expiry BEFORE the settle poll: the poll counts a (player,
/// task_judge) pair as pending until a terminal `judge_results` row exists, and
/// nothing else in the system writes rows for a session-scoped judge — the
/// per-task trigger deliberately skips them (see `retain_task_scoped`). If this
/// function fails to leave a terminal row for every reached task, the award
/// flow waits for its deadline instead of awarding on time, so every error path
/// here falls back to marking the remaining pairs `failed`.
pub async fn run_session_judges(state: &GameServerState, session_id: Uuid) {
    let db = &state.db;

    let Ok(Some(session)) = arena_core::entities::sessions::Entity::find_by_id(session_id)
        .one(db)
        .await
    else {
        return;
    };

    // Session-scoped judges attached anywhere in this session's project.
    let project_task_ids: Vec<Uuid> = match tasks::Entity::find()
        .filter(tasks::Column::ProjectIdFk.eq(session.project_id_fk))
        .all(db)
        .await
    {
        Ok(rows) => rows.into_iter().map(|t| t.id).collect(),
        Err(e) => {
            tracing::error!(session_id = %session_id, error = %e, "session judges: task query failed");
            return;
        }
    };
    if project_task_ids.is_empty() {
        return;
    }

    let attached = match task_judges::Entity::find()
        .filter(task_judges::Column::TaskId.is_in(project_task_ids))
        .all(db)
        .await
    {
        Ok(rows) => rows,
        Err(e) => {
            tracing::error!(session_id = %session_id, error = %e, "session judges: task_judges query failed");
            return;
        }
    };
    if attached.is_empty() {
        return;
    }

    let judge_ids: Vec<Uuid> = attached.iter().map(|tj| tj.judge_id).collect();
    let mut session_judges: Vec<judges::Model> = match judges::Entity::find()
        .filter(judges::Column::Id.is_in(judge_ids))
        .filter(judges::Column::Scope.eq(JUDGE_SCOPE_SESSION))
        .all(db)
        .await
    {
        Ok(rows) => rows,
        Err(e) => {
            tracing::error!(session_id = %session_id, error = %e, "session judges: judge query failed");
            return;
        }
    };
    if session_judges.is_empty() {
        return;
    }
    // The reporter summarises the panel, so it goes after the rest of it. In
    // database order it ran first as often as not, and the judges it could not
    // see — anti-cheat among them — were simply missing from the report's
    // account of what the panel said.
    session_judges.sort_by_key(|j| j.kind == arena_core::judging::JUDGE_KIND_REPORT);

    let eligible: Vec<players::Model> = match players::Entity::find()
        .filter(players::Column::SessionIdFk.eq(session_id))
        .filter(players::Column::RevokedAt.is_null())
        .all(db)
        .await
    {
        Ok(rows) => rows,
        Err(e) => {
            tracing::error!(session_id = %session_id, error = %e, "session judges: player query failed");
            return;
        }
    };

    for judge in &session_judges {
        for player in &eligible {
            run_one_session_judge(state, &session, player, judge, &attached, false).await;
        }
    }
}

/// Whether every judge run the session expects for this player — the
/// reporter's own excluded — has reached a terminal row.
///
/// The report quotes the panel, so it must be written after the panel is
/// done. Sorting the reporter last in the session pass is not enough: a
/// judge suspended on an artifact is re-driven on its own schedule, and in
/// FBTQYR the ux-review re-verdict landed eleven seconds after the report
/// that should have carried it.
pub async fn sibling_runs_terminal(
    db: &DatabaseConnection,
    session_id: Uuid,
    player_id: Uuid,
) -> Result<bool, sea_orm::DbErr> {
    use arena_core::session_completion::{
        JUDGE_RESULT_FAILED, JUDGE_RESULT_SCORED, session_task_judge_filter,
    };
    let Some(session) = arena_core::entities::sessions::Entity::find_by_id(session_id)
        .one(db)
        .await?
    else {
        return Ok(true);
    };
    // The tasks THIS PLAYER reached, not the session's whole ladder: the
    // panel writes rows only for reached tasks, so a pair on a task nobody
    // got to can never become terminal — counting it kept the reporter in
    // its wait loop for the full timeout on every session with an unreached
    // tail (6O3C3Y sat 10 minutes on a two-of-three ladder).
    let reached =
        arena_core::session_completion::reached_tasks_for_player(db, session_id, player_id).await?;
    if reached.is_empty() {
        return Ok(true);
    }
    let expected = session_task_judge_filter(task_judges::Entity::find(), &session)
        .filter(task_judges::Column::TaskId.is_in(reached))
        .all(db)
        .await?;
    if expected.is_empty() {
        return Ok(true);
    }
    let judge_kinds: std::collections::HashMap<Uuid, String> = judges::Entity::find()
        .filter(
            judges::Column::Id.is_in(
                expected
                    .iter()
                    .map(|tj| tj.judge_id)
                    .collect::<std::collections::HashSet<_>>(),
            ),
        )
        .all(db)
        .await?
        .into_iter()
        .map(|j| (j.id, j.kind))
        .collect();
    let awaited: Vec<Uuid> = expected
        .iter()
        .filter(|tj| {
            judge_kinds
                .get(&tj.judge_id)
                .is_none_or(|k| k != arena_core::judging::JUDGE_KIND_REPORT)
        })
        .map(|tj| tj.id)
        .collect();
    if awaited.is_empty() {
        return Ok(true);
    }
    let terminal: std::collections::HashSet<Uuid> = judge_results::Entity::find()
        .filter(judge_results::Column::SessionIdFk.eq(session_id))
        .filter(judge_results::Column::PlayerIdFk.eq(player_id))
        .filter(judge_results::Column::Status.is_in([JUDGE_RESULT_SCORED, JUDGE_RESULT_FAILED]))
        .all(db)
        .await?
        .into_iter()
        .map(|r| r.task_judge_id)
        .collect();
    Ok(awaited.iter().all(|tj| terminal.contains(tj)))
}

/// How long the reporter is willing to wait for the panel, and how often it
/// checks. The abandoned-run reaper makes a stuck judge terminal well within
/// this cap, so the timeout only fires when something is genuinely wedged —
/// and then a report missing one verdict beats no report at all.
const REPORT_WAIT_MAX_SECS: u64 = 600;
const REPORT_WAIT_POLL_SECS: u64 = 5;

/// One session-scoped judge for one player.
///
/// `force` is an operator asking for this run explicitly; it skips the
/// already-terminal guard below and lets the pass overwrite what is there.
async fn run_one_session_judge(
    state: &GameServerState,
    session: &arena_core::entities::sessions::Model,
    player: &players::Model,
    judge: &judges::Model,
    attached: &[task_judges::Model],
    force: bool,
) {
    let db = &state.db;
    let session_id = session.id;
    let player_id = player.id;

    let reached = match arena_core::session_completion::reached_tasks_for_player(
        db, session_id, player_id,
    )
    .await
    {
        Ok(v) => v,
        Err(e) => {
            tracing::error!(session_id = %session_id, player_id = %player_id, error = %e, "session judges: reached-task query failed");
            return;
        }
    };
    if reached.is_empty() {
        return; // nothing expected of this player, nothing to write
    }

    // The pairs the settle poll will wait for: this judge's task_judges rows
    // over exactly the reached tasks.
    let task_judge_map: std::collections::HashMap<Uuid, TaskJudgeRow> = attached
        .iter()
        .filter(|tj| tj.judge_id == judge.id && reached.contains(&tj.task_id))
        .map(|tj| {
            (
                tj.task_id,
                TaskJudgeRow {
                    id: tj.id,
                    task_id: tj.task_id,
                    judge_id: tj.judge_id,
                    rating_scale_override: tj.rating_scale_override.clone(),
                    weight: tj.weight,
                },
            )
        })
        .collect();
    if task_judge_map.is_empty() {
        return;
    }

    // Cost guardrails. The recovery sweeps enqueue one call PER missing
    // (task, player) pair, and each call runs this whole (billed) session
    // pass — an extreme-startup session turned one sweep into 17 full runs.
    // (a) Every pair already terminal → the verdict exists, nothing to buy.
    // (b) A pass for this (session, player, judge) is already in flight →
    //     let it finish and write the rows; a second concurrent pass would
    //     only double the bill.
    if !force {
        use arena_core::session_completion::{JUDGE_RESULT_FAILED, JUDGE_RESULT_SCORED};
        let pair_ids: Vec<Uuid> = task_judge_map.values().map(|tj| tj.id).collect();
        let terminal: std::collections::HashSet<Uuid> = judge_results::Entity::find()
            .filter(judge_results::Column::PlayerIdFk.eq(player_id))
            .filter(judge_results::Column::TaskJudgeId.is_in(pair_ids.clone()))
            .filter(judge_results::Column::Status.is_in([JUDGE_RESULT_SCORED, JUDGE_RESULT_FAILED]))
            .all(db)
            .await
            .unwrap_or_default()
            .into_iter()
            .map(|r| r.task_judge_id)
            .collect();
        if pair_ids.iter().all(|id| terminal.contains(id)) {
            tracing::debug!(
                session_id = %session_id, player_id = %player_id, judge = %judge.slug,
                "session judge: every pair already has a terminal verdict; skipping"
            );
            return;
        }
    }
    static IN_FLIGHT: std::sync::LazyLock<
        std::sync::Mutex<std::collections::HashSet<(Uuid, Uuid, Uuid)>>,
    > = std::sync::LazyLock::new(Default::default);
    let flight_key = (session_id, player_id, judge.id);
    if !IN_FLIGHT.lock().expect("in-flight lock").insert(flight_key) {
        tracing::debug!(
            session_id = %session_id, player_id = %player_id, judge = %judge.slug,
            "session judge: a pass is already in flight; skipping duplicate"
        );
        return;
    }
    // Remove the in-flight mark on every exit path below.
    struct FlightGuard((Uuid, Uuid, Uuid));
    impl Drop for FlightGuard {
        fn drop(&mut self) {
            if let Ok(mut set) = IN_FLIGHT.lock() {
                set.remove(&self.0);
            }
        }
    }
    let _flight = FlightGuard(flight_key);

    let judge_row = JudgeRow {
        criteria: judge.criteria.clone(),
        max_interactive: judge.max_interactive,
        slug: judge.slug.clone(),
        name: judge.name.clone(),
        prompt: judge.prompt.clone(),
        rating_scale: judge.rating_scale.clone(),
        kind: judge.kind.clone(),
        scope: judge.scope.clone(),
        evidence_mode: judge.evidence_mode.clone(),
        evidence_needs: judge.evidence_needs.clone(),
        llm_provider_id: judge.llm_provider_id_fk,
        llm_pool_id: judge.llm_pool_id_fk,
        llm_source_order: judge.llm_source_order.clone(),
        llm_model: judge.llm_model.clone(),
        ignore_paths: judge.ignore_paths.clone(),
    };
    // Per-run model: judge override → llm_op_judge → default → static fallback.
    // Unlike the task pipeline this pass has no retry loop, so a pool
    // assignment collapses to its first candidate here — there is nothing to
    // fail over into. Giving this path failover means restructuring the
    // terminal-row bookkeeping below, which captures one model/provider pair
    // up front.
    let session_judge_cfg = state
        .resolve_llm(
            "judge",
            &LlmOverride::for_judge(
                judge_row.llm_pool_id,
                judge_row.llm_provider_id,
                judge_row.llm_model.as_deref(),
                &judge_row.llm_source_order,
            ),
        )
        .await;
    let model = session_judge_cfg.model.clone();
    let provider = session_judge_cfg.provider.clone();

    // Any bail-out from here on must still leave terminal rows behind.
    let fail_all = |reason: String| {
        let pairs: Vec<Uuid> = task_judge_map.values().map(|tj| tj.id).collect();
        let model = model.clone();
        let provider = provider.clone();
        async move {
            for task_judge_id in pairs {
                let _ = arena_core::judging::record_judge_run_status(
                    db,
                    session_id,
                    player_id,
                    task_judge_id,
                    &model,
                    &provider,
                    "failed",
                    Some(&reason),
                    None,
                )
                .await;
            }
        }
    };

    // Account plans: the whole player pass is one metered unit. Checked
    // after the terminal/in-flight guards so replays of already-settled
    // pairs cost nothing, and before any LLM is built. Denial leaves the
    // failed rows the settle poll needs; a metering DB error fails open.
    match arena_core::quota::check_and_charge_judge_run(db, session_id, player_id, judge.id).await {
        Ok(None) => {}
        Ok(Some(q)) => {
            tracing::warn!(
                session_id = %session_id, player_id = %player_id, judge = %judge.slug,
                used = q.used, limit = q.limit, plan = %q.plan,
                "session judge denied: monthly judge-run limit reached"
            );
            fail_all(format!(
                "judge run quota exceeded: monthly judge-run limit reached for this \
                 player's account ({}/{} on the {} plan)",
                q.used, q.limit, q.plan
            ))
            .await;
            return;
        }
        Err(e) => {
            tracing::warn!(error = %e, "judge quota check failed; allowing run");
        }
    }

    let Some(base) = repos_base_dir() else {
        tracing::error!(session_id = %session_id, "session judges: no git repo store configured");
        fail_all("no git repo store configured".to_string()).await;
        return;
    };
    let repo_dir = player_repo_path(&base, session_id, player_id);
    // A judge that reads code cannot work without the code. The reporter can:
    // its evidence — the checks, the points, what was recorded of the agent —
    // is all in the database, and the git readers return "no commits" for an
    // absent repo rather than failing. A player whose snapshot never arrived
    // is precisely the one owed an explanation of what the session saw.
    if !repo_dir.join("HEAD").exists() {
        tracing::warn!(session_id = %session_id, player_id = %player_id, judge = %judge.slug, "session judges: player pushed no repo");
        if judge.kind != arena_core::judging::JUDGE_KIND_REPORT {
            fail_all("player repository is absent (no snapshot was pushed)".to_string()).await;
            return;
        }
    }

    // The report quotes the panel, so the panel finishes first. Waiting here
    // — before the semaphore — keeps a waiting reporter from starving the
    // runs it is waiting for.
    if judge.kind == arena_core::judging::JUDGE_KIND_REPORT {
        let waited = std::time::Instant::now();
        loop {
            match sibling_runs_terminal(db, session_id, player_id).await {
                Ok(true) => break,
                Ok(false) if waited.elapsed().as_secs() >= REPORT_WAIT_MAX_SECS => {
                    tracing::warn!(
                        session_id = %session_id, player_id = %player_id,
                        "report judge: sibling runs still open after {REPORT_WAIT_MAX_SECS}s; reporting without them"
                    );
                    break;
                }
                Ok(false) => {
                    tokio::time::sleep(std::time::Duration::from_secs(REPORT_WAIT_POLL_SECS)).await;
                }
                Err(e) => {
                    tracing::warn!(error = %e, "report judge: sibling check failed; proceeding");
                    break;
                }
            }
        }
    }

    // One permit for the whole player pass: it is one logical judge run.
    let _permit = match state.judge_semaphore.clone().acquire_owned().await {
        Ok(p) => p,
        Err(e) => {
            tracing::error!(session_id = %session_id, error = %e, "session judges: semaphore closed");
            fail_all("judge semaphore closed".to_string()).await;
            return;
        }
    };

    let recorder = std::sync::Arc::new(JudgeRunRecorder::default());
    let judge_llm = match session_judge_cfg.build_judge_recorded(
        repo_dir.clone(),
        None,
        None,
        Some(recorder.clone()),
    ) {
        Ok(llm) => llm,
        Err(e) => {
            tracing::error!(session_id = %session_id, error = %e, "session judges: LLM build failed");
            fail_all(format!("judge LLM unavailable: {e}")).await;
            return;
        }
    };

    let run_started = std::time::Instant::now();
    // A zero-width rating scale declares a reporter: it writes the player's
    // session report instead of scoring tasks. Both runners return the same
    // output, so everything below — telemetry, straggler closing, the
    // broadcast — is shared.
    let run = if arena_core::judging::report::is_report_judge(&judge_row) {
        arena_core::judging::report::run_session_report(
            db,
            judge_llm.as_ref(),
            &repo_dir,
            session_id,
            player_id,
            &reached,
            &task_judge_map,
            &judge_row,
            judge.id,
            &model,
            &provider,
        )
        .await
    } else if arena_core::judging::appraisal::is_appraisal_judge(&judge_row)
        && let Some((primary, scale, criteria)) =
            appraisal_inputs(db, &judge_row, &reached, &task_judge_map).await
    {
        // A session judge whose scale can pay rather than only take: one
        // verdict about the whole session, scored on the primary pair.
        let inputs = arena_core::judging::appraisal::AppraisalInputs {
            primary_task_judge_id: primary.id,
            scale,
            criteria: criteria.as_ref(),
            agent_setup: arena_core::judging::evidence::EvidenceNeeds::from_column(
                judge_row.evidence_needs.as_deref(),
            )
            .agent_setup,
        };
        arena_core::judging::appraisal::run_session_appraisal(
            db,
            judge_llm.as_ref(),
            &repo_dir,
            session_id,
            player_id,
            &reached,
            &task_judge_map,
            &judge_row,
            &inputs,
            &model,
            &provider,
        )
        .await
    } else {
        arena_core::judging::session::run_session_judge(
            db,
            judge_llm.as_ref(),
            &repo_dir,
            session_id,
            player_id,
            &reached,
            &task_judge_map,
            &judge_row,
            &model,
            &provider,
        )
        .await
    };
    let out = match run {
        Ok(out) => out,
        Err(e) => {
            tracing::error!(session_id = %session_id, player_id = %player_id, error = ?e, "session judges: run failed wholesale");
            // One telemetry row for the whole (failed) player pass — LLM
            // calls may already have been issued and cost tokens.
            record_judge_llm_request(
                db,
                &session_judge_cfg,
                session_id,
                player_id,
                None,
                &judge_row.slug,
                "failed",
                Some(&format!("session judge run failed: {e}")),
                run_started.elapsed().as_millis() as i64,
                Some(&recorder),
                None,
                None,
                serde_json::json!({ "outcome": "failed", "scope": "session" }),
            )
            .await;
            fail_all(format!("session judge run failed: {e}")).await;
            return;
        }
    };

    // Unified LLM telemetry: ONE row aggregating this player's whole
    // session-judge pass (all reached tasks in one run; task_id NULL).
    record_judge_llm_request(
        db,
        &session_judge_cfg,
        session_id,
        player_id,
        None,
        &judge_row.slug,
        "scored",
        None,
        out.duration_ms,
        Some(&recorder),
        None,
        None,
        serde_json::json!({
            "outcome": "scored_by_model",
            "scope": "session",
            "tasks_scored": out.scored,
            "tasks_failed": out.failed,
            "tasks_skipped": out.skipped,
        }),
    )
    .await;

    tracing::info!(
        session_id = %session_id, player_id = %player_id, judge = %judge.slug,
        scored = out.scored, failed = out.failed, duration_ms = out.duration_ms,
        "session judge complete"
    );

    // A pass only writes rows for tasks the dossier carried. Any expected
    // pair it left untouched would read as "pending" forever — and the
    // recovery sweeps would re-run (and re-bill) this whole pass on every
    // tick until the 24h window closes. Close the stragglers terminally.
    let written: std::collections::HashSet<Uuid> =
        out.verdicts.iter().map(|v| v.task_judge_id).collect();
    for tj in task_judge_map.values() {
        if written.contains(&tj.id) {
            continue;
        }
        tracing::warn!(
            session_id = %session_id, player_id = %player_id, judge = %judge.slug,
            task_id = %tj.task_id,
            "session judge: task absent from the dossier; closing its pair as failed"
        );
        let _ = arena_core::judging::record_judge_run_status(
            db,
            session_id,
            player_id,
            tj.id,
            &model,
            &provider,
            "failed",
            Some("task absent from the session dossier — not scored"),
            None,
        )
        .await;
    }

    // Persist the evidence + turn log next to the task-scoped judges' logs, so
    // the admin judge detail has something to show for this run.
    store_session_run_log(
        state, session, player_id, judge, &model, &provider, &out, &recorder,
    )
    .await;

    broadcast_session_verdicts(state, session, player, judge, &out).await;
}

/// Write the session run's log under each scored task's key, so the existing
/// per-(player, task, judge) log store and its admin endpoint keep working.
#[allow(clippy::too_many_arguments)]
async fn store_session_run_log(
    state: &GameServerState,
    session: &arena_core::entities::sessions::Model,
    player_id: Uuid,
    judge: &judges::Model,
    model: &str,
    provider: &str,
    out: &arena_core::judging::session::SessionJudgeOutput,
    recorder: &std::sync::Arc<JudgeRunRecorder>,
) {
    for verdict in &out.verdicts {
        store_run_log_file(
            &state.db,
            session.id,
            &session.join_code,
            player_id,
            verdict.task_id,
            &judge.slug,
            model,
            provider,
            if verdict.point_delta.is_some() {
                "scored"
            } else {
                "failed"
            },
            None,
            Some(recorder),
        )
        .await;
    }
}

/// Announce each scored task verdict the way the task-scoped path does, so the
/// player page and observers see them without a manual refresh.
async fn broadcast_session_verdicts(
    state: &GameServerState,
    session: &arena_core::entities::sessions::Model,
    player: &players::Model,
    judge: &judges::Model,
    out: &arena_core::judging::session::SessionJudgeOutput,
) {
    let db = &state.db;
    let join_code = session.join_code.clone();
    broadcast_leaderboard_ref(state, session.id, &join_code).await;

    // A report is not a verdict. It moves no points and its body is a document
    // for the report tab, not a sentence to read out — announcing it as one
    // would post the raw JSON into the chat as another judge card. It still
    // has to be announced: it is written after the session finished, when the
    // page has stopped polling, so without this frame it appeared only on a
    // manual reload. The cue carries no text; the page re-fetches.
    if judge.kind == arena_core::judging::JUDGE_KIND_REPORT {
        state
            .event_publisher
            .publish(&ZmqEvent::SessionReportReady {
                join_code: join_code.clone(),
                player_id: player.id,
                timestamp: Utc::now(),
            })
            .await;
        return;
    }

    let version = state
        .session_registry
        .get(&join_code)
        .and_then(|e| e.cache.read().ok().map(|c| c.version))
        .unwrap_or(0);
    let now = Utc::now();

    for verdict in &out.verdicts {
        let Some(point_delta) = verdict.point_delta else {
            continue; // failed verdicts carry no score to announce
        };
        let task = tasks::Entity::find_by_id(verdict.task_id)
            .one(db)
            .await
            .ok()
            .flatten();
        let (task_ordinal, task_title) = task
            .map(|t| (t.ordinal, t.title))
            .unwrap_or((0, String::new()));

        state
            .event_publisher
            .publish(&ZmqEvent::JudgeScored {
                join_code: join_code.clone(),
                player_id: player.id,
                player_display_name: player.display_name.clone(),
                task_id: verdict.task_id,
                task_ordinal,
                task_title,
                point_delta,
                judge_slug: judge.slug.clone(),
                judge_name: judge.name.clone(),
                rating: point_delta as f64,
                feedback: verdict.feedback.clone(),
                duration_ms: Some(out.duration_ms),
                timestamp: now,
                version,
                detail: None,
            })
            .await;

        if let Some(tx) = state.player_agent_registry.get(&player.id) {
            send_agent_frame(
                tx.value(),
                PlayerAgentFrame::JudgeScored(JudgeScoredPayload {
                    task_id: verdict.task_id,
                    judge_slug: judge.slug.clone(),
                    judge_name: judge.name.clone(),
                    rating: point_delta as f64,
                    feedback: verdict.feedback.clone(),
                    point_delta,
                    created_at: now,
                }),
                player.id,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{LlmPacer, is_rate_limit, is_retryable, retry_delay};
    use arena_core::judging::JudgeError;

    #[test]
    fn ai_behavior_errors_are_retryable() {
        assert!(is_retryable(&JudgeError::AiParseError));
        assert!(is_retryable(&JudgeError::AiTimeout));
        assert!(is_retryable(&JudgeError::AiRatingOutOfRange));
        assert!(is_retryable(&JudgeError::FeedbackTooLong));
        assert!(is_retryable(&JudgeError::TooManyToolCalls));
        assert!(is_retryable(&JudgeError::Llm("http 502".into())));
    }

    #[test]
    fn config_and_data_errors_are_not_retryable() {
        assert!(!is_retryable(&JudgeError::GitReadError(
            "task_judge not found".into()
        )));
        assert!(!is_retryable(&JudgeError::PlayerRepoNotFound));
        assert!(!is_retryable(&JudgeError::PlayerRepoEmpty));
        assert!(!is_retryable(&JudgeError::Db(sea_orm::DbErr::Custom(
            "x".into()
        ))));
    }

    #[test]
    fn rate_limit_errors_are_detected_from_provider_text() {
        let msg = r#"prompt: CompletionError: HttpError: Invalid status code 429 Too Many Requests with message: {"error":{"code":"provider_rate_limit_exceeded"}}"#;
        assert!(is_rate_limit(&JudgeError::Llm(msg.into())));
        assert!(is_rate_limit(&JudgeError::Llm(
            "Provider rate limit exceeded".into()
        )));
        assert!(!is_rate_limit(&JudgeError::Llm(
            "http 502 bad gateway".into()
        )));
        assert!(!is_rate_limit(&JudgeError::AiTimeout));
    }

    #[test]
    fn rate_limits_back_off_much_longer_than_other_failures() {
        let rl = JudgeError::Llm("429 rate limit".into());
        assert_eq!(retry_delay(&rl, 1).as_secs(), 15);
        assert_eq!(retry_delay(&rl, 2).as_secs(), 30);
        assert_eq!(retry_delay(&rl, 3).as_secs(), 60);
        assert_eq!(retry_delay(&rl, 4).as_secs(), 120);
        assert_eq!(retry_delay(&rl, 5).as_secs(), 240);
        assert_eq!(retry_delay(&rl, 6).as_secs(), 300, "capped at 5 minutes");
        assert_eq!(
            retry_delay(&rl, 20).as_secs(),
            300,
            "shift is clamped, no overflow"
        );

        let other = JudgeError::AiParseError;
        assert_eq!(retry_delay(&other, 1).as_secs(), 2);
        assert_eq!(retry_delay(&other, 2).as_secs(), 4);
    }

    #[test]
    fn rate_limit_budget_is_larger_than_ai_budget_by_default() {
        // Guards the two-budget split in run_judge_with_retries: an
        // upstream-saturated free-tier model gets ~12 minutes of retries
        // (15+30+60+120+240s between 6 attempts), while AI-behavior
        // failures keep the tight 3-attempt loop.
        assert_eq!(super::judge_attempts(), 3);
        assert_eq!(super::judge_rate_limit_attempts(), 6);
        let total_wait: u64 = (1..6)
            .map(|n| retry_delay(&JudgeError::Llm("429".into()), n).as_secs())
            .sum();
        assert_eq!(total_wait, 15 + 30 + 60 + 120 + 240);
    }

    #[tokio::test(start_paused = true)]
    async fn pacer_spaces_calls_by_min_interval() {
        // burst=1 → strictly one call per interval.
        let pacer = LlmPacer::new(std::time::Duration::from_millis(1000), 1);
        let start = tokio::time::Instant::now();
        pacer.wait_turn().await;
        assert!(
            start.elapsed() < std::time::Duration::from_millis(10),
            "first call is immediate"
        );
        pacer.wait_turn().await;
        assert!(
            start.elapsed() >= std::time::Duration::from_millis(1000),
            "second call waits out the interval"
        );
        pacer.wait_turn().await;
        assert!(start.elapsed() >= std::time::Duration::from_millis(2000));
    }

    #[tokio::test(start_paused = true)]
    async fn pacer_allows_a_burst_up_to_capacity() {
        // CONC-H3: burst=3 → the first three calls start immediately (respecting
        // configured concurrency), only the fourth waits out an interval.
        let pacer = LlmPacer::new(std::time::Duration::from_millis(1000), 3);
        let start = tokio::time::Instant::now();
        for _ in 0..3 {
            pacer.wait_turn().await;
        }
        assert!(
            start.elapsed() < std::time::Duration::from_millis(10),
            "first {} calls burst without waiting",
            3
        );
        pacer.wait_turn().await;
        assert!(
            start.elapsed() >= std::time::Duration::from_millis(1000),
            "the fourth call is paced"
        );
    }

    #[tokio::test]
    async fn zero_interval_pacer_is_a_no_op() {
        let pacer = LlmPacer::new(std::time::Duration::ZERO, 1);
        let start = std::time::Instant::now();
        for _ in 0..3 {
            pacer.wait_turn().await;
        }
        assert!(start.elapsed() < std::time::Duration::from_millis(100));
    }
}

/// Does this provider error say the model cannot take image content?
/// Matched loosely across providers: OpenAI-compatible `wrong_api_format`,
/// "image_url ... not supported", "does not support image/vision input".
fn model_rejected_images(error: &str) -> bool {
    let e = error.to_ascii_lowercase();
    e.contains("wrong_api_format")
        || (e.contains("image") && (e.contains("not supported") || e.contains("unsupported")))
        || (e.contains("vision") && e.contains("not "))
}

#[cfg(test)]
mod image_rejection_tests {
    use super::model_rejected_images;

    #[test]
    fn the_cerebras_shape_is_recognized() {
        assert!(model_rejected_images(
            "prompt: CompletionError: HttpError: Invalid status code 400 Bad Request with message: \
             {\"message\":\"Content type 'image_url' is not supported by selected model. Only 'text' \
             content type can be used.\",\"type\":\"invalid_request_error\",\"param\":\"prompt\",\
             \"code\":\"wrong_api_format\",\"id\":\"\"}"
        ));
    }

    #[test]
    fn unrelated_errors_are_not_matched() {
        assert!(!model_rejected_images(
            "rate limit exceeded, retry after 2s"
        ));
        assert!(!model_rejected_images("model 'llama3.2' not found"));
    }
}
