//! One snapshot of everything a judge is allowed to know.
//!
//! Until now the evidence a judge could reach depended on which of three
//! judges it happened to be. A tools judge got git and nothing else; a dossier
//! judge got a prose pack; the execution judge got probes and session memory
//! that no LLM judge could see. Nobody could reach the player's *other* tasks,
//! which is exactly what a golf ladder needs to know — that a later rung
//! already proved the solution fits.
//!
//! So: one value, assembled once, the same for every judge. It is plain data
//! and `Serialize`, which buys three things at once — a deterministic
//! pre-check can read it, a prompt can carry it, and a finished run can record
//! the exact snapshot its verdict was based on.
//!
//! Everything here is an observation, never a conclusion. `diff_is_empty` is a
//! fact; "the player cheated" is not.

use std::collections::BTreeMap;
use std::path::Path;

use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, QueryOrder};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::entities::{probes, projects, sessions, tasks, tests};
use crate::judging::JudgeError;
use crate::judging::dossier::{
    DiffStat, PointsAwarded, ProbeSummary, RepoActivity, SessionDossier, build_session_dossier,
};
use crate::validation::judge_results::RatingScale;

/// One probe of the task under judgement, as it was recorded live.
///
/// The execution judge has always had these; an LLM judge could only infer
/// them from counts. A byte-budget rung, for one, is unreadable without the
/// answer the probe actually returned.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProbeFacts {
    /// Position of the probe's test within the task, 0-based.
    pub test_ordinal: Option<i32>,
    /// `pass`, `fail`, `no_response`, `error`, `unavailable` — or absent
    /// while in flight.
    pub outcome: Option<String>,
    pub point_delta: i32,
    /// What the player's command printed, trimmed and capped.
    pub answer: Option<String>,
    pub expected: Option<String>,
    pub dispatched_at: Option<chrono::DateTime<chrono::Utc>>,
    pub resolved_at: Option<chrono::DateTime<chrono::Utc>>,
    /// Structured measurement (TODO report, analysis metrics, llm scores,
    /// snapshot sha/age) for probes that carry one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result_json: Option<serde_json::Value>,
}

/// A task of the same player's session, as context for the one being judged.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskBrief {
    pub id: Uuid,
    pub ordinal: i32,
    pub title: String,
    pub tags: Vec<String>,
    /// What this task banked for the player.
    pub earned: i32,
    /// The commit carrying this task's work, when there is one. A review
    /// checking that a verdict cites something real needs the shas of every
    /// task, not only the one under judgement.
    pub commit_sha: Option<String>,
    pub probes: ProbeSummary,
    pub diff: Option<DiffStat>,
    /// See the `FLAG_*` constants in [`crate::judging::dossier`].
    pub flags: Vec<String>,
}

/// The task under judgement.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskFacts {
    pub id: Uuid,
    pub ordinal: i32,
    pub title: String,
    pub description: String,
    pub tags: Vec<String>,
    /// What the task paid — and therefore the most a penalty may reverse.
    pub points: PointsAwarded,
    pub earned: i32,
    pub commit_sha: Option<String>,
    pub diff: Option<DiffStat>,
    /// True when the work window changed no lines at all. A fact worth
    /// looking into, never a verdict on its own: the solution may have been
    /// written before the window opened, or the task may have been satisfied
    /// by code an earlier task legitimately introduced.
    pub diff_is_empty: bool,
    pub probes: Vec<ProbeFacts>,
    pub probe_summary: ProbeSummary,
    pub flags: Vec<String>,
}

/// The judge asking, and the bounds it must answer within.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JudgeFacts {
    pub slug: String,
    pub scale_min: f64,
    pub scale_max: f64,
    pub scale_step: f64,
    /// True when the scale can only take points away.
    pub is_penalty: bool,
}

/// What this judge said about this task last time, if it has run before.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PriorVerdict {
    pub rating: f64,
    pub feedback: String,
}

/// Everything a judge may know about one (player, task).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Evidence {
    pub session_id: Uuid,
    pub player_id: Uuid,
    pub task: TaskFacts,
    /// Every task this player reached, in ordinal order — including the one
    /// under judgement. A ladder judge reads this to see that a tighter rung
    /// already passed.
    pub tasks: Vec<TaskBrief>,
    /// The project's memory schema merged with what was extracted from the
    /// player's AGENTS.md / README.md.
    pub memory: BTreeMap<String, String>,
    /// Lines the whole session wrote after the session-start snapshot —
    /// task commits and wip/memory snapshots alike. Zero, with probes
    /// passing, is the replay signature no per-task view can show.
    #[serde(default)]
    pub repo: RepoActivity,
    pub judge: JudgeFacts,
    pub prior: Option<PriorVerdict>,
}

