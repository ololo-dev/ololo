---
title: The Terminal App
section: ololo CLI
---

### One screen for the whole game

`ololo join` and `ololo start` open a full-screen terminal app — the cockpit
for your session:

- **Header** — session name, phase, and the live countdown. Finish all your
  tasks early and it flips to "tasks done — waiting for session finish" while
  the others race.
- **Sidebar** — the current task with its description, the session
  leaderboard, and your recent check results. Open one to see the exact
  command, expected against actual output, and the points it moved.
- **Agent terminal** — your coding agent runs embedded in the app, in your
  working directory. It is a real terminal: type to your agent, interrupt it,
  or leave it alone, without leaving the game.

### Hotkeys

| Key        | Action                                                      |
| ---------- | ----------------------------------------------------------- |
| `F1` / `?` | Help overlay with every hotkey                              |
| `F2`       | Open the details of the last failed check                   |
| `F3`       | Paste that failure into your agent's terminal               |
| `F4`       | Toggle the sidebar                                          |
| `F9`       | Switch focus between the game panels and the agent terminal |
| `F10`      | Quit (when the game panels have focus)                      |

`F3` is the power move: one key hands your agent exactly the check it should
fix next, with the command and the output it produced. Most of the game is
F2, F3, watch, repeat.

### Picking an agent

Started without `--agent`? The app scans your PATH and shows a picker of the
agents it recognizes — see [Supported Agents](/documentation/agents).

The app needs one of them to start. To play with no agent and write the code
yourself, use `--no-tui` instead: the checks score whatever is in your
directory, no matter who typed it.

Desktop and IDE agents — cursor, antigravity, zed — open in their own window.
The game runs exactly the same, but the agent panel shows a placeholder
rather than the conversation, because that conversation never passes through
ololo.

### Where things are stored

- Credentials: `~/.config/ololo/credentials.toml`, one entry per profile.
- Session log: `~/.config/ololo/PROFILE.tui.log` (`default.tui.log` unless
  you use profiles) — attach it when you report a problem.
