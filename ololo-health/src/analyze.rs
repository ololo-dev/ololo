//! The pipeline: survey the tree (size caps, `jscpd:ignore` markers, a
//! jscpd config we will not read), run jscpd in-process, fold its `Health`
//! into a [`HealthResult`]. All of it on a detached thread behind a timeout
//! with panics caught, so a hostile tree can neither hang nor take down
//! the process that asked.

use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use basta::config::BastaConfig;
use cpd_core::deadcode::Report as DeadCodeReport;
use cpd_core::health::Health;
use cpd_core::summary::{SummaryMetric, compute_summary};
use cpd_finder::orchestrate::RunConfig;
use serde::{Deserialize, Serialize};

use crate::config::{ALWAYS_IGNORED_DIRS, ConfigError, HealthConfig, PRUNED_DIRS};
use crate::score::{Level, level};
use crate::{HEALTH_SCHEMA, JSCPD_CORE_VERSION};

/// One scored tree. Travels on the wire as-is, hence `deny_unknown_fields`:
/// a field added here is a [`HEALTH_SCHEMA`] bump.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HealthResult {
    /// [`HEALTH_SCHEMA`] this result was produced under.
    pub schema: u32,
    /// [`JSCPD_CORE_VERSION`] of the producer — compare before comparing scores.
    pub jscpd_version: String,
    /// jscpd's 0..100 score; `None` when the tree has no code to measure.
    pub score: Option<f64>,
    /// jscpd's grade: `A` from 85, `B` from 70, `C` from 55, `D` from 40, else `E`.
    pub grade: Option<char>,
    pub level: Level,
    /// jscpd's `Health` exactly as it serializes it (camelCase): score,
    /// grade, size, every dimension with value/adjusted/lines/half-life/
    /// weight/score, and what was skipped. Deterministic for a fixed tree.
    pub health: serde_json::Value,
    pub metrics: Metrics,
    /// Wall-clock time of the survey and the scan.
    pub duration_ms: u64,
}

