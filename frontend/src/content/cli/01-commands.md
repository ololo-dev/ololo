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

### Projects with a repository

Some projects start from existing code: they name a git repository on their
page. `ololo start` and `ololo join` put it in place before the session does
anything else:

- **inside a clone of it** — any remote, any subfolder — the session works at
  the clone's top, exactly as it is;
- **in an empty folder**, ololo clones it right there;
- **anywhere else**, ololo clones it into `./<name>`, as `git clone` would, and
  the session works in that folder — or in the clone already there.

ololo never fetches into, checks out or resets a clone you already have, and
refuses to nest a clone inside another repository of yours. Cloning runs your
own `git`, so private repositories work with your usual credentials and ssh
keys.

### Your directory is your identity

Your player identity is fingerprinted from your machine plus your working
directory. In practice:

- Start every game in a **fresh, empty directory** — the fair-play judges
  read your history from the session's opening snapshot, and old work in the
  folder is what they flag. (A project with a repository starts from its
  code instead: the judges read that as the starting point, not your work.)
- While it runs, ololo keeps its connection alive by itself: a dropped
  connection is dialled again, and so is one that has gone silent — the
  game server speaks every second, so a minute without a word means the
  connection is dead even if it looks open.
- To reconnect after a crash, run `ololo join` again **from the same
  directory**: you come back as the same player, with your score and your
  task position.
- Two copies of the CLI in one directory will fight over the same snapshots —
  one game per folder.
- Keep `git` installed. Without it nothing reaches the server, and the judges
  have nothing to read. For a project that starts from scratch, keep the
  folder outside any repository of your own.

### Profiles

Playing on more than one server — say ololo.dev and a private event server?
Profiles hold separate credentials:

```bash
ololo -p work login --server https://ololo.example.com
ololo -p work join CODE
```

The default profile talks to ololo.dev.
