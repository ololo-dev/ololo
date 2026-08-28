//! Deterministic session evidence — the input a session-scoped judge reasons
//! over, assembled without an LLM.
//!
//! Three independent records of the same session already exist and nothing
//! correlates them:
//!
//! - `task_agent_stats` — the work window per task plus the client-reported
//!   agent activity inside it (tool calls, messages, tokens).
//! - `probes` — every attempt with its dispatch/resolve timestamps and outcome.
//! - git — one `feat(<task_id>): <title>` snapshot commit per task, over a
//!   root commit holding whatever the player started from.
//!
//! Correlated, those answer the question anti-cheat actually asks: did the code
//! for this task appear during the session, with agent activity to match? The
//! old per-task judge spent LLM turns re-deriving this from `get_commit_log` /
//! `list_files` / `read_file` / `get_diff` for every task of every player, and
//! could hallucinate the joins. Assembling it in one pass costs zero tokens and
//! leaves the model only the judgment call.
//!
//! Everything here is fact, not verdict. [`TaskEvidence::flags`] carries
//! objective observations ("no agent stats were reported") — never conclusions
//! ("the player cheated"). Note that `task_agent_stats` is client-reported
//! under the honest-trust model: absent or zeroed activity is weak evidence,
//! and the judge prompt must say so.

use std::collections::HashMap;
use std::path::Path;

use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, QueryOrder};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::entities::{probes, task_agent_stats, task_results, tasks, tests};

use super::tools::ToolScope;
use super::{JudgeError, resolve_task_commit, tools};

/// No task commit whose message carries this task's id.
pub const FLAG_NO_TASK_COMMIT: &str = "no_task_commit";
/// The task's work window changed nothing. The window runs from the previous
/// task's snapshot commit (or the session root) to this task's — the CLI
/// sweeps work into `flag(<task_id>)` and `artifact(<task_id>)` commits on
/// the way, so the `feat` commit alone is routinely empty for players who
/// delivered a done-flag or answered a capture request.
pub const FLAG_EMPTY_TASK_COMMIT: &str = "empty_task_commit";
/// The client never reported agent statistics for this task.
pub const FLAG_NO_AGENT_STATS: &str = "no_agent_stats";
/// Statistics were reported, but show no assistant turns and no tool calls.
pub const FLAG_ZERO_AGENT_ACTIVITY: &str = "zero_agent_activity";
/// Probes recorded a pass for this task, yet its whole work window changed
/// nothing.
pub const FLAG_PASSED_WITHOUT_CHANGES: &str = "passed_without_changes";

/// `git show --numstat` totals for one commit.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DiffStat {
    pub files_changed: usize,
    pub insertions: u64,
    pub deletions: u64,
    /// Paths touched, capped by [`MAX_FILES_LISTED`].
    pub files: Vec<String>,
}

impl DiffStat {
    fn is_empty(&self) -> bool {
        self.insertions == 0 && self.deletions == 0
    }
}

/// Lines the session actually wrote, summed over every in-session commit
/// EXCEPT the root (session-start) snapshot — wherever they were committed:
/// task commits and wip/memory snapshots alike. A ladder that passes probes
/// while `insertions_after_start` is zero ran on content that predates the
/// session; a player whose work lands in snapshots rather than task commits
/// still shows up here, so empty task-marker commits alone never trip it.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RepoActivity {
    pub commits_after_start: usize,
    pub insertions_after_start: u64,
    pub deletions_after_start: u64,
}

/// The repository state the player started from.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RootCommit {
    pub sha: String,
    /// Files present at the root commit, capped by [`MAX_FILES_LISTED`].
    pub files: Vec<RootFile>,
    pub total_files: usize,
    pub total_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RootFile {
    pub path: String,
    pub size_bytes: u64,
}

/// Client-reported agent activity inside a task's work window.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentActivity {
    pub user_messages: i64,
    pub assistant_messages: i64,
    pub tool_calls: i64,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub window_started_at: Option<chrono::DateTime<chrono::Utc>>,
    pub window_ended_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// What the player actually banked on one task, from `task_results`.
