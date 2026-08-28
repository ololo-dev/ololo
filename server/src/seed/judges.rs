//! Startup judge seeding. Scans `ARENA_JUDGES_DIR` (default `./judges`) for
//! `*.md` files with YAML frontmatter. Inserts judges whose slug is not yet
//! in the DB, and refreshes existing rows whose content (name, description,
//! prompt, rating_scale) differs from the file — the markdown files are the
//! source of truth on restart; admin UI edits survive only until the file
//! changes. Log-and-continue on per-file failure.

use arena_core::entities::judges;
use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, Set};
use serde::Deserialize;
use uuid::Uuid;

pub(crate) use super::markdown::split_frontmatter;

#[derive(Debug, Deserialize)]
struct JudgeFrontmatter {
    name: String,
    #[serde(default)]
    description: Option<String>,
    rating_scale: serde_json::Value,
    /// Judge type: "llm" (default) or "execution".
    #[serde(default)]
    kind: Option<String>,
    /// Judge scope: "task" (default) or "session".
    #[serde(default)]
    scope: Option<String>,
    /// How the judge gets its facts: "tools" (default) or "dossier".
    #[serde(default)]
    evidence: Option<String>,
    /// What the judge's programs need to see, e.g. `[tasks, probes]`. Absent
    /// means undeclared, and the judge keeps the whole snapshot.
    #[serde(default)]
    needs: Option<Vec<String>>,
    /// Open-ended criteria keys this judge scores, e.g. `[architecture]`.
    #[serde(default)]
    criteria: Option<Vec<String>>,
    /// Judge-declared probes (YAML, bridged to `JudgeProbeDef` JSON).
    #[serde(default)]
    probes: Option<serde_yaml::Value>,
    /// How many interactive probes this judge may register per task.
    #[serde(default)]
    max_interactive: Option<i32>,
    /// Repo paths this judge's git tools must not open, e.g. `[".ololo/"]`.
    #[serde(default)]
    ignore_paths: Option<Vec<String>>,
}

/// A judge definition parsed and validated from a `judges/*.md` seed file.
/// Field semantics match the `judges` table row the boot seed writes.
#[derive(Debug, Clone)]
pub struct JudgeSeedDef {
    pub slug: String,
    pub name: String,
    pub description: String,
    pub prompt: String,
    pub rating_scale: serde_json::Value,
    pub kind: String,
    pub scope: String,
    pub evidence_mode: String,
    /// JSON array for `judges.evidence_needs`; `None` when undeclared.
    pub evidence_needs: Option<String>,
    /// JSON array for `judges.criteria`; `None` when the judge scores no
    /// per-criterion sheet.
    pub criteria: Option<String>,
    /// JSON array of `JudgeProbeDef` for `judges.probes_config`.
    pub probes_config: Option<serde_json::Value>,
    /// `judges.max_interactive`; `None` = may register none.
    pub max_interactive: Option<i32>,
    /// JSON array for `judges.ignore_paths`; `None` = the judge sees the
    /// whole snapshot.
    pub ignore_paths: Option<String>,
}

/// Judge defs parsed from a directory, paired with their source paths.
pub type LoadedJudgeDefs = Vec<(std::path::PathBuf, JudgeSeedDef)>;
/// Per-file parse failures: `(path, reason)`.
pub type JudgeParseErrors = Vec<(std::path::PathBuf, String)>;

/// Parse every `*.md` judge definition in `dir`. Returns the valid defs plus
/// per-file failures — the boot seed logs-and-skips those, the push-seeds
/// command reports them to the operator.
pub fn load_judge_defs(dir: &std::path::Path) -> (LoadedJudgeDefs, JudgeParseErrors) {
    let mut defs = Vec::new();
    let mut errors = Vec::new();
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) => {
            errors.push((dir.to_path_buf(), format!("cannot read judges dir: {e}")));
            return (defs, errors);
        }
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("md") {
            continue;
        }
        match parse_judge_file(&path) {
            Ok(def) => defs.push((path, def)),
            Err(reason) => errors.push((path, reason)),
        }
    }
    // Deterministic order regardless of directory iteration.
    defs.sort_by(|a, b| a.1.slug.cmp(&b.1.slug));
    (defs, errors)
}

