//! Carrying a campaign part's work into the next part.
//!
//! Parts of a campaign continue one codebase, but each part is its own
//! session — and players routinely start the next one in a fresh directory.
//! So, when a part after the first begins:
//!
//! - **the folder already has work in it** → continue in place, touch
//!   nothing. This covers the common case of replaying or resuming right
//!   where the previous part was built, and it is why the rule is "is the
//!   directory empty", not "does a marker file exist".
//! - **the folder is empty** → import the player's own snapshot of the
//!   previous part from the server, so the agent opens onto the codebase it
//!   is meant to extend rather than a blank slate.
//!
//! The import is a plain `git clone` of the player's per-session snapshot
//! repo (their PAT already authorizes it) followed by `git archive` into the
//! workspace: the tree lands without a `.git`, because the CLI's own snapshot
//! repo lives outside the worktree and a stray `.git` here would confuse both
//! the player and the push path.

use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Where the previous part's work lives, as told by
/// `GET /api/projects/:id/previous-part-source`.
#[derive(Debug, Clone, Deserialize)]
pub struct PreviousPartSource {
    pub prev_project_slug: Option<String>,
    pub prev_part_ordinal: i32,
    pub session_id: String,
    pub player_id: String,
    pub git_remote_path: String,
}

impl PreviousPartSource {
    fn label(&self) -> String {
        self.prev_project_slug
            .clone()
            .unwrap_or_else(|| format!("part {}", self.prev_part_ordinal + 1))
    }
}

/// Lineage breadcrumb dropped in the workspace. Purely informational — the
/// import decision is made from the directory being empty, never from this
/// file — but it makes "which part is this folder, and where did it come
/// from" answerable after the fact.
#[derive(Debug, Clone, Serialize)]
struct CampaignMarker {
    part_ordinal: i32,
    imported_from_project: Option<String>,
    imported_from_session: Option<String>,
    /// The player row the snapshot came from. Together with the session it
    /// names the exact repo the tree was taken from, which is what makes a
    /// carried-over workspace traceable back to the run that produced it.
    imported_from_player: Option<String>,
    imported: bool,
}

/// Campaign position of the project a session is about to run, read off the
/// project JSON the CLI already fetches.
pub fn part_ordinal_of(project: &serde_json::Value) -> Option<i32> {
    project
        .get("part_ordinal")
        .and_then(|v| v.as_i64())
        .map(|v| v as i32)
}

/// True when the working directory holds nothing worth keeping. `.DS_Store`
/// is ignored because Finder leaves it in directories the player considers
/// empty, and refusing to import over it would be baffling.
pub fn workspace_is_empty(dir: &Path) -> bool {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return false;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        if name == ".DS_Store" {
            continue;
        }
        return false;
    }
    true
}

/// Ask the server where this part's predecessor work lives. `Ok(None)` covers
/// every "nothing to carry over" answer (first part, ordinary project, a
/// predecessor the player never completed) — those are normal, not failures.
pub async fn fetch_previous_part_source(
    client: &reqwest::Client,
    base: &str,
    pat: &str,
    project_id: &str,
) -> Result<Option<PreviousPartSource>> {
    let url = format!(
        "{}/api/projects/{project_id}/previous-part-source",
        base.trim_end_matches('/')
    );
    let resp = client
        .get(&url)
        .header("X-API-Key", pat)
        .send()
        .await
        .with_context(|| format!("GET {url}"))?;
    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(None);
    }
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_else(|_| "<no body>".into());
        return Err(anyhow!(
            "previous-part lookup failed (HTTP {status}): {body}"
        ));
    }
    Ok(Some(
        resp.json().await.context("parsing previous-part source")?,
    ))
}

/// Clone the previous part's snapshot and unpack its tree into `dest`.
///
/// The PAT travels in `http.extraHeader` through the environment rather than
/// in the URL or argv, so it never lands in `ps` output or a shell history.
fn clone_previous_snapshot(
    base: &str,
    pat: &str,
    source: &PreviousPartSource,
    dest: &Path,
) -> Result<()> {
    let git = which::which("git").context("git is required to import your previous part")?;
    let remote = format!("{}{}", base.trim_end_matches('/'), source.git_remote_path);
    let tmp: PathBuf =
        std::env::temp_dir().join(format!("ololo-carry-{}.git", uuid::Uuid::new_v4()));

    let clone = std::process::Command::new(&git)
        .arg("clone")
        .arg("--bare")
        .arg("--depth")
        .arg("1")
        .arg(&remote)
        .arg(&tmp)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_CONFIG_COUNT", "2")
        .env("GIT_CONFIG_KEY_0", "http.extraHeader")
        .env("GIT_CONFIG_VALUE_0", format!("Authorization: Bearer {pat}"))
        // Mirrors the push path: a buffered body with a Content-Length
        // survives the Cloudflare → proxy → CGI chain that mangles chunked
        // transfers.
        .env("GIT_CONFIG_KEY_1", "http.postBuffer")
        .env("GIT_CONFIG_VALUE_1", "536870912")
        .output()
        .context("running git clone")?;
    if !clone.status.success() {
        let _ = std::fs::remove_dir_all(&tmp);
        return Err(anyhow!(
            "git clone of your previous part failed: {}",
            String::from_utf8_lossy(&clone.stderr).trim_end()
        ));
    }

    // Unpack via a tar archive rather than a checkout: the workspace must
    // receive the files alone, with no `.git` alongside them.
    let archive = tmp.with_extension("tar");
    let export = std::process::Command::new(&git)
        .arg(format!("--git-dir={}", tmp.display()))
        .arg("archive")
        .arg("--format=tar")
        .arg("-o")
        .arg(&archive)
        .arg("HEAD")
        .output()
        .context("running git archive")?;
    if !export.status.success() {
        let _ = std::fs::remove_dir_all(&tmp);
        let _ = std::fs::remove_file(&archive);
        return Err(anyhow!(
            "reading your previous part's files failed: {}",
            String::from_utf8_lossy(&export.stderr).trim_end()
        ));
    }

    let untar = std::process::Command::new("tar")
        .arg("-xf")
        .arg(&archive)
        .arg("-C")
        .arg(dest)
        .output()
        .context("running tar to unpack your previous part")?;
    let _ = std::fs::remove_dir_all(&tmp);
    let _ = std::fs::remove_file(&archive);
    if !untar.status.success() {
        return Err(anyhow!(
            "unpacking your previous part failed: {}",
            String::from_utf8_lossy(&untar.stderr).trim_end()
        ));
    }
    Ok(())
}