///
/// This is the ceiling on what an anti-cheat verdict may claw back: a task can
/// lose at most what it earned, so a penalty is always bounded by, and
/// proportional to, the reward it is reversing.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PointsAwarded {
    /// Points from probe outcomes (passes minus failures).
    pub from_probes: i32,
    /// The task-completion bonus, if it was granted.
    pub completion_bonus: i32,
    /// `from_probes + completion_bonus` — the clawback ceiling.
    pub total: i32,
}

/// Probe outcomes recorded for one task.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProbeSummary {
    pub attempts: usize,
    pub passed: usize,
    pub failed: usize,
    pub no_response: usize,
    pub first_dispatched_at: Option<chrono::DateTime<chrono::Utc>>,
    pub last_resolved_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// Everything known about one task of one player's session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskEvidence {
    pub task_id: Uuid,
    pub ordinal: i32,
    pub title: String,
    pub commit_sha: Option<String>,
    pub diff: Option<DiffStat>,
    pub agent: Option<AgentActivity>,
    pub probes: ProbeSummary,
    /// What this task banked — and therefore the most a verdict can remove.
    pub points_awarded: PointsAwarded,
    /// Objective observations, never conclusions. See the `FLAG_*` constants.
    pub flags: Vec<String>,
}

/// The correlated evidence for one (session, player).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionDossier {
    pub session_id: Uuid,
    pub player_id: Uuid,
    pub root_commit: Option<RootCommit>,
    /// What the whole in-session history wrote. See [`RepoActivity`].
    #[serde(default)]
    pub repo: RepoActivity,
    /// One entry per task, in ordinal order. Only the tasks the caller asked
    /// for — which must be the set the settle poll expects rows for.
    pub tasks: Vec<TaskEvidence>,
}

impl SessionDossier {
    /// JSON for the judge prompt.
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_else(|_| "{}".to_string())
    }

    /// The evidence for one task, if it was collected.
    pub fn task(&self, task_id: Uuid) -> Option<&TaskEvidence> {
        self.tasks.iter().find(|t| t.task_id == task_id)
    }
}

/// Cap on per-commit file lists and the root-commit listing, so one pathological
/// repo cannot blow up the judge prompt.
pub const MAX_FILES_LISTED: usize = 200;

