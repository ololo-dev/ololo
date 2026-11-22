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
| `F3`       | Paste menu: hand your agent the brief, a failure, health…   |
| `F4`       | Toggle the sidebar                                          |
| `F9`       | Switch focus between the game panels and the agent terminal |
| `F10`      | Quit (when the game panels have focus)                      |

`F3` is the power move: it opens a menu of everything the game knows that
your agent may need, and `⏎` (or the row's number) pastes it into the agent's
terminal. Each row says what it hands over:

| Row              | What your agent gets                                                   |
| ---------------- | ---------------------------------------------------------------------- |
| Judge's request  | What to capture, where to save it, and the time left                   |
| Task brief       | The current task, in full                                              |
| Failed check     | The failing check: its command, what was expected, what came back      |
| Check results    | Every check of the task with its latest result                         |
| Code health      | The level (green, amber, red), what it is made of, what would raise it |
| Judges' verdicts | What the judges said about the last judged task, with the points       |
| Earlier briefs   | The briefs of the tasks already behind you                             |

That short list is the `All` tab. `←`/`→` (or `Tab`) switch to the tabs that
hold one kind in full — **Tasks** (every brief), **Judges** (every verdict,
one by one), **Requests** (closed ones too), **Probes** (each check of the
task) and **Health**; each tab shows how many rows it has.

Rows appear when there is something to hand over — health after the first
check is scored, verdicts once a judge has spoken. The cursor starts on the
most urgent thing your agent does not have yet, so `F3` `⏎` is still the
one-two that sends the next thing to fix. What the agent already has is
marked `✓ pasted`, in the menu and on its bubble in the chat, and nothing is
sent until you choose. Most of the game is F2, F3, watch, repeat.

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
