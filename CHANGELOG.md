# Changelog

All notable changes to the ololo CLI. Entries are generated
automatically from the source tree on every release build.
## [0.11.1] — 2026-08-15 (`app@cb0867a`)

- chore(ololo): v0.11.1 — picker event fix, permission popup arrows (cb0867a)
- fix(cli): picker survives stray terminal events; arrows in the permission popup (f36eee7)

## [0.11.0] — 2026-08-15 (`app@de0d47a`)

- fix(player-chat): honest judge/completion states, live-parity, CLI verdicts (de0d47a)

## [0.11.0] — 2026-08-15 (`app@c081991`)

- chore(ololo): v0.11.0 — probe-command permission gate (c081991)
- feat(cli): permission gate before probe commands (d150d77)

## [0.10.0] — 2026-08-15 (`app@2b38c2a`)

- style: rustfmt sweep — unblock the fmt gate on CI (2b38c2a)
- chore(ololo): v0.10.0 — built-in Antigravity usage sync (3b0ad01)
- feat(cli): built-in Antigravity IDE usage sync — tokscale no longer needed (32436dc)

## [0.10.0] — 2026-08-15 (`app@3b0ad01`)

- chore(ololo): v0.10.0 — built-in Antigravity usage sync (3b0ad01)
- feat(cli): built-in Antigravity IDE usage sync — tokscale no longer needed (32436dc)

## [0.9.1] — 2026-08-14 (`app@0fb550d`)

- feat(billing): re-grid review packs so they stop undercutting Premium (0fb550d)

## [0.9.1] — 2026-08-14 (`app@997d1b4`)

- fix(ololo): one env lock for every HOME-mutating test (997d1b4)

## [0.9.1] — 2026-08-14 (`app@f89f99c`)

- feat(billing): admin Payments tab fed by webhook-recorded transactions (f89f99c)

## [0.9.1] — 2026-08-14 (`app@4855e60`)

- feat(billing): paddle checkout and webhook fulfillment (4855e60)

## [0.9.1] — 2026-08-14 (`app@27584a9`)

- feat(judging): out-of-range ratings clamp to the nearest bound, never re-run (7bc734f)

## [0.9.1] — 2026-08-13 (`app@82f1af9`)

- feat(judging): artifact request registry — equivalent asks attach, not duplicate (086b722)

## [0.9.1] — 2026-08-13 (`app@794dc5e`)

- chore(ololo): v0.9.1 — buffered pushes survive the proxy chain (794dc5e)

## [0.9.0] — 2026-08-13 (`app@f651654`)

- feat(similarity)!: persisted reports with named sources; proportional penalty (f651654)

## [0.9.0] — 2026-08-13 (`app@d3c6a69`)

- feat(similarity): the score drop explains itself, source named (d3c6a69)

## [0.9.0] — 2026-08-13 (`app@3437bad`)

- feat(accounts): sellable review packs — credits spent after the monthly limit (3437bad)

## [0.9.0] — 2026-08-13 (`app@5497f5f`)

- fix(cli): buffered pushes only — streamed bodies die in the proxy chain (5497f5f)

## [0.9.0] — 2026-08-13 (`app@4127cf1`)

- fix(judges): artifact requests reuse what exists and stop wasting the player (4127cf1)

## [0.9.0] — 2026-08-12 (`app@724d687`)

- feat(probes)!: remove mode:llm rubric probes — judges are the only LLM evaluation (724d687)

## [0.9.0] — 2026-08-12 (`app@b647c1e`)

- feat(accounts): landing shows a pricing block while tiers are enforced (b647c1e)

## [0.9.0] — 2026-08-12 (`app@f44d7c8`)

- feat(accounts): master tiers switch, profile usage meter, tighter users table (f44d7c8)

## [0.9.0] — 2026-08-12 (`app@caf5286`)

- style: rustfmt and oxfmt the tree so ci's format gates pass again (caf5286)
- feat(accounts): free/premium plans meter judge runs per calendar month (502de1c)

## [0.9.0] — 2026-08-12 (`app@502de1c`)

- feat(accounts): free/premium plans meter judge runs per calendar month (502de1c)

## [0.9.0] — 2026-08-12 (`app@f6d6db6`)

