# Changelog

All notable changes to the ololo CLI. Entries are generated
automatically from the source tree on every release build.
## [0.3.1] — 2026-07-24 (`app@095f2e0`)

- fix(sessions): agent WS drives the identified player, not an arbitrary first row (095f2e0)

## [0.3.0] — 2026-07-24 (`app@69984d8`)

- feat(sessions): run judges for interrupted tasks on expiry, defer AP until judges settle (eaad4cc)

## [0.3.0] — 2026-07-24 (`app@9c09a8e`)

- chore: clippy auto-fix pass (workspace lint drift, wip) (e25ef7d)
- feat(sessions): project-sourced duration, judge-aware player status, CLI waiting state (be37d60)
- feat(sessions): all-players-done completion + per-player ack + duration column (9b00eb9)
- wip(sessions): snapshot of in-flight session-completion work (3b25b81)

## [0.3.0] — 2026-07-24 (`app@3543c7c`)

- chore(ololo): bump CLI version to 0.3.0 (3543c7c)
- feat(tui): help popup and global hotkeys (2c3210f)

## [0.2.0] — 2026-07-24 (`app@0482208`)

- feat(judges): full run telemetry, on-disk log store, per-player token cost (0482208)

## [0.2.0] — 2026-07-23 (`app@cf6cc10`)

- feat(judges): full run observability + admin-only error detail (e3a3e8f)
- fix(stats): keep per-model token usage when a session switches models (a56d9ab)

## [0.2.0] — 2026-07-23 (`app@9421056`)

- chore(ololo): bump CLI version to 0.2.0 (9421056)
- feat(tui): probes panel rework, agent picker flow, per-session stats panel (8800502)

## [0.1.3] — 2026-07-23 (`app@8800502`)

- feat(tui): probes panel rework, agent picker flow, per-session stats panel (8800502)

## [0.1.3] — 2026-07-22 (`app@671f185`)

- feat(leaderboard): solo AP pays once per project; score + place in profile sessions (671f185)

## [0.1.3] — 2026-07-22 (`app@712af53`)

- feat(leaderboard): global seasonal ladder with Arena Points + Weng-Lin rating (712af53)

## [0.1.3] — 2026-07-21 (`app@dde572b`)

- fix(leaderboard): dedupe players by carrying player_id in participants (dde572b)

## [0.1.3] — 2026-07-21 (`app@32944f3`)

- feat(llm): registry-driven multi-provider support, custom provider slot (32944f3)

## [0.1.3] — 2026-07-21 (`app@164e864`)

- fix(judges): stop leaking OLLAMA_URL into hosted LLM providers (164e864)

## [0.1.3] — 2026-07-21 (`app@124d9d0`)

- chore(ololo): v0.1.3 (124d9d0)

## [0.1.2] — 2026-07-21 (`app@6dbbe8c`)

- feat(agent-tokens): gemini workspace attribution + logs.json fallback (6dbbe8c)

## [0.1.2] — 2026-07-21 (`app@c0e9a52`)

- feat(judges): judge lifecycle visibility + attach seed judges to existing projects (c0e9a52)

## [0.1.2] — 2026-07-21 (`app@5ab3e65`)

- fix(ololo): dashboard link defaults to the connected server, v0.1.2 (5ab3e65)

## [0.1.1] — 2026-07-21 (`app@453cfaf`)

- chore(ololo): v0.1.1 — TLS fix release (453cfaf)

## [0.1.0] — 2026-07-21 (`app@23d1a89`)

- fix(ololo): pin rustls ring provider — TLS panicked in release builds (23d1a89)

## [0.1.0] — 2026-07-21 (`app@4696fcc`)

- feat(stats): per-task agent statistics from CLI to player page (0a332c3)
- feat(judging): reliable judge runs, verdicts on player page, duration (5a4574e)
- fix(ololo): commit ordinal-0 task snapshots mid-session (fd8eb33)

