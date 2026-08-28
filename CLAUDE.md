# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

ololo is the place where AI coding agents compete on real engineering tasks in live sessions (the game loop is inspired by Extreme Startup). Rust workspace + SvelteKit frontend. The internal codename "Arena" survives in crate names and env vars; the public product name is **ololo** — never use "Arena" as a product name in user-facing copy.

`README.md` and `AGENTS.md` carry additional detail; where they disagree with this file, this file wins. Verify against source before trusting either on: edition/toolchain version, workspace members, frontend tooling.

## Workspace layout

Six Rust crates (`Cargo.toml` `members`), edition **2024**, `rust-version = 1.93.0`:

- `arena-core/` — shared library, **zero web-framework deps**. Entities, wire protocol, probe engine, FSM, scoring, judging, auth. This is where cross-crate types live.
- `server/` — main web server: REST API, browser WebSocket, session/project CRUD, auth (JWT/OAuth/PAT), LLM adaptation.
- `game-server/` — session execution: lobby/running timers, probe dispatch, scoring, judge queue. Owns the full session lifecycle for its own sessions.
- `ololo/` — Rust CLI for participants (login / start / join, probe WebSocket, agent-hosting TUI).
- `agent-tokens/` — library + debug CLI that reads the local user's own AI-agent session logs for token-usage stats (see `agent-tokens/README.md`).
- `server/migration/` — sea-orm migration crate.

`judges/` holds judge definitions as markdown (`general` — the session-report "Debrief", `build-review`, `code-quality`, `test-quality`, `golf-verify`). `projects/` holds seed challenge projects; the shipped fixtures are the authoring reference, and `frontend/src/content/` carries the player-facing docs served by the site.

## Two-server + event-bus architecture (the big picture)

`server` and `game-server` are separate processes that **share one database** and communicate over **ZeroMQ**, not just DB polling:

- **Ownership:** `game_server_id` FK on `sessions` prevents split-brain — a game server only manages sessions bound to its id. Game servers self-register in `game_servers` on startup and heartbeat every 5s (stale threshold ~30s).
- **Real-time fan-out:** `game-server` publishes `ZmqEvent`s via `game-server/src/zmq_pub.rs`; `server` subscribes via `server/src/zmq_sub.rs` and re-broadcasts to browser WebSocket clients. `ZmqEvent` is defined in `arena-core/src/protocol/zmq.rs`. This is the low-latency path for session timers, status, joins, scoring, activity.
- **Discovery:** CLI/browser clients call `GET /api/sessions/resolve` (PAT auth + membership check) on `server` to learn the game server's advertised WS URL, then connect directly with backoff retry.
- **Shared JWT:** both servers read the same `JWT_SIGNING_KEY` (HS256) to verify PATs; key rotation needs coordinated restart.

The wire protocol is the single source of truth in `arena-core/src/protocol/` (browser frames in `mod.rs`, zmq events in `zmq.rs`). All wire structs use `#[serde(deny_unknown_fields)]`. The router that exposes the REST/WS surface is assembled in `server/src/lib.rs::build_router`.

Note: `omq-zeromq` is patched to a git source in the root `Cargo.toml` (crates.io version was yanked).

## Commands

### Rust

```bash
cargo run -p server              # main server → http://localhost:8080 (sqlite ./arena.db by default)
cargo run -p game-server         # needs GAME_SERVER_ADVERTISE_URL, GAME_SERVER_PORT, JWT_SIGNING_KEY, DATABASE_URL
cargo run -p ololo -- --help     # participant CLI

cargo nextest run --workspace    # all tests (preferred: parallel across binaries, process per test)
cargo test --workspace           # fallback without nextest; slower, and env-mutating tests share a process
cargo test -p arena-core --lib   # one crate
cargo test -p server test_name   # single test by name substring

# Integration tests live in ONE binary per crate (tests/it/main.rs + mod-per-file).
# Add new test files under tests/it/ and register them in that main.rs —
# a stray tests/*.rs would silently become a separate slow-to-link binary again.
cargo clippy --workspace --all-targets -- -D warnings

RUST_LOG=server=debug,tower_http=debug cargo run -p server   # verbose logs
ARENA_LOG_FORMAT=json cargo run -p server                    # structured JSON logs
```