impl Evidence {
    /// JSON for a prompt, or for the run record.
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_else(|_| "{}".to_string())
    }
}

/// What a judge declared it needs to see.
///
/// The snapshot is not free: the cross-task view walks git twice for every
/// task the player reached, and most judges never look at it — an execution
/// judge re-runs probes, a `tools` judge fetches what it wants itself. Only a
/// judge's own programs read the snapshot, and each reads a slice.
///
/// So a judge lists its slice in front-matter and pays for that much. An
/// undeclared judge ([`EvidenceNeeds::everything`]) keeps the whole snapshot,
/// which is what every judge had before declarations existed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EvidenceNeeds {
    /// Every task the player reached, not only the one under judgement. The
    /// expensive one: a commit lookup and a diff per task.
    pub tasks: bool,
    /// The judged task's individual probes, with the answers they recorded.
    pub probes: bool,
    /// The session's memory map.
    pub memory: bool,
    /// Delivered visual artifacts (screenshots, screencast frames) attached
    /// to the model turn. Only for judges that actually look at pixels — a
    /// text-only model 400s on image content.
    pub images: bool,
    /// The agent-setup pack: agent instructions, skills, subagents, hooks and
    /// MCP configuration in the final snapshot, joined with the tool and skill
    /// histograms the player's CLI reported. Session appraisals only — see
    /// [`crate::judging::agent_setup`].
    pub agent_setup: bool,
}

/// Section names a judge may declare, for parsing and for error messages.
pub const EVIDENCE_SECTIONS: [&str; 5] = ["tasks", "probes", "memory", "images", "agent_setup"];

impl EvidenceNeeds {
    /// What an undeclared judge gets: everything, as before declarations.
    pub const fn everything() -> Self {
        Self {
            tasks: true,
            probes: true,
            memory: true,
            images: true,
            agent_setup: true,
        }
    }

    /// The floor nobody can opt out of: the task under judgement, the judge's
    /// own bounds, and any prior verdict. Without those there is no verdict to
    /// reach and nothing to record.
    pub const fn minimal() -> Self {
        Self {
            tasks: false,
            probes: false,
            memory: false,
            images: false,
            agent_setup: false,
        }
    }

    /// Read a declaration. Unknown names are rejected by name so the author
    /// learns which one, rather than silently getting less than they asked
    /// for — a judge quietly reading an empty `tasks[]` would draw confident
    /// conclusions from evidence that was never gathered.
    pub fn parse(sections: &[String]) -> Result<Self, String> {
        let mut needs = Self::minimal();
        for section in sections {
            match section.trim() {
                "tasks" => needs.tasks = true,
                "probes" => needs.probes = true,
                "memory" => needs.memory = true,
                "images" => needs.images = true,
                "agent_setup" => needs.agent_setup = true,
                "all" => needs = Self::everything(),
                other => {
                    return Err(format!(
                        "unknown evidence section `{other}` (known: {}, all)",
                        EVIDENCE_SECTIONS.join(", ")
                    ));
                }
            }
        }
        Ok(needs)
    }

    /// Parse the `judges.evidence_needs` column: a JSON array of section
    /// names, or NULL for a judge that never declared.
    pub fn from_column(raw: Option<&str>) -> Self {
        let Some(raw) = raw.map(str::trim).filter(|s| !s.is_empty()) else {
            return Self::everything();
        };
        match serde_json::from_str::<Vec<String>>(raw) {
            Ok(sections) => Self::parse(&sections).unwrap_or_else(|e| {
                // Stored declarations are validated on the way in, so this is
                // a column edited around the API. Widening is the safe way to
                // be wrong: a judge sees more than it asked for, never less.
                tracing::warn!(error = %e, "evidence needs: bad declaration, using the whole snapshot");
                Self::everything()
            }),
            Err(e) => {
                tracing::warn!(error = %e, "evidence needs: unparseable column, using the whole snapshot");
                Self::everything()
            }
        }
    }

    /// Serialize back to the column shape.
    pub fn to_column(self) -> String {
        let mut out = Vec::new();
        if self.tasks {
            out.push("tasks");
        }
        if self.probes {
            out.push("probes");
        }
        if self.memory {
            out.push("memory");
        }
        if self.images {
            out.push("images");
        }
        if self.agent_setup {
            out.push("agent_setup");
        }
        serde_json::to_string(&out).unwrap_or_else(|_| "[]".to_string())
    }
}

