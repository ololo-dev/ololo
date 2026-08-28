//! Push local seed fixtures (judges/*.md + projects/) to a running server.
//!
//! The deployed containers bake `projects/` and `judges/` into the image, so a
//! fixture edit normally rides a deploy. This command short-circuits that:
//! it parses the local seed files with the same parsers the boot seed uses and
//! pushes them over the admin API — judges through the judges CRUD, projects
//! through `POST /api/admin/projects/apply-seed` (upsert by slug; task history
//! survives, tasks removed from the definition are deleted).
//!
//! Usage:
//!   cargo run -p server --bin push-seeds -- --url https://your-deployment.example [--dry-run]
//!   cargo run -p server --bin push-seeds -- --url https://your-deployment.example --only hop-hop
//!
//! Auth: an admin's token, resolved from (in order) `--token`, the
//! `ARENA_ADMIN_TOKEN` env var, or `~/.config/ololo/credentials.toml` — the
//! first profile whose `server_url` matches the target. PATs (`ololo_…`) work:
//! the admin API accepts them for admin users.

use serde::Deserialize;
use server::api::admin_export_import::{ApplySeedResponse, ExportEnvelope};
use server::seed::judges::load_judge_defs;
use server::seed::load_sources;

struct Args {
    base_url: String,
    token: Option<String>,
    projects_dir: String,
    judges_dir: String,
    dry_run: bool,
    only: Option<String>,
    skip_judges: bool,
}

fn usage() -> ! {
    eprintln!(
        "push-seeds — push local judges/ and projects/ seed fixtures to a server\n\n\
         USAGE:\n  cargo run -p server --bin push-seeds -- --url <base> [OPTIONS]\n\n\
         OPTIONS:\n  \
         --url <base>          base URL of the target deployment\n  \
         --token <token>       admin token; falls back to $ARENA_ADMIN_TOKEN, then\n                        \
         ~/.config/ololo/credentials.toml (profile matching the URL)\n  \
         --projects-dir <dir>  seed projects dir (default ./projects)\n  \
         --judges-dir <dir>    seed judges dir (default ./judges)\n  \
         --only <slug>         push only the project with this slug\n  \
         --skip-judges         do not push judges\n  \
         --dry-run             parse and diff, but send nothing"
    );
    std::process::exit(2);
}

fn parse_args() -> Args {
    let mut base_url: Option<String> = None;
    let mut token: Option<String> = None;
    let mut projects_dir = "./projects".to_string();
    let mut judges_dir = "./judges".to_string();
    let mut dry_run = false;
    let mut only: Option<String> = None;
    let mut skip_judges = false;

    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        let mut val = |name: &str| -> String {
            it.next().unwrap_or_else(|| {
                eprintln!("missing value for {name}");
                usage()
            })
        };
        match arg.as_str() {
            "--url" => base_url = Some(val("--url").trim_end_matches('/').to_string()),
            "--token" => token = Some(val("--token")),
            "--projects-dir" => projects_dir = val("--projects-dir"),
            "--judges-dir" => judges_dir = val("--judges-dir"),
            "--only" => only = Some(val("--only")),
            "--dry-run" => dry_run = true,
            "--skip-judges" => skip_judges = true,
            "--help" | "-h" => usage(),
            other => {
                eprintln!("unknown argument '{other}'");
                usage()
            }
        }
    }

    let Some(base_url) = base_url else {
        eprintln!("--url is required");
        usage()
    };
    Args {
        base_url,
        token,
        projects_dir,
        judges_dir,
        dry_run,
        only,
        skip_judges,
    }
}

/// Minimal parser for ~/.config/ololo/credentials.toml: `[profile]` sections
/// with `server_url = "…"` and `token = "…"` lines. Returns the token of the
/// first profile whose server_url matches `base_url` (trailing slash ignored).
fn token_from_ololo_credentials(base_url: &str) -> Option<String> {
    let home = std::env::var("HOME").ok()?;
    let path = std::path::Path::new(&home).join(".config/ololo/credentials.toml");
    let text = std::fs::read_to_string(path).ok()?;
    let want = base_url.trim_end_matches('/');
    let mut url: Option<String> = None;
    let mut token: Option<String> = None;
    let unquote = |s: &str| s.trim().trim_matches('"').to_string();
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            // New profile: check the finished one first.
            if let (Some(u), Some(t)) = (&url, &token)
                && u.trim_end_matches('/') == want
            {
                return Some(t.clone());
            }
            url = None;
            token = None;
        } else if let Some(v) = line.strip_prefix("server_url") {
            url = v.split_once('=').map(|(_, v)| unquote(v));
        } else if let Some(v) = line.strip_prefix("token") {
            token = v.split_once('=').map(|(_, v)| unquote(v));
        }
    }
    if let (Some(u), Some(t)) = (url, token)
        && u.trim_end_matches('/') == want
    {
        return Some(t);
    }
    None
}

