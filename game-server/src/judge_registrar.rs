//! Live probe registration for judges (`register_probe` tool).
//!
//! A judge that cannot answer its rubric from the evidence may register one
//! more probe mid-run. Deterministic registrations run synchronously in the
//! sandbox and return their graded result as tool output; interactive ones
//! ask the participant for an artifact and return `{probe_id, "waiting"}` —
//! the run then ends `waiting` and is re-driven when the artifact lands or
//! its deadline passes (see `probe_scheduler`).
//!
//! Every refusal — over the task limit, over the judge's own limit, bad
//! arguments — is a message to the model, never a run failure: the judge is
//! expected to verdict from what it has.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use arena_core::entities::{artifact_request_watchers, probes, projects, sessions, tests};
use arena_core::evaluation::{
    ARTIFACT_DEFAULT_MAX_BYTES_IMAGE, ArtifactSpec, INITIATOR_JUDGE, ProbeConfig, ProbeMode,
};
use arena_core::judging::ProbeRegistrar;
use async_trait::async_trait;
use chrono::Utc;
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, QueryOrder, Set};
use uuid::Uuid;

use crate::state::GameServerState;

/// Default and bounds for an interactive probe's participant deadline.
/// The floor is minutes, not seconds: a real capture means switching away
/// from the build, running the product, and recording — session 6YJGRC saw
/// a judge ask for a screencast with `deadline_secs: 30` and the request
/// expired before the participant could plausibly react.
const DEFAULT_INTERACTIVE_DEADLINE_SECS: i64 = 900;
/// Smallest window a request is given when the session is nearly over: the
/// participant may still be mid-capture as the clock runs out, and a request
/// that expires before the file lands scores a product nobody looked at.
const INTERACTIVE_DEADLINE_FLOOR_SECS: i64 = 60;

pub struct JudgeProbeRegistrar {
    pub state: GameServerState,
    pub session_id: Uuid,
    pub player_id: Uuid,
    pub task_id: Uuid,
    pub judge_id: Uuid,
    pub judge_slug: String,
    /// `judges.max_interactive` (None = 0: may register none).
    pub max_interactive: i32,
    /// `evaluation.limits.interactive_probes_per_task`.
    pub per_task_limit: u32,
    /// Whether this judge's evidence can carry visual attachments
    /// (`images` in its evidence needs). A judge that cannot see pixels
    /// must not send the participant off to record them.
    pub sees_images: bool,
    interactive_pending: AtomicBool,
    /// Requests THIS run opened. Distinct same-type asks within one run are
    /// legitimate (one per surface); the same-type guard only absorbs open
    /// requests it did NOT open itself — an earlier run's (the re-driven-
    /// and-rephrased case) or another judge's (attach-as-watcher).
    created_this_run: std::sync::Mutex<Vec<Uuid>>,
}

