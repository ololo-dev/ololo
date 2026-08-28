//! Startup project seeding.
//!
//! Scans a folder for project definitions and inserts any project whose slug
//! is not already present in the DB. Two on-disk formats are supported:
//!
//! - **JSON** — a single `*.json` file parsed as an `ExportEnvelope` (the wire
//!   format that round-trips with `/api/admin/projects/:id/export`).
//! - **Markdown** — a subdirectory containing `readme.md` (YAML frontmatter
//!   with the `project` block + `schema_version`; body is the description)
//!   and a `tasks/` folder of `*.md` files (YAML frontmatter with task fields
//!   and `test_template`; body is the `command_template`). See
//!   [`markdown::load_markdown_project`].
//!
//! Log-and-continue on per-entry failure (matches `spawn_zmq_subscribers`
//! pattern). Idempotent across restarts via the skip-on-slug check.

pub mod judges;
mod markdown;
pub mod report_judge;

pub use judges::seed_judges;

/// Seed the LLM provider defaults, converting legacy `ai_provider`/`ai_model`
/// settings into the new `llm_providers` + assignment system.
///
/// Two idempotent steps:
///
/// 1. When `llm_providers` is empty, insert one provider row. If the legacy
///    `ai_provider` setting names a registry provider, the row is built from
///    the legacy settings (kind, base URL, api key ciphertext copied
///    **verbatim** — it is already encrypted with the same
///    `SettingsEncryption` key). Otherwise (fresh install) the default
///    enabled keyless Ollama row is inserted, as before. Skipped as soon as
///    any provider row exists — admin edits are never clobbered.
/// 2. When the `llm_default` assignment is absent/blank and at least one
///    enabled provider row exists, write `llm_default` pointing at that
///    provider with the legacy `ai_model` (else the registry default model,
///    else `llama3.2`).
///
/// Existing deployments therefore keep working after the legacy
/// `ai_provider`/`ai_model` resolution path is removed: their settings are
/// converted into a provider row + default assignment on first boot.
pub async fn seed_llm_defaults(db: &sea_orm::DatabaseConnection) {
    use arena_core::entities::{app_settings, llm_providers};
    use arena_core::llm::resolve::{LLM_DEFAULT_KEY, LlmAssignment, SingleAssignment};
    use sea_orm::{ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter, QueryOrder};

    // Read one app_settings value; empty and the redaction sentinel count
    // as absent.
    async fn setting(db: &sea_orm::DatabaseConnection, key: &str) -> Option<String> {
        arena_core::entities::app_settings::Entity::find_by_id(key)
            .one(db)
            .await
            .ok()
            .flatten()
            .map(|r| r.value)
            .filter(|v| !v.trim().is_empty() && v != "[redacted]")
    }

    // Step 1: seed one provider row when none exists.
    let providers_empty = match llm_providers::Entity::find().count(db).await {
        Ok(0) => true,
        Ok(_) => false,
        Err(e) => {
            tracing::warn!(error = %e, "seed: llm provider count failed, skipping");
            return;
        }
    };

    if providers_empty {
        let legacy_provider = setting(db, "ai_provider").await;
        let legacy_spec = legacy_provider
            .as_deref()
            .and_then(arena_core::llm::lookup_provider);

        // Build the row from the legacy settings when possible; `None` falls
        // back to the plain Ollama default row.
        let mut migrated: Option<llm_providers::ActiveModel> = None;
        if let Some(spec) = legacy_spec {
            let kind = match spec.kind {
                arena_core::llm::ProviderKind::Ollama => "ollama",
                arena_core::llm::ProviderKind::OpenRouter => "openrouter",
                arena_core::llm::ProviderKind::OpenAiCompatible => "openai_compatible",
            };
            // Base URL mirrors the legacy resolution order: admin setting →
            // deployment env override → registry default. Required for
            // openai_compatible kinds; when a `custom` provider has no base
            // URL configured we fall back to seeding the Ollama default row
            // (the legacy config was unusable anyway).
            let base_url = match setting(db, spec.base_url_setting).await {
                Some(v) => Some(v),
                None => (!spec.base_url_env.is_empty())
                    .then(|| {
                        std::env::var(spec.base_url_env)
                            .ok()
                            .filter(|v| !v.is_empty())
                    })
                    .flatten()
                    .or_else(|| {
                        (kind == "openai_compatible")
                            .then(|| spec.default_base_url.map(str::to_string))
                            .flatten()
                    }),
            };
            if kind != "openai_compatible" || base_url.is_some() {
                // Copy the encrypted api key VERBATIM — it is already
                // encrypted with the same SettingsEncryption key; do not
                // decrypt/re-encrypt.
                let api_key_enc = if spec.api_key_setting.is_empty() {
                    None
                } else {
                    setting(db, spec.api_key_setting).await
                };
                let catalog_id = match kind {
                    "ollama" => Some("ollama".to_string()),
                    "openrouter" => Some("openrouter".to_string()),
                    _ => None,
                };
                let now = chrono::Utc::now();
                tracing::info!(
                    provider = spec.id,
                    kind,
                    has_api_key = api_key_enc.is_some(),
                    has_base_url = base_url.is_some(),
                    "seed: migrating legacy ai_provider settings to an llm_providers row"
                );
                migrated = Some(llm_providers::ActiveModel {
                    id: Set(Uuid::new_v4()),
                    name: Set(spec.label.to_string()),
                    kind: Set(kind.to_string()),
                    base_url: Set(base_url),
                    api_key_enc: Set(api_key_enc),
                    enabled: Set(true),
                    catalog_id: Set(catalog_id),
                    created_at: Set(now),
                    updated_at: Set(now),
                });
            } else {
                tracing::warn!(
                    provider = spec.id,
                    "seed: legacy provider needs a base URL but none is configured; seeding the default Ollama row instead"
                );
            }
        }

        let am = migrated.unwrap_or_else(|| {
            let now = chrono::Utc::now();
            llm_providers::ActiveModel {
                id: Set(Uuid::new_v4()),
                name: Set("Ollama".to_string()),
                kind: Set("ollama".to_string()),
                base_url: Set(None),
                api_key_enc: Set(None),
                enabled: Set(true),
                catalog_id: Set(Some("ollama".to_string())),
                created_at: Set(now),
                updated_at: Set(now),
            }
        });
        match llm_providers::Entity::insert(am).exec(db).await {
            Ok(_) => tracing::info!("seed: default LLM provider inserted"),
            Err(e) => tracing::warn!(error = %e, "seed: default LLM provider insert failed"),
        }
    }

    // Step 2: write the default assignment when absent and an enabled
    // provider exists.
    let default_present = setting(db, LLM_DEFAULT_KEY).await.is_some();
    if default_present {
        return;
    }
    let provider = match llm_providers::Entity::find()
        .filter(llm_providers::Column::Enabled.eq(true))
        .order_by_asc(llm_providers::Column::CreatedAt)
        .one(db)
        .await
    {
        Ok(Some(p)) => p,
        Ok(None) => return,
        Err(e) => {
            tracing::warn!(error = %e, "seed: enabled provider lookup failed, skipping llm_default");
            return;
        }
    };
    let legacy_default_model = setting(db, "ai_provider")
        .await
        .as_deref()
        .and_then(arena_core::llm::lookup_provider)
        .map(|s| s.default_model.to_string())
        .filter(|m| !m.is_empty());
    let model = match setting(db, "ai_model").await {
        Some(m) => m,
        None => legacy_default_model.unwrap_or_else(|| "llama3.2".to_string()),
    };
    let assignment = LlmAssignment::Single(SingleAssignment {
        provider_id: provider.id,
        model: model.clone(),
    });
    let value = match serde_json::to_string(&assignment) {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(error = %e, "seed: llm_default serialization failed");
            return;
        }
    };
    let res = app_settings::Entity::insert(app_settings::ActiveModel {
        key: Set(LLM_DEFAULT_KEY.to_string()),
        value: Set(value),
    })
    .on_conflict(
        sea_orm::sea_query::OnConflict::column(app_settings::Column::Key)
            .update_column(app_settings::Column::Value)
            .to_owned(),
    )
    .exec(db)
    .await;
    match res {
        Ok(_) => tracing::info!(
            provider_id = %provider.id,
            model = %model,
            "seed: llm_default assignment written"
        ),
        Err(e) => tracing::warn!(error = %e, "seed: llm_default write failed"),
    }
}

