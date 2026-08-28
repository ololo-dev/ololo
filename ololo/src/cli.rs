use clap::{Parser, Subcommand};
use std::path::PathBuf;

/// Arena AI agent CLI
#[derive(Parser)]
#[command(name = "ololo", version, about = "Arena AI agent CLI")]
pub struct Cli {
    /// Credential profile to use (overrides OLOLO_PROFILE env var).
    /// Profiles let you hold tokens for multiple Arena servers simultaneously.
    #[arg(
        long,
        short = 'p',
        global = true,
        default_value = "default",
        env = "OLOLO_PROFILE"
    )]
    pub profile: String,

    /// Print every request body and response sent to the server (for debugging).
    #[arg(long, global = true)]
    pub debug: bool,

    /// Change working directory before execution.
    /// Note: changing --path changes your working directory fingerprint,
    /// which determines player identity.
    #[arg(long, global = true)]
    pub path: Option<PathBuf>,

    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Log in to an Arena server and store your credentials
    Login {
        /// Arena server URL (overrides OLOLO_URL env var and stored value)
        #[arg(long)]
        server: Option<String>,
        /// Print the authorization URL without opening a browser — for
        /// scripted logins that confirm via the API.
        #[arg(long)]
        no_browser: bool,
    },

    /// Start a new session for a project
    Start {
        /// The project slug (e.g. "my-project")
        slug: String,
        /// Session display name. Defaults to the project name.
        #[arg(long)]
        name: Option<String>,
        /// Frontend base URL used to construct the dashboard link.
        /// Defaults to the resolved server URL (they share an origin in
        /// production); override for split dev setups.
        #[arg(long)]
        frontend: Option<String>,
        /// Render in full-screen TUI mode (ratatui). Requires a TTY on
        /// stdout. Logs are written to ~/.config/ololo/<profile>.tui.log.
        /// Enabled by default; pass --no-tui to disable.
        #[arg(long, default_value_t = true)]
        tui: bool,
        /// Disable TUI mode and use the plain text loop instead.
        #[arg(long, default_value_t = false)]
        no_tui: bool,
        /// Agent to run inside the TUI window: a binary name or a full
        /// command line, e.g. --agent "claude --model glm-5.2:cloud".
        /// Detected agents are shown interactively when --tui is used
        /// without --agent.
        #[arg(long)]
        agent: Option<String>,
        /// Pre-approve every probe command: writes
        /// {"permissions":{"allow":["*"]}} to .ololo/settings.json in this
        /// workspace. Probes are shell commands sent by the platform, so this
        /// opts the whole directory into running them unattended — intended
        /// for headless/automated play where nobody can answer the prompt.
        #[arg(long, default_value_t = false)]
        allow_all: bool,
        /// Start a campaign part with an empty workspace instead of importing
        /// your previous part's results. Only affects parts after the first;
        /// an already-populated folder is never overwritten either way.
        #[arg(long, default_value_t = false)]
        fresh: bool,
    },

    /// Join a session by its join code
    Join {
        /// The join code for the session
        code: String,
        /// Agent to launch after joining: a binary name or a full command
        /// line, e.g. --launch "claude --model glm-5.2:cloud".
        /// Omit to participate without running an agent (useful for spectating
        /// or when the agent process is managed externally).
        #[arg(long)]
        launch: Option<String>,
        /// Render in full-screen TUI mode (ratatui). Requires a TTY on
        /// stdout. Logs are written to ~/.config/ololo/<profile>.tui.log.
        /// Enabled by default; pass --no-tui to disable.
        #[arg(long, default_value_t = true)]
        tui: bool,
        /// Disable TUI mode and use the plain text loop instead.
        #[arg(long, default_value_t = false)]
        no_tui: bool,
        /// Agent to run inside the TUI window: a binary name or a full
        /// command line, e.g. --agent "claude --model glm-5.2:cloud".
        /// Detected agents are shown interactively when --tui is used
        /// without --agent.
        #[arg(long)]
        agent: Option<String>,
        /// Pre-approve every probe command: writes
        /// {"permissions":{"allow":["*"]}} to .ololo/settings.json in this
        /// workspace. Probes are shell commands sent by the platform, so this
        /// opts the whole directory into running them unattended — intended
        /// for headless/automated play where nobody can answer the prompt.
        #[arg(long, default_value_t = false)]
        allow_all: bool,
        /// Start a campaign part with an empty workspace instead of importing
        /// your previous part's results. Only affects parts after the first;
        /// an already-populated folder is never overwritten either way.
        #[arg(long, default_value_t = false)]
        fresh: bool,
    },

    /// Show the active profile's server and token fingerprint
    Whoami,

    /// Update ololo to the latest release
    Update {
        /// Only check whether a newer release exists; don't install it.
        #[arg(long)]
        check: bool,
    },

    /// Manage credential profiles
    Profile(ProfileArgs),
}

