//! Score the tree of every probe commit with `ololo-health` and report.
//!
//! The probe loop hands each probe's commit to this task and moves on —
//! the analysis never delays the probe answer. One analysis runs at a
//! time; while it runs, only the newest waiting probe is kept, and every
//! probe it displaces is reported `skipped` rather than forgotten. The
//! result, or why there is none (failed, timed out, skipped), goes to
//! the server as a `HealthReport` frame through the same channel the
//! memory sync and the flag watcher use, together with where the push
//! of that commit stands.
//!
//! What is scored is the committed tree, exported to a temp dir — not the
//! live working tree, which the agent keeps editing — so the client and
//! the server (which exports the same commit) read the same bytes.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use arena_core::protocol::{
    HealthProbeConfig, HealthReportPayload, HealthReportStatus, PlayerAgentClientFrame, PushState,
    PushStatus,
};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};

use crate::snapshot::SnapshotRepo;

/// One probe's analysis to run.
#[derive(Debug, Clone)]
pub struct HealthJob {
    pub probe_id: uuid::Uuid,
    pub probe_seq: u32,
    pub task_id: Option<uuid::Uuid>,
    /// For the commit of the test run's log.
    pub task_title: String,
    pub session_id: Option<uuid::Uuid>,
    pub player_id: Option<uuid::Uuid>,
    /// The probe commit, or why there is none.
    pub commit: Result<gix::ObjectId, String>,
    pub config: HealthProbeConfig,
}

#[derive(Clone)]
pub struct HealthRunnerHandle {
    tx: UnboundedSender<HealthJob>,
}

impl HealthRunnerHandle {
    /// Queue a probe's analysis. Never blocks.
    pub fn submit(&self, job: HealthJob) {
        let _ = self.tx.send(job);
    }
}

/// Spawn the runner. Reports go out on `frame_tx`; once a probe's report
/// is out, its code goes on to `suite` (the project's tests), when given.
pub fn spawn(
    snapshot: Arc<Mutex<SnapshotRepo>>,
    frame_tx: UnboundedSender<PlayerAgentClientFrame>,
    suite: Option<crate::suite_run::SuiteRunnerHandle>,
) -> (HealthRunnerHandle, tokio::task::JoinHandle<()>) {
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<HealthJob>();
    let task = tokio::spawn(run_loop(snapshot, frame_tx, rx, suite));
    (HealthRunnerHandle { tx }, task)
}

async fn run_loop(
    snapshot: Arc<Mutex<SnapshotRepo>>,
    frame_tx: UnboundedSender<PlayerAgentClientFrame>,
    mut rx: UnboundedReceiver<HealthJob>,
    suite: Option<crate::suite_run::SuiteRunnerHandle>,
) {
    while let Some(first) = rx.recv().await {
        // Newest probe wins; every displaced one still gets a report.
        let mut job = first;
        while let Ok(newer) = rx.try_recv() {
            let skipped = std::mem::replace(&mut job, newer);
            let report = skipped_report(&snapshot, skipped);
            if frame_tx
                .send(PlayerAgentClientFrame::HealthReport(Box::new(report)))
                .is_err()
            {
                return;
            }
        }
        let snap = Arc::clone(&snapshot);
        // The suite tests the code of the probe whose report goes out — and
        // only after it went out, so the server holds the checkpoint the
        // test report will name.
        let suite_job = match (&job.commit, job.task_id) {
            (Ok(commit), Some(task_id)) => Some(crate::suite_run::SuiteJob {
                probe_id: job.probe_id,
                probe_seq: job.probe_seq,
                task_id,
                task_title: job.task_title.clone(),
                commit: *commit,
            }),
            _ => None,
        };
        let report = tokio::task::spawn_blocking(move || run_job(&snap, job))
            .await
            .unwrap_or_else(|e| panic_report(e.to_string()));
        if frame_tx
            .send(PlayerAgentClientFrame::HealthReport(Box::new(report)))
            .is_err()
        {
            // Socket writer gone; the session is over.
            return;
        }
        if let (Some(suite), Some(job)) = (suite.as_ref(), suite_job) {
            suite.submit(job);
        }
    }
}