use arena_core::entities::{projects, users};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, Set, TransactionTrait};
use std::collections::HashSet;
use uuid::Uuid;

use crate::api::admin_export_import::ExportEnvelope;

const SCHEMA_VERSION: u32 = 1;

/// Seed projects from the folder pointed at by `ARENA_PROJECTS_DIR` (default
/// `./projects`). Two formats are supported:
///
/// - `*.json` files — parsed as an `ExportEnvelope`.
/// - subdirectories containing `readme.md` — parsed as a markdown project
///   (`readme.md` + `tasks/*.md`, see [`markdown::load_markdown_project`]).
///
/// Entries whose project slug already exists in the DB are skipped. Per-entry
/// failures are logged and skipped; the server always starts.
pub async fn seed_projects(db: &sea_orm::DatabaseConnection) {
    let dir = projects_dir();

    if !dir.is_dir() {
        tracing::debug!(dir = %dir.display(), "seed: projects dir missing or not a directory, skipping");
        return;
    }

    let sources = load_sources(&dir);

    if sources.is_empty() {
        tracing::info!(dir = %dir.display(), "seed: no projects found, nothing to do");
        return;
    }

    // Resolve owner: first admin user. Abort seeding entirely if none.
    let owner = match users::Entity::find()
        .filter(users::Column::IsAdmin.eq(true))
        .one(db)
        .await
    {
        Ok(Some(u)) => u,
        Ok(None) => {
            tracing::warn!("seed: no admin user found, skipping project seeding");
            return;
        }
        Err(e) => {
            tracing::error!(error = %e, "seed: admin lookup failed, skipping project seeding");
            return;
        }
    };

    let mut inserted = 0usize;
    let mut skipped = 0usize;
    for (path, env) in &sources {
        match seed_envelope(db, env, path, owner.id).await {
            SeedOutcome::Inserted(slug) => {
                tracing::info!(source = %path.display(), slug = %slug, "seed: inserted project");
                inserted += 1;
            }
            SeedOutcome::Skipped(reason) => {
                tracing::info!(source = %path.display(), reason = %reason, "seed: skipped");
                skipped += 1;
            }
            SeedOutcome::Failed(reason) => {
                tracing::error!(source = %path.display(), reason = %reason, "seed: failed");
                skipped += 1;
            }
        }
    }
    link_campaign_parts(db, &sources).await;
    // Every project carries the session report, including ones seeded before
    // the report judge existed.
    report_judge::ensure_report_judges_everywhere(db).await;
    tracing::info!(inserted, skipped, "seed: done");
}

