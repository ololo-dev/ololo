//! `ololo start` / `ololo join` command implementation and its private helpers.

use crate::config::Config;
use anyhow::{Context, Result, anyhow};

use super::tui_start::run_tui_start;
use crate::join;
use crate::player_ws;
use crate::ui;
use crate::util;

/// Load credentials for the active profile, or fail with a helpful message.
fn load_credentials(profile: &str) -> Result<Config> {
    Config::load(profile).ok_or_else(|| anyhow!("not logged in; run 'ololo login' first"))
}

/// TUI mode requires an agent: resolve `--agent` or run the interactive
/// picker, and fail instead of proceeding when nothing was selected
/// (picker cancelled or no agent detected on $PATH).
fn require_tui_agent(agent: Option<String>) -> Result<String> {
    crate::resolve_tui_agent(agent, true)?.ok_or_else(|| {
        anyhow!("no AI coding agent selected; the TUI requires one (re-run and pick an agent, or pass --agent <name>)")
    })
}

/// Agent name reported to the server: the program (file stem) of the
/// launch spec, which may be a full command line.
fn reported_agent_name(spec: Option<&str>) -> Result<Option<String>> {
    spec.map(|s| crate::agent_cmd::AgentCommand::parse(s).map(|c| c.program_name().to_string()))
        .transpose()
}

fn project_slug_for_display(project: &serde_json::Value) -> String {
    project
        .get("slug")
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .unwrap_or_else(|| "<unknown>".to_string())
}