/// Assemble the dossier for `player_id` over `task_ids`.
///
/// `task_ids` must come from `session_completion::reached_task_ids` — the same
/// set the settle poll waits for rows on. Tasks missing from the project are
/// skipped; a task the player never touched still gets an entry, flagged, so
/// the caller can score every expected pair.
pub async fn build_session_dossier(
    db: &DatabaseConnection,
    repo_dir: &Path,
    session_id: Uuid,
    player_id: Uuid,
    task_ids: &[Uuid],
    scope: &ToolScope,
) -> Result<SessionDossier, JudgeError> {
    let task_rows: Vec<tasks::Model> = tasks::Entity::find()
        .filter(tasks::Column::Id.is_in(task_ids.iter().copied()))
        .order_by_asc(tasks::Column::Ordinal)
        .all(db)
        .await?;

    let stats: HashMap<Uuid, task_agent_stats::Model> = task_agent_stats::Entity::find()
        .filter(task_agent_stats::Column::SessionIdFk.eq(session_id))
        .filter(task_agent_stats::Column::PlayerIdFk.eq(player_id))
        .all(db)
        .await?
        .into_iter()
        .filter_map(|row| row.task_id_fk.map(|task_id| (task_id, row)))
        .collect();

    let probe_summaries = summarize_probes(db, session_id, player_id).await?;
    let points = summarize_points(db, session_id, player_id).await?;
    let root_commit = read_root_commit(repo_dir, scope).await;
    let repo = read_repo_activity(repo_dir, root_commit.as_ref().map(|r| r.sha.as_str())).await;

    let mut evidence = Vec::with_capacity(task_rows.len());
    // Each task's diff covers its whole WORK WINDOW: from the previous task's
    // snapshot commit (the session root for the first) up to its own. The
    // `feat(<task_id>)` commit alone is the wrong thing to diff — the CLI has
    // already committed the work under `flag(...)` and `artifact(...): sync`
    // by the time it lands, so honest players showed empty task commits and
    // were flagged for it (prod, every Money Tracker part of 2026-08-23).
    let mut window_base = root_commit.as_ref().map(|r| r.sha.clone());
    for task in task_rows {
        let commit = resolve_task_commit(repo_dir, task.id).await.ok().flatten();
        let commit_sha = commit.map(|(sha, _subject)| sha);
        let diff = match &commit_sha {
            Some(sha) => match &window_base {
                Some(base) if base != sha => read_range_diff_stat(repo_dir, base, sha).await,
                // No base to diff from (unreadable root), or the task resolves
                // to the root itself: the single-commit stat is all there is.
                _ => read_diff_stat(repo_dir, sha).await,
            },
            None => None,
        };
        // The next task's window starts where this one ended. A task with no
        // commit moves no boundary — its work, if any, stays in the window of
        // the task that eventually commits.
        if let Some(sha) = &commit_sha {
            window_base = Some(sha.clone());
        }
        let agent = stats.get(&task.id).map(|row| AgentActivity {
            user_messages: row.user_messages,
            assistant_messages: row.assistant_messages,
            tool_calls: row.tool_calls,
            input_tokens: row.input_tokens,
            output_tokens: row.output_tokens,
            window_started_at: row.window_started_at,
            window_ended_at: row.window_ended_at,
        });
        let probes = probe_summaries.get(&task.id).cloned().unwrap_or_default();
        let points_awarded = points.get(&task.id).cloned().unwrap_or_default();

        let mut flags = Vec::new();
        if commit_sha.is_none() {
            flags.push(FLAG_NO_TASK_COMMIT.to_string());
        }
        if diff.as_ref().is_some_and(DiffStat::is_empty) {
            flags.push(FLAG_EMPTY_TASK_COMMIT.to_string());
            if probes.passed > 0 {
                flags.push(FLAG_PASSED_WITHOUT_CHANGES.to_string());
            }
        }
        match &agent {
            None => flags.push(FLAG_NO_AGENT_STATS.to_string()),
            Some(a) if a.assistant_messages == 0 && a.tool_calls == 0 => {
                flags.push(FLAG_ZERO_AGENT_ACTIVITY.to_string())
            }
            Some(_) => {}
        }

        evidence.push(TaskEvidence {
            task_id: task.id,
            ordinal: task.ordinal,
            title: task.title,
            commit_sha,
            diff,
            agent,
            probes,
            points_awarded,
            flags,
        });
    }

    Ok(SessionDossier {
        session_id,
        player_id,
        root_commit,
        repo,
        tasks: evidence,
    })
}

/// Sum `git log --all --numstat` over every commit except the root
/// (session-start) snapshot. Defaults to zeros when the repo is unreadable —
/// the same as a repo with no commits, which is what an unreadable repo
/// proves about in-session work anyway.
async fn read_repo_activity(repo_dir: &Path, root_sha: Option<&str>) -> RepoActivity {
    let repo_dir = repo_dir.to_path_buf();
    let root = root_sha.map(str::to_string);
    tokio::task::spawn_blocking(move || {
        let git = which::which("git").ok()?;
        let out = std::process::Command::new(&git)
            .arg("-C")
            .arg(&repo_dir)
            .arg("log")
            .arg("--all")
            .arg("--numstat")
            .arg("--no-color")
            .arg("--format=%H")
            .output()
            .ok()?;
        if !out.status.success() {
            return None;
        }
        let text = String::from_utf8_lossy(&out.stdout);
        let mut activity = RepoActivity::default();
        let mut in_root = false;
        for line in text.lines() {
            let line = line.trim_end_matches('\r');
            if line.is_empty() {
                continue;
            }
            // A 40-hex line is a commit header; anything else is a numstat
            // row ("<insertions>\t<deletions>\t<path>"; binary files "-").
            if line.len() == 40 && line.bytes().all(|b| b.is_ascii_hexdigit()) {
                in_root = root.as_deref() == Some(line);
                if !in_root {
                    activity.commits_after_start += 1;
                }
                continue;
            }
            if in_root {
                continue;
            }
            let mut parts = line.split('\t');
            let ins = parts.next().unwrap_or("");
            let del = parts.next().unwrap_or("");
            if parts.next().unwrap_or("").is_empty() {
                continue;
            }
            activity.insertions_after_start += ins.parse::<u64>().unwrap_or(0);
            activity.deletions_after_start += del.parse::<u64>().unwrap_or(0);
        }
        Some(activity)
    })
    .await
    .ok()
    .flatten()
    .unwrap_or_default()
}

