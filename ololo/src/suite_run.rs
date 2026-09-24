//! Run the project's tests after each probe's health analysis.
//!
//! The server reads which commands run the project's tests and their
//! coverage from the player's `AGENTS.md` / `README.md` and hands them
//! over (`HealthTests`). Once a probe's health report is out, the health
//! runner passes the probe here; when the code changed since the last run
//! and a minute has passed since that run started, the command runs in the
//! working folder — where the suite's dependencies live — with `CI=1`, so
//! runners with a watch mode stay one-shot.
//!
//! The run's whole output lands in `.ololo/probes/<seq>-tests.log`,
//! committed to the snapshot history on a `tests(<task>)` commit of its
//! own before anything is reported; the server gets only the numbers
//! parsed from it (`TestReport`), keeps them and composes them into the
//! health score — it never runs or re-checks the suite. The log is the
//! evidence, for the judges, the agent and anyone reading the history.
//!
//! One run at a time; while one runs only the newest probe waits. The
//! command obeys the probe permission rules in `.ololo/settings.json`: a
//! command no rule allows does not run, and the report says so — there is
//! no prompt for it, a background run must never pop a dialog over the
//! agent's work.

use std::collections::VecDeque;
use std::path::Path;
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use arena_core::protocol::{
    HealthTestsConfig, PlayerAgentClientFrame, SuiteResult, TestReportPayload, TestRunStatus,
};
use ololo_health::suite::{OutputParser, failing_pct, fresh_report};
use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};

use crate::permissions;
use crate::snapshot::SnapshotRepo;
use crate::tui::event::{EventSink, LogLevel, Origin, TuiEvent};

/// Fewest seconds between the starts of two runs: an agent that changes a
/// file every few seconds must not keep the suite running back to back.
pub const MIN_INTERVAL: Duration = Duration::from_secs(60);
/// Where the logs go, relative to the working folder.
pub const LOG_DIR: &str = ".ololo/probes";
/// The log keeps the first and the last bytes of a long output — runners
/// print their summary at the end.
const LOG_HEAD_BYTES: usize = 32 * 1024;
const LOG_TAIL_BYTES: usize = 224 * 1024;

/// One probe whose code the suite may test.
#[derive(Debug, Clone)]
pub struct SuiteJob {
    pub probe_id: uuid::Uuid,
    pub probe_seq: u32,
    pub task_id: uuid::Uuid,
    pub task_title: String,
    /// The probe commit: what the run is reported against.
    pub commit: gix::ObjectId,
}

enum Msg {
    Commands(HealthTestsConfig),
    Job(SuiteJob),
}

#[derive(Clone)]
pub struct SuiteRunnerHandle {
    tx: UnboundedSender<Msg>,
}

impl SuiteRunnerHandle {
    /// The commands the server read from the docs (both absent: none).
    pub fn set_commands(&self, cfg: HealthTestsConfig) {
        let _ = self.tx.send(Msg::Commands(cfg));
    }

    /// A probe's health report went out: test its code. Never blocks.
    pub fn submit(&self, job: SuiteJob) {
        let _ = self.tx.send(Msg::Job(job));
    }
}

/// Spawn the runner. Reports go out on `frame_tx`; `sink` carries the
/// narration to the TUI.
pub fn spawn(
    snapshot: Arc<Mutex<SnapshotRepo>>,
    frame_tx: UnboundedSender<PlayerAgentClientFrame>,
    sink: Option<Arc<dyn EventSink>>,
) -> (SuiteRunnerHandle, tokio::task::JoinHandle<()>) {
    spawn_with(snapshot, frame_tx, sink, MIN_INTERVAL)
}

fn spawn_with(
    snapshot: Arc<Mutex<SnapshotRepo>>,
    frame_tx: UnboundedSender<PlayerAgentClientFrame>,
    sink: Option<Arc<dyn EventSink>>,
    min_interval: Duration,
) -> (SuiteRunnerHandle, tokio::task::JoinHandle<()>) {
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    let task = tokio::spawn(run_loop(snapshot, frame_tx, sink, rx, min_interval));
    (SuiteRunnerHandle { tx }, task)
}

