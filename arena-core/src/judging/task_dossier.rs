//! Server-built evidence pack for a task-scoped judge.
//!
//! The agentic path lets a judge pull git evidence through tool calls, which
//! costs one provider round-trip per call and re-sends the growing tool
//! transcript every turn. For a judge whose investigation is the *same every
//! time* — locate the task's commit, diff its work window, check what existed
//! before it, read the agent activity — that adaptivity buys nothing and the
//! whole pack can be assembled up front in one pass of local git commands.
//!
//! What goes in mirrors the tool sequence the task anti-cheat judge used to
//! perform by hand:
//!
//! 1. the commit sequence (`get_commit_log`),
//! 2. this task's commit and the one before it — the work window
//!    (`find_task_commit`),
//! 3. the window's diff, which is what the player wrote for the task
//!    (`get_diff`),
//! 4. the pre-window content of every file the diff touches, so
//!    pre-implementation is visible without guessing which file to open
//!    (`read_file` at the base ref),
//! 5. the client-reported agent statistics (`get_task_stats`).
//!
//! Every section is bounded, and a section that was cut says so in-band: a
//! judge that cannot see the evidence must not infer guilt from its absence.

use std::path::Path;

use super::tools::{ToolScope, find_task_commit, get_commit_log, get_diff, read_file};

/// Diff text budget for the task's work window.
const DIFF_CAP: usize = 48_000;
/// Budget for one pre-window file, and for all of them together.
const FILE_CAP: usize = 8_000;
const FILES_TOTAL_CAP: usize = 32_000;
/// How many files from the diff to show the pre-window state of.
const MAX_PRE_FILES: usize = 12;
/// Commits listed from the task commit backwards.
const COMMIT_LOG_LIMIT: u32 = 40;
/// When the work window is empty: how many preceding commits to show the
/// diffs of, and the text budget for each and for all of them together.
const PRIOR_COMMITS_MAX: usize = 5;
const PRIOR_DIFF_CAP: usize = 8_000;
const PRIOR_DIFFS_TOTAL_CAP: usize = 24_000;