/// The numbers a reader wants without unpacking jscpd's result.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Metrics {
    /// Code files jscpd scored (prose, data and markup are not counted).
    pub files: u64,
    /// Lines in those files.
    pub code_lines: u64,
    /// Share of code lines that are duplicated, in percent.
    pub duplication_pct: Option<f64>,
    pub duplicated_lines: Option<u64>,
    /// Clone pairs jscpd found.
    pub clones: u64,
    /// Share of code lines that sit in complex files, in percent.
    pub complexity_pct: Option<f64>,
    pub complex_lines: Option<u64>,
    /// Share of the analyzable code lines that are dead (unused files,
    /// exports, symbols, imports), in percent. `None` when no file is in a
    /// language the analyzer reads (JavaScript, TypeScript, Python).
    #[serde(default)]
    pub dead_code_pct: Option<f64>,
    #[serde(default)]
    pub dead_lines: Option<u64>,
    /// Dead findings (files, exports, symbols, imports) behind `dead_lines`.
    #[serde(default)]
    pub dead_symbols: Option<u64>,
    /// Percent of the code lines the dead-code analyzer could read, when
    /// that is not all of them.
    #[serde(default)]
    pub dead_code_coverage: Option<f64>,
    /// Files carrying a `jscpd:ignore-start` marker. jscpd honours the
    /// markers at the tokenizer level (they cannot be switched off), so a
    /// tree can hide code from the scan — this count makes that visible.
    pub ignore_markers: u64,
    /// The tree carries a `.jscpd.json` (or `package.json#jscpd`). It is
    /// **not** applied — the shared config is the only one — but a reader
    /// may want to know the player tried.
    pub jscpd_config_present: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum HealthError {
    #[error("analysis exceeded {0:?}")]
    Timeout(Duration),
    #[error("analysis panicked: {0}")]
    Panicked(String),
    #[error(
        "tree too large to scan: {files} files / {bytes} bytes (limits {max_files} / {max_bytes})"
    )]
    TreeTooLarge {
        files: u64,
        bytes: u64,
        max_files: u64,
        max_bytes: u64,
    },
    #[error("not a directory: {0}")]
    NotADirectory(PathBuf),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("serializing the result: {0}")]
    Serialize(#[from] serde_json::Error),
    #[error("invalid config: {0}")]
    Config(#[from] ConfigError),
    /// The worker thread ended without delivering a value — never expected.
    #[error("analysis thread vanished")]
    Vanished,
}

/// Score the tree under `root`, blocking the caller for at most
/// `cfg.timeout`. The work runs on its own thread with panics caught; on
/// timeout that thread finishes on its own (jscpd's pipeline cannot be
/// interrupted) and its result is dropped, which is why the tree size caps
/// exist — they are the bound on how much an abandoned run can cost.
pub fn analyze(root: &Path, cfg: &HealthConfig) -> Result<HealthResult, HealthError> {
    cfg.validate()?;
    let root = root.to_path_buf();
    let cfg = cfg.clone();
    let timeout = cfg.timeout;
    let (tx, rx) = mpsc::sync_channel(1);
    spawn_worker(
        move || analyze_now(&root, &cfg),
        move |outcome| {
            let _ = tx.send(outcome);
        },
    )?;
    match rx.recv_timeout(timeout) {
        Ok(outcome) => unwrap_outcome(outcome),
        Err(mpsc::RecvTimeoutError::Timeout) => Err(HealthError::Timeout(timeout)),
        Err(mpsc::RecvTimeoutError::Disconnected) => Err(HealthError::Vanished),
    }
}

/// [`analyze`] for async callers: the same detached thread, awaited through
/// a oneshot with `cfg.timeout`. Dropping the future stops waiting at once;
/// the thread still runs to completion.
pub async fn analyze_async(root: PathBuf, cfg: HealthConfig) -> Result<HealthResult, HealthError> {
    cfg.validate()?;
    let timeout = cfg.timeout;
    let (tx, rx) = tokio::sync::oneshot::channel();
    spawn_worker(
        move || analyze_now(&root, &cfg),
        move |outcome| {
            let _ = tx.send(outcome);
        },
    )?;
    match tokio::time::timeout(timeout, rx).await {
        Ok(Ok(outcome)) => unwrap_outcome(outcome),
        Ok(Err(_)) => Err(HealthError::Vanished),
        Err(_) => Err(HealthError::Timeout(timeout)),
    }
}

type Outcome = std::thread::Result<Result<HealthResult, HealthError>>;

/// Run `job` on a named thread with panics caught, handing whatever came
/// out of it to `deliver` — a channel send in practice.
fn spawn_worker(
    job: impl FnOnce() -> Result<HealthResult, HealthError> + Send + 'static,
    deliver: impl FnOnce(Outcome) + Send + 'static,
) -> std::io::Result<()> {
    std::thread::Builder::new()
        .name("ololo-health".into())
        .spawn(move || {
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(job));
            deliver(outcome);
        })
        .map(drop)
}

fn unwrap_outcome(outcome: Outcome) -> Result<HealthResult, HealthError> {
    match outcome {
        Ok(result) => result,
        Err(payload) => Err(HealthError::Panicked(panic_message(payload.as_ref()))),
    }
}

fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(s) = payload.downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "non-string panic payload".to_string()
    }
}

/// The synchronous body: survey, scan, fold. Runs on the worker thread.
fn analyze_now(root: &Path, cfg: &HealthConfig) -> Result<HealthResult, HealthError> {
    let started = Instant::now();
    let root = root.canonicalize()?;
    if !root.is_dir() {
        return Err(HealthError::NotADirectory(root));
    }
    let survey = survey_tree(&root, cfg)?;
    let (health, clones, dead_code) = scan(&root, cfg);
    fold(health, clones, dead_code, survey, cfg, started)
}

/// What the survey learns before jscpd runs.
#[derive(Debug, Default)]
struct Survey {
    /// Code files the scan will read, and their bytes — what the caps bound.
    files: u64,
    bytes: u64,
    ignore_markers: u64,
    jscpd_config_present: bool,
}