#[derive(Default)]
struct Runner {
    commands: Option<HealthTestsConfig>,
    /// The tree and command of the last run: the same pair is not run twice.
    last_tree: Option<gix::ObjectId>,
    last_command: Option<String>,
    last_start: Option<Instant>,
    /// The command already reported as not allowed.
    declined: Option<String>,
}

impl Runner {
    fn absorb(
        &mut self,
        msg: Msg,
        pending: &mut Option<SuiteJob>,
        sink: &Option<Arc<dyn EventSink>>,
    ) {
        match msg {
            // Newest probe wins: the suite tests the code as it is now.
            Msg::Job(job) => *pending = Some(job),
            Msg::Commands(cfg) => {
                if self.commands.as_ref() == Some(&cfg) {
                    return;
                }
                narrate_commands(&cfg, sink);
                self.commands = Some(cfg);
                // A new command runs even on unchanged code.
                self.last_tree = None;
                self.declined = None;
            }
        }
    }
}

async fn run_loop(
    snapshot: Arc<Mutex<SnapshotRepo>>,
    frame_tx: UnboundedSender<PlayerAgentClientFrame>,
    sink: Option<Arc<dyn EventSink>>,
    mut rx: UnboundedReceiver<Msg>,
    min_interval: Duration,
) {
    let mut runner = Runner::default();
    let mut pending: Option<SuiteJob> = None;
    loop {
        if pending.is_none() {
            match rx.recv().await {
                Some(msg) => runner.absorb(msg, &mut pending, &sink),
                None => return,
            }
        }
        while let Ok(msg) = rx.try_recv() {
            runner.absorb(msg, &mut pending, &sink);
        }
        // Keep the spacing between run starts, still taking newer probes.
        if let Some(last) = runner.last_start {
            let ready_at = tokio::time::Instant::from_std(last + min_interval);
            loop {
                tokio::select! {
                    msg = rx.recv() => match msg {
                        Some(msg) => runner.absorb(msg, &mut pending, &sink),
                        None => return,
                    },
                    _ = tokio::time::sleep_until(ready_at) => break,
                }
            }
        }
        let Some(job) = pending.take() else {
            continue;
        };
        let Some(cfg) = runner.commands.clone() else {
            continue;
        };
        let Some((command, coverage_run)) = cfg.command().map(|(c, k)| (c.to_string(), k)) else {
            continue;
        };
        let workdir = match snapshot.lock() {
            Ok(guard) => guard.worktree().to_path_buf(),
            Err(_) => continue,
        };

        if permissions::check_in(&workdir, &command) != permissions::Verdict::Allowed {
            if runner.declined.as_deref() != Some(command.as_str()) {
                runner.declined = Some(command.clone());
                let message = format!(
                    "Health will not run your tests: `{command}` is not allowed. Approve probe \
                     commands for the session, or add it to {} to score tests and coverage.",
                    workdir.join(".ololo/settings.json").display()
                );
                narrate(&sink, LogLevel::Hint, &message);
                let report = TestReportPayload {
                    probe_id: job.probe_id,
                    probe_seq: job.probe_seq,
                    task_id: Some(job.task_id),
                    commit: job.commit.to_string(),
                    status: TestRunStatus::Declined,
                    command,
                    coverage_run,
                    result: None,
                    error: Some("not allowed by the probe permission rules".into()),
                    duration_ms: 0,
                    log: None,
                };
                if frame_tx
                    .send(PlayerAgentClientFrame::TestReport(Box::new(report)))
                    .is_err()
                {
                    return;
                }
            }
            continue;
        }

        let tree = snapshot
            .lock()
            .ok()
            .and_then(|g| g.code_tree_of(job.commit));
        if tree.is_some()
            && tree == runner.last_tree
            && runner.last_command.as_deref() == Some(command.as_str())
        {
            // The code is what the last run tested: its numbers still hold.
            continue;
        }
        runner.last_start = Some(Instant::now());
        let timeout = Duration::from_secs(u64::from(cfg.timeout_secs.max(1)));
        let run = run_suite(&command, &workdir, timeout).await;
        let report = finish(&snapshot, &workdir, &job, &cfg, &command, coverage_run, run).await;
        narrate(
            &sink,
            if report.status == TestRunStatus::Ok {
                LogLevel::Step
            } else {
                LogLevel::Warn
            },
            &format!(
                "Tests after check #{}: {}{}",
                job.probe_seq,
                summary_of(&report),
                report
                    .log
                    .as_deref()
                    .map(|l| format!(" · log {l}"))
                    .unwrap_or_default()
            ),
        );
        runner.last_tree = tree;
        runner.last_command = Some(command);
        if frame_tx
            .send(PlayerAgentClientFrame::TestReport(Box::new(report)))
            .is_err()
        {
            return; // socket writer gone; the session is over
        }
    }
}