/// Export the commit's tree, scan it, fold everything into the report.
fn run_job(snapshot: &Mutex<SnapshotRepo>, job: HealthJob) -> HealthReportPayload {
    let started = Instant::now();
    let mut report = base_report(&job);
    let commit = match job.commit {
        Ok(commit) => commit,
        Err(error) => {
            report.status = HealthReportStatus::Failed;
            report.error = Some(format!("snapshot commit failed: {error}"));
            report.duration_ms = elapsed_ms(started);
            return report;
        }
    };

    let cfg = ololo_health::HealthConfig {
        timeout: Duration::from_secs(u64::from(job.config.timeout_secs.max(1))),
        ..ololo_health::HealthConfig::default()
    };
    // `.git/` and `.ololo/` never enter the export; the scan ignores them
    // too, so a stray copy would not count — but not writing them is
    // cheaper than ignoring them.
    let skip: Vec<&str> = ololo_health::ALWAYS_IGNORED_DIRS
        .iter()
        .chain(ololo_health::PRUNED_DIRS.iter())
        .copied()
        .collect();

    let outcome = (|| -> Result<ololo_health::HealthResult, String> {
        let dir = tempfile::Builder::new()
            .prefix("ololo-health-")
            .tempdir()
            .map_err(|e| format!("temp dir: {e}"))?;
        {
            let guard = snapshot
                .lock()
                .map_err(|e| format!("snapshot lock poisoned: {e}"))?;
            guard
                .export_tree(commit, dir.path(), &skip)
                .map_err(|e| format!("exporting the commit tree: {e}"))?;
        }
        ololo_health::analyze(dir.path(), &cfg).map_err(|e| match e {
            ololo_health::HealthError::Timeout(d) => format!("timeout after {}s", d.as_secs()),
            other => other.to_string(),
        })
    })();

    match outcome {
        Ok(result) => {
            tracing::info!(
                "health: probe #{} scored {} ({:?})",
                job.probe_seq,
                result
                    .score
                    .map(|s| format!("{s:.1}"))
                    .unwrap_or_else(|| "n/a".into()),
                result.level
            );
            report.status = HealthReportStatus::Ok;
            report.result = Some(result);
        }
        Err(error) => {
            tracing::warn!("health: probe #{} analysis failed: {error}", job.probe_seq);
            report.status = if error.starts_with("timeout") {
                HealthReportStatus::Timeout
            } else {
                HealthReportStatus::Failed
            };
            report.error = Some(error);
        }
    }
    report.push = push_status(snapshot, Some(commit));
    report.duration_ms = elapsed_ms(started);
    report
}

fn skipped_report(snapshot: &Mutex<SnapshotRepo>, job: HealthJob) -> HealthReportPayload {
    let mut report = base_report(&job);
    report.status = HealthReportStatus::Skipped;
    report.error = Some("a newer probe arrived before this analysis started".into());
    report.push = push_status(snapshot, job.commit.ok());
    report
}

fn panic_report(error: String) -> HealthReportPayload {
    HealthReportPayload {
        probe_id: uuid::Uuid::nil(),
        probe_seq: 0,
        task_id: None,
        session_id: None,
        player_id: None,
        commit: None,
        status: HealthReportStatus::Failed,
        result: None,
        error: Some(format!("analysis task panicked: {error}")),
        duration_ms: 0,
        jscpd_version: ololo_health::JSCPD_CORE_VERSION.to_string(),
        health_schema: ololo_health::HEALTH_SCHEMA,
        push: PushStatus {
            state: PushState::Pending,
            error: None,
            pushed_commit: None,
        },
    }
}

fn base_report(job: &HealthJob) -> HealthReportPayload {
    HealthReportPayload {
        probe_id: job.probe_id,
        probe_seq: job.probe_seq,
        task_id: job.task_id,
        session_id: job.session_id,
        player_id: job.player_id,
        commit: job.commit.as_ref().ok().map(|id| id.to_string()),
        status: HealthReportStatus::Failed,
        result: None,
        error: None,
        duration_ms: 0,
        jscpd_version: ololo_health::JSCPD_CORE_VERSION.to_string(),
        health_schema: ololo_health::HEALTH_SCHEMA,
        push: PushStatus {
            state: PushState::Pending,
            error: None,
            pushed_commit: None,
        },
    }
}

fn push_status(snapshot: &Mutex<SnapshotRepo>, commit: Option<gix::ObjectId>) -> PushStatus {
    match (snapshot.lock(), commit) {
        (Ok(guard), Some(commit)) => guard.push_status_for(commit),
        _ => PushStatus {
            state: PushState::Pending,
            error: None,
            pushed_commit: None,
        },
    }
}

fn elapsed_ms(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}