/// Second pass: wire campaign parents to their parts once every project row
/// exists. Runs unconditionally — the insert pass skips slugs that are already
/// in the DB, so an edited `parts:` list would otherwise never reach a
/// long-lived database.
async fn link_campaign_parts(
    db: &sea_orm::DatabaseConnection,
    sources: &[(std::path::PathBuf, ExportEnvelope)],
) {
    for (path, env) in sources {
        if env.project.parts.is_empty() {
            continue;
        }
        let Some(slug) = env.project.slug.as_deref().filter(|s| !s.is_empty()) else {
            continue;
        };
        let parent = match projects::Entity::find()
            .filter(projects::Column::Slug.eq(slug))
            .one(db)
            .await
        {
            Ok(Some(p)) => p,
            Ok(None) => {
                tracing::warn!(source = %path.display(), slug = %slug, "seed: campaign parent row missing, cannot link parts");
                continue;
            }
            Err(e) => {
                tracing::error!(source = %path.display(), error = %e, "seed: campaign parent lookup failed");
                continue;
            }
        };
        match crate::campaign_link::link_parts_lenient(db, parent.id, slug, &env.project.parts)
            .await
        {
            Ok(linked) => {
                tracing::info!(slug = %slug, linked, "seed: campaign parts linked");
            }
            Err(e) => {
                tracing::error!(slug = %slug, error = %e, "seed: campaign part linking failed");
            }
        }
    }
}

