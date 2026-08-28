use crate::trait_::TokenExtractor;

pub mod amp;
pub mod antigravity;
pub mod antigravity_cli;
mod claude;
pub mod codex;
pub mod copilot;
pub mod cursor;
pub mod cursor_cli;
pub mod gemini;
pub mod goose;
pub mod kimi;
pub mod kiro;
mod omp;
mod opencode;
mod pi;
pub mod pi_common;
pub mod qwen;
pub mod zed;

pub use amp::Amp;
pub use antigravity::Antigravity;
pub use antigravity_cli::AntigravityCli;
pub use claude::Claude;
pub use codex::Codex;
pub use copilot::Copilot;
pub use cursor::Cursor;
pub use cursor_cli::CursorCli;
pub use gemini::Gemini;
pub use goose::Goose;
pub use kimi::Kimi;
pub use kiro::Kiro;
pub use omp::Omp;
pub use opencode::OpenCode;
pub use pi::Pi;
pub use qwen::Qwen;
pub use zed::Zed;

pub const EXTRACTORS: &[&dyn TokenExtractor] = &[
    &Claude,
    &OpenCode,
    &Codex,
    &Cursor,
    &CursorCli,
    &Pi,
    &Omp,
    &Kiro,
    &Copilot,
    &Gemini,
    &Qwen,
    &Kimi,
    &Goose,
    &Amp,
    &Antigravity,
    &AntigravityCli,
    &Zed,
];