/// Assemble the evidence pack. Never fails: a section that cannot be read is
/// reported as unavailable, because the judge's fallback for missing evidence
/// is to return a neutral verdict rather than to speculate.
pub async fn build_task_dossier(
    repo_dir: &Path,
    task_id: &str,
    task_commit_sha: Option<&str>,
    task_stats_json: Option<&str>,
    scope: &ToolScope,
) -> String {
    let mut s = String::new();
    s.push_str(
        "\n=== EVIDENCE PACK ===\n\
         Collected server-side from this player's repository. This is the whole \
         record available for this task; there are no tools to call.\n",
    );

    // 1. Which commit belongs to this task, and what came immediately before.
    let commits = find_task_commit(repo_dir, task_id)
        .await
        .unwrap_or_default();
    let resolved_sha = commits
        .first()
        .map(|c| c.sha.clone())
        .or_else(|| task_commit_sha.map(str::to_string));

    match &resolved_sha {
        Some(sha) => s.push_str(&format!("\n-- Task commit --\n{sha}\n")),
        None => s.push_str(
            "\n-- Task commit --\nNone found: no commit references this task id. \
             The work window cannot be isolated.\n",
        ),
    }

    // 2. Commit sequence, newest first, from the task commit backwards.
    // Kept around: when the work window turns out to be empty, the log is
    // walked again to show where the behaviour actually arrived.
    let commit_log = get_commit_log(
        repo_dir,
        resolved_sha.as_deref(),
        Some(COMMIT_LOG_LIMIT),
        task_commit_sha,
    )
    .await;
    s.push_str("\n-- Commit log (newest first) --\n");
    match &commit_log {
        Ok(log) if !log.is_empty() => {
            for c in log {
                s.push_str(&format!("{} {}\n", c.sha, c.subject));
            }
        }
        Ok(_) => s.push_str("(empty)\n"),
        Err(e) => s.push_str(&format!("(unavailable: {e})\n")),
    }

    // The window's base is the last commit that does NOT belong to this task.
    // "The commit immediately before" is the wrong base: the CLI commits the
    // work under `flag(<task_id>)` and `artifact(<task_id>): sync` before the
    // `feat` snapshot lands, so the immediate parent already contains
    // everything and the window diffed empty for honest players. Walk past
    // every commit carrying this task's id to the boundary of the previous
    // task (or the session start).
    let task_marker = format!("({task_id})");
    let base_sha =
        match get_commit_log(repo_dir, resolved_sha.as_deref(), Some(50), task_commit_sha).await {
            Ok(log) => log
                .iter()
                .skip(1)
                .find(|entry| !entry.subject.contains(&task_marker))
                .map(|entry| entry.sha.clone()),
            _ => None,
        };

    // 3. The window diff.
    let mut window_is_empty = false;
    s.push_str("\n-- Work-window diff --\n");
    match (&resolved_sha, &base_sha) {
        (Some(head), Some(base)) => {
            s.push_str(&format!("Range: {base}..{head}\n"));
            match get_diff(repo_dir, Some(base), Some(head), task_commit_sha, scope).await {
                Ok(d) if d.trim().is_empty() => {
                    window_is_empty = true;
                    s.push_str(
                        "(empty: nothing changed in this task's whole work window — \
                         see the preceding in-session commits section below for \
                         where the behaviour was written)\n",
                    );
                }
                Ok(d) => s.push_str(&capped(&d, DIFF_CAP, "diff")),
                Err(e) => s.push_str(&format!("(unavailable: {e})\n")),
            }
        }
        (Some(head), None) => {
            // First task of the session: no preceding commit, so the diff is
            // against the empty tree — everything in the commit is new work.
            s.push_str(&format!(
                "Range: (session start)..{head} — this is the first commit, so the \
                 whole diff is this task's work.\n"
            ));
            match get_diff(repo_dir, None, Some(head), task_commit_sha, scope).await {
                Ok(d) => s.push_str(&capped(&d, DIFF_CAP, "diff")),
                Err(e) => s.push_str(&format!("(unavailable: {e})\n")),
            }
        }
        (None, _) => s.push_str("(unavailable: no task commit to diff)\n"),
    }

    // 4. Pre-window state of the touched files — the pre-implementation check.
    s.push_str("\n-- Touched files, as they were BEFORE this task's window --\n");
    match (&base_sha, &resolved_sha) {
        (Some(base), Some(head)) => {
            let paths =
                match get_diff(repo_dir, Some(base), Some(head), task_commit_sha, scope).await {
                    Ok(d) => changed_paths(&d),
                    Err(_) => Vec::new(),
                };
            if paths.is_empty() {
                s.push_str("(no files changed in the window)\n");
            } else {
                let mut budget = FILES_TOTAL_CAP;
                let shown = paths.len().min(MAX_PRE_FILES);
                for path in paths.iter().take(MAX_PRE_FILES) {
                    if budget == 0 {
                        break;
                    }
                    let content = read_file(repo_dir, path, Some(base), task_commit_sha, scope)
                        .await
                        .unwrap_or_else(|e| format!("error: {e}"));
                    let body = if content.starts_with("error:") {
                        "(did not exist before this task)\n".to_string()
                    } else {
                        capped(&content, FILE_CAP.min(budget), "file")
                    };
                    budget = budget.saturating_sub(body.len());
                    s.push_str(&format!("\n--- {path} @ {base} ---\n{body}"));
                }
                if paths.len() > shown {
                    s.push_str(&format!(
                        "\n({} more changed file(s) not shown)\n",
                        paths.len() - shown
                    ));
                }
            }
        }
        _ => {
            s.push_str("(not applicable: no preceding commit — nothing existed before this task)\n")
        }
    }

    // 4b. An empty window means the behaviour arrived in some earlier
    // commit. Show which: the nearest preceding commits that actually
    // changed something, so the innocent reading (built live during an
    // earlier window, or in a wip snapshot) is checkable against the guilty
    // one (present since before the session) instead of being assumed.
    // Empty commits are skipped and counted rather than listed: session
    // 6MTAHS showed the model five empty task-marker commits and a
    // truncation notice, and it read "five empty" as "all empty" — the
    // exonerating diffs sat just past the cut.
    if window_is_empty {
        s.push_str("\n-- Preceding in-session commits that changed something --\n");
        match &commit_log {
            Ok(log) if log.len() >= 2 => {
                let mut budget = PRIOR_DIFFS_TOTAL_CAP;
                let mut shown = 0usize;
                let mut empty_skipped = 0usize;
                let mut not_reached = 0usize;
                // log[0] is the task commit itself; walk the ones before it.
                for i in 1..log.len() {
                    if shown == PRIOR_COMMITS_MAX || budget == 0 {
                        not_reached = log.len() - i;
                        break;
                    }
                    let commit = &log[i];
                    let parent = log.get(i + 1).map(|c| c.sha.as_str());
                    match get_diff(repo_dir, parent, Some(&commit.sha), task_commit_sha, scope)
                        .await
                    {
                        Ok(d) if d.trim().is_empty() => empty_skipped += 1,
                        Ok(d) => {
                            let body = capped(&d, PRIOR_DIFF_CAP.min(budget), "diff");
                            budget = budget.saturating_sub(body.len());
                            s.push_str(&format!("\n--- {} {} ---\n", commit.sha, commit.subject));
                            s.push_str(&body);
                            shown += 1;
                        }
                        Err(e) => {
                            s.push_str(&format!(
                                "\n--- {} {} ---\n(unavailable: {e})\n",
                                commit.sha, commit.subject
                            ));
                            shown += 1;
                        }
                    }
                }
                if empty_skipped > 0 {
                    s.push_str(&format!(
                        "\n({empty_skipped} preceding commit(s) changed nothing and were skipped — \
                         an empty task-marker commit is normal when the work landed earlier)\n"
                    ));
                }
                if shown == 0 && not_reached == 0 {
                    s.push_str(
                        "\nNone of the commits in the log above changed anything: every line the \
                         probes ran was already present at the first commit of the session.\n",
                    );
                }
                if not_reached > 0 {
                    s.push_str(&format!(
                        "\n({not_reached} earlier commit(s) not examined — NOT SEEN, not empty. A \
                         conclusion about where the behaviour arrived cannot rest on them.)\n"
                    ));
                }
            }
            _ => s.push_str("(no preceding commits available)\n"),
        }
    }

    // 5. Client-reported agent activity.
    s.push_str("\n-- Agent activity during this task --\n");
    match task_stats_json {
        Some(stats) if !stats.trim().is_empty() => {
            s.push_str(&capped(stats, FILE_CAP, "stats"));
        }
        _ => s.push_str(
            "(unavailable: the player's CLI reported no statistics for this task. \
             Missing statistics are not evidence of anything.)\n",
        ),
    }

    s.push_str("\n=== END EVIDENCE PACK ===\n");
    s
}

