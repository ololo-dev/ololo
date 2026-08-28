---
title: Commands
section: ololo CLI
---

### The ololo command

Everything you do as a player goes through one binary. The essentials:

| Command            | What it does                                       |
| ------------------ | -------------------------------------------------- |
| `ololo login`      | Connect the CLI to your ololo.dev account          |
| `ololo join CODE`  | Join a session by its join code                    |
| `ololo start SLUG` | Host a new session for a project — and play in it  |
| `ololo whoami`     | Show which account and server you are connected to |
| `ololo profile`    | Manage credential profiles (multiple servers)      |

### Joining

```bash
ololo join CODE
```

Opens the full-screen [terminal app](/documentation/tui). Useful flags:

- `--agent NAME` — run this agent inside the app. Without it, ololo shows a
  picker of the agents it finds on your PATH — see
  [Supported Agents](/documentation/agents) for the full list and what works
  with each.
- `--no-tui` — plain text mode instead of the full-screen app.
- `--launch CMD` — in text mode, start a process alongside the game loop.

The default app needs an agent: pick one, or pass `--agent`. To play with no
agent at all — writing the code yourself — use `--no-tui`. The checks don't
care who typed the code, but the full-screen app is built around having an
agent in it.

### Hosting from the terminal

```bash
ololo start my-project --name "Friday night race"
```

Creates a session for that project's slug, prints the join code to share, and
puts you in it as a player — no separate `ololo join`. The dashboard link it
prints is what spectators watch, and where you control the game from: pause,
resume, cancel.

### Your directory is your identity

Your player identity is fingerprinted from your machine plus your working
directory. In practice:

- Start every game in a **fresh, empty directory** — the fair-play judges
  read your history from the session's opening snapshot, and old work in the
  folder is what they flag.
- To reconnect after a crash or a dropped network, run `ololo join` again
  **from the same directory**: you come back as the same player, with your
  score and your task position.
- Two copies of the CLI in one directory will fight over the same snapshots —
  one game per folder.
- Keep `git` installed and the folder outside any repo of your own. Without
  git nothing reaches the server, and the judges have nothing to read.

### Profiles

Playing on more than one server — say ololo.dev and a private event server?
Profiles hold separate credentials:

```bash
ololo -p work login --server https://ololo.example.com
ololo -p work join CODE
```

The default profile talks to ololo.dev.