/// How a run went, before it is written down.
struct Run {
    status: TestRunStatus,
    result: Option<SuiteResult>,
    error: Option<String>,
    started: SystemTime,
    duration_ms: u64,
    output: Captured,
}

/// Write the log, commit it, build the report.
async fn finish(
    snapshot: &Arc<Mutex<SnapshotRepo>>,
    workdir: &Path,
    job: &SuiteJob,
    cfg: &HealthTestsConfig,
    command: &str,
    coverage_run: bool,
    run: Run,
) -> TestReportPayload {
    let mut report = TestReportPayload {
        probe_id: job.probe_id,
        probe_seq: job.probe_seq,
        task_id: Some(job.task_id),
        commit: job.commit.to_string(),
        status: run.status,
        command: command.to_string(),
        coverage_run,
        result: run.result.clone(),
        error: run.error.clone(),
        duration_ms: run.duration_ms,
        log: None,
    };
    let rel = log_name(job.probe_seq);
    let text = log_text(job, cfg, command, coverage_run, &run, &report);
    if let Err(e) = write_log(workdir, &rel, &text) {
        tracing::warn!("tests: writing {rel} failed: {e}");
        return report;
    }
    let summary = summary_of(&report);
    let snap = Arc::clone(snapshot);
    let (task_id, title, probe_id, seq, rel_c) = (
        job.task_id,
        job.task_title.clone(),
        job.probe_id,
        job.probe_seq,
        rel.clone(),
    );
    let committed = tokio::task::spawn_blocking(move || {
        let guard = snap.lock().map_err(|e| e.to_string())?;
        let id = guard
            .commit_probe_log(task_id, &title, probe_id, seq, &rel_c, &summary)
            .map_err(|e| e.to_string())?;
        guard.request_push();
        Ok::<_, String>(id)
    })
    .await
    .unwrap_or_else(|e| Err(e.to_string()));
    match committed {
        Ok(_) => report.log = Some(rel),
        Err(e) => tracing::warn!("tests: committing {rel} failed: {e}"),
    }
    report
}

fn log_name(seq: u32) -> String {
    if seq > 0 {
        format!("{LOG_DIR}/{seq:04}-tests.log")
    } else {
        let secs = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        format!("{LOG_DIR}/{secs}-tests.log")
    }
}

fn write_log(workdir: &Path, rel: &str, text: &str) -> std::io::Result<()> {
    let path = workdir.join(rel);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, text)
}