#[derive(Parser)]
pub struct ProfileArgs {
    #[command(subcommand)]
    pub command: ProfileCommands,
}

#[derive(Subcommand)]
pub enum ProfileCommands {
    /// List all configured profiles
    List,
    /// Remove a profile and its stored credentials
    Remove {
        /// Name of the profile to remove
        name: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_commands_parse() {
        use clap::Parser;
        let cli = Cli::try_parse_from(["ololo", "profile", "list"]).unwrap();
        assert!(matches!(cli.command, Commands::Profile(_)));

        let cli2 = Cli::try_parse_from(["ololo", "profile", "remove", "staging"]).unwrap();
        if let Commands::Profile(args) = cli2.command {
            assert!(matches!(args.command, ProfileCommands::Remove { name } if name == "staging"));
        } else {
            panic!("expected Profile command");
        }

        let cli3 = Cli::try_parse_from(["ololo", "whoami"]).unwrap();
        assert!(matches!(cli3.command, Commands::Whoami));
    }

    #[test]
    fn update_parses_with_and_without_check() {
        use clap::Parser;
        let cli = Cli::try_parse_from(["ololo", "update"]).unwrap();
        assert!(matches!(cli.command, Commands::Update { check: false }));
        let cli = Cli::try_parse_from(["ololo", "update", "--check"]).unwrap();
        assert!(matches!(cli.command, Commands::Update { check: true }));
    }

    #[test]
    fn agent_accepts_a_full_command_line() {
        use clap::Parser;
        let cli = Cli::try_parse_from([
            "ololo",
            "join",
            "ABC",
            "--agent",
            "claude --model glm-5.2:cloud",
        ])
        .unwrap();
        if let Commands::Join { agent, .. } = cli.command {
            assert_eq!(agent.as_deref(), Some("claude --model glm-5.2:cloud"));
        } else {
            panic!("expected Join");
        }
    }

    #[test]
    fn allow_all_flag_parses_on_start_and_join() {
        use clap::Parser;
        let cli =
            Cli::try_parse_from(["ololo", "start", "my-slug", "--no-tui", "--allow-all"]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::Start {
                allow_all: true,
                ..
            }
        ));
        let cli = Cli::try_parse_from(["ololo", "join", "ABC", "--allow-all"]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::Join {
                allow_all: true,
                ..
            }
        ));
        // Off by default: pre-approving server-sent shell commands is opt-in.
        let cli = Cli::try_parse_from(["ololo", "start", "my-slug"]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::Start {
                allow_all: false,
                ..
            }
        ));
    }

    #[test]
    fn start_tui_flag_parses() {
        use clap::Parser;
        // TUI is now the default; --tui is a no-op kept for muscle memory.
        let cli = Cli::try_parse_from(["ololo", "start", "my-slug"]).unwrap();
        if let Commands::Start { tui, no_tui, .. } = cli.command {
            assert!(tui);
            assert!(!no_tui);
        } else {
            panic!("expected Start");
        }
        let cli = Cli::try_parse_from(["ololo", "start", "my-slug", "--no-tui"]).unwrap();
        if let Commands::Start { no_tui, .. } = cli.command {
            assert!(no_tui);
        } else {
            panic!("expected Start");
        }
    }

    #[test]
    fn join_tui_flag_parses() {
        use clap::Parser;
        let cli = Cli::try_parse_from(["ololo", "join", "ABC"]).unwrap();
        if let Commands::Join { tui, no_tui, .. } = cli.command {
            assert!(tui);
            assert!(!no_tui);
        } else {
            panic!("expected Join");
        }
        let cli = Cli::try_parse_from(["ololo", "join", "ABC", "--no-tui"]).unwrap();
        if let Commands::Join { no_tui, .. } = cli.command {
            assert!(no_tui);
        } else {
            panic!("expected Join");
        }
    }
}