/// Cap on a recorded probe answer. Long enough for a byte count, a diff line
/// or a one-line response; short enough that a runaway program cannot bloat
/// the snapshot.
const MAX_ANSWER_LEN: usize = 2_000;

/// Assemble the evidence for one (player, task).
///
/// Reuses the session dossier for the cross-task view rather than re-deriving
/// it: that builder already correlates git, probes and payouts, and a second
/// implementation would drift from it. What it adds is the focused task's
/// tags, its individual probes, and the session's memory — the three things no
/// task-scoped judge could reach before.
#[allow(clippy::too_many_arguments)]
pub async fn build_evidence(
    db: &DatabaseConnection,
    repo_dir: &Path,
    session_id: Uuid,
    player_id: Uuid,
    task_id: Uuid,
    judge_slug: &str,
    scale: &RatingScale,
    prior: Option<PriorVerdict>,
    needs: EvidenceNeeds,
    // The judge's blind spots — see `crate::judging::tools::ToolScope`.
    scope: &crate::judging::tools::ToolScope,
) -> Result<Evidence, JudgeError> {
    // The cross-task view is the expensive half: a commit lookup and a diff
    // per task the player reached. A judge that did not ask for it pays for
    // one task instead of a session's worth.
    let task_ids = if needs.tasks {
        tasks_in_view(db, session_id, player_id, task_id).await
    } else {
        vec![task_id]
    };

    let dossier: SessionDossier =
        build_session_dossier(db, repo_dir, session_id, player_id, &task_ids, scope).await?;

    let tags_by_task = load_tags(db, &task_ids).await;
    let task_row = tasks::Entity::find_by_id(task_id)
        .one(db)
        .await?
        .ok_or(JudgeError::TaskNotFound)?;

    let tasks_brief: Vec<TaskBrief> = if !needs.tasks {
        // Not an empty list by accident: a judge that did not ask for the
        // cross-task view must not read one and conclude from its emptiness.
        Vec::new()
    } else {
        dossier
            .tasks
            .iter()
            .map(|t| TaskBrief {
                id: t.task_id,
                ordinal: t.ordinal,
                title: t.title.clone(),
                tags: tags_by_task.get(&t.task_id).cloned().unwrap_or_default(),
                earned: t.points_awarded.total,
                commit_sha: t.commit_sha.clone(),
                probes: t.probes.clone(),
                diff: t.diff.clone(),
                flags: t.flags.clone(),
            })
            .collect()
    };

    let focused = dossier.task(task_id);
    let points = focused
        .map(|t| t.points_awarded.clone())
        .unwrap_or_default();
    let diff = focused.and_then(|t| t.diff.clone());
    let task = TaskFacts {
        id: task_id,
        ordinal: task_row.ordinal,
        title: task_row.title.clone(),
        description: task_row.content.clone(),
        tags: tags_by_task.get(&task_id).cloned().unwrap_or_default(),
        earned: points.total,
        points,
        commit_sha: focused.and_then(|t| t.commit_sha.clone()),
        // No diff at all reads as empty: either the task made no commit, or
        // its commit changed nothing.
        diff_is_empty: diff
            .as_ref()
            .map(|d| d.insertions == 0 && d.deletions == 0)
            .unwrap_or(true),
        diff,
        probes: if needs.probes {
            load_probe_facts(db, session_id, player_id, task_id).await
        } else {
            Vec::new()
        },
        probe_summary: focused.map(|t| t.probes.clone()).unwrap_or_default(),
        flags: focused.map(|t| t.flags.clone()).unwrap_or_default(),
    };

    Ok(Evidence {
        session_id,
        player_id,
        task,
        tasks: tasks_brief,
        memory: if needs.memory {
            load_memory(db, session_id, player_id).await
        } else {
            BTreeMap::new()
        },
        repo: dossier.repo.clone(),
        judge: JudgeFacts {
            slug: judge_slug.to_string(),
            scale_min: scale.min,
            scale_max: scale.max,
            scale_step: scale.step,
            is_penalty: scale.max <= 0.0,
        },
        prior,
    })
}

