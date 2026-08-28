#![allow(
    clippy::too_many_arguments,
    clippy::type_complexity,
    clippy::module_inception
)]

mod agent_cmd;
mod antigravity_sync;
mod auth;
mod campaign;
mod cli;
mod commands;
mod config;
mod control;
mod done_flag;
mod error;
mod join;
mod memory_sync;
mod permissions;
mod player_ws;
mod probe;
mod snapshot;
mod task_stats;
mod tui;
mod ui;
mod util;

#[cfg(test)]
mod test_util;

use anyhow::{Result, anyhow};
use clap::Parser;
use cli::{Cli, Commands, ProfileCommands};
use config::resolve_server_url;
use std::io::IsTerminal;

fn pick_agent_interactively() -> Option<String> {
    use std::io::Write;

    let detected = crate::tui::agent_picker::detected_agents();

    // Prefer a real TUI selector when we own a terminal; fall back to a
    // plain stdin prompt (pipes, --no-tui without a TTY). Both offer a
    // free-form command besides the detected agents.
    if std::io::stdout().is_terminal() {
        return crate::tui::agent_picker::pick_agent_tui(&detected);
    }

    if detected.is_empty() {
        eprintln!("No AI coding agent found on $PATH.");
        eprintln!(
            "Install one of: {}",
            crate::probe::AI_AGENT_NAMES.join(", ")
        );
    }
    eprintln!("Select an AI coding agent to run:");
    for (i, name) in detected.iter().enumerate() {
        eprintln!("  [{}] {}", i + 1, name);
    }
    eprint!(
        "Enter number (1-{}) or a custom command (e.g. claude --model glm-5.2:cloud): ",
        detected.len()
    );
    let _ = std::io::stderr().flush();

    let mut input = String::new();
    if std::io::stdin().read_line(&mut input).is_err() {
        return None;
    }
    let input = input.trim();
    if let Ok(idx) = input.parse::<usize>() {
        return detected.get(idx.saturating_sub(1)).cloned();
    }
    if input.is_empty() {
        return None;
    }
    Some(input.to_string())
}

pub(crate) fn resolve_tui_agent(agent: Option<String>, tui: bool) -> Result<Option<String>> {
    match agent {
        Some(name) => Ok(Some(name)),
        None => {
            // In TUI mode, detect agents with a live progress screen first,
            // then pick from what was found. Otherwise fall back to the
            // plain stdin prompt (pipes / non-TTY).
            if tui && std::io::stdout().is_terminal() {
                Ok(crate::tui::agent_picker::detect_and_select_agent_tui())
            } else {
                Ok(pick_agent_interactively())
            }
        }
    }
}

/// Set up probe permissions before any session exists. `--allow-all` writes
/// the run-everything rule into `.ololo/settings.json`; otherwise a headless
/// run that could not approve anything gets warned here — at the start, with
/// the fix spelled out — instead of discovering it as a stack of `-5`
/// declines once probes begin. TUI mode always has its own prompt.
fn prepare_probe_permissions(tui: bool, allow_all: bool) {
    if allow_all {
        if let Err(e) = permissions::record_allow_all() {
            ui::fatal(format!("--allow-all could not write the rule: {e:#}"));
        }
        ui::step(format!(
            "--allow-all: every probe command is pre-approved in {}",
            permissions::settings_path().display()
        ));
        return;
    }
    if !tui {
        permissions::warn_if_cannot_approve_probes();
    }
}

#[tokio::main]
async fn main() {
    // The dependency graph enables both rustls crypto providers (ring via our
    // reqwest/tungstenite, aws-lc-rs via arena-core's rig-core), so rustls
    // cannot auto-select one and panics on first TLS use. Pin ring explicitly.
    let _ = rustls::crypto::ring::default_provider().install_default();
    let _ = dotenvy::dotenv();
    let cli = Cli::parse();
    // Tracing is installed in one of two ways:
    // - text-mode (no --tui): stderr writer, installed here.
    // - TUI mode (--tui): tui::log_redir::setup_tracing installs a
    //   file writer in ~/.config/ololo/<profile>.tui.log AFTER
    //   the TTY guard passes. set_global_default is Once-bound,
    //   so we MUST NOT install the stderr subscriber when --tui is
    //   set (it would shadow the file writer the TUI needs).
    let tui_active = matches!(
        cli.command,
        Commands::Start {
            tui: true,
            no_tui: false,
            ..
        } | Commands::Join {
            tui: true,
            no_tui: false,
            ..
        }
    );
    if !tui_active {
        tracing_subscriber::fmt()
            .with_env_filter(
                tracing_subscriber::EnvFilter::try_from_default_env()
                    .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
            )
            .with_writer(std::io::stderr)
            .init();
    }
    if let Some(ref p) = cli.path
        && let Err(e) = std::env::set_current_dir(p)
    {
        ui::fatal(anyhow!(
            "cannot set working directory to '{}': {e}",
            p.display()
        ));
    }
    // Kick off the passive update check for every command except `update`
    // itself (which does its own, forced). Best-effort and cached for 24h.
    let update_check = match cli.command {
        Commands::Update { .. } => None,
        _ => Some(commands::update::spawn_check()),
    };
    let result = match cli.command {
        Commands::Login { server, no_browser } => {
            let server_url = resolve_server_url(server, &cli.profile);
            auth::run_login(server_url, &cli.profile, no_browser).await
        }
        Commands::Start {
            slug,
            name,
            frontend,
            tui,
            no_tui,
            agent,
            allow_all,
            fresh,
        } => {
            let tui = tui && !no_tui;
            if tui && !std::io::stdout().is_terminal() {
                ui::fatal("--tui requires a TTY on stdout; re-run interactively or drop --tui");
            }
            prepare_probe_permissions(tui, allow_all);
            commands::run_start(
                &cli.profile,
                slug,
                name,
                frontend,
                cli.debug,
                tui,
                agent,
                fresh,
            )
            .await
        }
        Commands::Join {
            code,
            launch,
            tui,
            no_tui,
            agent,
            allow_all,
            fresh,
        } => {
            let tui = tui && !no_tui;
            if tui && !std::io::stdout().is_terminal() {
                ui::fatal("--tui requires a TTY on stdout; re-run interactively or drop --tui");
            }
            prepare_probe_permissions(tui, allow_all);
            commands::run_join(&cli.profile, code, launch, cli.debug, tui, agent, fresh).await
        }
        Commands::Whoami => commands::run_whoami(&cli.profile).await,
        Commands::Update { check } => commands::run_update(check).await,
        Commands::Profile(args) => match args.command {
            ProfileCommands::List => commands::run_profile_list(&cli.profile),
            ProfileCommands::Remove { name } => commands::run_profile_remove(&name).await,
        },
    };
    // The passive check ran alongside the command; if it found a newer
    // release, say so now — after the command's own output (and after the
    // TUI's alternate screen is gone). Give a stalled fetch 250ms, no more.
    if let Some(handle) = update_check
        && let Ok(Ok(Some(latest))) =
            tokio::time::timeout(std::time::Duration::from_millis(250), handle).await
    {
        commands::update::notice(&latest);
    }
    if let Err(e) = result {
        ui::fatal(e);
    }
}