/// Truncate to `cap`, announcing the cut in-band so the judge knows the
/// evidence is partial rather than absent.
fn capped(s: &str, cap: usize, what: &str) -> String {
    if s.len() <= cap {
        let mut out = s.to_string();
        if !out.ends_with('\n') {
            out.push('\n');
        }
        return out;
    }
    let mut cut = cap;
    while cut > 0 && !s.is_char_boundary(cut) {
        cut -= 1;
    }
    format!(
        "{}\n[... {what} truncated at {cap} bytes; {} bytes not shown. Judge only \
         what is visible above.]\n",
        &s[..cut],
        s.len() - cut
    )
}

/// Paths touched by a unified diff, read off the `+++ b/<path>` headers.
fn changed_paths(diff: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in diff.lines() {
        if let Some(rest) = line.strip_prefix("+++ b/") {
            let p = rest.trim();
            if !p.is_empty() && p != "/dev/null" && !out.iter().any(|e| e == p) {
                out.push(p.to_string());
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn changed_paths_reads_diff_headers_without_duplicates() {
        let diff = "diff --git a/src/a.rs b/src/a.rs\n--- a/src/a.rs\n+++ b/src/a.rs\n@@\n+x\n\
                    diff --git a/b.txt b/b.txt\n--- /dev/null\n+++ b/b.txt\n@@\n+y\n\
                    diff --git a/src/a.rs b/src/a.rs\n+++ b/src/a.rs\n";
        assert_eq!(changed_paths(diff), vec!["src/a.rs", "b.txt"]);
    }

    #[test]
    fn capped_announces_truncation_and_keeps_utf8_whole() {
        let long = "é".repeat(100);
        let out = capped(&long, 51, "diff");
        assert!(out.contains("truncated at 51 bytes"), "{out}");
        // Cut must land on a char boundary, so the prefix stays valid UTF-8.
        assert!(out.starts_with(&"é".repeat(25)));
    }

    #[test]
    fn capped_leaves_short_input_alone_but_terminates_it() {
        assert_eq!(capped("abc", 100, "file"), "abc\n");
        assert_eq!(capped("abc\n", 100, "file"), "abc\n");
    }
}
