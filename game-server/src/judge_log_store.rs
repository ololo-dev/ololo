//! On-disk judge run-log store.
//!
//! The bulky per-run telemetry (turn transcripts, prompts, tool outputs)
//! lives on the game-server's filesystem instead of the database:
//!
//! ```text
//! {ARENA_JUDGE_LOG_DIR}/{join_code}/{username}/{task_id}.json
//! ```
//!
//! Each file is a JSON object keyed by judge slug — one entry per judge
//! attached to the task, overwritten on re-runs. The main server fetches
//! entries through `GET /internal/judge-logs/...` (it may not share this
//! disk). Small metadata (tokens, provider, error) stays in
//! `judge_results`; only the event log moves here.

use std::path::{Path, PathBuf};

use uuid::Uuid;

/// Base directory: `ARENA_JUDGE_LOG_DIR`, default `./judge-logs`.
pub fn base_dir() -> PathBuf {
    std::env::var("ARENA_JUDGE_LOG_DIR")
        .ok()
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("./judge-logs"))
}

/// Filesystem-safe path component: keep `[A-Za-z0-9._-]`, map the rest to
/// `_`, strip leading dots (no hidden/parent traversal), cap length. Empty
/// results fall back to `"_"`.
pub fn sanitize_component(name: &str) -> String {
    let mut out: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                c
            } else {
                '_'
            }
        })
        .collect();
    while out.starts_with('.') {
        out.remove(0);
    }
    out.truncate(64);
    if out.is_empty() {
        out.push('_');
    }
    out
}

/// `{base}/{join_code}/{username}/{task_id}.json`
pub fn task_log_path(base: &Path, join_code: &str, username: &str, task_id: Uuid) -> PathBuf {
    base.join(sanitize_component(join_code))
        .join(sanitize_component(username))
        .join(format!("{task_id}.json"))
}

/// Merge `entry` under `judge_slug` into the task's log file, creating
/// parent directories as needed. Atomic (tmp + rename). Blocking IO runs
/// on the blocking pool. Failures are logged, never fatal — the judge run
/// itself must not fail because telemetry could not be written.
pub async fn store_entry(
    base: PathBuf,
    join_code: &str,
    username: &str,
    task_id: Uuid,
    judge_slug: &str,
    entry: serde_json::Value,
) {
    let path = task_log_path(&base, join_code, username, task_id);
    let slug = judge_slug.to_string();
    let res = tokio::task::spawn_blocking(move || store_entry_blocking(&path, &slug, entry)).await;
    match res {
        Ok(Ok(())) => {}
        Ok(Err(e)) => tracing::warn!(error = %e, "judge_log_store: write failed"),
        Err(e) => tracing::warn!(error = %e, "judge_log_store: join failed"),
    }
}

fn store_entry_blocking(
    path: &Path,
    judge_slug: &str,
    entry: serde_json::Value,
) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut doc = match std::fs::read(path) {
        Ok(bytes) => serde_json::from_slice::<serde_json::Value>(&bytes)
            .unwrap_or_else(|_| serde_json::json!({})),
        Err(_) => serde_json::json!({}),
    };
    if !doc.is_object() {
        doc = serde_json::json!({});
    }
    doc.as_object_mut()
        .expect("doc is an object")
        .insert(judge_slug.to_string(), entry);
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(&doc)?)?;
    std::fs::rename(&tmp, path)
}

/// Read a task's full log file (object keyed by judge slug).
pub async fn read_task_log(
    base: PathBuf,
    join_code: &str,
    username: &str,
    task_id: Uuid,
) -> Option<serde_json::Value> {
    let path = task_log_path(&base, join_code, username, task_id);
    let bytes = tokio::task::spawn_blocking(move || std::fs::read(path))
        .await
        .ok()?
        .ok()?;
    serde_json::from_slice(&bytes).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_keeps_safe_chars_and_blocks_traversal() {
        assert_eq!(sanitize_component("KCQFHR"), "KCQFHR");
        assert_eq!(sanitize_component("andrey"), "andrey");
        assert_eq!(sanitize_component("../../etc/passwd"), "_.._etc_passwd");
        assert_eq!(sanitize_component("héllo wörld"), "h_llo_w_rld");
        assert_eq!(sanitize_component(""), "_");
        assert_eq!(sanitize_component("..."), "_");
    }

    #[tokio::test]
    async fn store_merges_entries_per_judge_and_roundtrips() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path().to_path_buf();
        let task_id = Uuid::new_v4();

        store_entry(
            base.clone(),
            "KCQFHR",
            "andrey",
            task_id,
            "anti-cheater",
            serde_json::json!({"status": "scored", "events": [{"kind": "llm"}]}),
        )
        .await;
        store_entry(
            base.clone(),
            "KCQFHR",
            "andrey",
            task_id,
            "code-cleanliness",
            serde_json::json!({"status": "failed", "error": "429"}),
        )
        .await;
        // Re-run overwrites the same judge's entry.
        store_entry(
            base.clone(),
            "KCQFHR",
            "andrey",
            task_id,
            "anti-cheater",
            serde_json::json!({"status": "scored", "events": [{"kind": "llm"}, {"kind": "tool"}]}),
        )
        .await;

        let doc = read_task_log(base.clone(), "KCQFHR", "andrey", task_id)
            .await
            .expect("file readable");
        assert_eq!(doc["anti-cheater"]["events"].as_array().unwrap().len(), 2);
        assert_eq!(doc["code-cleanliness"]["error"], "429");

        let expected = base
            .join("KCQFHR")
            .join("andrey")
            .join(format!("{task_id}.json"));
        assert!(
            expected.is_file(),
            "file at join_code/username/task_id.json"
        );
    }

    #[tokio::test]
    async fn read_missing_returns_none() {
        let dir = tempfile::tempdir().unwrap();
        assert!(
            read_task_log(dir.path().to_path_buf(), "X", "y", Uuid::new_v4())
                .await
                .is_none()
        );
    }
}