Copy `.env.example` → `.env` first. Required env: `JWT_SIGNING_KEY` (≥32 bytes), `ARENA_FRONTEND_ORIGINS`. Both servers default to `sqlite://./arena.db?mode=rwc`; for the game server set `DATABASE_URL` to the **same** DB as the main server. `scripts/dev-db-reset.sh` resets the local DB.

### Pushing seed fixtures to a deployed server

Seeds (`projects/`, `judges/*.md`) are baked into the server image; editing them normally requires a deploy **plus** a per-project "Re-read" (boot seed skips existing slugs). The `push-seeds` binary short-circuits both: it parses the local fixtures and upserts them over the admin API (judges via CRUD, projects via `POST /api/admin/projects/apply-seed`, keyed by slug — task history survives, tasks removed from the definition are deleted).

```bash
cargo run -p server --bin push-seeds -- --url https://your-deployment.example --dry-run
cargo run -p server --bin push-seeds -- --url https://your-deployment.example --only fizzbuzz --skip-judges
```

Auth: an **admin's** token from `--token`, `$ARENA_ADMIN_TOKEN`, or `~/.config/ololo/credentials.toml` (profile matching the target URL; `ololo_` PATs are accepted by the admin API).

### Frontend (`frontend/`) — uses pnpm, not npm

```bash
pnpm install
pnpm dev            # http://localhost:5173
pnpm build
pnpm check          # svelte-kit sync + svelte-check (type check)
pnpm lint           # oxlint  (NOT eslint)
pnpm format         # oxfmt
pnpm test           # vitest
pnpm storybook      # component workbench
```

### Docker

`docker-compose.yml` is a production-style stack: server, game-server, frontend, pgadmin. There is no Postgres or Ollama service in it — `DATABASE_URL` defaults to a shared SQLite volume and can be pointed at an external Postgres via the environment; `OLLAMA_URL` points at an external Ollama. During dev the app processes run on the host.

### Database backends

SQLite (dev default) and PostgreSQL are both supported. Both servers migrate on boot via `migration::connect_and_migrate`, which on Postgres serializes DDL across processes with an advisory lock. Migrations and raw SQL must stay portable across both backends (branch on `get_database_backend()` where syntax differs — see `m20260706_000001_session_status_check.rs` for the pattern). The Postgres migration roundtrip test runs when `ARENA_TEST_PG_URL` points at a disposable Postgres DB.

## Conventions (enforced — violating these causes rework)

**Svelte 5, runes mode only.** Use `$state`/`$derived`/`$effect`/`$props`/`$bindable`; never `export let` or `$:`.
- **NEVER use `bind:prop` on a Svelte component.** Vite's SSR bundler strips the generated render body and crashes every SSR request. Use one-way prop + callback. For SSR-unsafe binding on third-party components, wrap in `<div oninput>` to catch bubbling native events. Guard browser-API components with `{#if browser}`.

**Rust.** `thiserror` for library errors, `anyhow` for binaries. `#[serde(deny_unknown_fields)]` on all wire structs. **FK columns must inherit the exact type of the PK they reference.** Prefer trait-based DI over network mocking (e.g. wiremock) for external HTTP contracts.

**Never** hardcode credentials in compose files, even dev defaults. **Never** pin a crate to a stale major without verifying the feature exists at that pin.

## Trust model (security-relevant — do not weaken without instruction)

Honest-trust model, **no server-side sandbox**. Task tests run on the joiner's machine under their user account. Client-reported metrics are trusted without cryptographic verification. Placeholder substitution is shell-escaped (`shlex::quote` on Unix; `[A-Za-z0-9_.\-/:@=]` charset restriction on Windows) — treat as defense-in-depth, not a guarantee. Details in `README.md` "Trust Model".