/// Whether jscpd reads this file: a format it knows by extension, within
/// the per-file size cap. (jscpd also sniffs shebangs of extensionless
/// files; the survey does not open files to find them, so those few are
/// read without counting towards the caps.)
fn scanned(path: &Path, len: u64, cfg: &HealthConfig) -> bool {
    len <= cfg.max_file_bytes
        && path
            .extension()
            .and_then(|e| e.to_str())
            .and_then(cpd_tokenizer::formats::get_format_by_extension)
            .is_some()
}

const IGNORE_MARKER: &[u8] = b"jscpd:ignore-start";

/// Walk the tree once, cheaply: count the code files the scan would read
/// (after the directory exclusions, through jscpd's format table), refuse
/// past the caps, count marker files, note a jscpd config.
fn survey_tree(root: &Path, cfg: &HealthConfig) -> Result<Survey, HealthError> {
    let mut survey = Survey {
        jscpd_config_present: jscpd_config_present(root),
        ..Survey::default()
    };
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir)? {
            let entry = entry?;
            let file_type = entry.file_type()?;
            // jscpd does not follow symlinks either (`follow_symlinks: false`).
            if file_type.is_symlink() {
                continue;
            }
            if file_type.is_dir() {
                let name = entry.file_name();
                let name = name.to_string_lossy();
                if ALWAYS_IGNORED_DIRS.contains(&&*name) || PRUNED_DIRS.contains(&&*name) {
                    continue;
                }
                stack.push(entry.path());
                continue;
            }
            if !file_type.is_file() {
                continue;
            }
            let len = entry.metadata()?.len();
            if !scanned(&entry.path(), len, cfg) {
                continue;
            }
            survey.files += 1;
            survey.bytes += len;
            if survey.files > cfg.max_files || survey.bytes > cfg.max_total_bytes {
                return Err(HealthError::TreeTooLarge {
                    files: survey.files,
                    bytes: survey.bytes,
                    max_files: cfg.max_files,
                    max_bytes: cfg.max_total_bytes,
                });
            }
            // Only files jscpd reads can carry a marker it would honour.
            if len > 0 && has_marker(&entry.path())? {
                survey.ignore_markers += 1;
            }
        }
    }
    Ok(survey)
}

fn has_marker(path: &Path) -> std::io::Result<bool> {
    let bytes = std::fs::read(path)?;
    Ok(bytes
        .windows(IGNORE_MARKER.len())
        .any(|w| w == IGNORE_MARKER))
}

/// The config files jscpd's CLI would discover in a working directory. We
/// never read them for the scan; the survey only reports their presence.
fn jscpd_config_present(root: &Path) -> bool {
    let files = [".jscpd.json", ".config/jscpd.json", ".config/.jscpd.json"];
    if files.iter().any(|f| root.join(f).is_file()) {
        return true;
    }
    std::fs::read_to_string(root.join("package.json"))
        .ok()
        .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
        .is_some_and(|json| json.get("jscpd").is_some())
}

/// The in-process twin of `jscpd --health` (`crates/cpd/src/dashboard.rs`).
fn scan(root: &Path, cfg: &HealthConfig) -> (Health, u64, Option<DeadCodeReport>) {
    let run_cfg = RunConfig {
        paths: vec![root.to_path_buf()],
        ignore: cfg.ignore.clone(),
        max_size: Some(cfg.max_file_bytes),
        // The tree is a committed snapshot, already filtered by the worktree
        // `.gitignore` at commit time. Consulting `.gitignore` here would make
        // the result depend on whether the directory happens to sit inside a
        // git repository — the `ignore` crate applies gitignore rules (the
        // user's global ones included) only there.
        no_gitignore: true,
        follow_symlinks: false,
        blame: false,
        workers: cfg.workers,
        ..RunConfig::default()
    };
    let result = match cpd_finder::orchestrate::run(&run_cfg) {
        Ok(result) => result,
        Err(never) => match never {},
    };
    // `top = usize::MAX`: `health::compute` needs every file, not a top-N cut.
    // Identity display paths keep the per-file duplication keyed by the same
    // ids the clone fragments carry (nothing here relativizes them).
    let summary = compute_summary(
        &result.sources,
        &result.clones,
        usize::MAX,
        SummaryMetric::Complexity,
        |id| id.to_string(),
    );
    // The third dimension: jscpd's dead-code analyzer over the same tree,
    // with the same exclusions and size cap. Nothing to say about a tree
    // with no file it reads; the other dimensions still stand.
    let dead_code = dead_code_report(root, cfg);
    let health = cpd_core::health::compute(
        &summary,
        &result.clones,
        dead_code.as_ref().map(|report| &report.statistics),
        &cfg.jscpd,
    );
    (health, result.clones.len() as u64, dead_code)
}