enum SeedOutcome {
    Inserted(String),
    Skipped(&'static str),
    Failed(String),
}

/// The projects dir used for seeding: `ARENA_PROJECTS_DIR`, default
/// `./projects`.
pub(crate) fn projects_dir() -> std::path::PathBuf {
    std::path::PathBuf::from(
        std::env::var("ARENA_PROJECTS_DIR").unwrap_or_else(|_| "./projects".to_string()),
    )
}

/// Scan a projects dir and load every parseable definition, sorted by path.
///
/// ponytail: scan-once, no watcher. Switch to notify crate if live-reload needed.
/// Each entry is (path, loaded-envelope). Envelopes are loaded eagerly so a
/// parse error is attributed to the right entry; failures are logged and
/// skipped.
pub fn load_sources(dir: &std::path::Path) -> Vec<(std::path::PathBuf, ExportEnvelope)> {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) => {
            tracing::warn!(dir = %dir.display(), error = %e, "seed: cannot read projects dir, skipping");
            return Vec::new();
        }
    };

    let mut sources: Vec<(std::path::PathBuf, ExportEnvelope)> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let env = if path.is_dir() {
            match markdown::load_markdown_project(&path) {
                Ok(e) => e,
                Err(e) => {
                    tracing::error!(dir = %path.display(), error = %e, "seed: markdown project parse failed, skipping");
                    continue;
                }
            }
        } else if path.extension().and_then(|s| s.to_str()) == Some("json") {
            match std::fs::read(&path)
                .map_err(|e| format!("read: {e}"))
                .and_then(|bytes| {
                    serde_json::from_slice::<ExportEnvelope>(&bytes)
                        .map_err(|e| format!("parse: {e}"))
                }) {
                Ok(e) => e,
                Err(e) => {
                    tracing::error!(file = %path.display(), error = %e, "seed: json project parse failed, skipping");
                    continue;
                }
            }
        } else {
            continue;
        };
        sources.push((path, env));
    }
    sources.sort_by(|a, b| a.0.cmp(&b.0));
    sources
}

/// Locate a project definition by slug in the seeding projects dir. Returns
/// the first source whose `project.slug` matches.
pub(crate) fn find_source_by_slug(slug: &str) -> Option<(std::path::PathBuf, ExportEnvelope)> {
    load_sources(&projects_dir())
        .into_iter()
        .find(|(_, env)| env.project.slug.as_deref() == Some(slug))
}

/// For a project that already exists (seeded before its seed definition
/// gained `judges:` attachments), attach the seed's judges to any task that
/// currently has none. Tasks are matched by ordinal. Tasks that already have
/// at least one judge are left untouched, so a deliberate admin re-attach or
/// scale override is never clobbered.
async fn top_up_task_judges(
    db: &sea_orm::DatabaseConnection,
    project_id: Uuid,
    envelope: &ExportEnvelope,
) -> SeedOutcome {
    use arena_core::entities::{judges, task_judges, tasks};

    if envelope.tasks.iter().all(|t| t.judges.is_empty()) {
        return SeedOutcome::Skipped("slug exists");
    }

    let judge_id_by_slug: std::collections::HashMap<String, Uuid> =
        match judges::Entity::find().all(db).await {
            Ok(rows) => rows.into_iter().map(|j| (j.slug, j.id)).collect(),
            Err(e) => return SeedOutcome::Failed(format!("judges lookup: {e}")),
        };

    let db_tasks = match tasks::Entity::find()
        .filter(tasks::Column::ProjectIdFk.eq(project_id))
        .all(db)
        .await
    {
        Ok(rows) => rows,
        Err(e) => return SeedOutcome::Failed(format!("tasks lookup: {e}")),
    };
    let task_id_by_ordinal: std::collections::HashMap<i32, Uuid> =
        db_tasks.iter().map(|t| (t.ordinal, t.id)).collect();

    let attached_task_ids: std::collections::HashSet<Uuid> = match task_judges::Entity::find()
        .filter(task_judges::Column::TaskId.is_in(db_tasks.iter().map(|t| t.id)))
        .all(db)
        .await
    {
        Ok(rows) => rows.into_iter().map(|tj| tj.task_id).collect(),
        Err(e) => return SeedOutcome::Failed(format!("task_judges lookup: {e}")),
    };

    let now = chrono::Utc::now();
    let mut attached = 0usize;
    for task in &envelope.tasks {
        if task.judges.is_empty() {
            continue;
        }
        let Some(task_id) = task_id_by_ordinal.get(&task.ordinal).copied() else {
            continue;
        };
        if attached_task_ids.contains(&task_id) {
            continue;
        }
        for (idx, jref) in task.judges.iter().enumerate() {
            let slug = jref.slug();
            let Some(judge_id) = judge_id_by_slug.get(slug).copied() else {
                tracing::warn!(ordinal = task.ordinal, slug = %slug, "seed: judge top-up references unknown judge slug, skipping");
                continue;
            };
            let tj_am = task_judges::ActiveModel {
                id: Set(Uuid::new_v4()),
                task_id: Set(task_id),
                judge_id: Set(judge_id),
                ordinal: Set(idx as i32),
                rating_scale_override: Set(None),
                weight: Set(jref.weight()),
                created_at: Set(now),
                updated_at: Set(now),
            };
            match task_judges::Entity::insert(tj_am).exec(db).await {
                Ok(_) => attached += 1,
                Err(e) => {
                    return SeedOutcome::Failed(format!(
                        "judge top-up insert (ordinal {}): {e}",
                        task.ordinal
                    ));
                }
            }
        }
    }

    if attached == 0 {
        SeedOutcome::Skipped("slug exists")
    } else {
        tracing::info!(project_id = %project_id, attached, "seed: topped up judge attachments on existing project");
        SeedOutcome::Skipped("slug exists (judges topped up)")
    }
}