/// Per-task point tallies for one player, keyed by task id.
///
/// `judge_results` rows are deliberately excluded: those are judge verdicts,
/// not task earnings, and letting one judge claw back another's penalty would
/// compound them.
async fn summarize_points(
    db: &DatabaseConnection,
    session_id: Uuid,
    player_id: Uuid,
) -> Result<HashMap<Uuid, PointsAwarded>, JudgeError> {
    let rows = task_results::Entity::find()
        .filter(task_results::Column::SessionIdFk.eq(session_id))
        .filter(task_results::Column::PlayerIdFk.eq(player_id))
        .all(db)
        .await?;

    let mut out: HashMap<Uuid, PointsAwarded> = HashMap::new();
    for r in rows {
        let Some(task_id) = r.task_id else { continue };
        let entry = out.entry(task_id).or_default();
        if r.is_bonus {
            entry.completion_bonus += r.point_delta;
        } else {
            entry.from_probes += r.point_delta;
        }
        entry.total += r.point_delta;
    }
    Ok(out)
}

/// Per-task probe outcome tallies for one player, keyed by task id.
///
/// `probes` references `tests`, not `tasks`, so the tally joins through the
/// session's `tests` rows.
async fn summarize_probes(
    db: &DatabaseConnection,
    session_id: Uuid,
    player_id: Uuid,
) -> Result<HashMap<Uuid, ProbeSummary>, JudgeError> {
    let test_to_task: HashMap<Uuid, Uuid> = tests::Entity::find()
        .filter(tests::Column::SessionId.eq(session_id))
        .all(db)
        .await?
        .into_iter()
        .map(|t| (t.id, t.task_id))
        .collect();

    let rows = probes::Entity::find()
        .filter(probes::Column::SessionId.eq(session_id))
        .filter(probes::Column::PlayerId.eq(player_id))
        .all(db)
        .await?;

    let mut out: HashMap<Uuid, ProbeSummary> = HashMap::new();
    for p in rows {
        let Some(task_id) = test_to_task.get(&p.test_id) else {
            continue;
        };
        let entry = out.entry(*task_id).or_default();
        entry.attempts += 1;
        match p.outcome.as_deref() {
            Some("pass") => entry.passed += 1,
            Some("no_response") => entry.no_response += 1,
            Some(_) => entry.failed += 1,
            None => {}
        }
        entry.first_dispatched_at = Some(match entry.first_dispatched_at {
            Some(cur) if cur <= p.dispatched_at => cur,
            _ => p.dispatched_at,
        });
        if let Some(resolved) = p.resolved_at {
            entry.last_resolved_at = Some(match entry.last_resolved_at {
                Some(cur) if cur >= resolved => cur,
                _ => resolved,
            });
        }
    }
    Ok(out)
}