- fix(judging): judge runs lost for good settle instead of pending forever (f6d6db6)

## [0.9.0] — 2026-08-12 (`app@aec0573`)

- fix(sessions): tasks seeded after a session finished are not expected of it (aec0573)

## [0.9.0] — 2026-08-11 (`app@1788d11`)

- feat(frontend): artifact requests read as clear deliverables (1788d11)

## [0.9.0] — 2026-08-11 (`app@70c35bd`)

- feat(protocol): carry the test's human label to the UI — no more shell parsing (70c35bd)

## [0.9.0] — 2026-08-09 (`app@17e724b`)

- fix(judging): session judges stop re-billing — terminal rows for every pair, dedup, in-flight guard (17e724b)

## [0.9.0] — 2026-08-09 (`app@63bb59d`)

- fix(judging): judge-registered validations can actually check exit codes (63bb59d)

## [0.9.0] — 2026-08-09 (`app@015552a`)

- style: cargo fmt --all — the garden runner finally reached the fmt gate (015552a)

## [0.9.0] — 2026-08-09 (`app@7f58c33`)

- fix(session): gallery dedupes re-requested captures; avg rating reads the verdict sheet (7f58c33)

## [0.9.0] — 2026-08-09 (`app@8a56ae7`)

- feat(ololo): auxiliary commits address the current task (e24cb32)

## [0.9.0] — 2026-08-09 (`app@b7e837e`)

- fix(artifacts): media type follows the delivered file, not the request (5036177)

## [0.9.0] — 2026-08-09 (`app@60659e7`)

- Maintenance rebuild (no CLI-facing commits detected)

## [0.9.0] — 2026-08-08 (`app@60659e7`)

- chore(ololo): v0.9.0 — headless play mode and the no-browser login (60659e7)

## [0.8.0] — 2026-08-08 (`app@a63500d`)

- feat(session): screencasts play everywhere; images reach only vision judges (6ba73d5)
- feat(session): an artifact request delivers up to five files (d1b6bac)
- feat(session): delivered artifacts land on the activity feed — screenshots and screencasts inline (5ade76f)
- feat(dev): play a session locally end to end — headless CLI mode + dev-play stack (96d9b22)
- feat(judging): the contract's constraints ride with the task into every judge prompt (f045e6d)
- feat(weather-widget): the brief is a product spec; the platform never dials the widget (599fe24)
- feat(done-flag): the flag file is noticed the moment it lands and judged at once (ee3bb51)

## [0.8.0] — 2026-08-07 (`app@1d5ca67`)

- feat(newsletter): who got it, what they got, and a list that fits a phone (1d5ca67)

## [0.8.0] — 2026-08-07 (`app@99c1e62`)

- fix(judging): a replay is decided by what the session wrote, not by a coin (99c1e62)

## [0.8.0] — 2026-08-07 (`app@0ae1eca`)

- fix(llm): an Ollama provider without a base URL finds the daemon via OLLAMA_URL (68b37d3)

## [0.8.0] — 2026-08-07 (`app@b3c8f64`)

- feat(badges): code golfers get their medal (b3c8f64)

## [0.8.0] — 2026-08-07 (`app@4c06658`)

- fix(judging): an evidence vacuum is the cheating signature, not an alibi (877fd9b)
- fix(judging): vision attachments carry an image detail level (9e0c833)

## [0.8.0] — 2026-08-06 (`app@c49388d`)

- fix(judging): a judge's probe explains itself, everywhere it shows (c49388d)

## [0.8.0] — 2026-08-06 (`app@e6cdfdc`)

- feat!: ArtifactRequest is gone — a judge's request IS a probe (e6cdfdc)

## [0.7.6] — 2026-08-06 (`app@d93064c`)

- feat(ololo): the artifact request nests under its task (d93064c)

## [0.7.5] — 2026-08-06 (`app@f59e461`)

- feat(judging): judge probes ride the regular probe discipline (f59e461)

## [0.7.4] — 2026-08-06 (`app@3a16ec3`)

- chore: drop an accidentally committed empty file (3a16ec3)
- feat(ololo): the judge's request reaches the one who can answer it (c94c814)