fn parse_judge_file(path: &std::path::Path) -> Result<JudgeSeedDef, String> {
    let label = path.file_name().and_then(|s| s.to_str()).unwrap_or("judge");
    let file_stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .ok_or_else(|| "filename has no stem".to_string())?;
    let slug = crate::api::projects::validate_slug(file_stem)
        .map_err(|_| format!("invalid judge slug '{file_stem}'"))?;

    let src = std::fs::read_to_string(path).map_err(|e| format!("read failed: {e}"))?;
    let (fm, body) = split_frontmatter(&src, label).map_err(|e| format!("frontmatter: {e}"))?;
    let fm: JudgeFrontmatter =
        serde_yaml::from_str(fm).map_err(|e| format!("frontmatter parse: {e}"))?;
    if fm.name.is_empty() {
        return Err("judge name empty".to_string());
    }
    if !fm.rating_scale.is_object() {
        return Err("rating_scale not an object".to_string());
    }
    arena_core::validation::judges::validate_rating_scale(&fm.rating_scale)
        .map_err(|e| format!("rating_scale invalid: {e}"))?;
    let kind = fm
        .kind
        .unwrap_or_else(|| arena_core::judging::JUDGE_KIND_LLM.to_string());
    if kind != arena_core::judging::JUDGE_KIND_LLM
        && kind != arena_core::judging::JUDGE_KIND_EXECUTION
        && kind != arena_core::judging::JUDGE_KIND_REPORT
    {
        return Err(format!("unknown judge kind '{kind}'"));
    }
    let scope = fm.scope.unwrap_or_else(|| "task".to_string());
    if scope != "task" && scope != "session" {
        return Err(format!("unknown judge scope '{scope}'"));
    }
    // A session-scoped execution judge has no defined meaning: execution
    // judges re-run one task's probes against that task's commit.
    if scope == "session" && kind == arena_core::judging::JUDGE_KIND_EXECUTION {
        return Err("session-scoped execution judge is not supported".to_string());
    }
    // A report is written about a session. A task-scoped one would have
    // nothing to say that the task's own judges have not already said.
    if kind == arena_core::judging::JUDGE_KIND_REPORT && scope != "session" {
        return Err("a report judge must be session-scoped".to_string());
    }
    let evidence_mode = fm
        .evidence
        .unwrap_or_else(|| arena_core::judging::EVIDENCE_MODE_TOOLS.to_string());
    if evidence_mode != arena_core::judging::EVIDENCE_MODE_TOOLS
        && evidence_mode != arena_core::judging::EVIDENCE_MODE_DOSSIER
    {
        return Err(format!("unknown judge evidence mode '{evidence_mode}'"));
    }
    // The dossier pack is built per task, from that task's commit window.
    if evidence_mode == arena_core::judging::EVIDENCE_MODE_DOSSIER && scope != "task" {
        return Err("dossier evidence is only defined for task-scoped judges".to_string());
    }
    // A judge may carry its own program alongside its prompt. Parse it here,
    // where the file is accepted: a syntax error caught at delivery costs the
    // author one line of output, and caught at run time costs one failed
    // judge run per player per task.
    let programs = arena_core::judging::programs::split_programs(body).1;
    for (fence, program) in [("decide", &programs.decide), ("review", &programs.review)] {
        if let Some(program) = program {
            arena_core::judging::programs::validate_program(program)
                .map_err(|e| format!("`js {fence}` program does not parse: {e}"))?;
        }
    }
    // What this judge needs to see. Validated here so a typo is one line of
    // output at delivery rather than a judge that quietly reasons from an
    // empty `tasks[]` it never asked for.
    let evidence_needs = match &fm.needs {
        None => None,
        Some(sections) => {
            let needs = arena_core::judging::evidence::EvidenceNeeds::parse(sections)
                .map_err(|e| format!("needs: {e}"))?;
            Some(needs.to_column())
        }
    };
    // Open-ended criteria keys: non-empty, unique, stored as a JSON array.
    let criteria = match &fm.criteria {
        None => None,
        Some(keys) => {
            let mut seen = std::collections::BTreeSet::new();
            for key in keys {
                if key.trim().is_empty() {
                    return Err("criteria: empty key".to_string());
                }
                if !seen.insert(key.trim().to_string()) {
                    return Err(format!("criteria: duplicate key '{key}'"));
                }
            }
            Some(serde_json::to_string(keys).map_err(|e| format!("criteria: {e}"))?)
        }
    };
    // Judge-declared probes: bridge YAML → JSON, then validate per mode with
    // the same rules task sections get.
    let probes_config = match fm.probes {
        None => None,
        Some(yaml) => {
            let json: serde_json::Value =
                serde_yaml::from_value(yaml).map_err(|e| format!("probes: {e}"))?;
            let defs: Vec<arena_core::evaluation::JudgeProbeDef> =
                serde_json::from_value(json.clone()).map_err(|e| format!("probes: {e}"))?;
            arena_core::evaluation::validate_judge_probes(&defs)
                .map_err(|e| format!("probes: {e}"))?;
            Some(json)
        }
    };
    if let Some(n) = fm.max_interactive
        && n < 0
    {
        return Err("max_interactive must be ≥ 0".to_string());
    }

    // Blind spots are path prefixes, not globs: the tools match by prefix, so
    // a `*` here would silently never match and the judge would keep paying
    // for the tree it meant to skip.
    let ignore_paths = match fm.ignore_paths {
        None => None,
        Some(list) => {
            let cleaned: Vec<String> = list
                .into_iter()
                .map(|p| p.trim().to_string())
                .filter(|p| !p.is_empty())
                .collect();
            if let Some(bad) = cleaned
                .iter()
                .find(|p| p.contains('*') || p.starts_with('/'))
            {
                return Err(format!(
                    "ignore_paths: '{bad}' — use a repo-relative path prefix like '.ololo/', not a glob or absolute path"
                ));
            }
            if cleaned.is_empty() {
                None
            } else {
                Some(serde_json::to_string(&cleaned).map_err(|e| format!("ignore_paths: {e}"))?)
            }
        }
    };
    Ok(JudgeSeedDef {
        slug,
        name: fm.name,
        evidence_needs,
        criteria,
        probes_config,
        max_interactive: fm.max_interactive,
        ignore_paths,
        description: fm.description.unwrap_or_default(),
        prompt: body.to_string(),
        rating_scale: fm.rating_scale,
        kind,
        scope,
        evidence_mode,
    })
}