pub async fn run_start(
    profile: &str,
    slug: String,
    name: Option<String>,
    frontend: Option<String>,
    debug: bool,
    tui: bool,
    agent: Option<String>,
    fresh: bool,
) -> Result<()> {
    let cfg = load_credentials(profile)?;
    crate::auth::validate_token(&cfg.server_url, &cfg.token).await?;
    let base = cfg.server_url.trim_end_matches('/').to_string();

    // Resolve the AI coding agent before any network work: the TUI cannot
    // run without one, and picking first avoids creating a session that is
    // immediately abandoned when the user cancels the picker.
    let chosen_agent = if tui {
        Some(require_tui_agent(agent)?)
    } else {
        agent
    };
    // The dashboard labels the player by agent *name*; a full command line
    // ("claude --model …") reports just its program. Parsing here also
    // rejects a malformed --agent before any session is created.
    let agent_report = reported_agent_name(chosen_agent.as_deref())?;

    let client = reqwest::Client::new();

    // 1. Look up project by slug.
    let project_url = format!("{base}/api/projects/by-slug/{slug}");
    ui::step(format!("Looking up project '{slug}'..."));
    let project_resp = client
        .get(&project_url)
        .bearer_auth(&cfg.token)
        .send()
        .await
        .with_context(|| format!("GET {project_url}"))?;

    let project_status = project_resp.status();
    if !project_status.is_success() {
        let body = project_resp
            .text()
            .await
            .unwrap_or_else(|_| "<no body>".to_string());
        return Err(anyhow!(
            "project lookup failed (HTTP {project_status}): {body}"
        ));
    }

    let project: serde_json::Value = project_resp
        .json()
        .await
        .context("parsing project response")?;
    let project_id = project
        .get("id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("project response missing 'id' field"))?;

    let session_name = name
        .or_else(|| {
            project
                .get("name")
                .and_then(|v| v.as_str())
                .map(str::to_string)
        })
        .unwrap_or_else(|| slug.clone());

    // 2. Create the session.
    let sessions_url = format!("{base}/api/sessions");
    ui::step(format!("Creating session '{session_name}'..."));
    let session_resp = client
        .post(&sessions_url)
        .bearer_auth(&cfg.token)
        .json(&serde_json::json!({
            "name": session_name,
            "project_id": project_id,
        }))
        .send()
        .await
        .with_context(|| format!("POST {sessions_url}"))?;

    let session_status = session_resp.status();
    if !session_status.is_success() {
        let body = session_resp
            .text()
            .await
            .unwrap_or_else(|_| "<no body>".to_string());
        // One-session-at-a-time refusal: point at the blocking session
        // instead of dumping the raw error JSON.
        if let Ok(err) = serde_json::from_str::<serde_json::Value>(&body)
            && err["error"] == "active_session_exists"
        {
            let code = err["active_join_code"].as_str().unwrap_or("?");
            let project = err["active_project"].as_str().unwrap_or("");
            return Err(anyhow!(
                "you are already playing session {code}{}: rejoin it with \
                 'ololo join {code}', or finish/cancel it first (one live \
                 session per player)",
                if project.is_empty() {
                    String::new()
                } else {
                    format!(" ({project})")
                }
            ));
        }
        if let Ok(err) = serde_json::from_str::<serde_json::Value>(&body) {
            // Campaign gates: name the part to clear, or point at the parts
            // inside the campaign, instead of echoing a 409.
            if err["error"] == "part_locked" {
                let required = err["required_project"]
                    .as_str()
                    .unwrap_or("the previous part");
                let ordinal = err["required_part_ordinal"].as_i64().unwrap_or(0) + 1;
                return Err(anyhow!(
                    "this part is locked: finish part {ordinal} ('{required}') first — \
                     'ololo start {required}'"
                ));
            }
            if err["error"] == "campaign_project" {
                return Err(anyhow!(
                    "'{slug}' is a campaign, not a playable project: open it on the site \
                     and start its first part"
                ));
            }
        }
        return Err(anyhow!(
            "session creation failed (HTTP {session_status}): {body}"
        ));
    }

    let session: serde_json::Value = session_resp
        .json()
        .await
        .context("parsing session response")?;
    let join_code = session
        .get("join_code")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("session response missing 'join_code' field"))?;

    // Register host as participant with probe metadata (best-effort, never blocks session
    // start). Any failure — including a fingerprint conflict on reconnect — is non-fatal:
    // the session host must stay alive to monitor the session regardless.
    let host_join_outcome = match join::run_join_subroutine(
        &cfg.token,
        &base,
        join_code,
        &client,
        debug,
        agent_report.as_deref(),
        tui,
    )
    .await
    {
        Ok(o) => Some(o),
        Err(e) => {
            tracing::warn!("player registration failed: {e}");
            None
        }
    };
    let git_remote_url = host_join_outcome
        .and_then(|o| o.git_remote_path)
        .map(|p| format!("{}{}", base.trim_end_matches('/'), p));

    // Dashboard link: explicit --frontend wins; otherwise the frontend
    // shares the server's origin (true in production).
    let frontend_base = frontend.as_deref().unwrap_or(&base).trim_end_matches('/');
    let dashboard_url = format!("{frontend_base}/s/{join_code}");

    ui::success("Session created");
    println!("join_code: {join_code}");
    println!("dashboard: {dashboard_url}");

    // Campaign carry-over runs before the snapshot repo is initialised (both
    // dispatch paths below open it and push a session-start commit), so the
    // baseline snapshot contains the imported codebase rather than an empty
    // tree followed by a mystery bulk commit.
    crate::campaign::prepare_part_workspace(&client, &base, &cfg.token, &project, fresh).await?;

    if tui {
        return run_tui_start(
            profile,
            &cfg,
            join_code.to_string(),
            project_slug_for_display(&project),
            debug,
            chosen_agent.expect("TUI agent resolved before session creation"),
            git_remote_url,
            cfg.token.clone(),
        )
        .await;
    }

    // 3. Headless play: the probe loop with full snapshot/flag wiring, no
    // terminal UI. With --agent the agent is PTY-hosted and remote-driven
    // (autonomous play); without, whoever edits this directory is the agent.
    super::tui_start::run_headless_start(
        profile,
        &cfg,
        join_code.to_string(),
        project_slug_for_display(&project),
        git_remote_url,
        cfg.token.clone(),
        chosen_agent,
    )
    .await
}