async fn seed_envelope(
    db: &sea_orm::DatabaseConnection,
    envelope: &ExportEnvelope,
    path: &std::path::Path,
    owner_id: Uuid,
) -> SeedOutcome {
    let _ = path; // kept for log attribution at call site
    if envelope.schema_version != SCHEMA_VERSION {
        return SeedOutcome::Failed(format!(
            "unsupported schema_version {}",
            envelope.schema_version
        ));
    }

    // Slug is required for dedup; without it every restart would re-insert.
    let raw_slug = match envelope.project.slug.as_deref() {
        Some(s) if !s.is_empty() => s,
        _ => return SeedOutcome::Skipped("no slug"),
    };
    let slug = match crate::api::projects::validate_slug(raw_slug) {
        Ok(s) => s,
        Err(_) => return SeedOutcome::Failed("invalid slug".to_string()),
    };

    // Skip-on-any-slug-match (Don decision: global dedup for seeding).
    // Existing projects still get a judge top-up: seed content can gain
    // `judges:` attachments after the project was first seeded, and those
    // would otherwise never reach a long-lived DB.
    match projects::Entity::find()
        .filter(projects::Column::Slug.eq(&slug))
        .one(db)
        .await
    {
        Ok(Some(existing)) => return top_up_task_judges(db, existing.id, envelope).await,
        Ok(None) => {}
        Err(e) => return SeedOutcome::Failed(format!("slug lookup: {e}")),
    }

    // Validate project tags.
    if let Err(e) = crate::validation::tags::validate_tags(&envelope.project.tags) {
        return SeedOutcome::Failed(format!("project tags: {e}"));
    }

    // Resolve category: find-or-create so a seed definition can introduce a
    // new category. Blank or over-long names are dropped with a warning.
    let resolved_category = match envelope.project.category.as_deref().map(str::trim) {
        Some(name) if !name.is_empty() && name.chars().count() <= 100 => {
            if let Err(e) = crate::api::categories::ensure_category(db, name).await {
                return SeedOutcome::Failed(format!("category ensure: {e}"));
            }
            Some(name.to_string())
        }
        Some(name) => {
            tracing::warn!(category = %name, "seed: invalid category name, dropping");
            None
        }
        None => None,
    };

    // Per-task validation + duplicate-ordinal check.
    let mut seen_ordinals: HashSet<i32> = HashSet::new();
    for task in &envelope.tasks {
        if let Err(e) = crate::api::project_tasks::validate_ordinal(task.ordinal) {
            return SeedOutcome::Failed(format!("task ordinal {e}"));
        }
        if let Err(e) = arena_core::validation::validate_template(&task.test_template) {
            return SeedOutcome::Failed(format!("task template: {e}"));
        }
        if let Err(e) = crate::validation::tags::validate_tags(&task.tags) {
            return SeedOutcome::Failed(format!("task tags: {e}"));
        }
        if let Err(e) = crate::api::admin_export_import::validate_task_extras(task) {
            return SeedOutcome::Failed(format!("task {}: {e}", task.ordinal));
        }
        if !seen_ordinals.insert(task.ordinal) {
            return SeedOutcome::Failed(format!("duplicate ordinal {}", task.ordinal));
        }
    }

    // Resolve judge slugs to ids (judges are seeded before projects at
    // startup). An unknown slug fails the whole project so the author
    // notices, instead of silently dropping the judge.
    let judge_id_by_slug: std::collections::HashMap<String, Uuid> =
        match arena_core::entities::judges::Entity::find().all(db).await {
            Ok(rows) => rows.into_iter().map(|j| (j.slug, j.id)).collect(),
            Err(e) => return SeedOutcome::Failed(format!("judges lookup: {e}")),
        };
    for task in &envelope.tasks {
        for jref in &task.judges {
            let slug = jref.slug();
            if !judge_id_by_slug.contains_key(slug) {
                return SeedOutcome::Failed(format!(
                    "task {} references unknown judge slug '{slug}'",
                    task.ordinal
                ));
            }
        }
    }

    let project_name = envelope.project.name.clone();
    let project_tags = envelope.project.tags.clone();
    let resolved_slug = slug.clone();
    let cover = envelope.project.cover_image_url.clone();
    let public = envelope.project.public;
    let description = envelope.project.description.clone().unwrap_or_default();
    let session_duration = envelope.project.session_duration_secs;
    let show_tasks = envelope.project.show_tasks;
    let memory_schema_json = envelope
        .project
        .memory_schema
        .as_ref()
        .map(|v| v.to_string());
    let tasks = envelope.tasks.clone();

    match db
        .transaction::<_, Uuid, sea_orm::DbErr>(|txn| {
            let project_name = project_name.clone();
            let project_tags = project_tags.clone();
            let resolved_slug = resolved_slug.clone();
            let resolved_category = resolved_category.clone();
            let cover = cover.clone();
            let tasks = tasks.clone();
            let proj_pts = envelope.project.points.clone();
            let proj_intervals = envelope.project.intervals.clone();
            let memory_schema_json = memory_schema_json.clone();
            let judge_id_by_slug = judge_id_by_slug.clone();
            Box::pin(async move {
                let project_id = Uuid::new_v4();
                let now = chrono::Utc::now();
                let tags_json =
                    serde_json::to_string(&project_tags).unwrap_or_else(|_| "[]".to_string());

                let project_am = projects::ActiveModel {
                    id: Set(project_id),
                    name: Set(project_name),
                    slug: Set(Some(resolved_slug)),
                    description: Set(description.clone()),
                    category: Set(resolved_category),
                    tags: Set(tags_json),
                    cover_image_url: Set(cover),
                    owner_user_id_fk: Set(owner_id),
                    public: Set(public),
                    archived_at: Set(None),
                    created_at: Set(now),
                    updated_at: Set(now),
                    default_value_points: Set(proj_pts.value),
                    default_fail_points: Set(proj_pts.fail),
                    default_no_response_points: Set(proj_pts.no_response),
                    default_completion_bonus_points: Set(proj_pts.completion_bonus),
                    default_deadline_secs: Set(proj_intervals.deadline_secs),
                    default_session_duration_secs: Set(session_duration),
                    idle_timeout_secs: Set(300),
                    default_min_interval_secs: Set(proj_intervals.min_interval_secs),
                    default_interval_increment_secs: Set(proj_intervals.interval_increment_secs),
                    default_max_interval_secs: Set(proj_intervals.max_interval_secs),
                    memory_schema: Set(memory_schema_json),
                    show_tasks: Set(show_tasks),
                    parent_project_id_fk: Set(None),
                    part_ordinal: Set(None),
                };
                projects::Entity::insert(project_am).exec(txn).await?;

                for task in &tasks {
                    crate::api::admin_export_import::insert_task_with_judges(
                        txn,
                        project_id,
                        task,
                        &proj_pts,
                        &judge_id_by_slug,
                        now,
                    )
                    .await?;
                }

                Ok(project_id)
            })
        })
        .await
    {
        Ok(_) => SeedOutcome::Inserted(slug),
        Err(e) => SeedOutcome::Failed(format!("transaction: {e}")),
    }
}

#[cfg(test)]
mod tests;