pub async fn seed_judges_with_dir(
    db: &DatabaseConnection,
    dir: &std::path::Path,
) -> Result<(usize, usize, usize), Box<dyn std::error::Error + Send + Sync>> {
    if !dir.is_dir() {
        tracing::warn!(dir = %dir.display(), "seed: judges dir missing or not a directory, skipping");
        return Ok((0, 0, 0));
    }

    let (defs, parse_errors) = load_judge_defs(dir);

    let mut inserted = 0usize;
    let mut updated = 0usize;
    let mut skipped = 0usize;
    for (path, reason) in &parse_errors {
        tracing::warn!(file = %path.display(), reason = %reason, "seed: judge file skipped");
        skipped += 1;
    }

    for (path, def) in defs {
        match judges::Entity::find()
            .filter(judges::Column::Slug.eq(&def.slug))
            .one(db)
            .await
        {
            Ok(Some(existing)) => {
                let unchanged = existing.name == def.name
                    && existing.description == def.description
                    && existing.prompt == def.prompt
                    && existing.rating_scale == def.rating_scale
                    && existing.kind == def.kind
                    && existing.scope == def.scope
                    && existing.evidence_mode == def.evidence_mode
                    && existing.evidence_needs == def.evidence_needs
                    && existing.criteria == def.criteria
                    && existing.probes_config == def.probes_config
                    && existing.max_interactive == def.max_interactive
                    && existing.ignore_paths == def.ignore_paths;
                if unchanged {
                    tracing::info!(slug = %def.slug, "seed: judge unchanged, skipping");
                    skipped += 1;
                } else {
                    // File differs from DB → refresh the row. The markdown
                    // files are the source of truth on restart.
                    let mut am: judges::ActiveModel = existing.into();
                    am.name = Set(def.name.clone());
                    am.description = Set(def.description.clone());
                    am.prompt = Set(def.prompt.clone());
                    am.rating_scale = Set(def.rating_scale.clone());
                    am.kind = Set(def.kind.clone());
                    am.scope = Set(def.scope.clone());
                    am.evidence_mode = Set(def.evidence_mode.clone());
                    am.evidence_needs = Set(def.evidence_needs.clone());
                    am.criteria = Set(def.criteria.clone());
                    am.probes_config = Set(def.probes_config.clone());
                    am.max_interactive = Set(def.max_interactive);
                    am.ignore_paths = Set(def.ignore_paths.clone());
                    am.updated_at = Set(chrono::Utc::now());
                    match judges::Entity::update(am).exec(db).await {
                        Ok(_) => {
                            tracing::info!(slug = %def.slug, "seed: refreshed judge from file");
                            updated += 1;
                        }
                        Err(e) => {
                            tracing::warn!(file = %path.display(), error = %e, "seed: judge update failed, skipping");
                            skipped += 1;
                        }
                    }
                }
            }
            Ok(None) => {
                let now = chrono::Utc::now();
                let am = judges::ActiveModel {
                    id: Set(Uuid::new_v4()),
                    slug: Set(def.slug.clone()),
                    name: Set(def.name.clone()),
                    description: Set(def.description.clone()),
                    prompt: Set(def.prompt.clone()),
                    rating_scale: Set(def.rating_scale.clone()),
                    kind: Set(def.kind.clone()),
                    scope: Set(def.scope.clone()),
                    evidence_mode: Set(def.evidence_mode.clone()),
                    evidence_needs: Set(def.evidence_needs.clone()),
                    criteria: Set(def.criteria.clone()),
                    probes_config: Set(def.probes_config.clone()),
                    max_interactive: Set(def.max_interactive),
                    avatar_url: Set(None),
                    ignore_paths: Set(def.ignore_paths.clone()),
                    llm_provider_id_fk: Set(None),
                    llm_model: Set(None),
                    llm_pool_id_fk: Set(None),
                    llm_source_order: Set(
                        arena_core::llm::resolve::SOURCE_ORDER_POOL_FIRST.to_string()
                    ),
                    created_at: Set(now),
                    updated_at: Set(now),
                };
                match judges::Entity::insert(am).exec(db).await {
                    Ok(_) => {
                        tracing::info!(slug = %def.slug, "seed: inserted judge");
                        inserted += 1;
                    }
                    Err(e) => {
                        tracing::warn!(file = %path.display(), error = %e, "seed: judge insert failed, skipping");
                        skipped += 1;
                    }
                }
            }
            Err(e) => {
                tracing::warn!(file = %path.display(), error = %e, "seed: judge slug lookup failed, skipping");
                skipped += 1;
            }
        }
    }

    tracing::info!(inserted, updated, skipped, "seed: judges done");
    Ok((inserted, updated, skipped))
}

