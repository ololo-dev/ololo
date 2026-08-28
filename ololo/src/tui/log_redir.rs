#![allow(dead_code)]

//! Tracing initialization for the TUI.
//!
//! The writer is fixed at init (a log file under the TUI, stderr
//! otherwise). The `EnvFilter` is reloadable via
//! `tracing_subscriber::reload::Layer` so a future feature can
//! dial verbosity up or down at runtime without re-initializing the
//! global subscriber (`tracing::subscriber::set_global_default` is
//! `Once`-bound and cannot be called twice in the same process).

use std::path::PathBuf;
use thiserror::Error;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, Registry, reload};

pub const LOG_FILE_NAME: &str = "ololo.tui.log";
pub const LOG_TRUNCATE_AT_BYTES: u64 = 1_048_576; // 1 MiB

pub struct TracingHandles {
    _file_guard: tracing_appender::non_blocking::WorkerGuard,
    pub _reload: reload::Handle<EnvFilter, Registry>,
}

#[derive(Debug, Error)]
pub enum TracingError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("set_global_default failed: {0}")]
    SetGlobal(String),
    #[error("home directory not found")]
    NoHome,
}

fn log_path_for(profile: &str) -> Result<PathBuf, TracingError> {
    match dirs::config_dir() {
        Some(d) => Ok(d.join("ololo").join(format!("{profile}.tui.log"))),
        None => Ok(std::env::temp_dir().join(format!("ololo-{}.tui.log", std::process::id()))),
    }
}

fn truncate_if_too_large(path: &std::path::Path) -> std::io::Result<()> {
    let metadata = std::fs::metadata(path)?;
    if metadata.len() > LOG_TRUNCATE_AT_BYTES {
        std::fs::write(path, b"")?;
    }
    Ok(())
}

pub fn setup_tracing(profile: &str) -> Result<TracingHandles, TracingError> {
    let path = log_path_for(profile)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    if path.exists() {
        let _ = truncate_if_too_large(&path);
    }
    let file = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&path)?;
    let (nb, guard) = tracing_appender::non_blocking(file);
    let env_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let (filter, reload_handle) = reload::Layer::new(env_filter);
    tracing_subscriber::registry()
        .with(filter)
        .with(
            tracing_subscriber::fmt::layer()
                .with_writer(nb)
                .with_ansi(false),
        )
        .try_init()
        .map_err(|e| TracingError::SetGlobal(e.to_string()))?;
    Ok(TracingHandles {
        _file_guard: guard,
        _reload: reload_handle,
    })
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_path_uses_config_dir_when_available() {
        // We can't easily override HOME in a test, but we can assert
        // the function returns a path with the expected file name.
        let p = log_path_for("default").unwrap();
        let s = p.to_string_lossy();
        assert!(s.ends_with("default.tui.log"));
    }

    #[test]
    fn truncate_if_too_large_does_not_touch_small_files() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("small.log");
        std::fs::write(&p, b"hello").unwrap();
        truncate_if_too_large(&p).unwrap();
        let contents = std::fs::read(&p).unwrap();
        assert_eq!(contents, b"hello");
    }
}