/// Mirror of the server's JudgeResponse (only the fields the diff needs).
#[derive(Debug, Deserialize)]
struct RemoteJudge {
    id: uuid::Uuid,
    slug: String,
    name: String,
    description: String,
    prompt: String,
    rating_scale: serde_json::Value,
    kind: String,
    scope: String,
    #[serde(default)]
    evidence_needs: Option<Vec<String>>,
    #[serde(default)]
    criteria: Option<Vec<String>>,
    #[serde(default)]
    max_interactive: Option<i32>,
    #[serde(default)]
    ignore_paths: Option<Vec<String>>,
}

struct Client {
    http: reqwest::Client,
    base: String,
    token: String,
}

impl Client {
    async fn get_json<T: serde::de::DeserializeOwned>(&self, path: &str) -> Result<T, String> {
        let resp = self
            .http
            .get(format!("{}{path}", self.base))
            .bearer_auth(&self.token)
            .send()
            .await
            .map_err(|e| format!("GET {path}: {e}"))?;
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            return Err(format!("GET {path}: HTTP {status} — {body}"));
        }
        serde_json::from_str(&body).map_err(|e| format!("GET {path}: bad response: {e}"))
    }

    async fn send_json(
        &self,
        method: reqwest::Method,
        path: &str,
        body: &serde_json::Value,
    ) -> Result<String, String> {
        let resp = self
            .http
            .request(method.clone(), format!("{}{path}", self.base))
            .bearer_auth(&self.token)
            .json(body)
            .send()
            .await
            .map_err(|e| format!("{method} {path}: {e}"))?;
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            return Err(format!("{method} {path}: HTTP {status} — {text}"));
        }
        Ok(text)
    }
}

async fn push_judges(client: &Client, dir: &std::path::Path, dry_run: bool) -> Result<(), String> {
    let (defs, errors) = load_judge_defs(dir);
    for (path, reason) in &errors {
        eprintln!("  ✗ {}: {reason}", path.display());
    }
    if !errors.is_empty() {
        return Err(format!("{} judge file(s) failed to parse", errors.len()));
    }

    let remote: Vec<RemoteJudge> = client.get_json("/api/admin/judges").await?;
    let by_slug: std::collections::HashMap<&str, &RemoteJudge> =
        remote.iter().map(|j| (j.slug.as_str(), j)).collect();

    // The seed stores list fields as canonical JSON strings; the API takes
    // arrays, and an absent list must stay absent rather than become `null`.
    let parse_list_for_create = |v: &Option<String>| -> serde_json::Value {
        v.as_deref()
            .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok())
            .unwrap_or(serde_json::Value::Null)
    };
    let mut failures = 0usize;
    for (_, def) in &defs {
        match by_slug.get(def.slug.as_str()) {
            None => {
                if dry_run {
                    println!("  + judge {} (create)", def.slug);
                    continue;
                }
                let body = serde_json::json!({
                    "slug": def.slug,
                    "name": def.name,
                    "description": def.description,
                    "prompt": def.prompt,
                    "rating_scale": def.rating_scale,
                    "kind": def.kind,
                    "scope": def.scope,
                    "ignore_paths": parse_list_for_create(&def.ignore_paths),
                });
                match client
                    .send_json(reqwest::Method::POST, "/api/admin/judges", &body)
                    .await
                {
                    Ok(_) => println!("  + judge {} created", def.slug),
                    Err(e) => {
                        eprintln!("  ✗ judge {}: {e}", def.slug);
                        failures += 1;
                    }
                }
            }
            Some(existing) => {
                // The seed defs store list fields as canonical JSON strings;
                // the API answers with parsed arrays — compare canonically.
                let to_json = |v: &Option<Vec<String>>| {
                    v.as_ref()
                        .map(|x| serde_json::to_string(x).unwrap_or_default())
                };
                let unchanged = existing.name == def.name
                    && existing.description == def.description
                    && existing.prompt == def.prompt
                    && existing.rating_scale == def.rating_scale
                    && to_json(&existing.evidence_needs) == def.evidence_needs
                    && to_json(&existing.criteria) == def.criteria
                    && existing.max_interactive == def.max_interactive
                    && to_json(&existing.ignore_paths) == def.ignore_paths;
                // The update endpoint cannot change kind/scope; surface a
                // drift instead of silently ignoring it.
                if existing.kind != def.kind || existing.scope != def.scope {
                    eprintln!(
                        "  ! judge {}: kind/scope differ (file {}/{}, server {}/{}) — not pushable via API, edit in admin UI or reseed the DB",
                        def.slug, def.kind, def.scope, existing.kind, existing.scope
                    );
                }
                if unchanged {
                    println!("  = judge {} unchanged", def.slug);
                    continue;
                }
                if dry_run {
                    println!("  ~ judge {} (update)", def.slug);
                    continue;
                }
                let parse_list = |v: &Option<String>| -> serde_json::Value {
                    v.as_deref()
                        .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok())
                        .unwrap_or(serde_json::Value::Null)
                };
                let body = serde_json::json!({
                    "name": def.name,
                    "description": def.description,
                    "prompt": def.prompt,
                    "rating_scale": def.rating_scale,
                    "needs": parse_list(&def.evidence_needs),
                    "criteria": parse_list(&def.criteria),
                    "max_interactive": def.max_interactive,
                    "ignore_paths": parse_list(&def.ignore_paths),
                });
                let path = format!("/api/admin/judges/{}", existing.id);
                match client.send_json(reqwest::Method::PUT, &path, &body).await {
                    Ok(_) => println!("  ~ judge {} updated", def.slug),
                    Err(e) => {
                        eprintln!("  ✗ judge {}: {e}", def.slug);
                        failures += 1;
                    }
                }
            }
        }
    }
    if failures > 0 {
        return Err(format!("{failures} judge push(es) failed"));
    }
    Ok(())
}