fn log_text(
    job: &SuiteJob,
    cfg: &HealthTestsConfig,
    command: &str,
    coverage_run: bool,
    run: &Run,
    report: &TestReportPayload,
) -> String {
    let started: chrono::DateTime<chrono::Utc> = run.started.into();
    let mut text = format!(
        "# ololo: the project's tests after check #{seq} ({title})\n\
         # command: {command}\n\
         # what:    {what}, read from {sources}\n\
         # started: {started}\n\
         # ----------------------------------------------------------------\n",
        seq = job.probe_seq,
        title = job.task_title,
        what = if coverage_run {
            "tests with coverage"
        } else {
            "tests"
        },
        sources = if cfg.sources.is_empty() {
            "the project's docs".to_string()
        } else {
            cfg.sources.join(", ")
        },
        started = started.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
    );
    text.push_str(&run.output.render());
    if !text.ends_with('\n') {
        text.push('\n');
    }
    text.push_str("# ----------------------------------------------------------------\n");
    let exit = run
        .result
        .as_ref()
        .and_then(|r| r.exit_code)
        .map(|c| format!("exit {c}"))
        .unwrap_or_else(|| "no exit code".into());
    text.push_str(&format!(
        "# ended:   {} · {exit} · {:.1}s\n",
        report.status.as_str(),
        run.duration_ms as f64 / 1000.0
    ));
    text.push_str(&format!("# result:  {}\n", summary_of(report)));
    if let Some(e) = &report.error {
        text.push_str(&format!("# error:   {e}\n"));
    }
    text
}

/// One line for the player, the commit subject and the log's footer.
pub fn summary_of(report: &TestReportPayload) -> String {
    match report.status {
        TestRunStatus::Timeout => return "timed out".into(),
        TestRunStatus::Failed => return "could not start".into(),
        TestRunStatus::Declined => return "not allowed".into(),
        TestRunStatus::Ok => {}
    }
    let Some(result) = &report.result else {
        return "no result".into();
    };
    let tests = match result.counts {
        Some(c) if c.passed + c.failed > 0 => {
            let mut s = format!("{} passed", c.passed);
            if c.failed > 0 {
                s.push_str(&format!(", {} failed", c.failed));
            }
            if c.skipped > 0 {
                s.push_str(&format!(", {} skipped", c.skipped));
            }
            s
        }
        Some(_) => "no tests ran".into(),
        None => match failing_pct(result) {
            Some(0.0) => format!("passed (exit {})", result.exit_code.unwrap_or(0)),
            _ => match result.exit_code {
                Some(code) => format!("failed (exit {code})"),
                None => "killed".into(),
            },
        },
    };
    match result.coverage_pct {
        Some(pct) => format!("{tests} · coverage {pct:.1}%"),
        None => tests,
    }
}

