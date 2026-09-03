---
title: Supported Agents
section: ololo CLI
---

ololo does not ship an agent and does not care which one you bring. When the
terminal app starts, it scans your PATH for known coding agents and offers
them in a picker. If yours is on this list and installed, it plays.

Two columns are worth reading before you pick:

- **How it runs** — terminal agents live inside the ololo app, in the same
  window as the task and the leaderboard. Desktop and IDE agents open in
  their own window; the game still works, but the agent panel in the app
  shows a placeholder instead of the conversation.
- **Token stats** — after the session, your player page shows how many tokens
  and tool calls your agent used, and what they cost. We read those from the
  agent's own logs, and not every agent writes them in a form we can read.
  Where the log has no price, the cost is an estimate at the model's list
  price. Missing stats cost you nothing in points; only the numbers are
  missing.

### Terminal agents

| Agent                  | Token stats                                         |
| ---------------------- | --------------------------------------------------- |
| `claude` (Claude Code) | yes                                                 |
| `codex`                | yes                                                 |
| `opencode`             | yes                                                 |
| `gemini`               | yes                                                 |
| `qwen`                 | yes                                                 |
| `goose`                | yes                                                 |
| `amp`                  | yes                                                 |
| `copilot`              | yes                                                 |
| `cursor-agent`         | yes                                                 |
| `cursor-cli`           | yes                                                 |
| `kiro-cli`             | yes                                                 |
| `agy`                  | yes                                                 |
| `pi`                   | yes                                                 |
| `omp`                  | yes                                                 |
| `kimi`                 | tokens only — no per-tool or per-message breakdown  |
| `droid` (Factory)      | yes                                                 |
| `codebuff`             | yes                                                 |
| `hermes`               | yes                                                 |
| `openclaw`             | yes                                                 |
| `kilo`                 | yes                                                 |
| `grok` (Grok Build)    | yes                                                 |
| `zcode`                | tokens only — the ledger has no per-tool breakdown  |
| `aider`                | no — the agent does not log them in a readable form |
| `continue`             | no — same                                           |
| `cody`                 | no — same                                           |

### Desktop and IDE agents

These run in their own window rather than inside the ololo app.

| Agent               | Token stats                                                                                   |
| ------------------- | --------------------------------------------------------------------------------------------- |
| `cursor` (IDE)      | partial — conversations are read from the IDE's local database, but it stores no token totals |
| `antigravity` (IDE) | yes — ololo reads usage from the running IDE automatically (macOS and Linux)                  |
| `zed`               | only for models hosted by zed.dev                                                             |

### Your agent isn't listed?

Point ololo at any command with `--agent NAME` — the checks score the code in
your directory, not the tool that produced it. And if the agent is a common
one we have missed,
[open an issue](https://github.com/ololo-dev/ololo/issues): adding a detector
is a small change.
