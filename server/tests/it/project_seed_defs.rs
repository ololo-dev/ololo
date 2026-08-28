//! The challenge projects this repo ships are parsed here, not only at boot.
//!
//! Projects are delivered as files — seeded on start, pushed with
//! `push-seeds` — so a broken one is not a compile error anywhere. The rules
//! asserted below are the ones that would otherwise be re-broken silently by
//! the next edit.

use std::path::PathBuf;

use server::seed::load_sources;

fn projects_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root")
        .join("projects")
}

fn code_golf(env: &server::api::admin_export_import::ExportEnvelope) -> bool {
    env.project.category.as_deref() == Some("Code Golf")
}

#[test]
fn every_shipped_project_parses() {
    let sources = load_sources(&projects_dir());
    assert!(!sources.is_empty(), "no project definitions found");
    // The loader silently skips what it cannot parse, so compare against
    // what is actually on disk rather than a hardcoded count — a definition
    // that stopped loading must not disappear from the seed unnoticed.
    let on_disk = std::fs::read_dir(projects_dir())
        .expect("read projects dir")
        .flatten()
        .filter(|e| {
            let p = e.path();
            p.join("readme.md").is_file() || p.extension().is_some_and(|x| x == "json")
        })
        .count();
    assert_eq!(
        sources.len(),
        on_disk,
        "{} project sources on disk but only {} parsed — did one stop loading?",
        on_disk,
        sources.len()
    );
}

#[test]
fn a_code_golf_task_is_judged_by_the_ladder_alone() {
    // Code golf is verified by re-running the task's own probes against the
    // committed solution. An anti-cheat judge on top reads every rung a
    // player already won as an empty commit, which is what winning early
    // looks like — the strongest player collects the most penalties.
    for (path, env) in load_sources(&projects_dir()) {
        if !code_golf(&env) {
            continue;
        }
        for task in &env.tasks {
            let offenders: Vec<&str> = task
                .judges
                .iter()
                .map(|j| j.slug())
                .filter(|s| s.contains("anti-cheat") || s.contains("anti-cheater"))
                .collect();
            assert!(
                offenders.is_empty(),
                "{}: task {} ({}) attaches {:?} — code golf is judged by golf-verify alone",
                path.display(),
                task.ordinal,
                task.title,
                offenders
            );
        }
    }
}

#[test]
fn a_golf_rung_counts_the_run_command_and_the_docs_it_names() {
    // The byte budget is the game. Documentation is excluded from it, so a
    // solution that lives in the run line — or in an AGENTS.md the run line
    // invokes — would be free. Both are counted instead, deterministically,
    // rather than left to a judge to notice.
    //
    // Asserted against the markdown itself: the probe shell is what ships and
    // what runs, and the parsed envelope does not carry the per-section
    // commands.
    let mut rungs = 0;
    for (path, env) in load_sources(&projects_dir()) {
        if !code_golf(&env) {
            continue;
        }
        // A source is either the project directory or the file inside it.
        let root = if path.is_dir() {
            path.clone()
        } else {
            path.parent().expect("project dir").to_path_buf()
        };
        let dir = root.join("tasks");
        let entries = std::fs::read_dir(&dir).unwrap_or_else(|e| {
            panic!("{}: cannot read tasks dir: {e}", dir.display());
        });
        for entry in entries.flatten() {
            let file = entry.path();
            let name = file.file_name().and_then(|s| s.to_str()).unwrap_or("");
            if !name.contains(".golf-") {
                continue; // the declare-and-implement task has no budget yet
            }
            rungs += 1;
            let body = std::fs::read_to_string(&file).expect("read task");
            assert!(
                body.contains(r#"printf %s "$R" | wc -c"#),
                "{}: the size probe does not count the run command",
                file.display()
            );
            assert!(
                body.contains("*AGENTS.md*") && body.contains("*README.md*"),
                "{}: the size probe does not count a doc file the run line names",
                file.display()
            );
        }
    }
    // Coverage floor, not an inventory: checkouts ship different subsets of
    // the golf projects, but at least one must be present to keep the gate
    // meaningful.
    assert!(rungs > 0, "no golf rungs found to check");
}

#[test]
fn every_campaign_parent_names_parts_that_actually_ship() {
    // A parent's `parts:` list is resolved by slug at seed time. A typo there
    // silently drops a chapter at boot (warn-and-skip) — a shipped campaign
    // must never be one restart away from losing a part.
    let sources = load_sources(&projects_dir());
    let shipped: std::collections::HashSet<String> = sources
        .iter()
        .filter_map(|(_, env)| env.project.slug.clone())
        .collect();
    let campaigns: Vec<_> = sources
        .iter()
        .filter(|(_, env)| !env.project.parts.is_empty())
        .collect();

    let mut claimed: std::collections::HashMap<&str, &str> = std::collections::HashMap::new();
    for (path, env) in &campaigns {
        let parent_slug = env.project.slug.as_deref().unwrap_or("<no slug>");
        assert!(
            env.tasks.is_empty(),
            "{}: a campaign parent hosts no sessions, so its tasks would never run",
            path.display()
        );
        let mut seen = std::collections::HashSet::new();
        for part in &env.project.parts {
            assert!(
                shipped.contains(part),
                "{}: parts names '{part}', which no shipped project provides",
                path.display()
            );
            assert!(
                seen.insert(part.as_str()),
                "{}: parts lists '{part}' twice",
                path.display()
            );
            if let Some(other) = claimed.insert(part.as_str(), parent_slug) {
                panic!("'{part}' is claimed by both '{other}' and '{parent_slug}'");
            }
        }
    }
}
