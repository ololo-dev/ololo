pub mod extractors;
mod jsonl;
pub mod paths;
mod skills;
mod sqlite;
mod trait_;
mod types;
mod watch;

pub use trait_::TokenExtractor;
pub use types::{AgentId, SessionCounts, SessionStats, TokenCounts};
pub use watch::{snapshot, stats_snapshot, watch};

/// Hand-maintained list of binary names for PATH-based agent detection.
/// NOT derived from AgentId variants — these are real binary names (e.g. "opencode" not "open-code",
/// "agy" not "antigravity-cli"). Adding an `AgentId` does NOT put an agent in front of a
/// participant: this list is what `ololo` scans on `$PATH` and offers in the start-of-session
/// picker, so a new agent has to be added here too.
///
/// Names are candidates for `which` — one that is not installed is silently skipped, so listing
/// several spellings of the same tool is safe and covers installers that disagree.
///
/// Detection is independent of token tracking: aider/continue/cody/droid are playable but have
/// no extractor here, so they are offered without contributing token usage.
pub const AI_AGENT_NAMES: &[&str] = &[
    "copilot",
    "cursor",
    "aider",
    "continue",
    "cody",
    "claude",
    "opencode",
    "pi",
    "omp",
    "kiro-cli",
    "codex",
    "gemini",
    "qwen",
    "kimi",
    "goose",
    "amp",
    // Cursor's CLI agent ships as `cursor-agent`; `cursor-cli` covers the
    // spelling used by some installs (the IDE itself is `cursor`, above).
    "cursor-agent",
    "cursor-cli",
    // Antigravity's CLI is `agy`; the IDE/launcher is `antigravity`.
    "agy",
    "antigravity",
    // Zed ships a `zed` launcher on PATH that opens the GUI editor.
    "zed",
    // Factory's CLI agent. No token extractor yet — `~/.factory/sessions` is
    // the store, but it was empty everywhere we looked, so the format is
    // unverified and guessing at it would be worse than reporting nothing.
    "droid",
];