## [0.7.4] — 2026-08-06 (`app@c94c814`)

- feat(ololo): the judge's request reaches the one who can answer it (c94c814)

## [0.7.3] — 2026-08-06 (`app@ba851b7`)

- feat(judges-tab): a judge's captures live in its own bubble (ba851b7)

## [0.7.3] — 2026-08-06 (`app@3b91719`)

- feat(ololo): the judge phase is visible in the TUI, and requests tick (3b91719)

## [0.7.2] — 2026-08-06 (`app@4963189`)

- feat(gallery): committed screenshots count too (4963189)

## [0.7.2] — 2026-08-06 (`app@6a8e7a7`)

- fix(ololo): the artifact request's actions live in the probe body (6a8e7a7)

## [0.7.1] — 2026-08-06 (`app@2aaa1b6`)

- feat(open-ended): the judging phase says its name (2aaa1b6)

## [0.7.1] — 2026-08-05 (`app@054fe2e`)

- feat(judging): the artifact request is a probe, everywhere (054fe2e)

## [0.7.0] — 2026-08-05 (`app@088e442`)

- feat(judges): a face for every judge (088e442)

## [0.7.0] — 2026-08-05 (`app@14842b9`)

- feat(judging): a judge that sees what the player shipped (14842b9)

## [0.7.0] — 2026-08-05 (`app@fb0ec49`)

- feat(activity): a verdict shows its criteria, not only its number (fb0ec49)

## [0.7.0] — 2026-08-05 (`app@7032d39`)

- chore(ololo): 0.7.0 — the agent answers snapshot and artifact requests (7032d39)
- style: rustfmt over the open-ended landing (0988ea1)

## [0.6.0] — 2026-08-05 (`app@0988ea1`)

- style: rustfmt over the open-ended landing (0988ea1)

## [0.6.0] — 2026-08-05 (`app@bc4a511`)

- feat(tasks): open-ended tasks — the probe measures, the judges convert it to score (bc4a511)

## [0.6.0] — 2026-08-04 (`app@30deb4d`)

- feat(newsletter): hear about bounces and complaints (30deb4d)

## [0.6.0] — 2026-08-04 (`app@52c9bae`)

- feat(newsletter): a list that only holds people who asked (52c9bae)

## [0.6.0] — 2026-08-03 (`app@20b414e`)

- fix(badges): breadth counts solo play, like the medals it is made of (20b414e)

## [0.6.0] — 2026-08-03 (`app@80927ee`)

- feat(judging): a judge declares what it needs to see, and pays for that much (80927ee)

## [0.6.0] — 2026-08-03 (`app@a374108`)

- feat(judging): a second cut point, after the model has answered (a374108)

## [0.6.0] — 2026-08-03 (`app@2b4f5ec`)

- feat(judging): a judge can carry its own program, not just a prompt (2b4f5ec)

## [0.6.0] — 2026-08-03 (`app@035a9bd`)

- feat(judging): one snapshot of what a judge is allowed to know (035a9bd)
- fix(judging): a judge cannot take more than the task paid (1411580)

## [0.6.0] — 2026-08-02 (`app@72037e5`)

- feat(badges): seed a badge for every agent ololo can attribute (488a825)

## [0.6.0] — 2026-08-01 (`app@f247f43`)

- feat(gamification): badges for what a player proved, and what they raced with (f247f43)

## [0.6.0] — 2026-08-01 (`app@e932fde`)

- feat(arena): past seasons on the ladder, both boards on a project (e932fde)

## [0.6.0] — 2026-08-01 (`app@041e2de`)

- feat(ololo): publish memory sources as they change, not once per task (b8e251a)

## [0.6.0] — 2026-07-31 (`app@7a5cdf2`)

- fix(telemetry): name the provider that ran, and the session, player and task (7a5cdf2)

## [0.6.0] — 2026-07-31 (`app@5bc9361`)

- release(ololo): v0.6.0 — Zed usage, archived Codex sessions (5bc9361)
- fix(agent-tokens): count archived Codex sessions and both cache spellings (4bdc552)
- feat(agent-tokens): report Zed agent usage (490c840)

## [0.5.0] — 2026-07-31 (`app@1aae76d`)