async fn push_projects(
    client: &Client,
    dir: &std::path::Path,
    only: Option<&str>,
    dry_run: bool,
) -> Result<(), String> {
    let mut sources: Vec<(std::path::PathBuf, ExportEnvelope)> = load_sources(dir);
    if sources.is_empty() {
        return Err(format!("no seed projects found in {}", dir.display()));
    }
    // Campaign parents last: apply-seed rejects a `parts` entry whose project
    // does not exist yet, and path order alone would push a parent before the
    // parts it names.
    sources.sort_by_key(|(_, env)| !env.project.parts.is_empty());

    let mut failures = 0usize;
    let mut pushed = 0usize;
    for (path, envelope) in sources {
        let slug = envelope.project.slug.clone().unwrap_or_default();
        if let Some(only) = only
            && slug != only
        {
            continue;
        }
        if slug.is_empty() {
            eprintln!(
                "  ✗ {}: seed has no slug — cannot upsert, skipping",
                path.display()
            );
            failures += 1;
            continue;
        }
        if dry_run {
            println!("  → project {slug} ({} tasks)", envelope.tasks.len());
            pushed += 1;
            continue;
        }
        let body = serde_json::to_value(&envelope)
            .map_err(|e| format!("{}: serialize: {e}", path.display()))?;
        match client
            .send_json(
                reqwest::Method::POST,
                "/api/admin/projects/apply-seed",
                &body,
            )
            .await
        {
            Ok(text) => {
                pushed += 1;
                match serde_json::from_str::<ApplySeedResponse>(&text) {
                    Ok(r) if r.created => {
                        println!("  + project {slug} created ({} tasks)", r.tasks_inserted)
                    }
                    Ok(r) => println!(
                        "  ~ project {slug} updated (tasks: {} updated, {} inserted, {} deleted)",
                        r.tasks_updated, r.tasks_inserted, r.tasks_deleted
                    ),
                    Err(_) => println!("  ~ project {slug} pushed"),
                }
            }
            Err(e) => {
                eprintln!("  ✗ project {slug}: {e}");
                failures += 1;
            }
        }
    }
    if let Some(only) = only
        && pushed == 0
        && failures == 0
    {
        return Err(format!("no seed project with slug '{only}' found"));
    }
    if failures > 0 {
        return Err(format!("{failures} project push(es) failed"));
    }
    Ok(())
}

#[tokio::main]
async fn main() {
    let args = parse_args();

    let token = args
        .token
        .clone()
        .or_else(|| {
            std::env::var("ARENA_ADMIN_TOKEN")
                .ok()
                .filter(|t| !t.is_empty())
        })
        .or_else(|| token_from_ololo_credentials(&args.base_url));
    let Some(token) = token else {
        eprintln!(
            "no admin token: pass --token, set ARENA_ADMIN_TOKEN, or log in with the ololo CLI \
             against {} first",
            args.base_url
        );
        std::process::exit(2);
    };

    println!(
        "Pushing seeds to {}{}",
        args.base_url,
        if args.dry_run { " (dry run)" } else { "" }
    );

    let client = Client {
        http: reqwest::Client::new(),
        base: args.base_url.clone(),
        token,
    };

    let mut failed = false;

    if args.skip_judges {
        println!("Judges: skipped (--skip-judges)");
    } else {
        // Judges first: project tasks reference judge slugs, and apply-seed
        // rejects envelopes naming a judge the server does not know yet.
        println!("Judges ({}):", args.judges_dir);
        if let Err(e) = push_judges(
            &client,
            std::path::Path::new(&args.judges_dir),
            args.dry_run,
        )
        .await
        {
            eprintln!("judges: {e}");
            failed = true;
        }
    }

    println!("Projects ({}):", args.projects_dir);
    if let Err(e) = push_projects(
        &client,
        std::path::Path::new(&args.projects_dir),
        args.only.as_deref(),
        args.dry_run,
    )
    .await
    {
        eprintln!("projects: {e}");
        failed = true;
    }

    if failed {
        std::process::exit(1);
    }
    println!("Done.");
}