fn write_marker(dir: &Path, marker: &CampaignMarker) {
    let ololo_dir = dir.join(".ololo");
    if std::fs::create_dir_all(&ololo_dir).is_err() {
        return;
    }
    if let Ok(json) = serde_json::to_string_pretty(marker) {
        let _ = std::fs::write(ololo_dir.join("campaign.json"), json);
    }
}

/// Prepare the workspace for a campaign part: import the previous part's work
/// when the folder is empty, otherwise leave the folder alone.
///
/// Only runs for parts after the first. A failed import is fatal rather than
/// a warning: silently starting part four on a blank directory wastes the
/// player's whole session, and `--fresh` is right there for anyone who
/// genuinely wants an empty start.
pub async fn prepare_part_workspace(
    client: &reqwest::Client,
    base: &str,
    pat: &str,
    project: &serde_json::Value,
    fresh: bool,
) -> Result<()> {
    let Some(ordinal) = part_ordinal_of(project) else {
        return Ok(());
    };
    if ordinal <= 0 {
        return Ok(());
    }
    let cwd = std::env::current_dir().context("resolving the working directory")?;

    if !workspace_is_empty(&cwd) {
        crate::ui::hint(
            "This folder already has work in it — continuing where the previous part left off.",
        );
        write_marker(
            &cwd,
            &CampaignMarker {
                part_ordinal: ordinal,
                imported_from_project: None,
                imported_from_session: None,
                imported_from_player: None,
                imported: false,
            },
        );
        return Ok(());
    }

    if fresh {
        crate::ui::hint("Starting this part with an empty workspace (--fresh).");
        return Ok(());
    }

    let Some(project_id) = project.get("id").and_then(|v| v.as_str()) else {
        return Ok(());
    };
    let Some(source) = fetch_previous_part_source(client, base, pat, project_id).await? else {
        // The session gate already refuses a locked part, so reaching this
        // point means the previous part was completed but left no snapshot.
        crate::ui::hint("No previous-part snapshot to import — starting empty.");
        return Ok(());
    };

    crate::ui::step(format!("Importing your '{}' results...", source.label()));
    clone_previous_snapshot(base, pat, &source, &cwd).map_err(|e| {
        anyhow!("{e}\nRetry once the connection is back, or pass --fresh to start this part empty.")
    })?;
    write_marker(
        &cwd,
        &CampaignMarker {
            part_ordinal: ordinal,
            imported_from_project: source.prev_project_slug.clone(),
            imported_from_session: Some(source.session_id.clone()),
            imported_from_player: Some(source.player_id.clone()),
            imported: true,
        },
    );
    crate::ui::success(format!("Imported your '{}' results", source.label()));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_directory_with_only_a_ds_store_counts_as_empty() {
        let dir = tempfile::tempdir().expect("tempdir");
        assert!(workspace_is_empty(dir.path()));
        std::fs::write(dir.path().join(".DS_Store"), b"junk").expect("write");
        assert!(workspace_is_empty(dir.path()));
        std::fs::write(dir.path().join("main.rs"), b"fn main() {}").expect("write");
        assert!(!workspace_is_empty(dir.path()));
    }

    #[test]
    fn a_hidden_workspace_file_still_counts_as_work() {
        // `.ololo/settings.json` from a previous run is real state: importing
        // over it would mean the player silently lost their permissions.
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir(dir.path().join(".ololo")).expect("mkdir");
        assert!(!workspace_is_empty(dir.path()));
    }

    #[test]
    fn the_source_payload_parses() {
        let json = serde_json::json!({
            "prev_project_slug": "handmade-postgresql-1-repl",
            "prev_part_ordinal": 0,
            "session_id": "11111111-1111-1111-1111-111111111111",
            "player_id": "22222222-2222-2222-2222-222222222222",
            "git_remote_path": "/git/11111111-1111-1111-1111-111111111111/22222222-2222-2222-2222-222222222222.git"
        });
        let source: PreviousPartSource = serde_json::from_value(json).expect("parse");
        assert_eq!(source.label(), "handmade-postgresql-1-repl");
        assert!(source.git_remote_path.starts_with("/git/"));
    }

    #[test]
    fn a_source_without_a_slug_labels_itself_by_part_number() {
        let source = PreviousPartSource {
            prev_project_slug: None,
            prev_part_ordinal: 2,
            session_id: "s".into(),
            player_id: "p".into(),
            git_remote_path: "/git/s/p.git".into(),
        };
        assert_eq!(source.label(), "part 3");
    }

    #[test]
    fn only_parts_after_the_first_carry_anything_over() {
        assert_eq!(part_ordinal_of(&serde_json::json!({})), None);
        assert_eq!(
            part_ordinal_of(&serde_json::json!({"part_ordinal": null})),
            None
        );
        assert_eq!(
            part_ordinal_of(&serde_json::json!({"part_ordinal": 3})),
            Some(3)
        );
    }
}
