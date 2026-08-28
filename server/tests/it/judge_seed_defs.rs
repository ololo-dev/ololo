//! The judge definitions this repo ships are parsed here, not only at boot.
//!
//! A judge is delivered as a file — seeded on start, pushed with
//! `push-seeds` — so a broken one is not a compile error anywhere. Since a
//! judge may now carry its own `js decide` program, a typo in it would
//! otherwise surface as a failed judge run per player per task, in
//! production, long after the edit.

use std::path::PathBuf;

use server::seed::judges::load_judge_defs;

fn judges_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root")
        .join("judges")
}

#[test]
fn every_shipped_judge_definition_parses() {
    let (defs, errors) = load_judge_defs(&judges_dir());
    assert!(
        errors.is_empty(),
        "judge definitions failed to parse: {errors:?}"
    );
    assert!(!defs.is_empty(), "no judge definitions found");
}

#[test]
fn the_anti_cheat_program_is_lifted_out_of_its_prompt() {
    // The program is harness instruction. If it stayed in the prompt, the
    // model would read a decision that was already made without it — and the
    // prompt it *does* read must still be the whole prompt.
    let (defs, _) = load_judge_defs(&judges_dir());
    let Some((_, def)) = defs.iter().find(|(_, d)| d.slug == "task-anti-cheat") else {
        return; // this checkout ships a subset of the judge library
    };

    let (prompt, programs) = arena_core::judging::programs::split_programs(&def.prompt);
    let program = programs.decide.expect("task-anti-cheat carries a program");
    assert!(program.contains("diff_is_empty"));
    assert!(
        !prompt.contains("js decide") && !prompt.contains("return ask"),
        "the program leaked into the prompt"
    );
    assert!(
        prompt.starts_with("You are an anti-cheat judge"),
        "prompt starts with: {:?}",
        &prompt[..prompt.len().min(60)]
    );
    assert!(
        prompt.contains("Evidence discipline"),
        "prompt was truncated"
    );
}

#[test]
fn the_shipped_judges_declare_what_they_read() {
    // The snapshot is not free — the cross-task view walks git twice per task
    // the player reached. A judge that reads none of it should say so, and a
    // judge whose programs read part of it must not have that part silently
    // withheld.
    let (defs, _) = load_judge_defs(&judges_dir());
    // A checkout ships a subset of the judge library, so an absent slug is
    // skipped, not failed; `check` compares only when the file exists.
    let shipped: std::collections::HashMap<&str, Option<String>> = defs
        .iter()
        .map(|(_, d)| (d.slug.as_str(), d.evidence_needs.clone()))
        .collect();
    let check = |slug: &str, expected: Option<&str>| {
        if let Some(needs) = shipped.get(slug) {
            assert_eq!(needs.as_deref(), expected, "{slug}");
        }
    };

    // Its `decide` reads task.probes and tasks[]; its `review` reads
    // tasks[].commit_sha. It reads no session memory.
    check("task-anti-cheat", Some(r#"["tasks","probes"]"#));

    // Execution judges re-run probes in a sandbox and carry no program:
    // nothing reads the snapshot, so nothing is assembled for them.
    check("golf-verify", Some("[]"));

    // The open-ended panel judges read exactly the slices their prompts
    // ground themselves in: probe measurements, plus session memory for the
    // judge that verifies participant-declared contracts (correctness).
    check("correctness", Some(r#"["probes","memory","images"]"#));
    // The single-reviewer judge stands in for that whole panel on a small
    // build, so it reads everything the panel reads between them.
    check("build-review", Some(r#"["probes","memory","images"]"#));
    // The workflow judge runs once, after the session, over an evidence pack
    // of its own: the agent configuration in the final snapshot joined with
    // the tool and skill histograms the player's CLI reported. It reads none
    // of the per-task sections.
    check("agentic", Some(r#"["agent_setup"]"#));
    check("architecture", Some(r#"["probes"]"#));
    check("data", Some(r#"["probes"]"#));
    check("code-quality", Some(r#"["probes"]"#));
    check("test-quality", Some(r#"["probes"]"#));

    // Its `decide` reads task.probes (the nothing-passed skip); its prompt
    // reads the repo through git tools, which need no declaration.
    check("from-scratch", Some(r#"["probes"]"#));

    // The vision judges also declare `images`: visual artifacts attach only
    // to judges that look at pixels — a text-only model 400s on them.
    check("ux-review", Some(r#"["probes","images"]"#));
    check("creativity", Some(r#"["probes","images"]"#));

    // Everything else is undeclared and keeps the whole snapshot, exactly as
    // before declarations existed.
    const DECLARED: [&str; 14] = [
        "build-review",
        "creativity",
        // The campaign's quality panel: both read probe evidence and the
        // repository through git tools, like their siblings above.
        "performance",
        "governance",
        "task-anti-cheat",
        "golf-verify",
        "ux-review",
        "correctness",
        "agentic",
        "architecture",
        "data",
        "code-quality",
        "test-quality",
        "from-scratch",
    ];
    for (path, def) in &defs {
        if DECLARED.contains(&def.slug.as_str()) {
            continue;
        }
        assert!(
            def.evidence_needs.is_none(),
            "{} declared needs without this test knowing: {:?}",
            path.display(),
            def.evidence_needs
        );
    }
}

#[test]
fn a_typo_in_a_declaration_is_rejected_at_delivery() {
    // The alternative is a judge reasoning from a section it asked for and
    // did not get.
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(
        dir.path().join("typo-judge.md"),
        "---\nname: Typo\nrating_scale: {min: 0, max: 10, step: 1}\nneeds: [tasks, diffs]\n---\nJudge it.\n",
    )
    .expect("write judge");

    let (defs, errors) = load_judge_defs(dir.path());
    assert!(
        defs.is_empty(),
        "a judge with a bad declaration must not load"
    );
    assert_eq!(errors.len(), 1);
    assert!(
        errors[0].1.contains("diffs"),
        "the error names the section: {}",
        errors[0].1
    );
}

#[test]
fn the_from_scratch_judge_is_blind_to_the_platform_tree() {
    // Its one question is whether the player implemented the tool or wrapped
    // the real one, and the answer is in their sources. `.ololo/` is the
    // platform's own tree inside the snapshot — probe scratch, delivered
    // artifacts — and in a five-part campaign it outnumbered the code 468
    // files to 6, so every listing came back as noise it then paid to carry.
    let (defs, _) = load_judge_defs(&judges_dir());
    let Some((_, def)) = defs.iter().find(|(_, d)| d.slug == "from-scratch") else {
        return; // this checkout ships a subset of the judge library
    };
    assert_eq!(def.ignore_paths.as_deref(), Some(r#"[".ololo/"]"#));
}

#[test]
fn a_judge_that_reads_artifacts_keeps_the_whole_snapshot() {
    // The blind spot is per judge on purpose: the UX review's evidence *is*
    // what was delivered under `.ololo/`.
    let (defs, _) = load_judge_defs(&judges_dir());
    for slug in ["ux-review", "correctness", "build-review"] {
        let Some((_, def)) = defs.iter().find(|(_, d)| d.slug == slug) else {
            continue; // this checkout ships a subset of the judge library
        };
        assert!(
            def.ignore_paths.is_none(),
            "{slug} must still see the delivered artifacts"
        );
    }
}