/// The root commit and the files it carried — what the player started from.
async fn read_root_commit(repo_dir: &Path, scope: &ToolScope) -> Option<RootCommit> {
    let sha = root_commit_sha(repo_dir).await?;
    let entries = tools::list_files(repo_dir, Some(&sha), None, scope)
        .await
        .ok()?;
    let total_files = entries.len();
    let total_bytes = entries.iter().map(|e| e.size_bytes).sum();
    let files = entries
        .into_iter()
        .take(MAX_FILES_LISTED)
        .map(|e| RootFile {
            path: e.path,
            size_bytes: e.size_bytes,
        })
        .collect();
    Some(RootCommit {
        sha,
        files,
        total_files,
        total_bytes,
    })
}

/// Parse `--numstat` lines: "<insertions>\t<deletions>\t<path>"; binary
/// files report "-".
fn parse_numstat(text: &str) -> DiffStat {
    let mut stat = DiffStat::default();
    for line in text.lines() {
        let line = line.trim_end_matches('\r');
        if line.is_empty() {
            continue;
        }
        let mut parts = line.split('\t');
        let ins = parts.next().unwrap_or("");
        let del = parts.next().unwrap_or("");
        let path = parts.next().unwrap_or("");
        if path.is_empty() {
            continue;
        }
        stat.files_changed += 1;
        stat.insertions += ins.parse::<u64>().unwrap_or(0);
        stat.deletions += del.parse::<u64>().unwrap_or(0);
        if stat.files.len() < MAX_FILES_LISTED {
            stat.files.push(path.to_string());
        }
    }
    stat
}

/// Sha of the session's root (start) snapshot — the tree the player began
/// from. Public so the vision loader can tell in-session images from
/// imported ones.
pub async fn root_commit_sha(repo_dir: &Path) -> Option<String> {
    let repo_dir = repo_dir.to_path_buf();
    tokio::task::spawn_blocking(move || {
        let git = which::which("git").ok()?;
        let out = std::process::Command::new(&git)
            .arg("-C")
            .arg(&repo_dir)
            .arg("rev-list")
            .arg("--max-parents=0")
            .arg("HEAD")
            .output()
            .ok()?;
        if !out.status.success() {
            return None;
        }
        let text = String::from_utf8_lossy(&out.stdout);
        let sha = text.lines().next()?.trim().to_string();
        (!sha.is_empty()).then_some(sha)
    })
    .await
    .ok()
    .flatten()
}

/// `git diff --numstat <base> <head>` totals — everything the work window
/// wrote, wherever the CLI committed it on the way to the task's snapshot.
async fn read_range_diff_stat(repo_dir: &Path, base: &str, head: &str) -> Option<DiffStat> {
    let repo_dir = repo_dir.to_path_buf();
    let base = base.to_string();
    let head = head.to_string();
    tokio::task::spawn_blocking(move || {
        let git = which::which("git").ok()?;
        let out = std::process::Command::new(&git)
            .arg("-C")
            .arg(&repo_dir)
            .arg("diff")
            .arg("--numstat")
            .arg("--no-color")
            .arg(&base)
            .arg(&head)
            .output()
            .ok()?;
        if !out.status.success() {
            return None;
        }
        Some(parse_numstat(&String::from_utf8_lossy(&out.stdout)))
    })
    .await
    .ok()
    .flatten()
}

/// `git show --numstat --format= <sha>` totals.
///
/// Returns `Some(DiffStat::default())` for a commit that changed nothing — the
/// caller distinguishes "no commit" (`None`) from "empty commit", which are
/// very different pieces of evidence.
async fn read_diff_stat(repo_dir: &Path, sha: &str) -> Option<DiffStat> {
    let repo_dir = repo_dir.to_path_buf();
    let sha = sha.to_string();
    tokio::task::spawn_blocking(move || {
        let git = which::which("git").ok()?;
        let out = std::process::Command::new(&git)
            .arg("-C")
            .arg(&repo_dir)
            .arg("show")
            .arg("--numstat")
            .arg("--format=")
            .arg("--no-color")
            .arg(&sha)
            .output()
            .ok()?;
        if !out.status.success() {
            return None;
        }
        Some(parse_numstat(&String::from_utf8_lossy(&out.stdout)))
    })
    .await
    .ok()
    .flatten()
}