pub async fn seed_judges(
    db: &DatabaseConnection,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let dir = std::env::var("ARENA_JUDGES_DIR").unwrap_or_else(|_| "./judges".to_string());
    let _ = seed_judges_with_dir(db, std::path::Path::new(&dir)).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use migration::{Migrator, MigratorTrait};
    use sea_orm::{Database, PaginatorTrait};

    async fn fresh_db() -> DatabaseConnection {
        let db = Database::connect("sqlite::memory:").await.expect("db");
        Migrator::up(&db, None).await.expect("migrate");
        db
    }

    fn tmp_dir() -> std::path::PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!("arena-seed-judges-test-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&p).expect("mkdir");
        p
    }

    const VALID: &str = concat!(
        "---\n",
        "name: Code Cleanliness\n",
        "description: Reads cleanly.\n",
        "rating_scale: {min: 0.0, max: 10.0, step: 0.5}\n",
        "---\n",
        "Rate the readability and structure of the submitted code.\n",
        "Prefer shorter clear code over long clever code.\n",
        "Penalize deep nesting and missing names.\n",
        "Return only a numeric score on the scale.\n",
    );

    const DUPE: &str = concat!(
        "---\n",
        "name: Code Cleanliness Duplicate\n",
        "rating_scale: {min: 0.0, max: 10.0, step: 0.5}\n",
        "---\n",
        "Body for dupe.\n",
    );

    const BAD_SCALE: &str = concat!(
        "---\n",
        "name: Bad Scale\n",
        "rating_scale: {min: 5.0, max: 0.0, step: 1.0}\n",
        "---\n",
        "Should be skipped.\n",
    );

    const NO_NAME: &str = concat!(
        "---\n",
        "name: ''\n",
        "rating_scale: {min: 0.0, max: 10.0, step: 0.5}\n",
        "---\n",
        "Should be skipped.\n",
    );

    const NO_FM: &str = "no frontmatter here";

    #[tokio::test]
    async fn imports_valid_skips_malformed_and_dupes() {
        let db = fresh_db().await;
        let dir = tmp_dir();

        std::fs::write(dir.join("code-cleanliness.md"), VALID).unwrap();
        std::fs::write(dir.join("bad-scale.md"), BAD_SCALE).unwrap();
        std::fs::write(dir.join("no-name.md"), NO_NAME).unwrap();
        std::fs::write(dir.join("no-fm.md"), NO_FM).unwrap();

        let (inserted, updated, skipped) = seed_judges_with_dir(&db, &dir).await.expect("seed run");
        assert_eq!(inserted, 1, "one valid judge inserted");
        assert_eq!(updated, 0);
        assert_eq!(skipped, 3, "bad-scale, no-name, no-fm skipped");

        let count = judges::Entity::find().count(&db).await.unwrap();
        assert_eq!(count, 1);

        let row = judges::Entity::find()
            .filter(judges::Column::Slug.eq("code-cleanliness"))
            .one(&db)
            .await
            .unwrap()
            .expect("row");
        assert_eq!(row.name, "Code Cleanliness");
        assert_eq!(row.description, "Reads cleanly.");
        assert!(row.prompt.contains("Rate the readability"));
        assert_eq!(
            row.rating_scale,
            serde_json::json!({"min": 0.0, "max": 10.0, "step": 0.5})
        );

        std::fs::write(dir.join("code-cleanliness-dupe.md"), DUPE).unwrap();
        std::fs::remove_file(dir.join("no-fm.md")).unwrap();
        std::fs::remove_file(dir.join("no-name.md")).unwrap();
        std::fs::remove_file(dir.join("bad-scale.md")).unwrap();

        let (inserted2, updated2, skipped2) =
            seed_judges_with_dir(&db, &dir).await.expect("seed rerun");
        assert_eq!(inserted2, 1, "dupe file has a different slug, inserts");
        assert_eq!(updated2, 0, "unchanged file does not update");
        assert_eq!(skipped2, 1, "original slug skipped on rerun");

        let count2 = judges::Entity::find().count(&db).await.unwrap();
        assert_eq!(count2, 2);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn missing_dir_is_silent() {
        let db = fresh_db().await;
        let (inserted, updated, skipped) =
            seed_judges_with_dir(&db, std::path::Path::new("/nonexistent/arena/judges/dir"))
                .await
                .expect("ok");
        assert_eq!(inserted, 0);
        assert_eq!(updated, 0);
        assert_eq!(skipped, 0);
        assert_eq!(judges::Entity::find().count(&db).await.unwrap(), 0);
    }

    #[tokio::test]
    async fn changed_file_refreshes_existing_judge() {
        let db = fresh_db().await;
        let dir = tmp_dir();

        std::fs::write(dir.join("code-cleanliness.md"), VALID).unwrap();
        let (inserted, _, _) = seed_judges_with_dir(&db, &dir).await.expect("seed");
        assert_eq!(inserted, 1);

        // Same slug, changed prompt body + description.
        const CHANGED: &str = concat!(
            "---\n",
            "name: Code Cleanliness\n",
            "description: Reads cleanly, now with tools.\n",
            "rating_scale: {min: 0.0, max: 10.0, step: 0.5}\n",
            "---\n",
            "Rate readability. Also call get_task_stats for context.\n",
        );
        std::fs::write(dir.join("code-cleanliness.md"), CHANGED).unwrap();

        let (inserted2, updated2, skipped2) =
            seed_judges_with_dir(&db, &dir).await.expect("reseed");
        assert_eq!(inserted2, 0);
        assert_eq!(updated2, 1, "changed file refreshes the row");
        assert_eq!(skipped2, 0);

        let row = judges::Entity::find()
            .filter(judges::Column::Slug.eq("code-cleanliness"))
            .one(&db)
            .await
            .unwrap()
            .expect("row");
        assert!(row.prompt.contains("get_task_stats"));
        assert_eq!(row.description, "Reads cleanly, now with tools.");

        // Idempotent: reseeding the same file changes nothing.
        let (_, updated3, skipped3) = seed_judges_with_dir(&db, &dir).await.expect("reseed2");
        assert_eq!(updated3, 0);
        assert_eq!(skipped3, 1);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn evidence_mode_defaults_to_tools_and_accepts_dossier() {
        let db = fresh_db().await;
        let dir = tmp_dir();

        // No `evidence:` key → the agentic default, so existing judge files
        // keep their behaviour.
        std::fs::write(dir.join("code-cleanliness.md"), VALID).unwrap();
        // Explicit dossier mode on a task-scoped judge.
        std::fs::write(
            dir.join("one-shot.md"),
            concat!(
                "---\n",
                "name: One Shot\n",
                "rating_scale: {min: -10.0, max: 0.0, step: 1.0}\n",
                "evidence: dossier\n",
                "---\n",
                "Judge from the evidence pack.\n",
            ),
        )
        .unwrap();
        // Dossier is only defined per task, so a session-scoped one is refused
        // rather than silently running without its pack.
        std::fs::write(
            dir.join("bad-combo.md"),
            concat!(
                "---\n",
                "name: Bad Combo\n",
                "rating_scale: {min: -10.0, max: 0.0, step: 1.0}\n",
                "scope: session\n",
                "evidence: dossier\n",
                "---\n",
                "Should be skipped.\n",
            ),
        )
        .unwrap();

        let (inserted, _, skipped) = seed_judges_with_dir(&db, &dir).await.expect("seed");
        assert_eq!(inserted, 2, "the two valid judges insert");
        assert_eq!(skipped, 1, "session+dossier is rejected");

        let tools_judge = judges::Entity::find()
            .filter(judges::Column::Slug.eq("code-cleanliness"))
            .one(&db)
            .await
            .unwrap()
            .expect("row");
        assert_eq!(tools_judge.evidence_mode, "tools");

        let dossier_judge = judges::Entity::find()
            .filter(judges::Column::Slug.eq("one-shot"))
            .one(&db)
            .await
            .unwrap()
            .expect("row");
        assert_eq!(dossier_judge.evidence_mode, "dossier");

        // Flipping the mode in the file refreshes the row like any other field.
        std::fs::write(
            dir.join("one-shot.md"),
            concat!(
                "---\n",
                "name: One Shot\n",
                "rating_scale: {min: -10.0, max: 0.0, step: 1.0}\n",
                "---\n",
                "Judge from the evidence pack.\n",
            ),
        )
        .unwrap();
        let (_, updated, _) = seed_judges_with_dir(&db, &dir).await.expect("reseed");
        assert_eq!(updated, 1, "evidence_mode change is picked up");
        let back_to_tools = judges::Entity::find()
            .filter(judges::Column::Slug.eq("one-shot"))
            .one(&db)
            .await
            .unwrap()
            .expect("row");
        assert_eq!(back_to_tools.evidence_mode, "tools");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn a_declared_blind_spot_reaches_the_row_and_a_reseed_updates_it() {
        let db = fresh_db().await;
        let dir = std::env::temp_dir().join(format!("judges-blind-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("scoped.md");
        std::fs::write(
            &file,
            concat!(
                "---\n",
                "name: Scoped\n",
                "rating_scale: {min: 0.0, max: 10.0, step: 1.0}\n",
                "ignore_paths: [\".ololo/\", \"vendor/\"]\n",
                "---\n",
                "Judge the player's own code.\n",
            ),
        )
        .unwrap();
        seed_judges_with_dir(&db, &dir).await.expect("seed");
        let row = judges::Entity::find()
            .filter(judges::Column::Slug.eq("scoped"))
            .one(&db)
            .await
            .unwrap()
            .expect("row");
        assert_eq!(
            row.ignore_paths.as_deref(),
            Some(r#"[".ololo/","vendor/"]"#)
        );

        // Dropping the declaration gives the whole snapshot back — a blind
        // spot that could not be removed would be a trap.
        std::fs::write(
            &file,
            concat!(
                "---\n",
                "name: Scoped\n",
                "rating_scale: {min: 0.0, max: 10.0, step: 1.0}\n",
                "---\n",
                "Judge the player's own code.\n",
            ),
        )
        .unwrap();
        let (_, updated, _) = seed_judges_with_dir(&db, &dir).await.expect("reseed");
        assert_eq!(updated, 1, "the change is noticed");
        let row = judges::Entity::find()
            .filter(judges::Column::Slug.eq("scoped"))
            .one(&db)
            .await
            .unwrap()
            .expect("row");
        assert_eq!(row.ignore_paths, None);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_glob_is_rejected_rather_than_silently_matching_nothing() {
        let dir = std::env::temp_dir().join(format!("judges-glob-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("globby.md"),
            concat!(
                "---\n",
                "name: Globby\n",
                "rating_scale: {min: 0.0, max: 10.0, step: 1.0}\n",
                "ignore_paths: [\".ololo/**\"]\n",
                "---\n",
                "Judge.\n",
            ),
        )
        .unwrap();
        let (defs, errors) = load_judge_defs(&dir);
        assert!(defs.is_empty());
        assert!(
            errors[0].1.contains("ignore_paths"),
            "the reason names the field: {errors:?}"
        );
        std::fs::remove_dir_all(&dir).ok();
    }
}