/// Which of the player's tasks a judge may look across.
///
/// The scheduler's reached set is the primary answer, but it is not the only
/// evidence a task was reached: a recorded result or probe says so on its own,
/// and the scheduler row can be missing or have moved on by the time a
/// recovery sweep re-drives a judge. Taking the union keeps a ladder judge's
/// view of its neighbouring rungs intact in exactly the cases where a scheduler
/// lookup would have quietly emptied it.
///
/// The task under judgement is always included: it is being judged, so it was
/// reached.
async fn tasks_in_view(
    db: &DatabaseConnection,
    session_id: Uuid,
    player_id: Uuid,
    task_id: Uuid,
) -> Vec<Uuid> {
    let mut seen: std::collections::BTreeSet<Uuid> =
        crate::session_completion::reached_tasks_for_player(db, session_id, player_id)
            .await
            .unwrap_or_default()
            .into_iter()
            .collect();
    seen.insert(task_id);

    // Tasks this player banked something on.
    if let Ok(rows) = crate::entities::task_results::Entity::find()
        .filter(crate::entities::task_results::Column::SessionIdFk.eq(session_id))
        .filter(crate::entities::task_results::Column::PlayerIdFk.eq(player_id))
        .all(db)
        .await
    {
        seen.extend(rows.into_iter().filter_map(|r| r.task_id));
    }

    // Tasks this player was probed on, joined through the session's tests.
    if let Ok(rows) = probes::Entity::find()
        .filter(probes::Column::SessionId.eq(session_id))
        .filter(probes::Column::PlayerId.eq(player_id))
        .all(db)
        .await
        && let Ok(test_rows) = tests::Entity::find()
            .filter(tests::Column::SessionId.eq(session_id))
            .all(db)
            .await
    {
        let task_by_test: BTreeMap<Uuid, Uuid> =
            test_rows.into_iter().map(|t| (t.id, t.task_id)).collect();
        seen.extend(
            rows.into_iter()
                .filter_map(|p| task_by_test.get(&p.test_id).copied()),
        );
    }

    seen.into_iter().collect()
}

async fn load_tags(db: &DatabaseConnection, task_ids: &[Uuid]) -> BTreeMap<Uuid, Vec<String>> {
    let rows = tasks::Entity::find()
        .filter(tasks::Column::Id.is_in(task_ids.iter().copied()))
        .all(db)
        .await
        .unwrap_or_default();
    rows.into_iter()
        .map(|t| {
            let tags = serde_json::from_str::<Vec<String>>(&t.tags).unwrap_or_default();
            (t.id, tags)
        })
        .collect()
}

/// The task's probes, oldest first, with the answers they recorded.
///
/// `probes` references `tests`, not `tasks`, so the lookup joins through the
/// session's tests — the same hop the dossier's tally makes.
async fn load_probe_facts(
    db: &DatabaseConnection,
    session_id: Uuid,
    player_id: Uuid,
    task_id: Uuid,
) -> Vec<ProbeFacts> {
    let test_rows = tests::Entity::find()
        .filter(tests::Column::SessionId.eq(session_id))
        .filter(tests::Column::TaskId.eq(task_id))
        .all(db)
        .await
        .unwrap_or_default();
    if test_rows.is_empty() {
        return Vec::new();
    }
    let ordinal_by_test: BTreeMap<Uuid, i32> =
        test_rows.iter().map(|t| (t.id, t.ordinal)).collect();

    let rows = probes::Entity::find()
        .filter(probes::Column::SessionId.eq(session_id))
        .filter(probes::Column::PlayerId.eq(player_id))
        .filter(probes::Column::TestId.is_in(ordinal_by_test.keys().copied()))
        .order_by_asc(probes::Column::DispatchedAt)
        .all(db)
        .await
        .unwrap_or_default();

    rows.into_iter()
        .map(|p| ProbeFacts {
            test_ordinal: ordinal_by_test.get(&p.test_id).copied(),
            outcome: p.outcome.clone(),
            point_delta: p.point_delta.unwrap_or(0),
            answer: p.resolved_answer.as_deref().map(cap),
            expected: p.expected_answer.as_deref().map(cap),
            dispatched_at: Some(p.dispatched_at),
            resolved_at: p.resolved_at,
            result_json: p.result_json.clone(),
        })
        .collect()
}

fn cap(s: &str) -> String {
    crate::judging::truncate_chars(s.trim(), MAX_ANSWER_LEN)
}

/// The session's memory map, or empty when the project declares no schema.
async fn load_memory(
    db: &DatabaseConnection,
    session_id: Uuid,
    player_id: Uuid,
) -> BTreeMap<String, String> {
    let schema = match sessions::Entity::find_by_id(session_id).one(db).await {
        Ok(Some(s)) => match projects::Entity::find_by_id(s.project_id_fk).one(db).await {
            Ok(p) => p.and_then(|p| p.memory_schema),
            Err(_) => None,
        },
        _ => None,
    };
    crate::memory::load_memory_map(db, session_id, player_id, schema.as_deref()).await
}