/// Spawn the text-mode player-agent probe loop in a background task. Errors
/// are logged; the loop never aborts the caller.
fn spawn_probe_loop(ws_base: &str, code: &str, pat: &str) -> tokio::task::JoinHandle<()> {
    let base = ws_base.to_string();
    let code = code.to_string();
    let pat = pat.to_string();
    tokio::spawn(async move {
        if let Err(e) = player_ws::run(&base, &code, &pat).await {
            tracing::warn!("player_ws error: {e}");
        }
    })
}

/// The project a session runs, or `None` when either lookup fails. Only the
/// campaign carry-over needs this, and it degrades to "no carry-over" rather
/// than blocking a join over a metadata hiccup.
async fn project_of_session(
    client: &reqwest::Client,
    base: &str,
    token: &str,
    session_id: &str,
) -> Option<serde_json::Value> {
    let session: serde_json::Value = client
        .get(format!("{base}/api/sessions/{session_id}"))
        .bearer_auth(token)
        .send()
        .await
        .ok()?
        .json()
        .await
        .ok()?;
    let project_id = session.get("project_id").and_then(|v| v.as_str())?;
    client
        .get(format!("{base}/api/projects/{project_id}"))
        .bearer_auth(token)
        .send()
        .await
        .ok()?
        .json()
        .await
        .ok()
}

pub async fn run_join(
    profile: &str,
    code: String,
    launch: Option<String>,
    debug: bool,
    tui: bool,
    agent: Option<String>,
    fresh: bool,
) -> Result<()> {
    let cfg = load_credentials(profile)?;
    crate::auth::validate_token(&cfg.server_url, &cfg.token).await?;
    let base = cfg.server_url.trim_end_matches('/').to_string();

    // Resolve the agent before joining: the TUI cannot run without one, and
    // picking first avoids registering a participant that immediately quits
    // when the user cancels the picker.
    let chosen_agent = if tui {
        Some(require_tui_agent(agent)?)
    } else {
        agent
    };
    let agent_report = reported_agent_name(chosen_agent.as_deref())?;

    let client = reqwest::Client::new();
    ui::step(format!("Joining session {code}..."));

    let join_outcome = join::run_join_subroutine(
        &cfg.token,
        &base,
        &code,
        &client,
        debug,
        agent_report.as_deref(),
        tui,
    )
    .await?;

    ui::success(format!("Joined session {code}"));

    // Same carry-over as `start`, before any snapshot work: a player who
    // joins a campaign part in a fresh folder gets the previous part's code.
    if let Some(project) =
        project_of_session(&client, &base, &cfg.token, &join_outcome.session_id).await
    {
        crate::campaign::prepare_part_workspace(&client, &base, &cfg.token, &project, fresh)
            .await?;
    }

    let ws_base = util::ws_base_url(&base);

    let git_remote_url = join_outcome
        .git_remote_path
        .map(|p| format!("{}{}", base.trim_end_matches('/'), p));

    if tui {
        return run_tui_start(
            profile,
            &cfg,
            code.clone(),
            code.clone(),
            debug,
            chosen_agent.expect("TUI agent resolved before joining"),
            git_remote_url,
            cfg.token.clone(),
        )
        .await;
    }

    match launch {
        Some(launch_cmd) => {
            // Launch the user's agent process alongside the plain probe loop
            // and wait for the agent to exit.
            let probe_task = spawn_probe_loop(&ws_base, &code, &cfg.token);
            let cmd = crate::agent_cmd::AgentCommand::parse(&launch_cmd)?;
            let mut child = std::process::Command::new(&cmd.program)
                .args(&cmd.args)
                .spawn()
                .with_context(|| format!("launching agent '{launch_cmd}'"))?;
            let exit = child.wait().context("waiting for agent process")?;

            // Agent exited — cancel the probe task and propagate exit code.
            probe_task.abort();

            if !exit.success() {
                std::process::exit(exit.code().unwrap_or(1));
            }
        }
        None => {
            // Headless play: full snapshot/flag wiring, no terminal UI.
            // With --agent, the agent is PTY-hosted and remote-driven.
            return super::tui_start::run_headless_start(
                profile,
                &cfg,
                code.clone(),
                code.clone(),
                git_remote_url,
                cfg.token.clone(),
                chosen_agent,
            )
            .await;
        }
    }

    Ok(())
}