/// Collect reports while a runner works, for tests.
#[cfg(test)]
pub(crate) async fn next_report(
    rx: &mut UnboundedReceiver<PlayerAgentClientFrame>,
) -> Option<HealthReportPayload> {
    match rx.recv().await? {
        PlayerAgentClientFrame::HealthReport(report) => Some(*report),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::test_util::{HOME_LOCK, HomeGuard};

    const CODE: &str = r#"export function summarize(items, options) {
  let total = 0;
  let count = 0;
  for (const item of items) {
    if (item.price > 0 && item.quantity > 0) {
      total += item.price * item.quantity;
      count += 1;
    } else if (options.strict) {
      throw new Error("invalid item: " + item.id);
    }
  }
  const average = count > 0 ? total / count : 0;
  return { total, count, average, currency: options.currency || "USD" };
}
"#;

    fn job(commit: Result<gix::ObjectId, String>, seq: u32) -> HealthJob {
        HealthJob {
            probe_id: uuid::Uuid::new_v4(),
            probe_seq: seq,
            task_id: Some(uuid::Uuid::new_v4()),
            task_title: "T".into(),
            session_id: None,
            player_id: None,
            commit,
            config: HealthProbeConfig { timeout_secs: 30 },
        }
    }

    /// A repo with one committed tree: code under src/, platform files
    /// under .ololo/ that the scan must not see.
    fn repo_with_commit(
        code: &str,
    ) -> (
        tempfile::TempDir,
        tempfile::TempDir,
        Arc<Mutex<SnapshotRepo>>,
        gix::ObjectId,
    ) {
        let _g = HOME_LOCK.lock().unwrap();
        let home = tempfile::tempdir().unwrap();
        let wt = tempfile::tempdir().unwrap();
        let _h = HomeGuard::set(home.path().to_str().unwrap());
        std::fs::create_dir_all(wt.path().join("src")).unwrap();
        std::fs::write(wt.path().join("src/a.js"), code).unwrap();
        std::fs::write(wt.path().join("src/b.js"), code).unwrap();
        std::fs::create_dir_all(wt.path().join(".ololo/artifacts")).unwrap();
        std::fs::write(wt.path().join(".ololo/artifacts/c.js"), code).unwrap();
        let repo = SnapshotRepo::new("default", "HEALTH1", wt.path(), None, None).unwrap();
        repo.commit_session_start().unwrap();
        let commit = repo
            .commit_probe(uuid::Uuid::new_v4(), "T", uuid::Uuid::new_v4(), 1)
            .unwrap();
        (home, wt, Arc::new(Mutex::new(repo)), commit)
    }

    #[tokio::test]
    async fn a_probe_commit_is_scored_from_its_exported_tree() {
        let (_home, wt, repo, commit) = repo_with_commit(CODE);
        // The working tree moves on; the score is of the commit.
        std::fs::remove_file(wt.path().join("src/b.js")).unwrap();
        let (frame_tx, mut frame_rx) = tokio::sync::mpsc::unbounded_channel();
        let (runner, _task) = spawn(Arc::clone(&repo), frame_tx, None);
        runner.submit(job(Ok(commit), 7));
        let report = next_report(&mut frame_rx).await.expect("a report");
        assert_eq!(report.status, HealthReportStatus::Ok, "{report:?}");
        assert_eq!(report.probe_seq, 7);
        assert_eq!(report.commit.as_deref(), Some(commit.to_string().as_str()));
        let result = report.result.expect("result");
        assert_eq!(
            result.metrics.files, 2,
            "src/a.js + src/b.js, never .ololo/"
        );
        assert!(result.metrics.clones >= 1, "b.js was still in the commit");
        assert_eq!(report.jscpd_version, ololo_health::JSCPD_CORE_VERSION);
        assert_eq!(
            report.push.state,
            PushState::Pending,
            "no remote configured"
        );
    }

    #[tokio::test]
    async fn a_missing_commit_is_reported_as_failed_never_skipped_silently() {
        let (_home, _wt, repo, _commit) = repo_with_commit(CODE);
        let (frame_tx, mut frame_rx) = tokio::sync::mpsc::unbounded_channel();
        let (runner, _task) = spawn(repo, frame_tx, None);
        runner.submit(job(Err("disk full".into()), 1));
        let report = next_report(&mut frame_rx).await.expect("a report");
        assert_eq!(report.status, HealthReportStatus::Failed);
        assert_eq!(report.commit, None);
        assert!(report.error.as_deref().unwrap().contains("disk full"));
    }

    #[tokio::test]
    async fn queued_probes_collapse_to_the_newest_and_the_rest_say_skipped() {
        let (_home, _wt, repo, commit) = repo_with_commit(CODE);
        let (frame_tx, mut frame_rx) = tokio::sync::mpsc::unbounded_channel();
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<HealthJob>();
        // Fill the queue before the runner exists, so the pickup sees all three.
        tx.send(job(Ok(commit), 1)).unwrap();
        tx.send(job(Ok(commit), 2)).unwrap();
        tx.send(job(Ok(commit), 3)).unwrap();
        let _task = tokio::spawn(run_loop(repo, frame_tx, rx, None));
        let first = next_report(&mut frame_rx).await.unwrap();
        let second = next_report(&mut frame_rx).await.unwrap();
        let third = next_report(&mut frame_rx).await.unwrap();
        assert_eq!(
            (first.probe_seq, first.status),
            (1, HealthReportStatus::Skipped)
        );
        assert_eq!(
            (second.probe_seq, second.status),
            (2, HealthReportStatus::Skipped)
        );
        assert_eq!((third.probe_seq, third.status), (3, HealthReportStatus::Ok));
    }

    #[tokio::test]
    async fn a_zero_budget_is_clamped_to_one_second() {
        let (_home, _wt, repo, commit) = repo_with_commit(CODE);
        let (frame_tx, mut frame_rx) = tokio::sync::mpsc::unbounded_channel();
        let (runner, _task) = spawn(repo, frame_tx, None);
        let mut j = job(Ok(commit), 1);
        j.config.timeout_secs = 0; // clamped to 1s — still ample for two files
        runner.submit(j);
        let report = next_report(&mut frame_rx).await.unwrap();
        assert_eq!(
            report.status,
            HealthReportStatus::Ok,
            "1s is enough: {report:?}"
        );
    }
}
