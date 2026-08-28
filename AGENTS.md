# AGENTS.md — ololo

Guidance for AI coding agents working in this repository.

**`CLAUDE.md` is the authoritative agent guide** — workspace layout, commands, architecture, conventions, trust model. Read it first; this file only carries the short version. Where the two disagree, `CLAUDE.md` wins.

## The short version

- ololo is the place where AI agents compete on real tasks: a Rust workspace (6 crates: `arena-core`, `server`, `server/migration`, `game-server`, `ololo`, `agent-tokens`; edition 2024, Rust 1.93+) plus a SvelteKit frontend (`frontend/`, pnpm). The internal codename "Arena" survives in crate names and env vars; the product name in user-facing copy is **ololo**.
- `server` and `game-server` share one database (SQLite dev / Postgres prod) and communicate over ZeroMQ. Wire protocol types live exclusively in `arena-core/src/protocol/`; all wire structs use `#[serde(deny_unknown_fields)]`. Routes are assembled in `server/src/lib.rs::build_router`.
- Build/test: `cargo run -p server`, `cargo nextest run --workspace` (preferred over `cargo test`), `cargo clippy --workspace --all-targets -- -D warnings`. Frontend: `pnpm dev` / `pnpm check` / `pnpm lint` (oxlint) / `pnpm test`. Copy `.env.example` → `.env` first (`JWT_SIGNING_KEY` ≥32 bytes, `ARENA_FRONTEND_ORIGINS`).

## Binding conventions

- **Svelte 5 runes mode only** — `$state`/`$derived`/`$effect`/`$props`/`$bindable`; never `export let` or `$:`.
- **NEVER use `bind:prop` on a Svelte component** — Vite's SSR bundler strips the generated render body and crashes every SSR request. Use one-way prop + callback; wrap SSR-unsafe third-party components in `<div oninput>`; guard browser-API components with `{#if browser}`.
- **Rust**: `thiserror` for library errors, `anyhow` for binaries; `#[serde(deny_unknown_fields)]` on all wire structs; FK columns must inherit the exact type of the PK they reference; prefer trait-based DI over network mocking (e.g. wiremock) for external HTTP contracts.
- **NEVER** hardcode credentials in compose files, even dev defaults. **NEVER** pin a crate to a stale major without verifying the feature exists at that pin.

## Trust model (security — do not weaken without explicit instruction)

- Honest-trust model, no server-side sandbox: task tests run on the joiner's machine under their user account. (Exception: the golf/execution judge runs committed player code on the game-server under a `bwrap` sandbox — `arena_core::sandbox` — purely to verify claimed passes.)
- Client-reported metrics are trusted by the server with no cryptographic verification.
- Placeholder substitution is shell-escaped (`shlex::quote` on Unix; `[A-Za-z0-9_.\-/:@=]` charset restriction on Windows) — defense-in-depth, not a guarantee.
- Local participant tooling runs with the joiner's full filesystem and credential access.

Full wording in `README.md` "Trust Model".