fn dead_code_report(root: &Path, cfg: &HealthConfig) -> Option<DeadCodeReport> {
    let config = BastaConfig {
        paths: vec![root.to_path_buf()],
        ignore: cfg.ignore.clone(),
        max_size: Some(cfg.max_file_bytes),
        no_gitignore: true,
        follow_symlinks: false,
        workers: cfg.workers,
        ..BastaConfig::default()
    };
    Some(basta::analyze::run(&config).report).filter(|report| report.statistics.files > 0)
}

fn fold(
    health: Health,
    clones: u64,
    dead_code: Option<DeadCodeReport>,
    survey: Survey,
    cfg: &HealthConfig,
    started: Instant,
) -> Result<HealthResult, HealthError> {
    let dimension = |id: &str| health.dimensions.iter().find(|d| d.id == id);
    let duplication = dimension("duplication");
    let complexity = dimension("complexity");
    let dead = dimension("dead-code");
    let metrics = Metrics {
        files: health.size.files,
        code_lines: health.size.lines,
        duplication_pct: duplication.and_then(|d| d.value),
        duplicated_lines: duplication.and_then(|d| d.lines),
        clones,
        complexity_pct: complexity.and_then(|d| d.value),
        complex_lines: complexity.and_then(|d| d.lines),
        dead_code_pct: dead.and_then(|d| d.value),
        dead_lines: dead.and_then(|d| d.lines),
        dead_symbols: dead_code.as_ref().map(|r| r.findings.len() as u64),
        dead_code_coverage: dead.and_then(|d| d.coverage),
        ignore_markers: survey.ignore_markers,
        jscpd_config_present: survey.jscpd_config_present,
    };
    Ok(HealthResult {
        schema: HEALTH_SCHEMA,
        jscpd_version: JSCPD_CORE_VERSION.to_string(),
        score: health.score,
        grade: health.grade,
        level: level(health.score, &cfg.thresholds),
        health: serde_json::to_value(&health)?,
        metrics,
        duration_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wait(
        job: impl FnOnce() -> Result<HealthResult, HealthError> + Send + 'static,
        timeout: Duration,
    ) -> Result<HealthResult, HealthError> {
        let (tx, rx) = mpsc::sync_channel(1);
        spawn_worker(job, move |o| {
            let _ = tx.send(o);
        })
        .unwrap();
        match rx.recv_timeout(timeout) {
            Ok(outcome) => unwrap_outcome(outcome),
            Err(mpsc::RecvTimeoutError::Timeout) => Err(HealthError::Timeout(timeout)),
            Err(mpsc::RecvTimeoutError::Disconnected) => Err(HealthError::Vanished),
        }
    }

    #[test]
    fn a_slow_job_times_out_instead_of_blocking() {
        let err = wait(
            || {
                std::thread::sleep(Duration::from_millis(300));
                Err(HealthError::Vanished)
            },
            Duration::from_millis(20),
        )
        .unwrap_err();
        assert!(
            matches!(err, HealthError::Timeout(t) if t == Duration::from_millis(20)),
            "{err}"
        );
    }

    #[test]
    fn a_panicking_job_is_reported_not_propagated() {
        let err = wait(|| panic!("boom {}", 42), Duration::from_secs(5)).unwrap_err();
        assert!(
            matches!(&err, HealthError::Panicked(m) if m == "boom 42"),
            "{err}"
        );
    }

    #[test]
    fn a_job_error_comes_back_as_is() {
        let err = wait(
            || Err(HealthError::NotADirectory("x".into())),
            Duration::from_secs(5),
        )
        .unwrap_err();
        assert!(matches!(err, HealthError::NotADirectory(_)));
    }
}
