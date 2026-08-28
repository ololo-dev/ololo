# agent-tokens

Library + debug CLI that reads the **local user's own** AI-coding-agent
session logs to answer one question: how many tokens (and which tools and
skills) did my agents use in a given time window?

ololo uses it to show a player their own agent's spend during a session and
to attach per-task usage stats to their session report.

## What it reads

Each supported agent keeps a local session store — JSONL transcripts, SQLite
databases, or cache CSVs — under the user's home directory (`paths.rs`;
overridable via `AGENT_TOKENS_HOME`). An extractor per agent
(`src/extractors/`) knows its format: Claude Code, Codex, Cursor (IDE and
CLI), Copilot, Gemini, Qwen, Kimi, Goose, Amp, OpenCode, pi, omp, Kiro,
Zed, Antigravity (IDE and CLI). Extractors whose store is absent are
skipped (`detect()`).

From those files it derives, per agent session:

- **token counts** — input, output, cache read/write, reasoning; model name;
  optional cost (`SessionCounts`);
- **behavioural stats** — user/assistant message counts, tool-call counts by
  tool name, skill-load counts by skill name (`SessionStats`).

It never extracts prompt or completion text; only counts, names, and
timestamps are aggregated.

## What leaves the machine, and when

The crate itself sends nothing anywhere — it is a local reader. The one
consumer that reports anything is the `ololo` CLI while you are **playing a
session** (`ololo start` / `ololo join`): when a task completes, the CLI
scans the local stores for that task's time window, keeps only sessions
whose recorded working directory matches the play workspace, and POSTs the
aggregates above (`TaskStatsReport` in `ololo/src/task_stats.rs`) to the
server you joined, where they appear on your own session page. Reporting is
best-effort — a failure is logged and never blocks gameplay. Outside a
session nothing is collected or sent.

What is reported: agent name, agent session id, model name, token counts,
optional cost, message/tool-call counts, tool and skill names with counts.
What is never reported: prompt or completion content, file contents, or the
paths of the log files.

## Debug CLI

```bash
cargo run -p agent-tokens -- snapshot --period 2h        # token usage, all detected agents
cargo run -p agent-tokens -- stats --agent claude --json # message/tool/skill stats
cargo run -p agent-tokens -- watch --interval 5          # live polling
```

Everything the CLI prints stays in your terminal.