/// Run `command` in `workdir` under `timeout`, reading stdout and stderr
/// line by line as they come: every line feeds the parser and the log.
async fn run_suite(command: &str, workdir: &Path, timeout: Duration) -> Run {
    let started = SystemTime::now();
    let clock = Instant::now();
    let mut cmd = shell(command);
    cmd.current_dir(workdir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        // One-shot, plain output: CI turns watch modes off (Jest, Vitest)
        // and colour codes would only have to be stripped again.
        .env("CI", "1")
        .env("NO_COLOR", "1")
        .env("FORCE_COLOR", "0")
        .kill_on_drop(true);
    #[cfg(unix)]
    cmd.process_group(0);
    let mut child = match cmd.spawn() {
        Ok(child) => child,
        Err(e) => {
            return Run {
                status: TestRunStatus::Failed,
                result: None,
                error: Some(format!("could not start the command: {e}")),
                started,
                duration_ms: 0,
                output: Captured::default(),
            };
        }
    };
    let pid = child.id();
    let (line_tx, mut line_rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    if let Some(out) = child.stdout.take() {
        tokio::spawn(forward_lines(out, line_tx.clone()));
    }
    if let Some(err) = child.stderr.take() {
        tokio::spawn(forward_lines(err, line_tx.clone()));
    }
    drop(line_tx);

    let mut parser = OutputParser::new();
    let mut output = Captured::default();
    let collect = async {
        while let Some(line) = line_rx.recv().await {
            parser.feed_line(&line);
            output.push(&line);
        }
    };
    let finished = tokio::time::timeout(timeout, async {
        let (status, ()) = tokio::join!(child.wait(), collect);
        status
    })
    .await;
    let duration_ms = u64::try_from(clock.elapsed().as_millis()).unwrap_or(u64::MAX);
    match finished {
        Ok(Ok(status)) => {
            let parsed = parser.finish();
            let (coverage_pct, coverage_source) = match fresh_report(workdir, started) {
                Some((path, pct)) => (Some(pct), Some(path)),
                None => (
                    parsed.coverage_pct,
                    parsed.coverage_pct.map(|_| "output".to_string()),
                ),
            };
            Run {
                status: TestRunStatus::Ok,
                result: Some(SuiteResult {
                    exit_code: status.code(),
                    counts: parsed.counts,
                    coverage_pct,
                    coverage_source,
                    summary: parsed.summary,
                }),
                error: None,
                started,
                duration_ms,
                output,
            }
        }
        Ok(Err(e)) => Run {
            status: TestRunStatus::Failed,
            result: None,
            error: Some(format!("waiting for the command: {e}")),
            started,
            duration_ms,
            output,
        },
        Err(_) => {
            kill_tree(&mut child, pid).await;
            Run {
                status: TestRunStatus::Timeout,
                result: None,
                error: Some(format!("timed out after {}s", timeout.as_secs())),
                started,
                duration_ms,
                output,
            }
        }
    }
}

#[cfg(unix)]
fn shell(command: &str) -> tokio::process::Command {
    let mut cmd = tokio::process::Command::new("sh");
    cmd.args(["-c", command]);
    cmd
}

#[cfg(windows)]
fn shell(command: &str) -> tokio::process::Command {
    let mut cmd = tokio::process::Command::new("cmd.exe");
    cmd.args(["/C", command]);
    cmd
}

/// A test suite spawns workers, servers, browsers: end the whole group,
/// not just the shell.
async fn kill_tree(child: &mut tokio::process::Child, pid: Option<u32>) {
    #[cfg(unix)]
    if let Some(pid) = pid.and_then(|p| i32::try_from(p).ok()) {
        let _ = nix::sys::signal::killpg(
            nix::unistd::Pid::from_raw(pid),
            nix::sys::signal::Signal::SIGKILL,
        );
    }
    #[cfg(windows)]
    if let Some(pid) = pid {
        let _ = std::process::Command::new("taskkill")
            .args(["/T", "/F", "/PID", &pid.to_string()])
            .output();
    }
    let _ = child.kill().await;
    let _ = child.wait().await;
}

/// Lines of `stream`, as lossy UTF-8, until it closes.
async fn forward_lines(stream: impl AsyncRead + Unpin, tx: UnboundedSender<String>) {
    let mut reader = BufReader::new(stream);
    let mut buf = Vec::new();
    loop {
        buf.clear();
        match reader.read_until(b'\n', &mut buf).await {
            Ok(0) | Err(_) => return,
            Ok(_) => {
                let line = String::from_utf8_lossy(&buf);
                let line = line.trim_end_matches(['\n', '\r']);
                if tx.send(line.to_string()).is_err() {
                    return;
                }
            }
        }
    }
}

/// The output as the log keeps it: the first [`LOG_HEAD_BYTES`] and the
/// last [`LOG_TAIL_BYTES`], with what fell between counted.
#[derive(Default)]
struct Captured {
    head: String,
    tail: VecDeque<String>,
    tail_bytes: usize,
    elided_lines: u64,
}

impl Captured {
    fn push(&mut self, line: &str) {
        if self.tail.is_empty() && self.head.len() + line.len() < LOG_HEAD_BYTES {
            self.head.push_str(line);
            self.head.push('\n');
            return;
        }
        self.tail_bytes += line.len() + 1;
        self.tail.push_back(line.to_string());
        while self.tail_bytes > LOG_TAIL_BYTES
            && let Some(old) = self.tail.pop_front()
        {
            self.tail_bytes -= old.len() + 1;
            self.elided_lines += 1;
        }
    }

    fn render(&self) -> String {
        let mut text = self.head.clone();
        if self.elided_lines > 0 {
            text.push_str(&format!(
                "[… {} lines left out of the log …]\n",
                self.elided_lines
            ));
        }
        for line in &self.tail {
            text.push_str(line);
            text.push('\n');
        }
        text
    }
}

fn narrate_commands(cfg: &HealthTestsConfig, sink: &Option<Arc<dyn EventSink>>) {
    let from = if cfg.sources.is_empty() {
        "the project's docs".to_string()
    } else {
        cfg.sources.join(" and ")
    };
    let message = match cfg.command() {
        Some((command, true)) => format!(
            "Health runs your tests with coverage after each check: `{command}` (from {from}); logs go to {LOG_DIR}/"
        ),
        Some((command, false)) => format!(
            "Health runs your tests after each check: `{command}` (from {from}); logs go to {LOG_DIR}/"
        ),
        None => "Health found no test command in AGENTS.md or README.md — say how to run the \
                 tests (and their coverage) there, and the checks score them too."
            .to_string(),
    };
    narrate(sink, LogLevel::Hint, &message);
}

fn narrate(sink: &Option<Arc<dyn EventSink>>, level: LogLevel, message: &str) {
    match level {
        LogLevel::Warn => crate::ui::warn(message),
        LogLevel::Step => crate::ui::step(message),
        _ => crate::ui::hint(message),
    }
    if let Some(sink) = sink {
        let _ = sink.send(
            Origin::Network,
            TuiEvent::Log {
                level,
                msg: message.to_string(),
            },
        );
    }
}

/// The folder the runner works in, for tests.
#[cfg(test)]
fn workdir_of(snapshot: &Mutex<SnapshotRepo>) -> std::path::PathBuf {
    snapshot.lock().unwrap().worktree().to_path_buf()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::test_util::{HOME_LOCK, HomeGuard};

    struct Fixture {
        _home: tempfile::TempDir,
        wt: tempfile::TempDir,
        repo: Arc<Mutex<SnapshotRepo>>,
    }

    fn fixture() -> Fixture {
        let _g = HOME_LOCK.lock().unwrap();
        let home = tempfile::tempdir().unwrap();
        let wt = tempfile::tempdir().unwrap();
        let _h = HomeGuard::set(home.path().to_str().unwrap());
        std::fs::write(wt.path().join("a.js"), "export const a = 1;\n").unwrap();
        // Probe commands (and so the suite) are allowed in this workspace.
        std::fs::create_dir_all(wt.path().join(".ololo")).unwrap();
        std::fs::write(
            wt.path().join(".ololo/settings.json"),
            r#"{"permissions":{"allow":["*"],"deny":[]}}"#,
        )
        .unwrap();
        let repo = SnapshotRepo::new("default", "SUITE1", wt.path(), None, None).unwrap();
        repo.commit_session_start().unwrap();
        Fixture {
            _home: home,
            wt,
            repo: Arc::new(Mutex::new(repo)),
        }
    }

    fn probe(f: &Fixture, seq: u32, task: uuid::Uuid) -> SuiteJob {
        let probe_id = uuid::Uuid::new_v4();
        let commit = f
            .repo
            .lock()
            .unwrap()
            .commit_probe(task, "Build it", probe_id, seq)
            .unwrap();
        SuiteJob {
            probe_id,
            probe_seq: seq,
            task_id: task,
            task_title: "Build it".into(),
            commit,
        }
    }

    fn commands(test: &str) -> HealthTestsConfig {
        HealthTestsConfig {
            test: Some(test.into()),
            coverage: None,
            sources: vec!["README.md".into()],
            timeout_secs: 30,
        }
    }

    async fn next(rx: &mut UnboundedReceiver<PlayerAgentClientFrame>) -> TestReportPayload {
        match tokio::time::timeout(Duration::from_secs(30), rx.recv()).await {
            Ok(Some(PlayerAgentClientFrame::TestReport(r))) => *r,
            other => panic!("expected a test report, got {other:?}"),
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_run_is_logged_committed_and_reported() {
        let f = fixture();
        let (frame_tx, mut frame_rx) = tokio::sync::mpsc::unbounded_channel();
        let (runner, _task) = spawn_with(Arc::clone(&f.repo), frame_tx, None, Duration::ZERO);
        runner.set_commands(commands(
            "echo 'Tests:       1 failed, 12 passed, 13 total'; echo 'All good?' >&2; \
             echo 'Lines        : 81.2% ( 13/16 )'; exit 1",
        ));
        let task = uuid::Uuid::new_v4();
        let job = probe(&f, 3, task);
        let commit = job.commit;
        runner.submit(job);
        let report = next(&mut frame_rx).await;
        assert_eq!(report.status, TestRunStatus::Ok, "{report:?}");
        assert_eq!(report.commit, commit.to_string());
        let result = report.result.clone().unwrap();
        assert_eq!(result.exit_code, Some(1));
        assert_eq!(result.counts.map(|c| (c.passed, c.failed)), Some((12, 1)));
        assert_eq!(result.coverage_pct, Some(81.2));
        assert_eq!(result.coverage_source.as_deref(), Some("output"));
        assert_eq!(report.log.as_deref(), Some(".ololo/probes/0003-tests.log"));
        assert_eq!(summary_of(&report), "12 passed, 1 failed · coverage 81.2%");

        // The whole output, both streams, is in the log…
        let log =
            std::fs::read_to_string(f.wt.path().join(".ololo/probes/0003-tests.log")).unwrap();
        assert!(log.contains("# command: echo"), "{log}");
        assert!(log.contains("All good?"), "stderr is logged too: {log}");
        assert!(
            log.contains("# result:  12 passed, 1 failed · coverage 81.2%"),
            "{log}"
        );
        // …and committed on its own `tests(<task>)` commit on top of the probe.
        let repo = f.repo.lock().unwrap();
        let head = repo.head_id().unwrap();
        assert_ne!(head, commit);
        let message = repo.message_of(head);
        let parsed = arena_core::snapshot_message::SnapshotMessage::parse(&message);
        assert_eq!(parsed.kind, arena_core::snapshot_message::Kind::Tests);
        assert_eq!(parsed.task_id(), Some(task));
        assert!(
            parsed.subject.starts_with("#3 12 passed, 1 failed"),
            "{message}"
        );
        assert!(repo.tree_has_path(head, ".ololo/probes/0003-tests.log"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn unchanged_code_is_not_tested_twice() {
        let f = fixture();
        let (frame_tx, mut frame_rx) = tokio::sync::mpsc::unbounded_channel();
        let (runner, _task) = spawn_with(Arc::clone(&f.repo), frame_tx, None, Duration::ZERO);
        runner.set_commands(commands("echo '3 passing'"));
        let task = uuid::Uuid::new_v4();
        runner.submit(probe(&f, 1, task));
        let first = next(&mut frame_rx).await;
        assert_eq!(first.status, TestRunStatus::Ok);
        // The next probe's tree is the first one plus the first run's log:
        // `.ololo/` changed, the code did not — no run.
        runner.submit(probe(&f, 2, task));
        assert!(
            tokio::time::timeout(Duration::from_millis(400), frame_rx.recv())
                .await
                .is_err(),
            "the same code is not tested again"
        );
        std::fs::write(f.wt.path().join("a.js"), "export const a = 2;\n").unwrap();
        runner.submit(probe(&f, 3, task));
        runner.submit(probe(&f, 4, task));
        let report = next(&mut frame_rx).await;
        assert_eq!(
            report.probe_seq, 4,
            "the changed code is tested, the newest probe wins"
        );
        assert!(
            tokio::time::timeout(Duration::from_millis(300), frame_rx.recv())
                .await
                .is_err(),
            "nothing else ran"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_command_the_rules_refuse_is_reported_once_and_never_run() {
        let f = fixture();
        // A deny rule wins over every approval (a session-wide one set by
        // another test in this process included).
        std::fs::write(
            f.wt.path().join(".ololo/settings.json"),
            r#"{"permissions":{"allow":[],"deny":["touch ran.txt"]}}"#,
        )
        .unwrap();
        let (frame_tx, mut frame_rx) = tokio::sync::mpsc::unbounded_channel();
        let (runner, _task) = spawn_with(Arc::clone(&f.repo), frame_tx, None, Duration::ZERO);
        runner.set_commands(commands("touch ran.txt"));
        let task = uuid::Uuid::new_v4();
        runner.submit(probe(&f, 1, task));
        let report = next(&mut frame_rx).await;
        assert_eq!(report.status, TestRunStatus::Declined);
        assert!(report.log.is_none());
        std::fs::write(f.wt.path().join("a.js"), "export const a = 3;\n").unwrap();
        runner.submit(probe(&f, 2, task));
        assert!(
            tokio::time::timeout(Duration::from_millis(300), frame_rx.recv())
                .await
                .is_err(),
            "declined once, then quiet"
        );
        assert!(!f.wt.path().join("ran.txt").exists());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_run_past_its_budget_is_killed_with_its_children() {
        let f = fixture();
        let (frame_tx, mut frame_rx) = tokio::sync::mpsc::unbounded_channel();
        let (runner, _task) = spawn_with(Arc::clone(&f.repo), frame_tx, None, Duration::ZERO);
        let mut cfg = commands("echo started; (sleep 2; touch late.txt) & sleep 30");
        cfg.timeout_secs = 1;
        runner.set_commands(cfg);
        runner.submit(probe(&f, 1, uuid::Uuid::new_v4()));
        let report = next(&mut frame_rx).await;
        assert_eq!(report.status, TestRunStatus::Timeout);
        assert!(report.result.is_none());
        let log = std::fs::read_to_string(workdir_of(&f.repo).join(".ololo/probes/0001-tests.log"))
            .unwrap();
        assert!(log.contains("started"), "the partial output is kept: {log}");
        assert!(log.contains("timed out"), "{log}");
        // The background child died with the group.
        tokio::time::sleep(Duration::from_millis(2_500)).await;
        assert!(!f.wt.path().join("late.txt").exists());
    }

    #[tokio::test]
    async fn no_commands_means_no_runs() {
        let f = fixture();
        let (frame_tx, mut frame_rx) = tokio::sync::mpsc::unbounded_channel();
        let (runner, _task) = spawn_with(Arc::clone(&f.repo), frame_tx, None, Duration::ZERO);
        runner.submit(probe(&f, 1, uuid::Uuid::new_v4()));
        runner.set_commands(HealthTestsConfig {
            test: None,
            coverage: None,
            sources: vec![],
            timeout_secs: 30,
        });
        runner.submit(probe(&f, 2, uuid::Uuid::new_v4()));
        assert!(
            tokio::time::timeout(Duration::from_millis(300), frame_rx.recv())
                .await
                .is_err()
        );
    }

    #[test]
    fn a_long_output_keeps_its_head_and_its_tail() {
        let mut out = Captured::default();
        for i in 0..40_000 {
            out.push(&format!("line {i:05} {}", "x".repeat(20)));
        }
        let text = out.render();
        assert!(text.starts_with("line 00000"));
        assert!(text.contains("lines left out of the log"));
        assert!(
            text.trim_end()
                .ends_with(&format!("line 39999 {}", "x".repeat(20)))
        );
        assert!(text.len() < LOG_HEAD_BYTES + LOG_TAIL_BYTES + 200);
    }
}