impl JudgeProbeRegistrar {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        state: GameServerState,
        session_id: Uuid,
        player_id: Uuid,
        task_id: Uuid,
        judge_id: Uuid,
        judge_slug: String,
        max_interactive: i32,
        per_task_limit: u32,
        sees_images: bool,
    ) -> Arc<Self> {
        Arc::new(Self {
            state,
            session_id,
            player_id,
            task_id,
            judge_id,
            judge_slug,
            max_interactive,
            per_task_limit,
            sees_images,
            interactive_pending: AtomicBool::new(false),
            created_this_run: std::sync::Mutex::new(Vec::new()),
        })
    }

    /// Artifacts already delivered for this task by ANY judge's request:
    /// what a newly-asking judge should reuse instead of sending the
    /// participant off to capture the same thing again.
    async fn delivered_artifacts(&self) -> Vec<serde_json::Value> {
        let mut out = Vec::new();
        for t in self.interactive_tests().await {
            let Some(cfg) = t
                .probe_config
                .as_ref()
                .and_then(|c| ProbeConfig::from_json(c).ok())
            else {
                continue;
            };
            let Some(artifact) = cfg.artifact else {
                continue;
            };
            let delivered = probes::Entity::find()
                .filter(probes::Column::TestId.eq(t.id))
                .filter(probes::Column::PlayerId.eq(self.player_id))
                .filter(probes::Column::ArtifactPath.is_not_null())
                .order_by_desc(probes::Column::DispatchedAt)
                .one(&self.state.db)
                .await
                .ok()
                .flatten();
            if delivered.is_none() {
                continue;
            }
            out.push(serde_json::json!({
                "artifact_path": artifact.path,
                "content_type": artifact.content_type,
                "requested_by": t.prompt,
                "instruction": cfg
                    .instruction
                    .map(|i| i.chars().take(160).collect::<String>()),
            }));
        }
        out
    }

    /// All interactive tests rows registered on this task (any judge).
    /// The participant's scheduler cursor points at a later task than the
    /// one this judge is judging. No cursor row (a run outside the socket
    /// loop, e.g. an admin re-run) is not "moved on".
    async fn participant_moved_on(&self) -> bool {
        use arena_core::entities::session_scheduler_state;
        session_scheduler_state::Entity::find()
            .filter(session_scheduler_state::Column::SessionIdFk.eq(self.session_id))
            .filter(session_scheduler_state::Column::PlayerIdFk.eq(self.player_id))
            .one(&self.state.db)
            .await
            .ok()
            .flatten()
            .and_then(|row| row.task_id)
            .is_some_and(|current| current != self.task_id)
    }

    async fn interactive_tests(&self) -> Vec<tests::Model> {
        tests::Entity::find()
            .filter(tests::Column::SessionId.eq(self.session_id))
            .filter(tests::Column::TaskId.eq(self.task_id))
            .filter(tests::Column::Initiator.eq(INITIATOR_JUDGE))
            .all(&self.state.db)
            .await
            .unwrap_or_default()
            .into_iter()
            .filter(|t| {
                t.probe_config
                    .as_ref()
                    .and_then(|c| ProbeConfig::from_json(c).ok())
                    .is_some_and(|c| c.mode == ProbeMode::Interactive)
            })
            .collect()
    }

    /// How long this session still has to run — the window an artifact
    /// request is given. Falls back to the default when the session's clock
    /// cannot be read (a project row deleted under it, a row without
    /// `started_at`), which is the old behaviour rather than a zero-second
    /// request nobody could answer.
    async fn remaining_session_secs(&self, session: Option<&sessions::Model>) -> i64 {
        let Some(session) = session else {
            return DEFAULT_INTERACTIVE_DEADLINE_SECS;
        };
        let Some(started_at) = session.started_at else {
            return DEFAULT_INTERACTIVE_DEADLINE_SECS;
        };
        let duration = match projects::Entity::find_by_id(session.project_id_fk)
            .one(&self.state.db)
            .await
        {
            Ok(Some(p)) => p.default_session_duration_secs.max(0) as u64,
            _ => return DEFAULT_INTERACTIVE_DEADLINE_SECS,
        };
        let remaining = crate::state::compute_remaining(
            duration,
            started_at,
            Utc::now(),
            session.paused_duration_secs,
        );
        // A request with no time left is still worth registering: it keeps the
        // judge honest about what it could not see, and the floor means a
        // capture landing in the last moments still counts.
        remaining.max(INTERACTIVE_DEADLINE_FLOOR_SECS)
    }

    async fn register_interactive(&self, args: &serde_json::Value) -> String {
        // A finished session has nobody left to answer: the participant's
        // loop is gone, so a new request would dangle forever (6YJGRC saw
        // judges re-driven at expiry ask for fresh captures 15s after the
        // session ended). Verdict from the evidence at hand instead.
        let session = sessions::Entity::find_by_id(self.session_id)
            .one(&self.state.db)
            .await
            .ok()
            .flatten();
        if session.as_ref().is_none_or(|s| s.finished_at.is_some()) {
            return serde_json::json!({
                "error": "the session is over — no participant is left to deliver an artifact; \
                          give your verdict from the evidence you have (null with a rationale \
                          where you genuinely cannot assess)"
            })
            .to_string();
        }
        let instruction = args["instruction"]
            .as_str()
            .unwrap_or("")
            .trim()
            .to_string();
        if instruction.is_empty() {
            return r#"{"error": "interactive registration needs `instruction`"}"#.to_string();
        }
        let content_type = args["content_type"]
            .as_str()
            .unwrap_or("image/png")
            .to_string();
        // A judge whose evidence cannot carry visual attachments would never
        // see the capture it asks for — the participant's recording time
        // would be spent on a file nobody reads (6YJGRC: the data and
        // code-quality judges both requested screencasts they cannot view).
        if !self.sees_images
            && (content_type.starts_with("image/") || content_type.starts_with("video/"))
        {
            return serde_json::json!({
                "error": "your evidence cannot include visual artifacts, so you would never see \
                          this capture — request a text artifact (a note or report) instead, or \
                          verdict from the code and probe results you already have"
            })
            .to_string();
        }
        let max_bytes = args["max_bytes"]
            .as_u64()
            .unwrap_or(ARTIFACT_DEFAULT_MAX_BYTES_IMAGE);
        // The request runs on the session's clock, not on a number the judge
        // picked. Any shorter window expires while the participant is still
        // building — session TJQJPJ lost a ux-review verdict that way, the
        // request timing out five minutes before the session did and the
        // judge then scoring a product it had never seen. The session ending
        // is the one deadline that cannot be wrong, and while a request is
        // open the session is not finished: an unanswered capture holds it to
        // its full time rather than closing the book early.
        let deadline_secs = self.remaining_session_secs(session.as_ref()).await;

        // A judge may hold several open requests, one per distinct artifact
        // (e.g. a landing-page shot AND a profile shot), bounded by its own
        // budget and the task-wide cap. Dedupe is by instruction: a re-driven
        // run asking for the same thing finds its earlier request instead of
        // opening another.
        let existing = self.interactive_tests().await;
        let mine: Vec<&tests::Model> = existing
            .iter()
            .filter(|t| t.registered_by_judge_id == Some(self.judge_id))
            .collect();
        let same_instruction = mine.iter().find(|t| {
            t.probe_config
                .as_ref()
                .and_then(|c| ProbeConfig::from_json(c).ok())
                .and_then(|c| c.instruction)
                .is_some_and(|i| i.trim() == instruction)
        });
        if let Some(earlier) = same_instruction {
            let latest = probes::Entity::find()
                .filter(probes::Column::TestId.eq(earlier.id))
                .filter(probes::Column::PlayerId.eq(self.player_id))
                .order_by_desc(probes::Column::DispatchedAt)
                .one(&self.state.db)
                .await
                .ok()
                .flatten();
            return match latest {
                Some(p) if p.outcome.is_some() => serde_json::json!({
                    "probe_id": p.id,
                    "status": p.outcome,
                    "note": "you already requested exactly this — its result is in the task's probes",
                })
                .to_string(),
                Some(p) => {
                    self.interactive_pending.store(true, Ordering::SeqCst);
                    serde_json::json!({
                        "probe_id": p.id,
                        "status": "waiting",
                        "note": "you already requested exactly this — it is still waiting for the participant",
                    })
                    .to_string()
                }
                None => serde_json::json!({
                    "test_id": earlier.id,
                    "status": "queued",
                    "note": "you already requested exactly this — it is queued for the participant",
                })
                .to_string(),
            };
        }

        // The registry of open requests. A re-driven run rephrases:
        // exact-instruction dedupe missed the second "capture desktop.png +
        // mobile.png" in session TTGNDZ, and the participant recorded the
        // same screenshots twice. Any OPEN request of the same content type
        // on this task — this judge's earlier run OR another judge's — is
        // treated as the same ask unless the model insists (`confirm`) that
        // it needs a genuinely different capture. Matching another judge's
        // request ATTACHES this judge as a watcher: no second request goes
        // out, and the one delivery re-drives every subscriber. Requests
        // this run opened itself stay distinct — one per surface is the
        // sanctioned flow.
        let this_run: Vec<Uuid> = self
            .created_this_run
            .lock()
            .map(|v| v.clone())
            .unwrap_or_default();
        if args["confirm"].as_bool() != Some(true) {
            // Own open requests first: pointing the judge at its own earlier
            // ask beats subscribing it to somebody else's.
            let mut candidates: Vec<&tests::Model> = existing
                .iter()
                .filter(|t| !this_run.contains(&t.id))
                .collect();
            candidates.sort_by_key(|t| t.registered_by_judge_id != Some(self.judge_id));
            for t in candidates {
                let Some(cfg) = t
                    .probe_config
                    .as_ref()
                    .and_then(|c| ProbeConfig::from_json(c).ok())
                else {
                    continue;
                };
                if cfg.artifact.as_ref().map(|a| a.content_type.as_str()) != Some(&content_type) {
                    continue;
                }
                let latest = probes::Entity::find()
                    .filter(probes::Column::TestId.eq(t.id))
                    .filter(probes::Column::PlayerId.eq(self.player_id))
                    .order_by_desc(probes::Column::DispatchedAt)
                    .one(&self.state.db)
                    .await
                    .ok()
                    .flatten();
                let open = match &latest {
                    Some(p) => p.outcome.is_none(),
                    None => true, // queued, not yet dispatched
                };
                if !open {
                    continue;
                }
                let own = t.registered_by_judge_id == Some(self.judge_id);
                if !own {
                    let _ = artifact_request_watchers::Entity::insert(
                        artifact_request_watchers::ActiveModel {
                            test_id: Set(t.id),
                            judge_id: Set(self.judge_id),
                            created_at: Set(Utc::now()),
                        },
                    )
                    .on_conflict(
                        sea_orm::sea_query::OnConflict::columns([
                            artifact_request_watchers::Column::TestId,
                            artifact_request_watchers::Column::JudgeId,
                        ])
                        .do_nothing()
                        .to_owned(),
                    )
                    .exec(&self.state.db)
                    .await;
                }
                self.interactive_pending.store(true, Ordering::SeqCst);
                return serde_json::json!({
                    "test_id": t.id,
                    "status": "waiting",
                    "requested_by": t.prompt,
                    "open_instruction": cfg.instruction,
                    "note": if own {
                        "you already have an OPEN capture request of this type on this task — \
                         its delivery will land in your evidence, do not ask the participant \
                         to record it again. If you truly need a DIFFERENT additional capture, \
                         repeat this call with `confirm: true`."
                    } else {
                        "an equivalent capture request is already OPEN on this task (see \
                         `open_instruction`) — you are now attached to it: the one delivery \
                         lands in your evidence and this run is revisited with it, so do not \
                         ask the participant to record it again. If it does NOT cover what \
                         you need, repeat this call with `confirm: true`."
                    },
                })
                .to_string();
            }
        }

        // A request nobody can answer must not be opened: the probe is
        // dispatched to the participant's agent, so a disconnected agent
        // (or an ended session) means a guaranteed expiry — the judge
        // should verdict from the evidence it has instead of waiting.
        let agent_reachable = self
            .state
            .player_agent_registry
            .get(&self.player_id)
            .is_some();
        let session_running = sessions::Entity::find_by_id(self.session_id)
            .one(&self.state.db)
            .await
            .ok()
            .flatten()
            .is_some_and(|s| s.status == arena_core::session_status::SessionStatus::Running);
        if !agent_reachable || !session_running {
            return serde_json::json!({
                "error": "the participant's agent is no longer reachable (session over or                           disconnected); verdict from the evidence you have"
            })
            .to_string();
        }

        // Limits: the judge's own budget, then the task-wide cap.
        if self.max_interactive <= 0 {
            return serde_json::json!({
                "error": format!(
                    "judge `{}` declared no interactive budget (max_interactive)",
                    self.judge_slug
                )
            })
            .to_string();
        }
        if mine.len() as i32 >= self.max_interactive {
            return serde_json::json!({
                "error": format!(
                    "your interactive budget ({}) is spent on the requests you already made;                      verdict from what you have and what arrives",
                    self.max_interactive
                )
            })
            .to_string();
        }
        if existing.len() as u32 >= self.per_task_limit {
            return serde_json::json!({
                "error": format!(
                    "task-wide interactive limit reached ({}); verdict from the evidence you have",
                    self.per_task_limit
                )
            })
            .to_string();
        }

        // Reuse before re-capture: when this task already holds delivered
        // artifacts (any judge's request — 6YJGRC had three judges collect
        // the same forecast screencast), surface the inventory once. The
        // model either uses what exists — visual artifacts are already
        // attached to every image-capable judge's evidence — or insists on
        // a genuinely new capture by repeating the call with `confirm: true`.
        if args["confirm"].as_bool() != Some(true) {
            let delivered = self.delivered_artifacts().await;
            if !delivered.is_empty() {
                return serde_json::json!({
                    "already_delivered": delivered,
                    "note": "these artifacts were already delivered for this task — if one \
                             covers your need, use it instead of asking again (visual ones are \
                             attached to your evidence when you can see images). If you need a \
                             capture these do NOT show, repeat this call with the same arguments \
                             plus `confirm: true`.",
                })
                .to_string();
            }
        }

        // Materialize the request as a REGULAR probe: a tests row whose
        // command is a runnable availability check. The ordinary dispatch
        // loop delivers it (TestPush), the agent satisfies it by saving the
        // file, ololo commits and pushes the file (git is the only channel
        // for content), and ordinary grading marks it passed. The artifact
        // path is keyed by the tests row id so every retry checks the same
        // folder.
        let test_id = Uuid::new_v4();
        let artifact_repo_path = format!(".ololo/artifacts/{test_id}/");
        // The judge wrote its instruction BEFORE this folder existed, so any
        // destination it named is invented — and a probe naming two folders
        // is exactly how a participant gets lost. Rewrite every mention to
        // the authoritative one.
        let instruction = rewrite_artifact_paths(&instruction, &artifact_repo_path);
        let config = ProbeConfig {
            mode: ProbeMode::Interactive,
            executor: None,
            schedule: None,
            points: None,
            report: None,
            tool: None,
            rubric: None,
            instruction: Some(instruction.clone()),
            artifact: Some(ArtifactSpec {
                content_type: content_type.clone(),
                max_bytes,
                path: Some(artifact_repo_path.clone()),
            }),
            deadline_secs: Some(deadline_secs),
            capture_memory: None,
        };
        let next_ordinal = tests::Entity::find()
            .filter(tests::Column::SessionId.eq(self.session_id))
            .filter(tests::Column::TaskId.eq(self.task_id))
            .all(&self.state.db)
            .await
            .unwrap_or_default()
            .iter()
            .map(|t| t.ordinal)
            .filter(|o| *o >= 2000)
            .max()
            .map(|o| o + 1)
            .unwrap_or(2000);
        // Plain ASCII only: this text renders in arbitrary agent terminals,
        // and a mangled dash turns an instruction into noise.
        let command = format!(
            "# ARTIFACT REQUEST from {judge}: {instruction}\n\
             # Save the file(s) (up to 5) under {path}; the ololo CLI commits and pushes \
             them automatically; do NOT run git.\n\
             # If an earlier delivered capture already shows exactly this, copying that \
             file into this folder is a valid delivery.\n\
             test -n \"$(ls -A {path} 2>/dev/null)\" && echo delivered || echo \"waiting-for-file: save the capture into {path}\"",
            judge = self.judge_slug,
            instruction = header_line(&instruction),
            path = artifact_repo_path,
        );
        if let Err(e) = (tests::ActiveModel {
            id: Set(test_id),
            command_template: Set(command),
            answer_template: Set("result.includes(\"delivered\")".to_string()),
            fixture_definitions: Set(r#"{"kind":"js","script":"({})"}"#.to_string()),
            created_at: Set(Utc::now()),
            session_id: Set(self.session_id),
            task_id: Set(self.task_id),
            ordinal: Set(next_ordinal),
            prompt: Set(format!("interactive: {}", self.judge_slug)),
            description: Set(Some(instruction.to_string())),
            probe_config: Set(serde_json::to_value(&config).ok()),
            initiator: Set(INITIATOR_JUDGE.to_string()),
            registered_by_judge_id: Set(Some(self.judge_id)),
        }
        .insert(&self.state.db)
        .await)
        {
            return serde_json::json!({ "error": format!("registration failed: {e}") }).to_string();
        }

        // Observability: the registration.
        let join_code = session.map(|s| s.join_code).unwrap_or_default();
        self.state
            .event_publisher
            .publish(&arena_core::protocol::ZmqEvent::ProbeRegistered {
                join_code,
                player_id: self.player_id,
                task_id: self.task_id,
                probe_id: test_id,
                judge_slug: self.judge_slug.clone(),
                mode: "interactive".to_string(),
                timestamp: Utc::now(),
            })
            .await;

        self.interactive_pending.store(true, Ordering::SeqCst);
        if let Ok(mut v) = self.created_this_run.lock() {
            v.push(test_id);
        }
        serde_json::json!({
            "test_id": test_id,
            "status": "queued",
            "artifact_path": artifact_repo_path,
            "note": "registered; the participant's loop will dispatch it. Your run pauses \
                     here and resumes with what became of the request (the capture \
                     attached, or word that it expired) — do not write a verdict now",
        })
        .to_string()
    }

    async fn register_deterministic(&self, args: &serde_json::Value) -> String {
        let command = args["command"].as_str().unwrap_or("").trim().to_string();
        if command.is_empty() {
            return r#"{"error": "deterministic registration needs `command`"}"#.to_string();
        }
        let validation = args["validation"].as_str().unwrap_or("").to_string();
        // A judge-authored script with no context is unreadable on every
        // probe surface — the header says whose probe it is and why.
        let purpose = args["purpose"].as_str().unwrap_or("").trim().to_string();
        let command = if purpose.is_empty() {
            format!(
                "# PROBE registered by judge '{}'\n{command}",
                self.judge_slug
            )
        } else {
            format!(
                "# PROBE registered by judge '{}': {purpose}\n{command}",
                self.judge_slug
            )
        };
        let config = ProbeConfig {
            mode: ProbeMode::Deterministic,
            executor: None,
            schedule: None,
            points: None,
            report: None,
            tool: None,
            rubric: None,
            instruction: None,
            artifact: None,
            deadline_secs: None,
            capture_memory: None,
        };
        let next_ordinal = tests::Entity::find()
            .filter(tests::Column::SessionId.eq(self.session_id))
            .filter(tests::Column::TaskId.eq(self.task_id))
            .all(&self.state.db)
            .await
            .unwrap_or_default()
            .iter()
            .map(|t| t.ordinal)
            .filter(|o| *o >= 2000)
            .max()
            .map(|o| o + 1)
            .unwrap_or(2000);
        let test_row = match (tests::ActiveModel {
            id: Set(Uuid::new_v4()),
            command_template: Set(command),
            answer_template: Set(validation),
            fixture_definitions: Set(r#"{"kind":"js","script":"({})"}"#.to_string()),
            created_at: Set(Utc::now()),
            session_id: Set(self.session_id),
            task_id: Set(self.task_id),
            ordinal: Set(next_ordinal),
            prompt: Set(format!("registered: {}", self.judge_slug)),
            description: Set(Some(purpose.clone()).filter(|s| !s.is_empty())),
            probe_config: Set(serde_json::to_value(&config).ok()),
            initiator: Set(INITIATOR_JUDGE.to_string()),
            registered_by_judge_id: Set(Some(self.judge_id)),
        }
        .insert(&self.state.db)
        .await)
        {
            Ok(t) => t,
            Err(e) => {
                return serde_json::json!({ "error": format!("registration failed: {e}") })
                    .to_string();
            }
        };

        // The participant's loop dispatches it, exactly like an artifact
        // request. It used to run here instead, in a scratch copy of the
        // snapshot — and a judge's `npm test` failed there
        // (`Could not find '/tmp/.tmpXXXX/test/**/*.test.ts'`) while the same
        // command passed in the player's workspace, which is the only place
        // that holds their toolchain, their dependencies and their running
        // process. A judge asks the player to run something; it does not run
        // it behind their back on a different machine.
        let join_code = sessions::Entity::find_by_id(self.session_id)
            .one(&self.state.db)
            .await
            .ok()
            .flatten()
            .map(|s| s.join_code)
            .unwrap_or_default();
        self.state
            .event_publisher
            .publish(&arena_core::protocol::ZmqEvent::ProbeRegistered {
                join_code,
                player_id: self.player_id,
                task_id: self.task_id,
                probe_id: test_row.id,
                judge_slug: self.judge_slug.clone(),
                mode: "deterministic".to_string(),
                timestamp: Utc::now(),
            })
            .await;

        // Same waiting contract as an artifact request: this run verdicts on
        // what it has, and the scheduler re-drives it once the probe lands.
        self.interactive_pending.store(true, Ordering::SeqCst);
        if let Ok(mut v) = self.created_this_run.lock() {
            v.push(test_row.id);
        }
        serde_json::json!({
            "test_id": test_row.id,
            "status": "queued",
            "note": "registered; the participant's loop runs it on THEIR machine and \
                     reports back. Your run pauses here and resumes with the result — do \
                     not write a verdict now",
        })
        .to_string()
    }
}

#[async_trait]
impl ProbeRegistrar for JudgeProbeRegistrar {
    async fn register(&self, args: &serde_json::Value) -> String {
        // The queue dispatches probes for the participant's CURRENT task
        // only. A judge re-driven after the judge phase gave up on its
        // capture — the participant already on the next task — used to
        // register a fresh one that nobody would ever hand over, and then
        // wait its whole cap for it (ZNQZEB: five dead minutes, a third
        // full run). Once the participant has moved on, the evidence at
        // hand is all there will be.
        if self.participant_moved_on().await {
            return serde_json::json!({
                "error": "the participant has already moved on to the next task — nothing \
                          registered now can be dispatched to them; give your verdict from \
                          the evidence you have (null with a rationale where you genuinely \
                          cannot assess)"
            })
            .to_string();
        }
        match args["mode"].as_str() {
            Some("interactive") => self.register_interactive(args).await,
            Some("deterministic") => self.register_deterministic(args).await,
            other => serde_json::json!({
                "error": format!("unknown register_probe mode {other:?} (deterministic|interactive)")
            })
            .to_string(),
        }
    }

    fn interactive_pending(&self) -> bool {
        self.interactive_pending.load(Ordering::SeqCst)
    }
}

/// The instruction as the ONE comment line the request's shell command
/// opens with. A judge writes prose — a numbered list, blank lines, an
/// apostrophe in "Bangkok's" — and every line of it past the first would
/// run as shell: `1. **rome.png** ...` is a command, an unmatched quote is
/// a syntax error that kills the script before the availability check at
/// the bottom ever prints `delivered`. The full text, line breaks intact,
/// travels as the tests row's description; the shell sees one `#` line.
fn header_line(instruction: &str) -> String {
    instruction.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Rewrite every `.ololo/artifacts/<segment>/` (or a bare
/// `.ololo/artifacts/<segment>`) a judge's instruction mentions to the
/// authoritative request folder. File names after the folder survive:
/// `.ololo/artifacts/made-up/desktop.png` keeps `desktop.png`.
fn rewrite_artifact_paths(instruction: &str, real_path: &str) -> String {
    const PREFIX: &str = ".ololo/artifacts/";
    let mut out = String::with_capacity(instruction.len());
    let mut rest = instruction;
    while let Some(i) = rest.find(PREFIX) {
        out.push_str(&rest[..i]);
        let after = &rest[i + PREFIX.len()..];
        // The invented folder segment runs to the next '/', or — for a bare
        // folder mention — to the next delimiter.
        let seg_end = after
            .find(|c: char| {
                c == '/' || c.is_whitespace() || matches!(c, '`' | '\'' | '"' | ')' | ',' | ';')
            })
            .unwrap_or(after.len());
        out.push_str(real_path); // ends with '/'
        rest = match after[seg_end..].strip_prefix('/') {
            Some(tail) => tail,
            None => &after[seg_end..],
        };
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod instruction_tests {
    use super::{header_line, rewrite_artifact_paths};

    #[test]
    fn a_multi_line_instruction_folds_into_one_comment_line() {
        let got = header_line(
            "Capture four screenshots.\n\n1. **rome.png** — open `/?city=rome`.\n2. \
             **bangkok.png** — must show Bangkok's card reading 91°F.\n",
        );
        assert!(!got.contains('\n'), "{got}");
        assert_eq!(
            got,
            "Capture four screenshots. 1. **rome.png** — open `/?city=rome`. 2. \
             **bangkok.png** — must show Bangkok's card reading 91°F."
        );
    }

    const REAL: &str = ".ololo/artifacts/785c412c-d73f-4cb8-beab-c4a26f035472/";

    #[test]
    fn invented_folders_are_rewritten_and_file_names_survive() {
        let got = rewrite_artifact_paths(
            "Save them as `.ololo/artifacts/probe-weather-widget-f/desktop.png` and \
             `.ololo/artifacts/probe-weather-widget-f/mobile.png` respectively.",
            REAL,
        );
        assert_eq!(
            got,
            format!("Save them as `{REAL}desktop.png` and `{REAL}mobile.png` respectively.")
        );
    }

    #[test]
    fn bare_folder_mentions_are_rewritten_too() {
        let got = rewrite_artifact_paths("Put the PNG into .ololo/artifacts/shots and push.", REAL);
        assert_eq!(got, format!("Put the PNG into {REAL} and push."));
    }

    #[test]
    fn instructions_without_paths_pass_through() {
        let s = "Capture the page at 1280px and 375px as separate PNG files.";
        assert_eq!(rewrite_artifact_paths(s, REAL), s);
    }
}