- feat(llm): model pools with tiered failover, and judge pool overrides (1aae76d)

## [0.5.0] — 2026-07-31 (`app@721cec7`)

- release(ololo): v0.5.0 — Cursor, Antigravity, Zed and droid agents (721cec7)
- feat(ololo): explain the session when the agent runs in its own window (b671fe5)
- fix(ololo): detect Factory's droid at session start (b1988be)
- fix(ololo): offer the Cursor, Antigravity and Zed agents at session start (8df0576)

## [0.4.0] — 2026-07-31 (`app@cdb5313`)

- feat(projects): live sessions above the tabs, richer session rows (cdb5313)

## [0.4.0] — 2026-07-31 (`app@4aacd64`)

- perf(judges): one-shot the task anti-cheat judge from a server-built dossier (4aacd64)

## [0.4.0] — 2026-07-30 (`app@de050c5`)

- feat(llm): per-turn trace timeline in the telemetry drawer (de050c5)

## [0.4.0] — 2026-07-30 (`app@aa93a30`)

- fix(llm): let an Ollama provider carry an API key; flag the fallback misroute (aa93a30)

## [0.4.0] — 2026-07-30 (`app@89378b0`)

- release(ololo): v0.4.0 — cursor, cursor-cli, antigravity, antigravity-cli token tracking (89378b0)

## [0.3.2] — 2026-07-30 (`app@5091ddd`)

- feat(llm): unified request telemetry with settings page and drawer (5091ddd)

## [0.3.2] — 2026-07-30 (`app@8e84765`)

- refactor(llm)!: remove the legacy ai_provider/ai_model path; record per-run judge models (8e84765)
- feat(agent-tokens): cursor, cursor-cli, antigravity, antigravity-cli support (60f2e23)

## [0.3.2] — 2026-07-30 (`app@6f45ce6`)

- style: rustfmt the session-memory and llm-provider additions (6f45ce6)

## [0.3.2] — 2026-07-30 (`app@35e57ed`)

- feat: per-player session memory + multi-provider LLM configuration (7fedd97)

## [0.3.2] — 2026-07-30 (`app@fd3d75f`)

- fix(ololo): build CLI auth URL from resolved server URL, not server env (fd3d75f)

## [0.3.1] — 2026-07-29 (`app@6c5914e`)

- chore: format the workspace and clear clippy -D warnings (Wave 5 prep) (aba7a03)

## [0.3.1] — 2026-07-29 (`app@cda17dc`)

- perf(scoring): coalesce score-history samples by second (PERF-M1) (cda17dc)
- fix(scoring): enforce one completion bonus per player+task (DB-M1) (f326655)
- fix(awards): make session awards + rating updates atomic (DB-H3) (7e59635)
- perf(scoring): aggregate scores in SQL instead of folding in Rust (PERF-H3) (df63bb1)
- perf(probe): bound untrusted boa JS eval with runtime limits (PERF-H2) (344d31c)

## [0.3.1] — 2026-07-28 (`app@a121add`)

- fix(security): authenticate game-server /internal/* routes (SEC-H3) (b6f2bee)
- fix(security): sanitize session description, harden judge git tools, stop logging admin token (6f2808e)

## [0.3.1] — 2026-07-28 (`app@f6b2c41`)

- fix(game-server): require PAT auth and player ownership on the agent WS (f6b2c41)

## [0.3.1] — 2026-07-27 (`app@24d6cd8`)

- feat(sessions): name the cancel reason, and hold a "judges working" dialog (24d6cd8)

## [0.3.1] — 2026-07-27 (`app@e6493f5`)

- feat(sessions): agent presence, idle auto-cancel, and session-end notices (e6493f5)

## [0.3.1] — 2026-07-27 (`app@bbf6fc7`)

- fix(judging): never charge a player for a judge that could not run (0ac5b78)

## [0.3.1] — 2026-07-26 (`app@3c71dfe`)

- feat(judging): execution judges — run committed code server-side for code golf (3c71dfe)

## [0.3.1] — 2026-07-24 (`app@a258dde`)

- feat(awards): configurable AP formula with 1v1 bonus, solo/negative dampers, record chase (97f284e)

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

